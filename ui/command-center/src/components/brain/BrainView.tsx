import { useEffect, useRef, useState, useCallback, useMemo } from 'react';
import { font, ease } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { apiFetch } from '../../lib/api';
import { useCommandCenter, navigateToTool } from '../../lib/store';
import { Mobius } from '../mobius/Mobius';
import { BrainScene, type TypeFilters } from './BrainScene';
import { useBrainData, type GraphMemory, type GraphEntity } from './useBrainData';
import { BrainList } from './BrainList';

type ViewMode = 'graph' | 'list';

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
interface SelectedInfo { id: string; kind: string; label: string; note: string; data: any }

export function BrainView() {
  const { gradient, colors, theme } = useTheme();

  // Glass overlay style: white translucent for silver, dark for dark themes
  const glass = theme === 'silver'
    ? { bg: 'rgba(255,255,255,0.88)', border: 'rgba(167,176,190,0.35)' }
    : { bg: 'rgba(20,28,48,0.75)', border: 'rgba(255,255,255,0.06)' };
  const containerRef = useRef<HTMLDivElement>(null);
  const sceneRef = useRef<BrainScene | null>(null);

  const [viewMode, setViewMode] = useState<ViewMode>('graph');
  const [search, setSearch] = useState('');
  const [debouncedSearch, setDebouncedSearch] = useState('');
  const [modeBeforeSearch, setModeBeforeSearch] = useState<ViewMode>('graph');

  const { data, loading, error, refresh } = useBrainData(debouncedSearch);
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

  // Recency label
  const recencyLabel = (age: number) =>
    age < 0.2 ? 'this week' : age < 0.5 ? 'this month' : age < 0.8 ? '~3 months' : '~year';

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
          <p style={{ fontFamily: font.body, fontSize: 13, color: colors.textMuted, maxWidth: 340, textAlign: 'center', lineHeight: 1.5 }}>
            The memory graph didn't load. It may be a brief hiccup — try again.
          </p>
          <button onClick={() => refresh()} style={{
            marginTop: 4, padding: '8px 18px', borderRadius: 8,
            border: `1px solid ${colors.borderHi}`, background: colors.cyanSoft,
            color: colors.cyan, fontSize: 13, fontWeight: 600, cursor: 'pointer',
          }}>Try again</button>
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
          <p style={{ fontFamily: font.mono, fontSize: 11, color: colors.textDim, letterSpacing: '0.06em' }}>
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
          <p style={{ fontFamily: font.body, fontSize: 14, color: colors.textMuted, marginTop: 8 }}>
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
          border: `1px solid ${theme === 'silver' ? colors.cyan + '20' : 'rgba(0,213,255,0.12)'}`, borderRadius: 999,
        }}>
          <Mobius size={28} state="idle" logoMode />
          <span style={{ fontFamily: font.display, fontSize: 13, fontWeight: 700, color: colors.text }}>
            {data?.self.name || 'Agent'}'s mind
          </span>
        </div>

        {/* Search */}
        <div style={{ flex: 1, maxWidth: 360 }}>
          <input
            value={search} onChange={e => setSearch(e.target.value)}
            placeholder="search the shape of what we've built..."
            style={{
              width: '100%', fontFamily: font.body, fontSize: 13, color: colors.text,
              background: theme === 'silver' ? 'rgba(255,255,255,0.92)' : 'rgba(20,28,48,0.65)', backdropFilter: 'blur(12px)',
              border: `1px solid ${glass.border}`, borderRadius: 999,
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
            <button key={f.key} onClick={() => toggleFilter(f.key)} style={{
              fontFamily: font.body, fontSize: 11, fontWeight: 500,
              color: filters[f.key] ? colors.text : colors.textDim,
              background: filters[f.key] ? colors.cyanSoft : 'transparent',
              border: 'none', borderRadius: 6, padding: '4px 8px', cursor: 'pointer',
              transition: `all 160ms ${ease.out}`,
            }}>
              <span style={{ marginRight: 4 }}>{f.shape}</span>{f.label}
            </button>
          ))}

          {/* Topics group with drilldown */}
          <span style={{
            display: 'inline-flex', alignItems: 'center', gap: 2,
            borderLeft: `1px solid ${glass.border}`, paddingLeft: 6,
          }}>
            <button onClick={() => {
              const allOn = TOPIC_KEYS.every(k => filters[k]);
              setFilters(f => {
                const next = { ...f };
                for (const k of TOPIC_KEYS) next[k] = !allOn;
                return next;
              });
            }} style={{
              fontFamily: font.body, fontSize: 11, fontWeight: 500,
              color: TOPIC_KEYS.some(k => filters[k]) ? colors.text : colors.textDim,
              background: TOPIC_KEYS.every(k => filters[k]) ? colors.cyanSoft : TOPIC_KEYS.some(k => filters[k]) ? `${colors.cyanSoft}88` : 'transparent',
              border: 'none', borderRadius: 6, padding: '4px 8px', cursor: 'pointer',
              transition: `all 160ms ${ease.out}`,
            }}>
              <span style={{ marginRight: 4 }}>◆</span>topics
            </button>
            <button onClick={() => setTopicsExpanded(e => !e)} style={{
              fontFamily: font.mono, fontSize: 9, color: colors.textDim,
              background: 'transparent', border: 'none', cursor: 'pointer', padding: '2px 4px',
              transition: `transform 160ms ${ease.out}`,
              transform: topicsExpanded ? 'rotate(90deg)' : 'rotate(0deg)',
            }}>▸</button>
            {topicsExpanded && TOPIC_SUB_FILTERS.map(f => (
              <button key={f.key} onClick={() => toggleFilter(f.key)} style={{
                fontFamily: font.body, fontSize: 10, fontWeight: 500,
                color: filters[f.key] ? colors.text : colors.textDim,
                background: filters[f.key] ? colors.cyanSoft : 'transparent',
                border: 'none', borderRadius: 5, padding: '3px 6px', cursor: 'pointer',
                transition: `all 160ms ${ease.out}`,
              }}>
                <span style={{ marginRight: 3 }}>{f.shape}</span>{f.label}
              </button>
            ))}
          </span>

          <span style={{ borderLeft: `1px solid ${glass.border}`, paddingLeft: 6 }}>
            <button onClick={() => toggleFilter('memory')} style={{
              fontFamily: font.body, fontSize: 11, fontWeight: 500,
              color: filters.memory ? colors.text : colors.textDim,
              background: filters.memory ? colors.cyanSoft : 'transparent',
              border: 'none', borderRadius: 6, padding: '4px 8px', cursor: 'pointer',
              transition: `all 160ms ${ease.out}`,
            }}>
              <span style={{ marginRight: 4 }}>·</span>memories
            </button>
          </span>
        </div>

        {/* Graph / List toggle */}
        <div style={{
          display: 'flex', alignItems: 'center', gap: 2, padding: '4px 6px',
          background: glass.bg, backdropFilter: 'blur(16px)',
          border: `1px solid ${glass.border}`, borderRadius: 8,
        }}>
          {(['graph', 'list'] as const).map(mode => (
            <button key={mode} onClick={() => setViewMode(mode)} style={{
              fontFamily: font.mono, fontSize: 10, fontWeight: 600,
              color: viewMode === mode ? colors.text : colors.textDim,
              background: viewMode === mode ? colors.cyanSoft : 'transparent',
              border: 'none', borderRadius: 6, padding: '4px 10px', cursor: 'pointer',
              textTransform: 'uppercase', letterSpacing: '0.05em',
              transition: `all 160ms ${ease.out}`,
            }}>
              {mode}
            </button>
          ))}
        </div>
      </div>

      {/* Hover tooltip */}
      {hover && (
        <div style={{
          position: 'fixed', left: hover.x + 14, top: hover.y + 14, zIndex: 20,
          maxWidth: 280, padding: '8px 12px',
          background: theme === 'silver' ? 'rgba(255,255,255,0.95)' : 'rgba(20,28,48,0.9)', backdropFilter: 'blur(12px)',
          border: `1px solid ${colors.borderHi}`, borderRadius: 8,
          boxShadow: theme === 'silver' ? '0 2px 12px rgba(30,37,48,0.10)' : 'none',
          fontFamily: font.body, fontSize: 12, color: colors.text,
          pointerEvents: 'none',
        }}>
          <div style={{ fontWeight: 600 }}>{hover.label}</div>
          {hover.note && hover.note !== hover.label && (
            <div style={{ color: colors.textMuted, marginTop: 2, fontSize: 11 }}>{hover.note.slice(0, 120)}</div>
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
          borderRadius: 16, padding: 24,
          boxShadow: theme === 'silver'
            ? '0 24px 60px rgba(30,37,48,0.12), inset 0 1px 0 rgba(255,255,255,0.8)'
            : '0 24px 60px rgba(0,0,0,0.5), inset 0 1px 0 rgba(255,255,255,0.06)',
          display: 'flex', flexDirection: 'column',
        }}>
          {selected && (<>
            {/* Close */}
            <button onClick={() => setSelected(null)} style={{
              position: 'absolute', top: 28, right: 28,
              background: 'transparent', border: 'none', color: colors.textMuted,
              fontSize: 18, cursor: 'pointer',
            }}>×</button>

            {/* Type label */}
            <span style={{
              fontFamily: font.mono, fontSize: 10, fontWeight: 600,
              color: colors.cyan, textTransform: 'uppercase', letterSpacing: '0.1em',
            }}>
              {selected.kind === 'memory' ? 'MEMORY' : (selected.data as GraphEntity)?.type?.toUpperCase() || selected.kind.toUpperCase()}
            </span>

            {/* Name / title */}
            <h3 style={{ fontFamily: font.display, fontSize: 20, fontWeight: 700, color: colors.text, margin: '8px 0 12px' }}>
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
                <p style={{ fontFamily: font.body, fontSize: 13, color: colors.textMuted, lineHeight: 1.6, margin: '0 0 16px' }}>
                  {selected.note || 'No description yet — the Librarian writes one on its next run.'}
                </p>

                {fields.length > 0 && (
                  <div style={{ marginBottom: 16, overflowY: 'auto', maxHeight: 200 }}>
                    {fields.map(f => (
                      <div key={f.field_name} style={{ display: 'flex', alignItems: 'baseline', gap: 8, padding: '5px 0', borderBottom: `1px solid ${colors.border}` }}>
                        <span style={{ fontFamily: font.mono, fontSize: 10, color: colors.textDim, textTransform: 'uppercase', letterSpacing: '0.06em', minWidth: 84 }}>
                          {f.field_name.replace(/_/g, ' ')}
                        </span>
                        <span style={{ fontFamily: font.body, fontSize: 12, color: colors.text, flex: 1, overflowWrap: 'anywhere' }}>
                          {f.source_url ? (
                            <a href={f.source_url} target="_blank" rel="noreferrer" style={{ color: colors.cyan, textDecoration: 'none' }}>{f.value}</a>
                          ) : f.value}
                        </span>
                        <span style={{
                          fontFamily: font.mono, fontSize: 8, padding: '1px 5px', borderRadius: 3,
                          color: f.source === 'manual' ? colors.cyan : colors.textMuted,
                          border: `1px solid ${f.source === 'manual' ? colors.cyan : colors.border}`,
                          textTransform: 'uppercase', letterSpacing: '0.08em', flexShrink: 0,
                        }}>{f.source}</span>
                      </div>
                    ))}
                  </div>
                )}

                {selected.kind === 'project' && projectMatch && (
                  <button
                    type="button"
                    onClick={openProjectWorkspace}
                    title={`Open ${projectMatch.name} in Projects`}
                    style={{
                      alignSelf: 'flex-start', marginBottom: 16,
                      fontFamily: font.body, fontSize: 12, fontWeight: 600, color: colors.cyan,
                      background: colors.cyanSoft, cursor: 'pointer',
                      border: `1px solid ${colors.borderHi}`, borderRadius: 8, padding: '6px 12px',
                      transition: chipTransition,
                    }}
                    onMouseEnter={e => { (e.currentTarget as HTMLButtonElement).style.background = colors.cyanGlow; }}
                    onMouseLeave={e => { (e.currentTarget as HTMLButtonElement).style.background = colors.cyanSoft; }}
                    onFocus={e => { (e.currentTarget as HTMLButtonElement).style.outline = `2px solid ${colors.cyan}`; }}
                    onBlur={e => { (e.currentTarget as HTMLButtonElement).style.outline = 'none'; }}
                  >
                    Open project →
                  </button>
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
                          cursor: interactive ? 'pointer' : 'default', borderRadius: 6,
                          transition: chipTransition,
                        }}
                        onMouseEnter={e => { if (interactive) (e.currentTarget as HTMLButtonElement).style.opacity = '0.7'; }}
                        onMouseLeave={e => { (e.currentTarget as HTMLButtonElement).style.opacity = '1'; }}
                        onFocus={e => { if (interactive) (e.currentTarget as HTMLButtonElement).style.outline = `2px solid ${colors.cyan}`; }}
                        onBlur={e => { (e.currentTarget as HTMLButtonElement).style.outline = 'none'; }}
                      >
                        <div style={{ fontFamily: font.display, fontSize: 18, fontWeight: 700, color: interactive ? colors.cyan : colors.text }}>{stat.value}</div>
                        <div style={{ fontFamily: font.mono, fontSize: 9, color: colors.textDim, letterSpacing: '0.08em' }}>{stat.label}</div>
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
                      <p style={{ fontFamily: font.body, fontSize: 13, color: colors.text, lineHeight: 1.7, margin: 0 }}>
                        {mem.description}
                      </p>
                    )}
                    <p style={{
                      fontFamily: font.mono, fontSize: 11, color: colors.textMuted, lineHeight: 1.6, margin: 0,
                      padding: '10px 12px', background: theme === 'silver' ? 'rgba(30,37,48,0.04)' : 'rgba(0,0,0,0.2)', borderRadius: 8,
                      whiteSpace: 'pre-wrap', wordBreak: 'break-word',
                    }}>
                      {mem.text}
                    </p>
                    {mem.ent.length > 0 && (
                      <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}>
                        {mem.ent.map(id => {
                          const ent = entityById.get(id);
                          // Unresolved id — render an inert chip rather than a
                          // dead link. Resolved entities become clickable and
                          // select the entity via the shared mechanism.
                          if (!ent) {
                            return (
                              <span key={id} title="Unknown entity" style={{
                                fontFamily: font.mono, fontSize: 10, color: colors.textDim,
                                border: `1px solid ${colors.border}`, borderRadius: 999, padding: '4px 10px',
                              }}>{id}</span>
                            );
                          }
                          return (
                            <button
                              key={id}
                              type="button"
                              onClick={() => selectEntity(ent)}
                              title={`View ${ent.name}`}
                              style={{
                                fontFamily: font.body, fontSize: 11, fontWeight: 500, color: colors.cyan,
                                background: colors.cyanSoft, cursor: 'pointer',
                                border: `1px solid ${colors.borderHi}`, borderRadius: 999, padding: '4px 10px',
                                transition: chipTransition,
                              }}
                              onMouseEnter={e => { (e.currentTarget as HTMLButtonElement).style.background = colors.cyanGlow; }}
                              onMouseLeave={e => { (e.currentTarget as HTMLButtonElement).style.background = colors.cyanSoft; }}
                              onFocus={e => { (e.currentTarget as HTMLButtonElement).style.outline = `2px solid ${colors.cyan}`; }}
                              onBlur={e => { (e.currentTarget as HTMLButtonElement).style.outline = 'none'; }}
                            >{ent.name}</button>
                          );
                        })}
                      </div>
                    )}
                  </div>
                  {/* Pinned stats footer */}
                  <div style={{ flexShrink: 0, display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 8, borderTop: `1px solid ${colors.borderHi}`, paddingTop: 12 }}>
                    <Stat label="reinforcement" value={`${Math.round(mem.weight * 100)}%`} />
                    <Stat label="recency" value={recencyLabel(mem.age)} />
                    <Stat label="last recalled" value={mem.age < 0.1 ? 'today' : mem.age < 0.3 ? '3 days ago' : '2 weeks ago'} />
                  </div>
                </div>
              );
            })()}
          </>)}
        </div>
      </div>

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
        <span style={{ fontFamily: font.mono, fontSize: 9, color: colors.textDim, opacity: 0.6 }}
          title="Imported memories are dated by import time, not original event time">
          *
        </span>
      </div>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  const { colors } = useTheme();
  return (
    <div style={{ textAlign: 'center' }}>
      <div style={{ fontFamily: font.mono, fontSize: 10, color: colors.textDim, marginBottom: 2, textTransform: 'uppercase' }}>{label}</div>
      <div style={{ fontFamily: font.body, fontSize: 13, fontWeight: 600, color: colors.text }}>{value}</div>
    </div>
  );
}
