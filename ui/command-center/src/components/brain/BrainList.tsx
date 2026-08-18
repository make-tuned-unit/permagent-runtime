import { useState, useEffect, useRef, useCallback } from 'react';
import { font, ease } from '../../styles/tokens';
import { api } from '../../lib/api';
import type { GraphMemory, GraphEntity } from './useBrainData';
import type { TypeFilters } from './BrainScene';
import { useTheme } from '../../styles/useTheme';
// Title derivation is shared with the cross-surface "View in Brain" focus seam
// (brainMemoryFocus) so a memory reads the same in the list and when deep-linked.
import { deriveMemoryTitle } from './brainMemoryFocus';

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
  onSelect: (info: { id: string; kind: string; label: string; note: string; data: GraphMemory | GraphEntity }) => void;
  selectedId: string | null;
  timeValue: number;
  searchQuery?: string;
  /** Ranked hits from GET /api/brain/search — when set, browse pagination is bypassed. */
  searchResults?: GraphMemory[] | null;
  searchLoading?: boolean;
  searchError?: string | null;
  entities?: GraphEntity[];
  filters?: TypeFilters;
}

interface PageState {
  memories: GraphMemory[];
  total: number;
  hasMore: boolean;
  loading: boolean;
  /** Last page fetch failed — render as an error, never as "No memories yet"
   *  (2026-07 wiring audit: a dead daemon looked identical to an empty brain). */
  error: boolean;
  // Browse cursor
  lastTimestamp: string | null;
  lastId: string | null;
  // Search offset
  searchOffset: number;
}

export function BrainList({
  onSelect, selectedId, timeValue, searchQuery, searchResults, searchLoading, searchError,
  entities = [], filters,
}: BrainListProps) {
  const { colors } = useTheme();

  // Filter entities by type
  const filteredEntities = entities.filter(ent => {
    if (!filters) return true;
    const key = ent.type as keyof TypeFilters;
    return key in filters ? filters[key] : true;
  });
  const showEntities = !filters || filteredEntities.length > 0;
  const showMemories = !filters || filters.memory;
  const [state, setState] = useState<PageState>({
    memories: [], total: 0, hasMore: false, loading: true, error: false,
    lastTimestamp: null, lastId: null, searchOffset: 0,
  });
  const scrollRef = useRef<HTMLDivElement>(null);
  const loadingMore = useRef(false);
  const requestGeneration = useRef(0);
  const isSearch = !!(searchQuery && searchQuery.trim());

  // Reset and load first page when query or time changes (browse mode only)
  useEffect(() => {
    if (isSearch) return;
    setState({ memories: [], total: 0, hasMore: false, loading: true, error: false, lastTimestamp: null, lastId: null, searchOffset: 0 });
    loadPage(true);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchQuery, timeValue, isSearch]);

  const loadPage = useCallback(async (reset = false) => {
    if (isSearch) return;
    if (loadingMore.current && !reset) return;
    loadingMore.current = true;
    const generation = reset ? ++requestGeneration.current : requestGeneration.current;

    try {
      const currentState = reset ? {
        memories: [] as GraphMemory[], total: 0, hasMore: false, loading: true, error: false,
        lastTimestamp: null as string | null, lastId: null as string | null, searchOffset: 0,
      } : state;

      const params: Parameters<typeof api.getBrainMemories>[0] = { limit: 50 };

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

      const res = await api.getBrainMemories(params);
      if (generation !== requestGeneration.current) return;
      if (!res || !Array.isArray(res.memories)) throw new Error('Invalid memories response');

      setState(prev => {
        const combined = reset ? res.memories : [...prev.memories, ...res.memories];
        const last = combined[combined.length - 1];
        return {
          memories: combined,
          total: res.total,
          hasMore: res.has_more,
          loading: false,
          error: false,
          lastTimestamp: last?.timestamp ?? null,
          lastId: last?.id ?? null,
          searchOffset: (reset ? 0 : prev.searchOffset) + res.memories.length,
        };
      });
    } catch {
      if (generation !== requestGeneration.current) return;
      setState(prev => ({ ...prev, loading: false, error: true }));
    } finally {
      if (generation === requestGeneration.current) loadingMore.current = false;
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state, searchQuery, isSearch, timeValue]);

  // Infinite scroll
  const handleScroll = useCallback(() => {
    if (isSearch) return;
    const el = scrollRef.current;
    if (!el || !state.hasMore || state.loading) return;
    const threshold = 200;
    if (el.scrollHeight - el.scrollTop - el.clientHeight < threshold) {
      loadPage(false);
    }
  }, [loadPage, state.hasMore, state.loading, isSearch]);

  const displayMemories = isSearch ? (searchResults ?? []) : state.memories;
  const displayLoading = isSearch ? !!searchLoading : state.loading;
  const displayError = isSearch ? searchError : (state.error ? 'Could not load memories' : null);
  const displayTotal = isSearch ? displayMemories.length : state.total;

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
        <span>
          {!isSearch && filteredEntities.length > 0 && `${filteredEntities.length} entities`}
          {!isSearch && filteredEntities.length > 0 && showMemories && ' · '}
          {showMemories && `${displayTotal.toLocaleString()} memories`}
          {isSearch ? ` matching "${searchQuery}"` : ''}
        </span>
        <span>{(isSearch ? 0 : filteredEntities.length) + displayMemories.length} shown</span>
      </div>

      {/* Scrollable list */}
      <div
        ref={scrollRef}
        onScroll={handleScroll}
        style={{
          flex: 1, overflowY: 'auto', padding: '0 12px 12px',
        }}
      >
        {/* Entity rows */}
        {showEntities && !isSearch && filteredEntities.length > 0 && (
          <>
            <div style={{
              padding: '6px 14px 4px', fontFamily: font.mono, fontSize: 10,
              color: colors.textDim, textTransform: 'uppercase', letterSpacing: '0.08em',
            }}>
              Entities ({filteredEntities.length})
            </div>
            {filteredEntities.map(ent => (
              <EntityRow
                key={ent.id}
                entity={ent}
                selected={ent.id === selectedId}
                onClick={() => onSelect({
                  id: ent.id,
                  kind: ent.type,
                  label: ent.name,
                  note: ent.note,
                  data: ent,
                })}
              />
            ))}
            {showMemories && (
              <div style={{
                padding: '10px 14px 4px', fontFamily: font.mono, fontSize: 10,
                color: colors.textDim, textTransform: 'uppercase', letterSpacing: '0.08em',
              }}>
                Memories
              </div>
            )}
          </>
        )}

        {showMemories && displayMemories.map(mem => (
          <MemoryRow
            key={mem.id}
            memory={mem}
            selected={mem.id === selectedId}
            highlightTerms={isSearch ? searchQuery!.trim().toLowerCase().split(/\s+/) : []}
            onClick={() => onSelect({
              id: mem.id,
              kind: 'memory',
              label: deriveMemoryTitle(mem),
              note: mem.text.slice(0, 120),
              data: mem,
            })}
          />
        ))}

        {showMemories && displayLoading && (
          <div style={{ padding: 20, textAlign: 'center', fontFamily: font.mono, fontSize: 11, color: colors.textDim }}>
            Loading...
          </div>
        )}

        {showMemories && !displayLoading && displayError && displayMemories.length === 0 && (
          <div style={{ padding: 40, textAlign: 'center', fontFamily: font.body, fontSize: 13 }}>
            <div style={{ color: colors.textMuted, marginBottom: 10 }}>
              {isSearch ? `Could not search your Brain: ${displayError}` : "Couldn't load memories."}
            </div>
            {!isSearch && (
              <button
                onClick={() => loadPage(true)}
                style={{
                  fontSize: 12, fontFamily: font.body, fontWeight: 600, color: colors.cyan,
                  background: 'none', border: `1px solid ${colors.borderHi}`, borderRadius: 8,
                  padding: '5px 14px', cursor: 'pointer',
                }}
              >
                Retry
              </button>
            )}
          </div>
        )}

        {showMemories && !displayLoading && !displayError && displayMemories.length === 0 && (
          <div style={{ padding: 40, textAlign: 'center', fontFamily: font.body, fontSize: 13, color: colors.textMuted }}>
            {isSearch ? `No memories match "${searchQuery}"` : 'No memories yet.'}
          </div>
        )}

        {showMemories && !isSearch && state.hasMore && !state.loading && (
          <div style={{ padding: 12, textAlign: 'center', fontFamily: font.mono, fontSize: 10, color: colors.textDim }}>
            Scroll for more...
          </div>
        )}
      </div>
    </div>
  );
}

// ── Highlight helper ─────────────────────────────────────────────────

function highlightText(text: string, terms: string[], colors: { text: string; cyanGlow: string }): React.ReactNode {
  if (terms.length === 0) return text;
  const escaped = terms.filter(t => t.length > 1).map(t => t.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
  if (escaped.length === 0) return text;
  const regex = new RegExp(`(${escaped.join('|')})`, 'gi');
  const parts = text.split(regex);
  return parts.map((part, i) =>
    regex.test(part)
      ? <mark key={i} style={{ background: colors.cyanGlow, color: colors.text, borderRadius: 2, padding: '0 1px' }}>{part}</mark>
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
  const { colors, theme } = useTheme();
  const rowBg = selected
    ? colors.cyanSoft
    : theme === 'silver' ? 'rgba(255,255,255,0.7)' : 'rgba(20,28,48,0.4)';
  const rowHoverBg = theme === 'silver' ? 'rgba(255,255,255,0.9)' : 'rgba(20,28,48,0.65)';
  const title = deriveMemoryTitle(memory);
  const preview = memory.text.slice(0, 100) + (memory.text.length > 100 ? '...' : '');
  const descPreview = memory.description
    ? memory.description.slice(0, 120) + (memory.description.length > 120 ? '...' : '')
    : null;

  return (
    <div
      role="button"
      tabIndex={0}
      aria-label={`Memory: ${title}`}
      onClick={onClick}
      onKeyDown={e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); onClick(); } }}
      style={{
        padding: '10px 14px', marginBottom: 2, cursor: 'pointer',
        borderRadius: 8,
        background: rowBg,
        border: selected ? `1px solid ${colors.cyan}40` : '1px solid transparent',
        outline: 'none',
        transition: `all 160ms ${ease.out}`,
      }}
      onMouseEnter={e => { if (!selected) (e.currentTarget as HTMLDivElement).style.background = rowHoverBg; }}
      onMouseLeave={e => { if (!selected) (e.currentTarget as HTMLDivElement).style.background = rowBg; }}
      onFocus={e => { (e.currentTarget as HTMLDivElement).style.outline = `2px solid ${colors.cyan}`; }}
      onBlur={e => { (e.currentTarget as HTMLDivElement).style.outline = 'none'; }}
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
        fontFamily: font.mono, fontSize: 10, color: colors.textDim,
      }}>
        <span>signal {Math.round(memory.weight * 100)}%</span>
        <span>{memory.age < 0.02 ? 'today' : memory.age < 0.11 ? 'this week' : memory.age < 0.33 ? 'this month' : memory.age < 0.67 ? '~3 months' : 'older'}</span>
      </div>
    </div>
  );
}

// ── Entity row component ────────────────────────────────────────────

/**
 * Per-type accent, resolved from theme tokens so it stays legible on both the
 * dark and silver themes (the old hardcoded palette was tuned for dark and went
 * low-contrast/washed on white). The accent drives a small color dot — a
 * non-text category cue — while the type label itself uses `textMuted`, which is
 * AA on both themes.
 */
function typeAccent(type: string, colors: ReturnType<typeof useTheme>['colors']): string {
  switch (type) {
    case 'person': return colors.cyan;
    case 'project': return colors.purple;
    case 'tool': return colors.purpleBright;
    case 'location': return colors.success;
    case 'organization': return colors.warning;
    case 'concept': return colors.textMuted;
    default: return colors.textDim;
  }
}

function EntityRow({ entity, selected, onClick }: {
  entity: GraphEntity;
  selected: boolean;
  onClick: () => void;
}) {
  const { colors, theme } = useTheme();
  const rowBg = selected
    ? colors.cyanSoft
    : theme === 'silver' ? 'rgba(255,255,255,0.7)' : 'rgba(20,28,48,0.4)';
  const rowHoverBg = theme === 'silver' ? 'rgba(255,255,255,0.9)' : 'rgba(20,28,48,0.65)';
  const typeColor = typeAccent(entity.type, colors);

  return (
    <div
      role="button"
      tabIndex={0}
      aria-label={`${entity.type}: ${entity.name}`}
      onClick={onClick}
      onKeyDown={e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); onClick(); } }}
      style={{
        padding: '8px 14px', marginBottom: 2, cursor: 'pointer',
        borderRadius: 8, background: rowBg,
        border: selected ? `1px solid ${colors.cyan}40` : '1px solid transparent',
        outline: 'none',
        transition: `all 160ms ${ease.out}`,
        display: 'flex', alignItems: 'center', gap: 10,
      }}
      onMouseEnter={e => { if (!selected) (e.currentTarget as HTMLDivElement).style.background = rowHoverBg; }}
      onMouseLeave={e => { if (!selected) (e.currentTarget as HTMLDivElement).style.background = rowBg; }}
      onFocus={e => { (e.currentTarget as HTMLDivElement).style.outline = `2px solid ${colors.cyan}`; }}
      onBlur={e => { (e.currentTarget as HTMLDivElement).style.outline = 'none'; }}
    >
      <span style={{
        display: 'inline-flex', alignItems: 'center', gap: 6,
        flexShrink: 0, width: 62, justifyContent: 'flex-end',
      }}>
        <span aria-hidden style={{
          width: 6, height: 6, borderRadius: 999, background: typeColor, flexShrink: 0,
        }} />
        <span style={{
          fontFamily: font.mono, fontSize: 10, fontWeight: 600,
          color: colors.textMuted, textTransform: 'uppercase', letterSpacing: '0.06em',
        }}>
          {entity.type}
        </span>
      </span>
      <span style={{
        fontFamily: font.body, fontSize: 13, fontWeight: 600, color: colors.text,
        overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', flex: 1,
      }}>
        {entity.name}
      </span>
      {entity.note && (
        <span style={{
          fontFamily: font.body, fontSize: 11, color: colors.textMuted,
          overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
          maxWidth: 200, flexShrink: 1,
        }}>
          {entity.note}
        </span>
      )}
    </div>
  );
}
