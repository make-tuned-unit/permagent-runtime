import { useEffect, useRef, useState, useCallback, useMemo, type CSSProperties } from 'react';
import { ease, font, radius, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { apiFetch } from '../../lib/api';
import { useCommandCenter, navigateToTool } from '../../lib/store';
import { Button } from '../common/Button';
import { Mobius } from '../mobius/Mobius';
import { BrainScene, type TypeFilters } from './BrainScene';
import { useBrainData, type GraphMemory, type GraphEntity } from './useBrainData';
import { BrainList } from './BrainList';
import { resolveFocusedMemory, deriveMemoryTitle, formatMemoryAge } from './brainMemoryFocus';
import { Chip } from '../common/Chip';
import { CanvasLegend } from '../common/CanvasLegend';
import { BRAIN_GESTURES, BRAIN_VOCABULARY } from './brainLegend';
import { MEMORY_STRENGTH } from '../../lib/vocabulary';
import {
  resolveSearchGraphNode,
  searchResultToGraphMemory,
  useBrainSearch,
  type BrainSearchResult,
} from './brainSearch';
import { readViewMode, rememberViewMode, type BrainViewMode as ViewMode } from './viewMode';

const TOP_FILTERS: { key: keyof TypeFilters; label: string; shape: string }[] = [
  { key: 'person', label: 'people', shape: '●' },
  { key: 'project', label: 'projects', shape: '■' },
];

const TOPIC_SUB_FILTERS: { key: keyof TypeFilters; label: string; shape: string }[] = [
  { key: 'tool', label: 'tools', shape: '■' },
  { key: 'location', label: 'locations', shape: '◇' },
  { key: 'organization', label: 'orgs', shape: '■' },
  { key: 'concept', label: 'concepts', shape: '◆' },
];

const TOPIC_KEYS: (keyof TypeFilters)[] = ['tool', 'location', 'organization', 'concept'];

interface HoverInfo { id: string; kind: string; label: string; note: string; x: number; y: number }
interface SelectedInfo {
  id: string; kind: string; label: string; note: string; data: any;
  /** True when a focus deep-link resolved via the caller's PREVIEW rather than
      the live graph (fresh writes aren't in the graph until the Librarian
      enriches them; preview text may be a truncated content_summary). The side
      panel badges it so a snapshot never masquerades as the whole memory. */
  preview?: boolean;
}

export function BrainView() {
  const { gradient, colors, theme } = useTheme();

  // Glass overlay style: white translucent for silver, dark for dark themes
  const glass = theme === 'silver'
    ? { bg: 'rgba(255,255,255,0.88)', border: 'rgba(167,176,190,0.35)' }
    : { bg: 'rgba(20,28,48,0.75)', border: 'rgba(255,255,255,0.06)' };
  const containerRef = useRef<HTMLDivElement>(null);
  const sceneRef = useRef<BrainScene | null>(null);

  // List by default; Graph is a toggle away and the choice sticks (J12). The
  // graph has no legend and an undiscoverable interaction model, so it was the
  // hardest surface in the app to meet first.
  const [viewMode, setViewMode] = useState<ViewMode>(readViewMode);
  const [search, setSearch] = useState('');
  const [debouncedSearch, setDebouncedSearch] = useState('');
  const [modeBeforeSearch, setModeBeforeSearch] = useState<ViewMode>(readViewMode);

  const { data, loading, error, refresh } = useBrainData();
  const { results: searchResults, loading: searchLoading, error: searchError } = useBrainSearch(debouncedSearch);
  const searchMemories = useMemo(
    () => (searchResults ?? []).map(searchResultToGraphMemory),
    [searchResults],
  );
  const [filters, setFilters] = useState<TypeFilters>({ person: true, project: true, tool: true, location: true, organization: true, concept: true, memory: true });
  const [topicsExpanded, setTopicsExpanded] = useState(false);
  const [timeValue, setTimeValue] = useState(1);
  const [hover, setHover] = useState<HoverInfo | null>(null);
  const [selected, setSelected] = useState<SelectedInfo | null>(null);

  const onHover = useCallback((item: HoverInfo | null) => setHover(item), []);
  const onSelect = useCallback((item: SelectedInfo | null) => setSelected(item), []);

  // Resolve opaque entity IDs → entity records so memory→entity chips can show
  // a human name and reuse the shared selection mechanism (same shape onSelect
  // receives from the list rows and the 3D scene).
  const entityById = useMemo(
    () => new Map((data?.entities ?? []).map(e => [e.id, e])),
    [data],
  );

  // Select an entity by id using the same channel the list/scene use.
  const selectEntity = useCallback((ent: GraphEntity) => {
    setSelected({ id: ent.id, kind: ent.type, label: ent.name, note: ent.note, data: ent });
  }, []);

  // Select a memory through the same channel (mirrors selectEntity + the list
  // row's onSelect shape, so the side panel renders identically). `viaPreview`
  // marks a preview-resolved focus (P4) so the panel can badge it honestly.
  const selectMemory = useCallback((mem: GraphMemory, viaPreview = false) => {
    setSelected({ id: mem.id, kind: 'memory', label: deriveMemoryTitle(mem), note: mem.text.slice(0, 120), data: mem, preview: viaPreview });
  }, []);

  // Brain-loop deep-link (#587-adjacent): focus the memory a product surface
  // asked to surface. Graph-preferred (real recency/chips); the caller's preview
  // is the fallback when the memory isn't in the current graph — fresh,
  // description-less writes are excluded from the graph's default view until the
  // Librarian enriches them, so a just-created note/code memory needs the
  // preview to render at all.
  const pendingBrainMemory = useCommandCenter(s => s.pendingBrainMemory);
  const clearPendingBrainMemory = useCommandCenter(s => s.clearPendingBrainMemory);
  useEffect(() => {
    if (!pendingBrainMemory) return;
    const resolution = resolveFocusedMemory(pendingBrainMemory, data?.memories ?? []);
    if (resolution.kind === 'none') {
      // No graph hit and no preview. If the graph hasn't loaded yet, wait — it
      // may still carry the memory. Once loaded, best-effort seed the search
      // with the key rather than strand the click, then stop retrying.
      if (data) {
        if (pendingBrainMemory.key) setSearch(pendingBrainMemory.key);
        clearPendingBrainMemory();
      }
      return;
    }
    selectMemory(resolution.memory, resolution.kind === 'preview');
    clearPendingBrainMemory();
  }, [pendingBrainMemory, data, selectMemory, clearPendingBrainMemory]);

  // Project entities link out to their real workspace: resolve the graph
  // entity to a project by name/slug, and offer "Open project" instead of a
  // dead-end. No match (or an unreachable projects API) simply shows nothing.
  const setPendingProjectNavigation = useCommandCenter(s => s.setPendingProjectNavigation);
  const [projectMatch, setProjectMatch] = useState<{ id: string; name: string } | null>(null);
  useEffect(() => {
    setProjectMatch(null);
    if (selected?.kind !== 'project' || !selected.label) return;
    let active = true;
    apiFetch<{ id: string; slug: string; name: string }[]>('/api/projects')
      .then(list => {
        if (!active) return;
        const needle = selected.label.trim().toLowerCase();
        const hit = list.find(p => p.name.trim().toLowerCase() === needle || p.slug.toLowerCase() === needle);
        if (hit) setProjectMatch({ id: hit.id, name: hit.name });
      })
      .catch(() => { /* no affordance when projects can't be resolved */ });
    return () => { active = false; };
  }, [selected?.id, selected?.kind, selected?.label]);

  const openProjectWorkspace = useCallback(() => {
    if (!projectMatch) return;
    setPendingProjectNavigation(projectMatch.id);
    navigateToTool('projects');
  }, [projectMatch, setPendingProjectNavigation]);

  const reduceMotion = typeof window !== 'undefined'
    && window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
  const chipTransition = reduceMotion ? 'none' : `all 140ms ${ease.out}`;

  /**
   * The filter bar's chips. Every one of them was `border: 'none'` plus an
   * inline background, which cannot express `:hover` or `:active` — so the
   * whole bar was pressable with no acknowledgement whatsoever. The resting
   * fills and type are exactly the ones that were here; only the states are
   * new. `lit` is the on/off that drives the text colour, `fill` is passed
   * separately because the topics chip has a third, half-on fill.
   */
  const chipVars = (fill: string, lit: boolean, pad: string, r: number, size: number): CSSProperties => ({
    '--pa-btn-bg': fill,
    '--pa-btn-fg': lit ? colors.text : colors.textDim,
    '--pa-btn-fg-hover': colors.text,
    '--pa-btn-bg-hover': lit ? `${colors.cyan}33` : `${colors.cyan}12`,
    '--pa-btn-bg-active': fill,
    '--pa-btn-border': 'transparent',
    '--pa-btn-border-hover': 'transparent',
    '--pa-btn-pad': pad,
    '--pa-btn-radius': `${r}px`,
    '--pa-btn-weight': 500,
    fontFamily: font.body,
    fontSize: size,
  } as CSSProperties);

  // Initialize scene
  useEffect(() => {
    if (!containerRef.current) return;
    const scene = new BrainScene(containerRef.current, { onHover, onSelect });
    sceneRef.current = scene;
    const obs = new ResizeObserver(() => scene.resize());
    obs.observe(containerRef.current);
    return () => { obs.disconnect(); scene.dispose(); sceneRef.current = null; };
  }, [onHover, onSelect]);

  // Feed data to scene
  useEffect(() => {
    if (sceneRef.current && data) sceneRef.current.setData(data);
  }, [data]);

  // Search: debounce 300ms, auto-switch to list mode on non-empty query
  useEffect(() => {
    const t = setTimeout(() => {
      const q = search.trim();
      setDebouncedSearch(q);
      if (q) {
        if (viewMode !== 'list') {
          setModeBeforeSearch(viewMode);
          setViewMode('list');
        }
      } else if (debouncedSearch) {
        // Query cleared — restore previous mode
        setViewMode(modeBeforeSearch);
      }
      // Also update graph-side dimming
      sceneRef.current?.setSearch(search);
    }, 300);
    return () => clearTimeout(t);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [search]);

  // Centre the graph on the top ranked hit once search results land.
  useEffect(() => {
    if (!debouncedSearch.trim()) {
      sceneRef.current?.clearSearchFocus();
      return;
    }
    const top = searchResults?.[0];
    if (!top) return;
    const nodeId = resolveSearchGraphNode(top, data?.memories ?? []);
    sceneRef.current?.focusSearchHit(nodeId, top.preview);
  }, [debouncedSearch, searchResults, data]);

  const openSearchResult = useCallback((result: BrainSearchResult) => {
    const mem = searchResultToGraphMemory(result);
    selectMemory(mem, !resolveSearchGraphNode(result, data?.memories ?? []));
  }, [data, selectMemory]);

  // Filters
  useEffect(() => { sceneRef.current?.setTypeFilter(filters); }, [filters]);

  // Time
  useEffect(() => { sceneRef.current?.setTimeRange([0, timeValue]); }, [timeValue]);

  const isEmpty = !loading && data && data.entities.length === 0 && data.memories.length === 0;
  const wrapperRef = useRef<HTMLDivElement>(null);

  // Suppress WebKit's HTML5 drag indicator — Brain is a read-only view, not a drop target.
  useEffect(() => {
    const el = wrapperRef.current;
    if (!el) return;
    const prevent = (e: Event) => e.preventDefault();
    el.addEventListener('dragover', prevent);
    el.addEventListener('dragenter', prevent);
    return () => {
      el.removeEventListener('dragover', prevent);
      el.removeEventListener('dragenter', prevent);
    };
  }, []);

  const toggleFilter = (key: keyof TypeFilters) =>
    setFilters(f => ({ ...f, [key]: !f[key] }));

  // Recency reads from the memory's own timestamp, not from the scene's
  // clamped 0..1 age — that scalar tops out at 90 days, so it rendered a
  // three-year-old memory and a three-month-old one with the same four words.

  return (
    <div ref={wrapperRef} style={{ position: 'relative', width: '100%', height: '100%', background: gradient.workspace, overflow: 'hidden' }}>
      {/* Three.js canvas container — hidden in list mode */}
      <div ref={containerRef} style={{ position: 'absolute', inset: 0, display: viewMode === 'graph' ? 'block' : 'none' }} />

      {/* List view */}
      {viewMode === 'list' && (
        <div style={{ position: 'absolute', inset: 0, top: 64, bottom: 64 }}>
          <BrainList
            onSelect={onSelect}
            selectedId={selected?.id ?? null}
            timeValue={timeValue}
            searchQuery={debouncedSearch}
            searchResults={searchMemories}
            searchLoading={searchLoading}
            searchError={searchError}
            entities={data?.entities ?? []}
            filters={filters}
          />
        </div>
      )}

      {/* Empty state overlay */}
      {error && !data && (
        <div style={{
          position: 'absolute', inset: 0, display: 'flex', flexDirection: 'column',
          alignItems: 'center', justifyContent: 'center', zIndex: 11, gap: 12,
          background: 'radial-gradient(ellipse 70% 50% at 50% 45%, rgba(0,213,255,0.04) 0%, transparent 70%)',
        }}>
          <div style={{ fontSize: 26, color: colors.textMuted }}>◇</div>
          <h2 style={{ fontFamily: font.display, fontSize: 18, fontWeight: 700, color: colors.text }}>
            Can't reach the Brain
          </h2>
          <p style={{ fontFamily: font.body, fontSize: textSize.small, color: colors.textMuted, maxWidth: 340, textAlign: 'center', lineHeight: 1.5 }}>
            The memory graph didn't load. It may be a brief hiccup — try again.
          </p>
          <Button
            colors={colors}
            variant="ghostOn"
            type="button"
            onClick={() => refresh()}
            style={{
              '--pa-btn-bg': colors.cyanSoft,
              '--pa-btn-fg': colors.cyan,
              '--pa-btn-border': colors.borderHi,
              '--pa-btn-bg-hover': `${colors.cyan}26`,
              '--pa-btn-border-hover': colors.cyan,
              '--pa-btn-bg-active': colors.cyanSoft,
              '--pa-btn-pad': '8px 18px',
              '--pa-btn-radius': `${radius.md}px`,
              '--pa-btn-weight': 600,
              marginTop: 4, fontSize: textSize.small, lineHeight: 1.5,
            } as CSSProperties}
          >Try again</Button>
        </div>
      )}

      {/* Loading state — distinct from empty: we don't yet know if the graph
          has content, so show motion, not the "grows here" empty prompt. */}
      {loading && !data && !error && viewMode === 'graph' && (
        <div style={{
          position: 'absolute', inset: 0, display: 'flex', flexDirection: 'column',
          alignItems: 'center', justifyContent: 'center', zIndex: 10, gap: 16,
          background: 'radial-gradient(ellipse 70% 50% at 50% 45%, rgba(0,213,255,0.04) 0%, transparent 70%)',
        }}>
          <Mobius size={160} state={reduceMotion ? 'idle' : 'thinking'} />
          <p style={{ fontFamily: font.mono, fontSize: textSize.micro, color: colors.textDim, letterSpacing: '0.06em' }}>
            recalling the graph…
          </p>
        </div>
      )}

      {isEmpty && !error && viewMode === 'graph' && (
        <div style={{
          position: 'absolute', inset: 0, display: 'flex', flexDirection: 'column',
          alignItems: 'center', justifyContent: 'center', zIndex: 10,
          background: 'radial-gradient(ellipse 70% 50% at 50% 45%, rgba(0,213,255,0.04) 0%, transparent 70%)',
        }}>
          <Mobius size={200} state="idle" />
          <h2 style={{ fontFamily: font.display, fontSize: 22, fontWeight: 700, color: colors.text, marginTop: 24 }}>
            Your agent's memory grows here.
          </h2>
          <p style={{ fontFamily: font.body, fontSize: textSize.body, color: colors.textMuted, marginTop: 8 }}>
            Begin a conversation.
          </p>
        </div>
      )}

      {/* Header */}
      <div style={{
        position: 'absolute', top: 16, left: 16, right: 16, zIndex: 10,
        display: 'flex', alignItems: 'center', gap: 12,
      }}>
        {/* Mind label */}
        <div style={{
          display: 'flex', alignItems: 'center', gap: 10, padding: '8px 14px',
          background: glass.bg, backdropFilter: 'blur(16px)',
          border: `1px solid ${theme === 'silver' ? colors.cyan + '20' : 'rgba(0,213,255,0.12)'}`, borderRadius: radius.pill,
        }}>
          <Mobius size={28} state="idle" logoMode />
          <span style={{ fontFamily: font.display, fontSize: textSize.small, fontWeight: 700, color: colors.text }}>
            {data?.self.name || 'Agent'}'s mind
          </span>
        </div>

        {/* Search */}
        <div style={{ flex: 1, maxWidth: 360 }}>
          <input
            value={search} onChange={e => setSearch(e.target.value)}
            onKeyDown={e => {
              if (e.key === 'Enter' && debouncedSearch.trim() && searchResults?.[0]) {
                e.preventDefault();
                openSearchResult(searchResults[0]);
              }
            }}
            placeholder="try a name or project…"
            style={{
              width: '100%', fontFamily: font.body, fontSize: textSize.small, color: colors.text,
              background: theme === 'silver' ? 'rgba(255,255,255,0.92)' : 'rgba(20,28,48,0.65)', backdropFilter: 'blur(12px)',
              border: `1px solid ${glass.border}`, borderRadius: radius.pill,
              padding: '9px 16px', outline: 'none',
            }}
          />
        </div>

        {/* Filter chips */}
        <div style={{
          display: 'flex', alignItems: 'center', gap: 6, padding: '6px 10px',
          background: glass.bg, backdropFilter: 'blur(16px)',
          border: `1px solid ${glass.border}`, borderRadius: 10,
        }}>
          <span style={{ fontFamily: font.body, fontSize: 10, color: colors.textDim, marginRight: 4 }}>show</span>
          {TOP_FILTERS.map(f => (
            <Button
              key={f.key}
              colors={colors}
              variant="bare"
              type="button"
              onClick={() => toggleFilter(f.key)}
              style={chipVars(
                filters[f.key] ? colors.cyanSoft : 'transparent',
                filters[f.key], '4px 8px', radius.sm, 11,
              )}
            >
              <span style={{ marginRight: 4 }}>{f.shape}</span>{f.label}
            </Button>
          ))}

          {/* Topics group with drilldown */}
          <span style={{
            display: 'inline-flex', alignItems: 'center', gap: 2,
            borderLeft: `1px solid ${glass.border}`, paddingLeft: 6,
          }}>
            <Button
              colors={colors}
              variant="bare"
              type="button"
              onClick={() => {
                const allOn = TOPIC_KEYS.every(k => filters[k]);
                setFilters(f => {
                  const next = { ...f };
                  for (const k of TOPIC_KEYS) next[k] = !allOn;
                  return next;
                });
              }}
              style={chipVars(
                TOPIC_KEYS.every(k => filters[k])
                  ? colors.cyanSoft
                  : TOPIC_KEYS.some(k => filters[k]) ? `${colors.cyanSoft}88` : 'transparent',
                TOPIC_KEYS.some(k => filters[k]), '4px 8px', radius.sm, 11,
              )}
            >
              <span style={{ marginRight: 4 }}>◆</span>topics
            </Button>
            {/* A disclosure toggle for the sub-filters beside it: nothing to
                await, so the pending floor and the success tick are both wrong
                for it, and it keeps being a plain element so `aria-expanded`
                describes what it does. It takes the shared `.pa-btn` rules —
                which it had none of — but not the primitive. */}
            <button
              type="button"
              className="pa-btn"
              aria-expanded={topicsExpanded}
              aria-label="Topic types"
              onClick={() => setTopicsExpanded(e => !e)}
              style={{
                '--pa-btn-fg': colors.textDim,
                '--pa-btn-fg-hover': colors.text,
                '--pa-btn-bg-hover': `${colors.cyan}12`,
                '--pa-btn-pad': '2px 4px',
                '--pa-btn-radius': `${radius.xs}px`,
                fontFamily: font.mono, fontSize: 10,
                // The rotation is this control's whole read-out, so it stays an
                // inline transform — which does mean `.pa-btn`'s press scale
                // cannot apply to this one button.
                transition: `transform 160ms ${ease.out}`,
                transform: topicsExpanded ? 'rotate(90deg)' : 'rotate(0deg)',
              } as CSSProperties}
            >▸</button>
            {topicsExpanded && TOPIC_SUB_FILTERS.map(f => (
              <Button
                key={f.key}
                colors={colors}
                variant="bare"
                type="button"
                onClick={() => toggleFilter(f.key)}
                style={chipVars(
                  filters[f.key] ? colors.cyanSoft : 'transparent',
                  filters[f.key], '3px 6px', 5, 10,
                )}
              >
                <span style={{ marginRight: 3 }}>{f.shape}</span>{f.label}
              </Button>
            ))}
          </span>

          <span style={{ borderLeft: `1px solid ${glass.border}`, paddingLeft: 6 }}>
            <Button
              colors={colors}
              variant="bare"
              type="button"
              onClick={() => toggleFilter('memory')}
              style={chipVars(
                filters.memory ? colors.cyanSoft : 'transparent',
                filters.memory, '4px 8px', radius.sm, 11,
              )}
            >
              <span style={{ marginRight: 4 }}>·</span>memories
            </Button>
          </span>
        </div>

        {/* Graph / List toggle */}
        <div style={{
          display: 'flex', alignItems: 'center', gap: 2, padding: '4px 6px',
          background: glass.bg, backdropFilter: 'blur(16px)',
          border: `1px solid ${glass.border}`, borderRadius: radius.md,
        }}>
          {(['graph', 'list'] as const).map(mode => (
            <Button
              key={mode}
              colors={colors}
              variant="bare"
              type="button"
              onClick={() => {
                // A deliberate choice, so it is the one that gets remembered —
                // unlike the search auto-switch below, which is the app
                // changing the view on the user's behalf.
                setViewMode(mode);
                setModeBeforeSearch(mode);
                rememberViewMode(mode);
              }}
              style={{
                ...chipVars(
                  viewMode === mode ? colors.cyanSoft : 'transparent',
                  viewMode === mode, '4px 10px', radius.sm, 10,
                ),
                '--pa-btn-weight': 600,
                fontFamily: font.mono,
                textTransform: 'uppercase',
                letterSpacing: '0.05em',
              } as CSSProperties}
            >
              {mode}
            </Button>
          ))}
        </div>
      </div>

      {/* Hover tooltip */}
      {hover && (
        <div style={{
          position: 'fixed', left: hover.x + 14, top: hover.y + 14, zIndex: 20,
          maxWidth: 280, padding: '8px 12px',
          background: theme === 'silver' ? 'rgba(255,255,255,0.95)' : 'rgba(20,28,48,0.9)', backdropFilter: 'blur(12px)',
          border: `1px solid ${colors.borderHi}`, borderRadius: radius.md,
          boxShadow: theme === 'silver' ? '0 2px 12px rgba(30,37,48,0.10)' : 'none',
          fontFamily: font.body, fontSize: textSize.caption, color: colors.text,
          pointerEvents: 'none',
        }}>
          <div style={{ fontWeight: 600 }}>{hover.label}</div>
          {hover.note && hover.note !== hover.label && (
            <div style={{ color: colors.textMuted, marginTop: 2, fontSize: textSize.micro }}>{hover.note.slice(0, 120)}</div>
          )}
        </div>
      )}

      {/* Side panel — glass card with 16px inset */}
      <div style={{
        position: 'absolute', top: 0, right: 0, bottom: 0, width: 360, zIndex: 15,
        padding: 16, pointerEvents: selected ? 'auto' : 'none',
        transform: selected ? 'translateX(0)' : 'translateX(105%)',
        transition: `transform 320ms ${ease.out}`,
      }}>
        <div style={{
          height: '100%', overflow: 'hidden',
          background: theme === 'silver' ? 'rgba(255,255,255,0.92)' : 'rgba(20,28,48,0.78)',
          backdropFilter: 'blur(24px) saturate(140%)',
          WebkitBackdropFilter: 'blur(24px) saturate(140%)',
          border: `1px solid ${theme === 'silver' ? 'rgba(167,176,190,0.35)' : 'rgba(0,213,255,0.16)'}`,
          borderRadius: radius.xl, padding: 24,
          boxShadow: theme === 'silver'
            ? '0 24px 60px rgba(30,37,48,0.12), inset 0 1px 0 rgba(255,255,255,0.8)'
            : '0 24px 60px rgba(0,0,0,0.5), inset 0 1px 0 rgba(255,255,255,0.06)',
          display: 'flex', flexDirection: 'column',
        }}>
          {selected && (<>
            {/* Close */}
            <Button
              colors={colors}
              variant="bare"
              type="button"
              onClick={() => setSelected(null)}
              aria-label="Close details"
              style={{
                '--pa-btn-fg': colors.textMuted,
                '--pa-btn-fg-hover': colors.text,
                '--pa-btn-bg-hover': 'transparent',
                '--pa-btn-bg-active': 'transparent',
                '--pa-btn-pad': '0',
                '--pa-btn-radius': '0',
                position: 'absolute', top: 28, right: 28, zIndex: 1,
                fontSize: 18, lineHeight: 1,
              } as CSSProperties}
            >×</Button>

            {/* Type label (+ P4 honesty badge: a preview-resolved memory is the
                caller's snapshot — possibly a truncated content_summary — not
                the enriched graph copy; styled like the field-provenance chip) */}
            <span style={{ display: 'inline-flex', alignItems: 'center', gap: 8 }}>
              <span style={{
                fontFamily: font.mono, fontSize: 10, fontWeight: 600,
                color: colors.cyan, textTransform: 'uppercase', letterSpacing: '0.1em',
              }}>
                {selected.kind === 'memory' ? 'MEMORY' : (selected.data as GraphEntity)?.type?.toUpperCase() || selected.kind.toUpperCase()}
                {selected.kind === 'memory' && (selected.data as GraphMemory)?.layer && (
                  <span style={{
                    fontFamily: font.mono, fontSize: 10, fontWeight: 500,
                    color: colors.textMuted, letterSpacing: '0.06em',
                    border: `1px solid ${colors.border}`, borderRadius: radius.pill, padding: '1px 7px',
                    textTransform: 'uppercase',
                  }}>{(selected.data as GraphMemory).layer}</span>
                )}
              </span>
              {selected.kind === 'memory' && selected.preview && (
                <span
                  title="Preview from the surface you came from — not yet enriched into the Brain graph; the text may be truncated."
                  style={{
                    fontFamily: font.mono, fontSize: 10, padding: '1px 5px', borderRadius: 3,
                    color: colors.warning, border: `1px solid ${colors.warning}`,
                    textTransform: 'uppercase', letterSpacing: '0.08em',
                  }}
                >preview — not in graph yet</span>
              )}
            </span>

            {/* Name / title */}
            <h3 style={{ fontFamily: font.display, fontSize: textSize.title, fontWeight: 700, color: colors.text, margin: '8px 0 12px' }}>
              {selected.label}
            </h3>

            {/* Entity card: description + typed fields with provenance +
                connection stats — the entity now reads like a memory card. */}
            {selected.kind !== 'memory' && (() => {
              const ent = selected.data as GraphEntity | undefined;
              const fields = ent?.fields ?? [];
              const degree = data?.edges?.filter(e => e.from === selected.id || e.to === selected.id).length ?? 0;
              const memLinks = data?.memories?.filter(m => m.ent.includes(selected.id)).length ?? 0;
              return (<>
                <p style={{ fontFamily: font.body, fontSize: textSize.small, color: colors.textMuted, lineHeight: 1.6, margin: '0 0 16px' }}>
                  {selected.note || 'No description yet — the Librarian writes one on its next run.'}
                </p>

                {fields.length > 0 && (
                  <div style={{ marginBottom: 16, overflowY: 'auto', maxHeight: 200 }}>
                    {fields.map(f => (
                      <div key={f.field_name} style={{ display: 'flex', alignItems: 'baseline', gap: 8, padding: '5px 0', borderBottom: `1px solid ${colors.border}` }}>
                        <span style={{ fontFamily: font.mono, fontSize: 10, color: colors.textDim, textTransform: 'uppercase', letterSpacing: '0.06em', minWidth: 84 }}>
                          {f.field_name.replace(/_/g, ' ')}
                        </span>
                        <span style={{ fontFamily: font.body, fontSize: textSize.caption, color: colors.text, flex: 1, overflowWrap: 'anywhere' }}>
                          {f.source_url ? (
                            <a href={f.source_url} target="_blank" rel="noreferrer" style={{ color: colors.cyan, textDecoration: 'none' }}>{f.value}</a>
                          ) : f.value}
                        </span>
                        <span style={{
                          fontFamily: font.mono, fontSize: 10, padding: '1px 5px', borderRadius: 3,
                          color: f.source === 'manual' ? colors.cyan : colors.textMuted,
                          border: `1px solid ${f.source === 'manual' ? colors.cyan : colors.border}`,
                          textTransform: 'uppercase', letterSpacing: '0.08em', flexShrink: 0,
                        }}>{f.source}</span>
                      </div>
                    ))}
                  </div>
                )}

                {selected.kind === 'project' && projectMatch && (
                  <Button
                    colors={colors}
                    variant="ghostOn"
                    type="button"
                    onClick={openProjectWorkspace}
                    title={`Open ${projectMatch.name} in Projects`}
                    style={{
                      '--pa-btn-bg': colors.cyanSoft,
                      '--pa-btn-fg': colors.cyan,
                      '--pa-btn-border': colors.borderHi,
                      '--pa-btn-bg-hover': colors.cyanGlow,
                      '--pa-btn-border-hover': colors.borderHi,
                      '--pa-btn-bg-active': colors.cyanSoft,
                      '--pa-btn-pad': '6px 12px',
                      '--pa-btn-radius': `${radius.md}px`,
                      '--pa-btn-weight': 600,
                      alignSelf: 'flex-start', marginBottom: 16,
                      fontFamily: font.body, fontSize: textSize.caption, lineHeight: 1.5,
                    } as CSSProperties}
                  >
                    Open project →
                  </Button>
                )}

                <div style={{ display: 'flex', gap: 18, marginTop: 'auto', paddingTop: 12, borderTop: `1px solid ${colors.border}` }}>
                  {[
                    { label: 'CONNECTIONS', value: degree, onClick: undefined as (() => void) | undefined },
                    // Clicking MEMORIES surfaces the memories that mention this
                    // entity — search the entity name, which auto-switches to the
                    // list view (see the search effect above).
                    {
                      label: 'MEMORIES', value: memLinks,
                      onClick: memLinks > 0 && ent?.name ? () => setSearch(ent.name) : undefined,
                    },
                    { label: 'FIELDS', value: fields.length, onClick: undefined },
                  ].map(stat => {
                    const interactive = !!stat.onClick;
                    return (
                      <button
                        key={stat.label}
                        type="button"
                        onClick={stat.onClick}
                        disabled={!interactive}
                        title={interactive ? `Show memories that mention ${selected.label}` : undefined}
                        style={{
                          textAlign: 'left', background: 'transparent', border: 'none', padding: 0,
                          cursor: interactive ? 'pointer' : 'default', borderRadius: radius.sm,
                          transition: chipTransition,
                        }}
                        onMouseEnter={e => { if (interactive) (e.currentTarget as HTMLButtonElement).style.opacity = '0.7'; }}
                        onMouseLeave={e => { (e.currentTarget as HTMLButtonElement).style.opacity = '1'; }}
                        onFocus={e => { if (interactive) (e.currentTarget as HTMLButtonElement).style.outline = `2px solid ${colors.cyan}`; }}
                        onBlur={e => { (e.currentTarget as HTMLButtonElement).style.outline = 'none'; }}
                      >
                        <div style={{ fontFamily: font.display, fontSize: 18, fontWeight: 700, color: interactive ? colors.cyan : colors.text }}>{stat.value}</div>
                        <div style={{ fontFamily: font.mono, fontSize: 10, color: colors.textDim, letterSpacing: '0.08em' }}>{stat.label}</div>
                      </button>
                    );
                  })}
                </div>
              </>);
            })()}

            {/* Memory: description + content + chips + stats */}
            {selected.kind === 'memory' && selected.data && (() => {
              const mem = selected.data as GraphMemory;
              return (
                <div style={{ display: 'flex', flexDirection: 'column', gap: 0, minHeight: 0, flex: 1 }}>
                  {/* Scrollable content area */}
                  <div style={{ flex: 1, overflowY: 'auto', display: 'flex', flexDirection: 'column', gap: 14, marginBottom: 14 }}>
                    {mem.description && (
                      <p style={{ fontFamily: font.body, fontSize: textSize.small, color: colors.text, lineHeight: 1.7, margin: 0 }}>
                        {mem.description}
                      </p>
                    )}
                    <p style={{
                      fontFamily: font.mono, fontSize: textSize.micro, color: colors.textMuted, lineHeight: 1.6, margin: 0,
                      padding: '10px 12px', background: theme === 'silver' ? 'rgba(30,37,48,0.04)' : 'rgba(0,0,0,0.2)', borderRadius: radius.md,
                      whiteSpace: 'pre-wrap', wordBreak: 'break-word',
                    }}>
                      {mem.text}
                    </p>
                    {mem.why && (
                      <details>
                        <summary style={{
                          fontFamily: font.body, fontSize: textSize.micro, color: colors.textDim, cursor: 'pointer',
                        }}>Why this?</summary>
                        <p style={{ fontFamily: font.mono, fontSize: textSize.micro, color: colors.textMuted, margin: '6px 0 0' }}>
                          {mem.why}
                        </p>
                      </details>
                    )}
                    {mem.ent.length > 0 && (
                      <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}>
                        {mem.ent.map(id => {
                          const ent = entityById.get(id);
                          // Unresolved id — render an inert chip rather than a
                          // dead link. Resolved entities become clickable and
                          // select the entity via the shared mechanism.
                          if (!ent) {
                            // Static, and shaped like it: this id is a
                            // reference the graph can no longer resolve, so
                            // there is nothing to open. Sitting among live,
                            // clickable siblings in the same pill shape, it
                            // used to read as a link that simply ignored you.
                            return (
                              <Chip
                                key={id}
                                kind="static"
                                title="This memory references an entity that is not in the graph — nothing to open"
                                style={{ fontFamily: font.mono, fontSize: 10, letterSpacing: 0, padding: '4px 10px' }}
                              >
                                {id}
                              </Chip>
                            );
                          }
                          return (
                            <Button
                              key={id}
                              colors={colors}
                              variant="ghostOn"
                              type="button"
                              onClick={() => selectEntity(ent)}
                              title={`View ${ent.name}`}
                              style={{
                                '--pa-btn-bg': colors.cyanSoft,
                                '--pa-btn-fg': colors.cyan,
                                '--pa-btn-border': colors.borderHi,
                                '--pa-btn-bg-hover': colors.cyanGlow,
                                '--pa-btn-border-hover': colors.borderHi,
                                '--pa-btn-bg-active': colors.cyanSoft,
                                '--pa-btn-pad': '4px 10px',
                                '--pa-btn-radius': `${radius.pill}px`,
                                '--pa-btn-weight': 500,
                                fontFamily: font.body, fontSize: textSize.micro,
                              } as CSSProperties}
                            >{ent.name}</Button>
                          );
                        })}
                      </div>
                    )}
                  </div>
                  {/* Pinned stats footer. "last recalled" removed (2026-07
                      wiring audit): it fabricated concrete recall dates
                      ("3 days ago") from the same age bucket recency already
                      shows — the backend tracks no recall timestamp. */}
                  <div style={{ flexShrink: 0, display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8, borderTop: `1px solid ${colors.borderHi}`, paddingTop: 12 }}>
                    {/* One number, one word. This said "reinforcement" while
                        the List view called the same field "signal" — same
                        memory, same tab, two vocabularies and neither defined.
                        The word and the gloss both come from the shared
                        vocabulary now, so the two surfaces cannot drift. */}
                    <Stat
                      label={MEMORY_STRENGTH.one}
                      value={`${Math.round(mem.weight * 100)}%`}
                      title={MEMORY_STRENGTH.gloss}
                    />
                    {(() => {
                      const age = formatMemoryAge(mem.timestamp);
                      return (
                        <Stat
                          label="recency"
                          value={age.label}
                          // Past the staleness threshold the age is the point:
                          // it must not sit at the same quiet weight as "today".
                          tone={age.stale ? 'stale' : undefined}
                          title={mem.timestamp || undefined}
                        />
                      );
                    })()}
                  </div>
                </div>
              );
            })()}
          </>)}
        </div>
      </div>

      {/* The graph's key. Only in graph mode, and not over an empty or broken
          one: a vocabulary for shapes that are not on screen is noise. It sits
          above the time slider, on the side the side-panel never covers. */}
      {viewMode === 'graph' && !error && !isEmpty && (
        <CanvasLegend
          canvasId="brain-graph"
          gestures={BRAIN_GESTURES}
          vocabulary={BRAIN_VOCABULARY}
          palette={{ bg: glass.bg, border: glass.border }}
          style={{ bottom: 72 }}
        />
      )}

      {/* Time slider */}
      <div style={{
        position: 'absolute', bottom: 16, left: 16, right: selected ? 376 : 16, zIndex: 10,
        display: 'flex', alignItems: 'center', gap: 12, padding: '10px 16px',
        background: glass.bg, backdropFilter: 'blur(16px)',
        border: `1px solid ${glass.border}`, borderRadius: 10,
      }}>
        <span style={{ fontFamily: font.mono, fontSize: 10, color: colors.textDim }}>today</span>
        <input type="range" min={0} max={1} step={0.01} value={timeValue}
          onChange={e => setTimeValue(parseFloat(e.target.value))}
          style={{ flex: 1, accentColor: colors.cyan }}
        />
        <span style={{ fontFamily: font.mono, fontSize: 10, color: colors.textDim }}>all time</span>
        <span style={{ fontFamily: font.mono, fontSize: 10, color: colors.textDim, opacity: 0.6 }}
          title="Imported memories are dated by import time, not original event time">
          *
        </span>
      </div>
    </div>
  );
}

function Stat({ label, value, tone, title }: {
  label: string; value: string;
  /** `stale` colours the figure as a caution rather than a plain fact — for a
   *  number whose age is the thing worth noticing. */
  tone?: 'stale';
  title?: string;
}) {
  const { colors } = useTheme();
  return (
    <div style={{ textAlign: 'center' }} title={title}>
      <div style={{ fontFamily: font.mono, fontSize: 10, color: colors.textDim, marginBottom: 2, textTransform: 'uppercase' }}>{label}</div>
      <div style={{
        fontFamily: font.body, fontSize: textSize.small, fontWeight: 600,
        color: tone === 'stale' ? colors.stale : colors.text,
      }}>{value}</div>
    </div>
  );
}
