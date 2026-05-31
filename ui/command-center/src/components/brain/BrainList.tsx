import { useState, useEffect, useRef, useCallback } from 'react';
import { font, ease } from '../../styles/tokens';
import { api } from '../../lib/api';
import type { GraphMemory } from './useBrainData';
import { useTheme } from '../../styles/useTheme';

// ── Title derivation ─────────────────────────────────────────────────

function deriveTitle(mem: GraphMemory): string {
  // Description-first: use first sentence
  if (mem.description) {
    const firstSentence = mem.description.split(/\.\s/)[0];
    if (firstSentence.length > 0 && firstSentence.length <= 80) {
      return firstSentence.endsWith('.') ? firstSentence : firstSentence + '.';
    }
    if (firstSentence.length > 80) {
      return firstSentence.slice(0, 77) + '...';
    }
  }

  // Key fallback: humanize
  if (mem.key) {
    return humanizeKey(mem.key);
  }

  // Last resort: content preview
  return mem.text.slice(0, 60) + (mem.text.length > 60 ? '...' : '');
}

function humanizeKey(key: string): string {
  // Split on : _ -
  const parts = key.split(/[:_\-]+/);
  // Drop hex hashes (>6 hex chars), timestamps (all-digit >8 chars)
  const clean = parts.filter(p => {
    if (p.length > 8 && /^\d+$/.test(p)) return false;
    if (p.length > 6 && /^[0-9a-f]+$/i.test(p)) return false;
    return p.length > 0;
  });
  if (clean.length === 0) return key.slice(0, 40);
  // Title case
  return clean.map(w => w.charAt(0).toUpperCase() + w.slice(1).toLowerCase()).join(' ');
}

// ── Date formatting ──────────────────────────────────────────────────

function formatDate(ts: string): string {
  const d = new Date(ts);
  if (isNaN(d.getTime())) return ts;
  const now = new Date();
  const diff = now.getTime() - d.getTime();
  const mins = Math.floor(diff / 60000);
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  const days = Math.floor(hrs / 24);
  if (days < 7) return `${days}d ago`;
  return d.toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
}

// ── Component ────────────────────────────────────────────────────────

interface BrainListProps {
  onSelect: (info: { id: string; kind: string; label: string; note: string; data: GraphMemory }) => void;
  selectedId: string | null;
  timeValue: number;
  searchQuery?: string;
}

interface PageState {
  memories: GraphMemory[];
  total: number;
  hasMore: boolean;
  loading: boolean;
  // Browse cursor
  lastTimestamp: string | null;
  lastId: string | null;
  // Search offset
  searchOffset: number;
}

export function BrainList({ onSelect, selectedId, timeValue, searchQuery }: BrainListProps) {
  const { colors } = useTheme();
  const [state, setState] = useState<PageState>({
    memories: [], total: 0, hasMore: false, loading: true,
    lastTimestamp: null, lastId: null, searchOffset: 0,
  });
  const scrollRef = useRef<HTMLDivElement>(null);
  const loadingMore = useRef(false);
  const isSearch = !!(searchQuery && searchQuery.trim());

  // Reset and load first page when query or time changes
  useEffect(() => {
    setState({ memories: [], total: 0, hasMore: false, loading: true, lastTimestamp: null, lastId: null, searchOffset: 0 });
    loadPage(true);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchQuery, timeValue]);

  const loadPage = useCallback(async (reset = false) => {
    if (loadingMore.current && !reset) return;
    loadingMore.current = true;

    try {
      const currentState = reset ? {
        memories: [] as GraphMemory[], total: 0, hasMore: false, loading: true,
        lastTimestamp: null as string | null, lastId: null as string | null, searchOffset: 0,
      } : state;

      const params: Parameters<typeof api.getBrainMemories>[0] = { limit: 50 };

      if (isSearch) {
        params.q = searchQuery!.trim();
        params.offset = reset ? 0 : currentState.searchOffset;
      } else {
        if (!reset && currentState.lastTimestamp) {
          params.before = currentState.lastTimestamp;
          if (currentState.lastId) params.before_id = currentState.lastId;
        }
        // Time slider → server-side after filter (0 = today, 1 = all time)
        if (timeValue < 1.0) {
          const maxAgeDays = 90;
          const cutoffMs = Date.now() - timeValue * maxAgeDays * 24 * 60 * 60 * 1000;
          params.after = new Date(cutoffMs).toISOString();
        }
      }

      const res = await api.getBrainMemories(params);

      setState(prev => {
        const combined = reset ? res.memories : [...prev.memories, ...res.memories];
        const last = combined[combined.length - 1];
        return {
          memories: combined,
          total: res.total,
          hasMore: res.has_more,
          loading: false,
          lastTimestamp: last?.timestamp ?? null,
          lastId: last?.id ?? null,
          searchOffset: (reset ? 0 : prev.searchOffset) + res.memories.length,
        };
      });
    } catch {
      setState(prev => ({ ...prev, loading: false }));
    } finally {
      loadingMore.current = false;
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state, searchQuery, isSearch, timeValue]);

  // Infinite scroll
  const handleScroll = useCallback(() => {
    const el = scrollRef.current;
    if (!el || !state.hasMore || state.loading) return;
    const threshold = 200;
    if (el.scrollHeight - el.scrollTop - el.clientHeight < threshold) {
      loadPage(false);
    }
  }, [loadPage, state.hasMore, state.loading]);

  const filtered = state.memories;

  return (
    <div style={{
      position: 'absolute', inset: 0, display: 'flex', flexDirection: 'column',
      background: 'transparent',
    }}>
      {/* Stats bar */}
      <div style={{
        padding: '8px 16px', display: 'flex', justifyContent: 'space-between',
        fontFamily: font.mono, fontSize: 10, color: colors.textDim,
      }}>
        <span>{state.total.toLocaleString()} memories{isSearch ? ` matching "${searchQuery}"` : ''}</span>
        <span>{filtered.length} shown</span>
      </div>

      {/* Scrollable list */}
      <div
        ref={scrollRef}
        onScroll={handleScroll}
        style={{
          flex: 1, overflowY: 'auto', padding: '0 12px 12px',
        }}
      >
        {filtered.map(mem => (
          <MemoryRow
            key={mem.id}
            memory={mem}
            selected={mem.id === selectedId}
            highlightTerms={isSearch ? searchQuery!.trim().toLowerCase().split(/\s+/) : []}
            onClick={() => onSelect({
              id: mem.id,
              kind: 'memory',
              label: deriveTitle(mem),
              note: mem.text.slice(0, 120),
              data: mem,
            })}
          />
        ))}

        {state.loading && (
          <div style={{ padding: 20, textAlign: 'center', fontFamily: font.mono, fontSize: 11, color: colors.textDim }}>
            Loading...
          </div>
        )}

        {!state.loading && filtered.length === 0 && (
          <div style={{ padding: 40, textAlign: 'center', fontFamily: font.body, fontSize: 13, color: colors.textMuted }}>
            {isSearch ? 'No memories match your search.' : 'No memories yet.'}
          </div>
        )}

        {state.hasMore && !state.loading && (
          <div style={{ padding: 12, textAlign: 'center', fontFamily: font.mono, fontSize: 10, color: colors.textDim }}>
            Scroll for more...
          </div>
        )}
      </div>
    </div>
  );
}

// ── Highlight helper ─────────────────────────────────────────────────

function highlightText(text: string, terms: string[], colors: { text: string }): React.ReactNode {
  if (terms.length === 0) return text;
  const escaped = terms.filter(t => t.length > 1).map(t => t.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
  if (escaped.length === 0) return text;
  const regex = new RegExp(`(${escaped.join('|')})`, 'gi');
  const parts = text.split(regex);
  return parts.map((part, i) =>
    regex.test(part)
      ? <mark key={i} style={{ background: 'rgba(0,213,255,0.25)', color: colors.text, borderRadius: 2, padding: '0 1px' }}>{part}</mark>
      : part
  );
}

// ── Row component ────────────────────────────────────────────────────

function MemoryRow({ memory, selected, highlightTerms, onClick }: {
  memory: GraphMemory;
  selected: boolean;
  highlightTerms: string[];
  onClick: () => void;
}) {
  const { colors } = useTheme();
  const title = deriveTitle(memory);
  const preview = memory.text.slice(0, 100) + (memory.text.length > 100 ? '...' : '');
  const descPreview = memory.description
    ? memory.description.slice(0, 120) + (memory.description.length > 120 ? '...' : '')
    : null;

  return (
    <div
      onClick={onClick}
      style={{
        padding: '10px 14px', marginBottom: 2, cursor: 'pointer',
        borderRadius: 8,
        background: selected ? 'rgba(0,213,255,0.08)' : 'rgba(20,28,48,0.4)',
        border: selected ? `1px solid rgba(0,213,255,0.25)` : '1px solid transparent',
        transition: `all 160ms ${ease.out}`,
      }}
      onMouseEnter={e => { if (!selected) (e.currentTarget as HTMLDivElement).style.background = 'rgba(20,28,48,0.65)'; }}
      onMouseLeave={e => { if (!selected) (e.currentTarget as HTMLDivElement).style.background = 'rgba(20,28,48,0.4)'; }}
    >
      {/* Title row */}
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline', marginBottom: 4 }}>
        <div style={{
          fontFamily: font.body, fontSize: 13, fontWeight: 600, color: colors.text,
          overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', flex: 1, marginRight: 12,
        }}>
          {highlightText(title, highlightTerms, colors)}
        </div>
        <span style={{ fontFamily: font.mono, fontSize: 10, color: colors.textDim, flexShrink: 0 }}>
          {formatDate(memory.timestamp)}
        </span>
      </div>

      {/* Description preview */}
      {descPreview && (
        <div style={{
          fontFamily: font.body, fontSize: 12, color: colors.textMuted, lineHeight: 1.5,
          marginBottom: 4, overflow: 'hidden', textOverflow: 'ellipsis',
          display: '-webkit-box', WebkitLineClamp: 2, WebkitBoxOrient: 'vertical',
        }}>
          {highlightText(descPreview, highlightTerms, colors)}
        </div>
      )}

      {/* Content preview */}
      <div style={{
        fontFamily: font.mono, fontSize: 10, color: colors.textDim, lineHeight: 1.4,
        overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
      }}>
        {highlightText(preview, highlightTerms, colors)}
      </div>

      {/* Metadata footer */}
      <div style={{
        display: 'flex', gap: 16, marginTop: 6,
        fontFamily: font.mono, fontSize: 9, color: colors.textDim,
      }}>
        <span>signal {Math.round(memory.weight * 100)}%</span>
        <span>{memory.age < 0.02 ? 'today' : memory.age < 0.11 ? 'this week' : memory.age < 0.33 ? 'this month' : memory.age < 0.67 ? '~3 months' : 'older'}</span>
      </div>
    </div>
  );
}
