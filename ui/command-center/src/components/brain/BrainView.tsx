import { useEffect, useRef, useState, useCallback } from 'react';
import { color, font, ease } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Mobius } from '../mobius/Mobius';
import { BrainScene, type TypeFilters } from './BrainScene';
import { useBrainData, type GraphMemory, type GraphEntity } from './useBrainData';
import { BrainList } from './BrainList';

type ViewMode = 'graph' | 'list';

const FILTERS: { key: keyof TypeFilters; label: string; shape: string }[] = [
  { key: 'person', label: 'people', shape: '●' },
  { key: 'project', label: 'projects', shape: '■' },
  { key: 'topic', label: 'topics', shape: '◆' },
  { key: 'memory', label: 'memories', shape: '·' },
];

interface HoverInfo { id: string; kind: string; label: string; note: string; x: number; y: number }
interface SelectedInfo { id: string; kind: string; label: string; note: string; data: any }

export function BrainView() {
  const { gradient } = useTheme();
  const containerRef = useRef<HTMLDivElement>(null);
  const sceneRef = useRef<BrainScene | null>(null);
  const { data, loading } = useBrainData();

  const [viewMode, setViewMode] = useState<ViewMode>('graph');
  const [search, setSearch] = useState('');
  const [debouncedSearch, setDebouncedSearch] = useState('');
  const [modeBeforeSearch, setModeBeforeSearch] = useState<ViewMode>('graph');
  const [filters, setFilters] = useState<TypeFilters>({ person: true, project: true, topic: true, memory: true });
  const [timeValue, setTimeValue] = useState(1);
  const [hover, setHover] = useState<HoverInfo | null>(null);
  const [selected, setSelected] = useState<SelectedInfo | null>(null);

  const onHover = useCallback((item: HoverInfo | null) => setHover(item), []);
  const onSelect = useCallback((item: SelectedInfo | null) => setSelected(item), []);

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
          />
        </div>
      )}

      {/* Empty state overlay */}
      {isEmpty && viewMode === 'graph' && (
        <div style={{
          position: 'absolute', inset: 0, display: 'flex', flexDirection: 'column',
          alignItems: 'center', justifyContent: 'center', zIndex: 10,
          background: 'radial-gradient(ellipse 70% 50% at 50% 45%, rgba(0,213,255,0.04) 0%, transparent 70%)',
        }}>
          <Mobius size={200} state="idle" />
          <h2 style={{ fontFamily: font.display, fontSize: 22, fontWeight: 700, color: color.text, marginTop: 24 }}>
            Your agent's memory grows here.
          </h2>
          <p style={{ fontFamily: font.body, fontSize: 14, color: color.textMuted, marginTop: 8 }}>
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
          background: 'rgba(20,28,48,0.75)', backdropFilter: 'blur(16px)',
          border: '1px solid rgba(0,213,255,0.12)', borderRadius: 999,
        }}>
          <Mobius size={28} state="idle" logoMode />
          <span style={{ fontFamily: font.display, fontSize: 13, fontWeight: 700, color: color.text }}>
            {data?.self.name || 'Agent'}'s mind
          </span>
        </div>

        {/* Search */}
        <div style={{ flex: 1, maxWidth: 360 }}>
          <input
            value={search} onChange={e => setSearch(e.target.value)}
            placeholder="search the shape of what we've built..."
            style={{
              width: '100%', fontFamily: font.body, fontSize: 13, color: color.text,
              background: 'rgba(20,28,48,0.65)', backdropFilter: 'blur(12px)',
              border: '1px solid rgba(255,255,255,0.06)', borderRadius: 999,
              padding: '9px 16px', outline: 'none',
            }}
          />
        </div>

        {/* Filter chips */}
        <div style={{
          display: 'flex', alignItems: 'center', gap: 6, padding: '6px 10px',
          background: 'rgba(20,28,48,0.75)', backdropFilter: 'blur(16px)',
          border: '1px solid rgba(255,255,255,0.06)', borderRadius: 10,
        }}>
          <span style={{ fontFamily: font.body, fontSize: 10, color: color.textDim, marginRight: 4 }}>show</span>
          {FILTERS.map(f => (
            <button key={f.key} onClick={() => toggleFilter(f.key)} style={{
              fontFamily: font.body, fontSize: 11, fontWeight: 500,
              color: filters[f.key] ? color.text : color.textDim,
              background: filters[f.key] ? 'rgba(0,213,255,0.10)' : 'transparent',
              border: 'none', borderRadius: 6, padding: '4px 8px', cursor: 'pointer',
              transition: `all 160ms ${ease.out}`,
            }}>
              <span style={{ marginRight: 4 }}>{f.shape}</span>{f.label}
            </button>
          ))}
        </div>

        {/* Graph / List toggle */}
        <div style={{
          display: 'flex', alignItems: 'center', gap: 2, padding: '4px 6px',
          background: 'rgba(20,28,48,0.75)', backdropFilter: 'blur(16px)',
          border: '1px solid rgba(255,255,255,0.06)', borderRadius: 8,
        }}>
          {(['graph', 'list'] as const).map(mode => (
            <button key={mode} onClick={() => setViewMode(mode)} style={{
              fontFamily: font.mono, fontSize: 10, fontWeight: 600,
              color: viewMode === mode ? color.text : color.textDim,
              background: viewMode === mode ? 'rgba(0,213,255,0.12)' : 'transparent',
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
          background: 'rgba(20,28,48,0.9)', backdropFilter: 'blur(12px)',
          border: `1px solid ${color.borderHi}`, borderRadius: 8,
          fontFamily: font.body, fontSize: 12, color: color.text,
          pointerEvents: 'none',
        }}>
          <div style={{ fontWeight: 600 }}>{hover.label}</div>
          {hover.note && hover.note !== hover.label && (
            <div style={{ color: color.textMuted, marginTop: 2, fontSize: 11 }}>{hover.note.slice(0, 120)}</div>
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
          background: 'rgba(20,28,48,0.78)',
          backdropFilter: 'blur(24px) saturate(140%)',
          WebkitBackdropFilter: 'blur(24px) saturate(140%)',
          border: '1px solid rgba(0,213,255,0.16)',
          borderRadius: 16, padding: 24,
          boxShadow: '0 24px 60px rgba(0,0,0,0.5), inset 0 1px 0 rgba(255,255,255,0.06)',
          display: 'flex', flexDirection: 'column',
        }}>
          {selected && (<>
            {/* Close */}
            <button onClick={() => setSelected(null)} style={{
              position: 'absolute', top: 28, right: 28,
              background: 'transparent', border: 'none', color: color.textMuted,
              fontSize: 18, cursor: 'pointer',
            }}>×</button>

            {/* Type label */}
            <span style={{
              fontFamily: font.mono, fontSize: 10, fontWeight: 600,
              color: color.cyan, textTransform: 'uppercase', letterSpacing: '0.1em',
            }}>
              {selected.kind === 'memory' ? 'MEMORY' : (selected.data as GraphEntity)?.type?.toUpperCase() || selected.kind.toUpperCase()}
            </span>

            {/* Name / title */}
            <h3 style={{ fontFamily: font.display, fontSize: 20, fontWeight: 700, color: color.text, margin: '8px 0 12px' }}>
              {selected.label}
            </h3>

            {/* Entity: note or fallback */}
            {selected.kind !== 'memory' && (
              <p style={{ fontFamily: font.body, fontSize: 13, color: color.textMuted, lineHeight: 1.6, margin: '0 0 16px' }}>
                {selected.note || 'No description yet.'}
              </p>
            )}

            {/* Memory: description + content + chips + stats */}
            {selected.kind === 'memory' && selected.data && (() => {
              const mem = selected.data as GraphMemory;
              return (
                <div style={{ display: 'flex', flexDirection: 'column', gap: 0, minHeight: 0, flex: 1 }}>
                  {/* Scrollable content area */}
                  <div style={{ flex: 1, overflowY: 'auto', display: 'flex', flexDirection: 'column', gap: 14, marginBottom: 14 }}>
                    {mem.description && (
                      <p style={{ fontFamily: font.body, fontSize: 13, color: color.text, lineHeight: 1.7, margin: 0 }}>
                        {mem.description}
                      </p>
                    )}
                    <p style={{
                      fontFamily: font.mono, fontSize: 11, color: color.textMuted, lineHeight: 1.6, margin: 0,
                      padding: '10px 12px', background: 'rgba(0,0,0,0.2)', borderRadius: 8,
                      whiteSpace: 'pre-wrap', wordBreak: 'break-word',
                    }}>
                      {mem.text}
                    </p>
                    {mem.ent.length > 0 && (
                      <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}>
                        {mem.ent.map(id => (
                          <span key={id} style={{
                            fontFamily: font.mono, fontSize: 10, color: color.cyan,
                            border: `1px solid ${color.borderHi}`, borderRadius: 999, padding: '4px 10px',
                          }}>{id}</span>
                        ))}
                      </div>
                    )}
                  </div>
                  {/* Pinned stats footer */}
                  <div style={{ flexShrink: 0, display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 8, borderTop: `1px solid ${color.borderHi}`, paddingTop: 12 }}>
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
        background: 'rgba(20,28,48,0.75)', backdropFilter: 'blur(16px)',
        border: '1px solid rgba(255,255,255,0.06)', borderRadius: 10,
      }}>
        <span style={{ fontFamily: font.mono, fontSize: 10, color: color.textDim }}>today</span>
        <input type="range" min={0} max={1} step={0.01} value={timeValue}
          onChange={e => setTimeValue(parseFloat(e.target.value))}
          style={{ flex: 1, accentColor: color.cyan }}
        />
        <span style={{ fontFamily: font.mono, fontSize: 10, color: color.textDim }}>all time</span>
        <span style={{ fontFamily: font.mono, fontSize: 9, color: color.textDim, opacity: 0.6 }}
          title="Imported memories are dated by import time, not original event time">
          *
        </span>
      </div>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div style={{ textAlign: 'center' }}>
      <div style={{ fontFamily: font.mono, fontSize: 10, color: color.textDim, marginBottom: 2, textTransform: 'uppercase' }}>{label}</div>
      <div style={{ fontFamily: font.body, fontSize: 13, fontWeight: 600, color: color.text }}>{value}</div>
    </div>
  );
}
