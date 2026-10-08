import { useState, useEffect, useRef, useCallback, useMemo, type CSSProperties } from 'react';
import { concentric, duration, ease, font, radius, space, textSize } from '../../styles/tokens';
import { api } from '../../lib/api';
import { Button } from '../common/Button';
import type { GraphMemory, GraphEntity } from './useBrainData';
import type { TypeFilters } from './BrainScene';
import { useTheme } from '../../styles/useTheme';
// Title derivation is shared with the cross-surface "View in Brain" focus seam
// (brainMemoryFocus) so a memory reads the same in the list and when deep-linked.
import { deriveMemoryTitle, MEMORY_STALE_AFTER_DAYS } from './brainMemoryFocus';
import { AsOf } from '../common/AsOf';
import { MEMORY_STRENGTH } from '../../lib/vocabulary';
import { Chip } from '../common/Chip';
import { humanizeMemorySource, isForgettableMemoryKey, SOURCE_FILTERS, UNKNOWN_SOURCE } from './brainMemorySource';
import { scheduleForget, type PendingForget } from './brainForget';
import { toast } from '../../lib/notifications';

import { Tooltip } from '../common/Tooltip';
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
  /** The graph's memory sample — used to count each entity's linked memories,
   *  the same count the side panel's MEMORIES stat shows. */
  graphMemories?: GraphMemory[];
  filters?: TypeFilters;
}

/** Per-entity row facts: linked memories in the graph sample, and the most
 *  recent field write (the only per-entity timestamp the graph API carries). */
export function entityRowStats(entities: GraphEntity[], memories: GraphMemory[]): Map<string, { memories: number; updatedAt: string | null }> {
  const counts = new Map<string, number>();
  for (const m of memories) {
    for (const id of m.ent) counts.set(id, (counts.get(id) ?? 0) + 1);
  }
  const out = new Map<string, { memories: number; updatedAt: string | null }>();
  for (const e of entities) {
    let updatedAt: string | null = null;
    for (const f of e.fields ?? []) {
      if (f.updated_at && (!updatedAt || f.updated_at > updatedAt)) updatedAt = f.updated_at;
    }
    out.set(e.id, { memories: counts.get(e.id) ?? 0, updatedAt });
  }
  return out;
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
  entities = [], graphMemories = [], filters,
}: BrainListProps) {
  const { colors, reduceMotion } = useTheme();
  // Hard scroll edge (D11/§1.8): the stats bar is pinned above the scrolling
  // rows. An opaque hairline appears under it only once something has
  // actually scrolled beneath — never decorative, off at rest.
  const [listScrolled, setListScrolled] = useState(false);

  // Filter entities by type
  const filteredEntities = entities.filter(ent => {
    if (!filters) return true;
    const key = ent.type as keyof TypeFilters;
    return key in filters ? filters[key] : true;
  });
  const showEntities = !filters || filteredEntities.length > 0;
  const rowStats = useMemo(() => entityRowStats(entities, graphMemories), [entities, graphMemories]);
  const showMemories = !filters || filters.memory;
  const [state, setState] = useState<PageState>({
    memories: [], total: 0, hasMore: false, loading: true, error: false,
    lastTimestamp: null, lastId: null, searchOffset: 0,
  });
  const scrollRef = useRef<HTMLDivElement>(null);
  const loadingMore = useRef(false);
  const requestGeneration = useRef(0);
  const isSearch = !!(searchQuery && searchQuery.trim());

  // The List's "sources" filter row (R5, UX_BENCHMARK_RESEARCH §5). `null` =
  // all sources. `knownSources` only ever grows — every browse page's rows
  // (filtered or not) contribute the tags actually seen, so the row shows
  // real, present categories rather than the full ~18-tag vocabulary most
  // brains will only ever populate a handful of (NN/g H8, recognition over
  // recall). It can end up offering a tag that yields zero rows under the
  // CURRENT time-slider window; the stats bar's own count says so honestly.
  const [sourceFilter, setSourceFilter] = useState<string | null>(null);
  const [knownSources, setKnownSources] = useState<Set<string>>(new Set());

  // Rows mid-"Forget": hidden immediately (optimistic) regardless of whether
  // they came from browse or search — see `brainForget.ts` for why the actual
  // delete is scheduled outside React state/lifecycle.
  const [hiddenIds, setHiddenIds] = useState<Set<string>>(new Set());
  const forgetCancels = useRef<Map<string, PendingForget>>(new Map());

  // Reset and load first page when query, time or the source filter changes
  // (browse mode only).
  useEffect(() => {
    if (isSearch) return;
    setState({ memories: [], total: 0, hasMore: false, loading: true, error: false, lastTimestamp: null, lastId: null, searchOffset: 0 });
    loadPage(true);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchQuery, timeValue, sourceFilter, isSearch]);

  // Resolves `true` only when a page actually landed, so the Retry button that
  // awaits it cannot tick success over the error it is standing next to.
  const loadPage = useCallback(async (reset = false) => {
    if (isSearch) return false;
    if (loadingMore.current && !reset) return false;
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
      if (sourceFilter) params.source = sourceFilter;

      const res = await api.getBrainMemories(params);
      if (generation !== requestGeneration.current) return false;
      if (!res || !Array.isArray(res.memories)) throw new Error('Invalid memories response');

      setKnownSources(prev => {
        const next = new Set(prev);
        for (const m of res.memories) next.add(m.source ?? UNKNOWN_SOURCE);
        return next;
      });

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
      return true;
    } catch {
      if (generation !== requestGeneration.current) return false;
      setState(prev => ({ ...prev, loading: false, error: true }));
      return false;
    } finally {
      if (generation === requestGeneration.current) loadingMore.current = false;
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state, searchQuery, isSearch, timeValue, sourceFilter]);

  // Restore a hidden row — from Undo (cancels the pending delete before it
  // fires) or from a failed delete (the timer already fired; cancelling again
  // is a harmless no-op). Never touches the network itself.
  const restoreRow = useCallback((id: string) => {
    const pending = forgetCancels.current.get(id);
    if (pending && !pending()) return;
    forgetCancels.current.delete(id);
    setHiddenIds(prev => {
      if (!prev.has(id)) return prev;
      const next = new Set(prev);
      next.delete(id);
      return next;
    });
  }, []);

  // Forget (R5 "explicit forget with undo", R4 "reversible actions offer
  // undo instead of a confirm"): hide the row now, offer ~8s to undo via the
  // toast's action slot, and only then let the DELETE fire — see
  // `brainForget.scheduleForget`. Never a ConfirmDialog: this IS the confirm.
  const handleForget = useCallback((mem: GraphMemory) => {
    if (!isForgettableMemoryKey(mem.key)) return;
    const key = mem.key;
    const title = deriveMemoryTitle(mem);
    setHiddenIds(prev => new Set(prev).add(mem.id));
    const cancel = scheduleForget(key, outcome => {
      if (outcome === 'restored') {
        forgetCancels.current.delete(mem.id);
        restoreRow(mem.id);
        toast('Could not forget that memory', `"${title}" is back — try again.`, undefined, {
          tone: 'danger', force: true,
        });
      } else {
        forgetCancels.current.delete(mem.id);
      }
    });
    forgetCancels.current.set(mem.id, cancel);
    toast('Memory forgotten', `"${title}" will be removed from memory.`, undefined, {
      action: { label: 'Undo', onAction: () => restoreRow(mem.id) },
      expiresAt: cancel.expiresAt, onDismiss: cancel.commit,
      force: true,
    });
  }, [restoreRow]);

  // Infinite scroll
  const handleScroll = useCallback(() => {
    const el = scrollRef.current;
    setListScrolled(!!el && el.scrollTop > 2);
    if (isSearch) return;
    if (!el || !state.hasMore || state.loading) return;
    const threshold = 200;
    if (el.scrollHeight - el.scrollTop - el.clientHeight < threshold) {
      loadPage(false);
    }
  }, [loadPage, state.hasMore, state.loading, isSearch]);

  const rawMemories = isSearch ? (searchResults ?? []) : state.memories;
  // Rows mid-"Forget" vanish immediately (R8-adjacent instant feedback) —
  // filtered at render time rather than spliced out of `state`/`searchResults`
  // so Undo (or a failed delete) can bring a row back with no refetch.
  const displayMemories = hiddenIds.size === 0 ? rawMemories : rawMemories.filter(m => !hiddenIds.has(m.id));
  const displayLoading = isSearch ? !!searchLoading : state.loading;
  const displayError = isSearch ? searchError : (state.error ? 'Could not load memories' : null);
  const displayTotal = isSearch ? displayMemories.length : Math.max(0, state.total - hiddenIds.size);

  return (
    <div style={{
      position: 'absolute', inset: 0, display: 'flex', flexDirection: 'column',
      background: 'transparent',
    }}>
      {/* Stats bar — pinned above the scrolling rows; see `listScrolled`. */}
      <div style={{
        padding: `${space.md}px ${space.xxl}px`, display: 'flex', justifyContent: 'space-between',
        fontFamily: font.mono, fontSize: 10, color: colors.textDim,
        borderBottom: `1px solid ${listScrolled ? colors.borderHi : 'transparent'}`,
        transition: reduceMotion ? 'none' : `border-color ${duration.fast}ms ${ease.out}`,
      }}>
        <span>
          {!isSearch && filteredEntities.length > 0 && `${filteredEntities.length} entities`}
          {!isSearch && filteredEntities.length > 0 && showMemories && ' · '}
          {showMemories && `${displayTotal.toLocaleString()} memories`}
          {isSearch ? ` matching "${searchQuery}"` : ''}
        </span>
        <span>{(isSearch ? 0 : filteredEntities.length) + displayMemories.length} shown</span>
      </div>

      {/* "Sources" filter row (R5; UX_BENCHMARK_RESEARCH §5) — only once the
          API has actually returned a source for something to filter by, and
          only in browse mode (a search query is its own, narrower filter). */}
      {!isSearch && knownSources.size > 0 && (
        <div
          data-testid="brain-source-filters"
          style={{
            display: 'flex', flexWrap: 'wrap', gap: space.xs,
            padding: `${space.sm}px ${space.xxl}px`,
            borderBottom: `1px solid ${colors.border}`,
          }}
        >
          <Chip kind="filter" quiet={sourceFilter !== null} pressed={sourceFilter === null} onClick={() => setSourceFilter(null)}>
            all sources
          </Chip>
          {SOURCE_FILTERS.filter(f => knownSources.has(f.tag)).map(f => (
            <Chip
              key={f.tag}
              kind="filter"
              quiet={sourceFilter !== f.tag}
              pressed={sourceFilter === f.tag}
              onClick={() => setSourceFilter(prev => prev === f.tag ? null : f.tag)}
            >
              {f.label}
            </Chip>
          ))}
          {knownSources.has(UNKNOWN_SOURCE) && (
            <Chip
              kind="filter"
              quiet={sourceFilter !== UNKNOWN_SOURCE}
              pressed={sourceFilter === UNKNOWN_SOURCE}
              onClick={() => setSourceFilter(prev => prev === UNKNOWN_SOURCE ? null : UNKNOWN_SOURCE)}
              title="Memories written before source tracking existed"
            >
              source unknown
            </Chip>
          )}
        </div>
      )}

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
              padding: `${space.sm}px 14px ${space.xs}px`, fontFamily: font.mono, fontSize: 10,
              color: colors.textDim, textTransform: 'uppercase', letterSpacing: '0.08em',
            }}>
              Entities ({filteredEntities.length})
            </div>
            {filteredEntities.map(ent => (
              <EntityRow
                key={ent.id}
                entity={ent}
                stats={rowStats.get(ent.id)}
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
                padding: `${space.lg}px 14px ${space.xs}px`, fontFamily: font.mono, fontSize: 10,
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
            onForget={handleForget}
          />
        ))}

        {showMemories && displayLoading && (
          <div style={{ padding: space.xxxl, textAlign: 'center', fontFamily: font.mono, fontSize: textSize.micro, color: colors.textDim }}>
            Loading...
          </div>
        )}

        {showMemories && !displayLoading && displayError && displayMemories.length === 0 && (
          <div style={{ padding: 40, textAlign: 'center', fontFamily: font.body, fontSize: textSize.small }}>
            <div style={{ color: colors.textMuted, marginBottom: space.lg }}>
              {isSearch ? `Could not search your Brain: ${displayError}` : "Couldn't load memories."}
            </div>
            {!isSearch && (
              <Button
                colors={colors}
                variant="ghostOn"
                type="button"
                onClick={() => loadPage(true)}
                style={{
                  '--pa-btn-bg': 'transparent',
                  '--pa-btn-fg': colors.cyan,
                  '--pa-btn-border': colors.borderHi,
                  '--pa-btn-bg-hover': colors.cyanSoft,
                  '--pa-btn-border-hover': colors.cyan,
                  '--pa-btn-pad': '5px 14px',
                  '--pa-btn-radius': `${radius.md}px`,
                  '--pa-btn-weight': 600,
                  fontSize: textSize.caption, lineHeight: 1.5, fontFamily: font.body,
                } as CSSProperties}
              >
                Retry
              </Button>
            )}
          </div>
        )}

        {showMemories && !displayLoading && !displayError && displayMemories.length === 0 && (
          <div style={{ padding: 40, textAlign: 'center', fontFamily: font.body, fontSize: textSize.small, color: colors.textMuted }}>
            {isSearch ? `No memories match "${searchQuery}" — try a name or project.` : sourceFilter || timeValue < 1 ? 'No memories match these filters.' : 'No memories yet.'}
          </div>
        )}

        {showMemories && !isSearch && state.hasMore && !state.loading && (
          <div style={{ padding: space.xl, textAlign: 'center', fontFamily: font.mono, fontSize: 10, color: colors.textDim }}>
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
      ? <mark key={i} style={{ background: colors.cyanGlow, color: colors.text, borderRadius: concentric(radius.xs, 2), padding: '0 1px' }}>{part}</mark>
      : part
  );
}

// ── Row component ────────────────────────────────────────────────────

function MemoryRow({ memory, selected, highlightTerms, onClick, onForget }: {
  memory: GraphMemory;
  selected: boolean;
  highlightTerms: string[];
  onClick: () => void;
  onForget: (memory: GraphMemory) => void;
}) {
  const { colors, reduceMotion } = useTheme();
  // Rows are content (D1): a solid theme fill, never a translucent wash
  // pretending to be elevation. Resting/hover are both fully opaque —
  // `surface` then `surfaceHi`, the app's own "one step up" pair — and
  // selection is the same accent tint every chip in this app uses.
  const rowBg = selected ? colors.cyanSoft : colors.surface;
  const rowHoverBg = colors.surfaceHi;
  const rowTransition = reduceMotion ? 'none' : `background ${duration.fast}ms ${ease.out}, border-color ${duration.fast}ms ${ease.out}, transform ${duration.fast}ms ${ease.out}, filter ${duration.fast}ms ${ease.out}`;
  const title = deriveMemoryTitle(memory);
  const preview = memory.text.slice(0, 100) + (memory.text.length > 100 ? '...' : '');
  const descPreview = memory.description
    ? memory.description.slice(0, 120) + (memory.description.length > 120 ? '...' : '')
    : null;
  const sourceInfo = humanizeMemorySource(memory.source);
  const forgettable = isForgettableMemoryKey(memory.key);

  return (
    <div
      role="button"
      tabIndex={0}
      aria-label={`Memory: ${title}`}
      onClick={onClick}
      onKeyDown={e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); onClick(); } }}
      style={{
        padding: `${space.lg}px 14px`, marginBottom: space.xxs, cursor: 'pointer',
        borderRadius: radius.md,
        background: rowBg,
        border: selected ? `1px solid ${colors.borderHi}` : '1px solid transparent',
        outline: 'none',
        transition: rowTransition,
      }}
      onMouseEnter={e => { if (!selected) (e.currentTarget as HTMLDivElement).style.background = rowHoverBg; }}
      onMouseLeave={e => {
        const el = e.currentTarget as HTMLDivElement;
        el.style.background = rowBg; el.style.filter = 'none'; el.style.transform = 'none';
      }}
      onPointerDown={e => { (e.currentTarget as HTMLDivElement).style.filter = 'brightness(0.95)'; (e.currentTarget as HTMLDivElement).style.transform = 'scale(0.995)'; }}
      onPointerUp={e => { (e.currentTarget as HTMLDivElement).style.filter = 'none'; (e.currentTarget as HTMLDivElement).style.transform = 'none'; }}
      onFocus={e => { (e.currentTarget as HTMLDivElement).style.outline = `2px solid ${colors.cyan}`; }}
      onBlur={e => { (e.currentTarget as HTMLDivElement).style.outline = 'none'; }}
    >
      {/* Title row */}
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline', marginBottom: space.xs }}>
        <div style={{
          fontFamily: font.body, fontSize: textSize.small, fontWeight: 600, color: colors.text,
          overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', flex: 1, marginRight: space.xl,
        }}>
          {highlightText(title, highlightTerms, colors)}
          {memory.layer && (
            <span style={{
              marginLeft: space.md, fontFamily: font.mono, fontSize: 10, fontWeight: 500,
              color: colors.textDim, textTransform: 'uppercase', letterSpacing: '0.06em',
            }}>{memory.layer}</span>
          )}
        </div>
        <span style={{ fontFamily: font.mono, fontSize: 10, color: colors.textDim, flexShrink: 0 }}>
          {formatDate(memory.timestamp)}
        </span>
      </div>

      {/* Description preview */}
      {descPreview && (
        <div style={{
          fontFamily: font.body, fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.5,
          marginBottom: space.xs, overflow: 'hidden', textOverflow: 'ellipsis',
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
        display: 'flex', alignItems: 'center', gap: space.xxl, marginTop: space.sm,
        fontFamily: font.mono, fontSize: 10, color: colors.textDim,
      }}>
        {/* The same field the graph panel shows. It used to say "signal" here
            and "reinforcement" there — one number, one tab, two words. */}
        <Tooltip content={MEMORY_STRENGTH.gloss}>
          <span tabIndex={0} style={{ outline: 'none' }}>
            <span style={{ cursor: 'help' }}>
              {MEMORY_STRENGTH.one} {Math.round(memory.weight * 100)}%
            </span>
          </span>
        </Tooltip>
        {/* Read from the memory's own timestamp, never from `age`: that is a
            0..1 scene coordinate, clamped at 90 days, so every memory past the
            window shared one bucket and a three-year-old note read as merely
            "older" — on the list whose subject is what was learned when. */}
        <AsOf asOf={memory.timestamp} granularity="calendar" staleAfterMs={MEMORY_STALE_AFTER_DAYS * 86_400_000} />
        {/* Last referenced (R5's "created/last-referenced" pair) — Spectral's
            Memify reinforcement timestamp, the closest durable signal it
            carries; never marked stale, since "not yet referenced again" is
            the normal state for most memories, not a problem. */}
        <AsOf asOf={memory.last_reinforced_at} prefix="referenced" unknownLabel="not yet referenced" />
        {/* Source (R5 "a memory shows its source") — humanized, never the raw
            RememberOpts tag; "quiet" per Chip's own density rule, since this
            renders on every row in the list. */}
        <Chip kind="static" quiet title={sourceInfo.known ? 'Where this memory came from' : "No source was recorded for this memory."}>
          {sourceInfo.phrase}
        </Chip>
        {forgettable && (
          <Button
            colors={colors}
            variant="ghost"
            type="button"
            onClick={e => { e.stopPropagation(); onForget(memory); }}
            aria-label={`Forget "${title}"`}
            style={{
              marginLeft: 'auto',
              '--pa-btn-fg': colors.danger,
              '--pa-btn-fg-hover': colors.danger,
              '--pa-btn-border': 'transparent',
              '--pa-btn-border-hover': colors.danger,
              '--pa-btn-bg-hover': `${colors.danger}1a`,
              '--pa-btn-pad': '2px 8px',
              '--pa-btn-radius': `${radius.sm}px`,
              fontFamily: font.body, fontSize: textSize.micro,
            } as CSSProperties}
          >
            Forget
          </Button>
        )}
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

function EntityRow({ entity, stats, selected, onClick }: {
  entity: GraphEntity;
  stats?: { memories: number; updatedAt: string | null };
  selected: boolean;
  onClick: () => void;
}) {
  const { colors, reduceMotion } = useTheme();
  // Same opaque-content pair as `MemoryRow` (D1): `surface` resting, the
  // theme's own "one step up" for hover, the shared accent tint for selected.
  const rowBg = selected ? colors.cyanSoft : colors.surface;
  const rowHoverBg = colors.surfaceHi;
  const rowTransition = reduceMotion ? 'none' : `background ${duration.fast}ms ${ease.out}, border-color ${duration.fast}ms ${ease.out}, transform ${duration.fast}ms ${ease.out}, filter ${duration.fast}ms ${ease.out}`;
  const typeColor = typeAccent(entity.type, colors);
  const meta = [
    stats && stats.memories > 0 ? `${stats.memories} ${stats.memories === 1 ? 'memory' : 'memories'}` : null,
    stats?.updatedAt ? `updated ${formatDate(stats.updatedAt)}` : null,
  ].filter(Boolean).join(' · ');

  return (
    <div
      role="button"
      tabIndex={0}
      aria-label={`${entity.type}: ${entity.name}`}
      data-testid="brain-entity-row"
      onClick={onClick}
      onKeyDown={e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); onClick(); } }}
      style={{
        padding: `${space.md}px 14px`, marginBottom: space.xxs, cursor: 'pointer',
        borderRadius: radius.md, background: rowBg,
        border: selected ? `1px solid ${colors.borderHi}` : '1px solid transparent',
        outline: 'none',
        transition: rowTransition,
        // name | kind | description | meta (QA 2026-09-24: the description was
        // a 200px sliver pinned to the far right with ~1,200px of nothing
        // between it and the name). The description column takes the rest.
        display: 'grid',
        gridTemplateColumns: 'minmax(140px, 22%) 112px minmax(0, 1fr) auto',
        alignItems: 'center', columnGap: space.xl,
      }}
      onMouseEnter={e => { if (!selected) (e.currentTarget as HTMLDivElement).style.background = rowHoverBg; }}
      onMouseLeave={e => {
        const el = e.currentTarget as HTMLDivElement;
        el.style.background = rowBg; el.style.filter = 'none'; el.style.transform = 'none';
      }}
      onPointerDown={e => { (e.currentTarget as HTMLDivElement).style.filter = 'brightness(0.95)'; (e.currentTarget as HTMLDivElement).style.transform = 'scale(0.995)'; }}
      onPointerUp={e => { (e.currentTarget as HTMLDivElement).style.filter = 'none'; (e.currentTarget as HTMLDivElement).style.transform = 'none'; }}
      onFocus={e => { (e.currentTarget as HTMLDivElement).style.outline = `2px solid ${colors.cyan}`; }}
      onBlur={e => { (e.currentTarget as HTMLDivElement).style.outline = 'none'; }}
    >
      <span data-testid="brain-entity-name" style={{
        fontFamily: font.body, fontSize: textSize.small, fontWeight: 600, color: colors.text,
        overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', minWidth: 0,
      }}>
        {entity.name}
      </span>
      <span data-testid="brain-entity-kind" style={{ display: 'inline-flex', alignItems: 'center', gap: space.sm, minWidth: 0 }}>
        <span aria-hidden style={{
          width: 6, height: 6, borderRadius: radius.pill, background: typeColor, flexShrink: 0,
        }} />
        <span style={{
          fontFamily: font.mono, fontSize: 10, fontWeight: 600,
          color: colors.textMuted, textTransform: 'uppercase', letterSpacing: '0.06em',
          overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
        }}>
          {entity.type}
        </span>
      </span>
      <span data-testid="brain-entity-description" style={{
        fontFamily: font.body, fontSize: textSize.caption, lineHeight: 1.45,
        color: entity.note ? colors.textMuted : colors.textDim,
        overflow: 'hidden', display: '-webkit-box', WebkitLineClamp: 2, WebkitBoxOrient: 'vertical',
        minWidth: 0,
      }}>
        {entity.note || 'No description yet'}
      </span>
      <span style={{
        fontFamily: font.mono, fontSize: textSize.micro, color: colors.textDim, whiteSpace: 'nowrap', textAlign: 'right',
      }}>
        {meta}
      </span>
    </div>
  );
}
