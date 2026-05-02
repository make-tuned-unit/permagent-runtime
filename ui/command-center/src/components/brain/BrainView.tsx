import { useEffect, useRef, useState, useCallback } from 'react';
import { color, font, ease } from '../../styles/tokens';
import { Mobius } from '../mobius/Mobius';
import { BrainScene, type TypeFilters } from './BrainScene';
import { useBrainData, type GraphMemory } from './useBrainData';

const FILTERS: { key: keyof TypeFilters; label: string; shape: string }[] = [
  { key: 'person', label: 'people', shape: '●' },
  { key: 'project', label: 'projects', shape: '■' },
  { key: 'topic', label: 'topics', shape: '◆' },
  { key: 'memory', label: 'memories', shape: '·' },
];

interface HoverInfo { id: string; kind: string; label: string; note: string; x: number; y: number }
interface SelectedInfo { id: string; kind: string; label: string; note: string; data: any }

export function BrainView() {
  const containerRef = useRef<HTMLDivElement>(null);
  const sceneRef = useRef<BrainScene | null>(null);
  const { data, loading } = useBrainData();

  const [search, setSearch] = useState('');
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

  // Search
  useEffect(() => {
    const t = setTimeout(() => sceneRef.current?.setSearch(search), 200);
    return () => clearTimeout(t);
  }, [search]);

  // Filters
  useEffect(() => { sceneRef.current?.setTypeFilter(filters); }, [filters]);

  // Time
  useEffect(() => { sceneRef.current?.setTimeRange([0, timeValue]); }, [timeValue]);

  const isEmpty = !loading && data && data.entities.length === 0 && data.memories.length === 0;

  const toggleFilter = (key: keyof TypeFilters) =>
    setFilters(f => ({ ...f, [key]: !f[key] }));

  // Recency label
  const recencyLabel = (age: number) =>
    age < 0.2 ? 'this week' : age < 0.5 ? 'this month' : age < 0.8 ? '~3 months' : '~year';

  return (
    <div style={{ position: 'relative', width: '100%', height: '100%', background: '#070B14', overflow: 'hidden' }}>
      {/* Three.js canvas container */}
      <div ref={containerRef} style={{ position: 'absolute', inset: 0 }} />

      {/* Empty state overlay */}
      {isEmpty && (
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

      {/* Side panel */}
      {selected && (
        <div style={{
          position: 'absolute', top: 0, right: 0, bottom: 0, width: 360, zIndex: 15,
          background: 'rgba(20,28,48,0.85)', backdropFilter: 'blur(24px)',
          borderLeft: '1px solid rgba(0,213,255,0.12)',
          transform: 'translateX(0)', transition: `transform 320ms ${ease.out}`,
          display: 'flex', flexDirection: 'column', padding: 24,
          overflowY: 'auto',
        }}>
          {/* Close */}
          <button onClick={() => setSelected(null)} style={{
            position: 'absolute', top: 16, right: 16,
            background: 'transparent', border: 'none', color: color.textMuted,
            fontSize: 18, cursor: 'pointer',
          }}>×</button>

          {/* Type label */}
          <span style={{
            fontFamily: font.mono, fontSize: 10, fontWeight: 600,
            color: color.cyan, textTransform: 'uppercase', letterSpacing: '0.1em',
          }}>
            {selected.kind}
          </span>

          {/* Name */}
          <h3 style={{ fontFamily: font.display, fontSize: 20, fontWeight: 700, color: color.text, margin: '8px 0 12px' }}>
            {selected.label}
          </h3>

          {/* Note / text */}
          {selected.note && (
            <p style={{ fontFamily: font.body, fontSize: 13, color: color.textMuted, lineHeight: 1.6, margin: '0 0 16px' }}>
              {selected.note}
            </p>
          )}

          {/* Memory stats */}
          {selected.kind === 'memory' && selected.data && (() => {
            const mem = selected.data as GraphMemory;
            return (
              <div style={{ display: 'flex', flexDirection: 'column', gap: 12, marginTop: 8 }}>
                {mem.ent.length > 0 && (
                  <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}>
                    {mem.ent.map(id => (
                      <span key={id} style={{
                        fontFamily: font.mono, fontSize: 10, color: color.cyan,
                        border: `1px solid ${color.borderHi}`, borderRadius: 999, padding: '3px 8px',
                      }}>{id}</span>
                    ))}
                  </div>
                )}
                <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 8 }}>
                  <Stat label="reinforcement" value={`${Math.round(mem.weight * 100)}%`} />
                  <Stat label="recency" value={recencyLabel(mem.age)} />
                  <Stat label="last recalled" value={mem.age < 0.1 ? 'today' : mem.age < 0.3 ? '3 days ago' : '2 weeks ago'} />
                </div>
              </div>
            );
          })()}
        </div>
      )}

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
