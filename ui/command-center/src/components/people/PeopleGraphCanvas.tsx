/**
 * 3D people graph — you at the center, project clusters around you.
 *
 * Layout is a pure function (`layoutPeopleGraph` in peopleGraph.ts); this file
 * only draws it. Named *Canvas so it cannot collide with peopleGraph.ts on a
 * case-insensitive filesystem. Click a person to open the same detail panel
 * the list uses. You sit at the origin and are not clickable.
 */

import { useEffect, useMemo, useState } from 'react';
import { Canvas } from '@react-three/fiber';
import { Html, Line, OrbitControls } from '@react-three/drei';
import { apiFetch } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';
import { font, radius, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import type { DirectoryPerson } from '../projects/types';
import { CanvasLegend } from '../common/CanvasLegend';
import { PEOPLE_GESTURES, PEOPLE_VOCABULARY } from './peopleLegend';
import { isBridge, isYou, layoutPeopleGraph, type GraphNode } from './peopleGraph';
import { PersonFace } from './PersonFace';
import { shouldShowLabel } from './peopleFace';
import { isQuiet } from './contactAge';

type Status = 'loading' | 'error' | 'ready';

export function PeopleGraph() {
  const { colors, gradient, reduceMotion } = useTheme();
  const openPersonDetail = useCommandCenter(s => s.openPersonDetail);
  const personDetail = useCommandCenter(s => s.personDetail);
  const peopleRev = useCommandCenter(s => s.peopleRev);
  const [people, setPeople] = useState<DirectoryPerson[]>([]);
  const [status, setStatus] = useState<Status>('loading');
  const [query, setQuery] = useState('');
  const [hovered, setHovered] = useState<string | null>(null);
  /** Keyboard focus on a person's face — tabbing lights a node up like hover. */
  const [focusedId, setFocusedId] = useState<string | null>(null);
  /** The person whose detail modal is open, from the same `openPersonDetail(null, …)`
   *  call this graph makes on click (PeopleView.tsx applies the same
   *  `projectId == null` filter before rendering its modal — a person opened
   *  from a project's People panel has projectId set and is a different
   *  surface, not this graph's own selection). */
  const selectedId = personDetail && personDetail.projectId == null ? personDetail.person.entity_uuid : null;
  const activeIds = useMemo(
    () => new Set([hovered, focusedId, selectedId].filter((id): id is string => id != null)),
    [hovered, focusedId, selectedId],
  );

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const rows = await apiFetch<DirectoryPerson[]>('/api/people/directory');
        if (cancelled) return;
        if (!Array.isArray(rows)) throw new Error('Invalid directory response');
        setPeople(rows);
        setStatus('ready');
      } catch {
        if (!cancelled) setStatus('error');
      }
    })();
    return () => { cancelled = true; };
  }, [peopleRev]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return people;
    return people.filter(p => {
      const hay = [p.display_name, p.company, p.role, ...p.projects.map(pr => pr.project_name)]
        .filter(Boolean)
        .join(' ')
        .toLowerCase();
      return hay.includes(q);
    });
  }, [people, query]);

  const layout = useMemo(() => layoutPeopleGraph(filtered), [filtered]);
  const byId = useMemo(() => new Map(layout.nodes.map(n => [n.id, n])), [layout.nodes]);

  return (
    <div style={{ position: 'relative', width: '100%', height: '100%', background: gradient.workspace }}>
      <input
        value={query}
        onChange={e => setQuery(e.target.value)}
        placeholder="Filter people…"
        aria-label="Filter people"
        style={{
          position: 'absolute',
          top: 12,
          left: 24,
          zIndex: 2,
          width: 240,
          fontSize: textSize.caption,
          fontFamily: font.body,
          padding: '6px 10px',
          borderRadius: radius.sm,
          border: `1px solid ${colors.border}`,
          background: colors.inputBg,
          color: colors.text,
          outline: 'none',
        }}
      />
      {status === 'loading' && (
        <div style={{ position: 'absolute', inset: 0, display: 'grid', placeItems: 'center', color: colors.textDim, fontFamily: font.body, fontSize: textSize.caption }}>
          Loading people…
        </div>
      )}
      {status === 'error' && (
        <div style={{ position: 'absolute', inset: 0, display: 'grid', placeItems: 'center', color: colors.danger, fontFamily: font.body, fontSize: textSize.caption }}>
          Couldn't load people.
        </div>
      )}
      {status === 'ready' && filtered.length === 0 && query.trim() !== '' && (
        <div style={{
          position: 'absolute', top: 48, left: 24, zIndex: 2,
          color: colors.textDim, fontFamily: font.body, fontSize: textSize.caption,
        }}>
          No people match that search.
        </div>
      )}
      {/* The key. The graph's whole premise — you at the centre, everyone else
          grouped by a shared project, a bigger face for whoever bridges two
          groups — lived only in the layout code until now. Not shown over a
          failed or still-loading load: there is nothing to explain yet. */}
      {status === 'ready' && (
        <CanvasLegend
          canvasId="people-graph"
          gestures={PEOPLE_GESTURES}
          vocabulary={PEOPLE_VOCABULARY}
        />
      )}
      {status === 'ready' && (
        <Canvas
          camera={{ position: [0, 7.5, 14], fov: 50 }}
          gl={{ alpha: true, antialias: true }}
          style={{ width: '100%', height: '100%', background: 'transparent' }}
          onPointerMissed={() => setHovered(null)}
        >
          <ambientLight intensity={0.55} />
          <pointLight position={[8, 10, 6]} intensity={1.1} />
          <OrbitControls enableDamping makeDefault />
          {layout.edges.map(edge => {
            const from = byId.get(edge.from);
            const to = byId.get(edge.to);
            if (!from || !to) return null;
            const ego = edge.kind === 'ego';
            // An edge touching the active person (hovered, keyboard-focused,
            // or selected) reads as "this connects to them" — brighter and
            // thicker than the baseline ego/project treatment.
            const active = activeIds.has(edge.from) || activeIds.has(edge.to);
            return (
              <Line
                key={`${edge.from}|${edge.to}|${edge.via}`}
                points={[[from.x, from.y, from.z], [to.x, to.y, to.z]]}
                color={active ? colors.cyan : ego ? colors.cyan : colors.textMuted}
                transparent
                opacity={active ? 0.75 : ego ? 0.4 : 0.18}
                lineWidth={active ? 2 : ego ? 1.4 : 1}
              />
            );
          })}
          {layout.clusters.map(cluster => (
            <Html key={cluster.id} position={[cluster.x, cluster.y + 1.6, cluster.z]} center style={{ pointerEvents: 'none' }}>
              <div style={{
                fontFamily: font.mono,
                fontSize: 10,
                letterSpacing: '0.06em',
                textTransform: 'uppercase',
                color: colors.textDim,
                whiteSpace: 'nowrap',
              }}>
                {cluster.name}
              </div>
            </Html>
          ))}
          {layout.nodes.map(node => (
            <PersonNode
              key={node.id}
              node={node}
              accent={colors.cyan}
              muted={colors.textMuted}
              hovered={hovered === node.id}
              focused={focusedId === node.id}
              selected={selectedId === node.id}
              reducedMotion={reduceMotion}
              onHover={setHovered}
              onFocusChange={setFocusedId}
              onOpen={isYou(node) ? undefined : () => {
                const person = people.find(p => p.entity_uuid === node.id);
                if (person) openPersonDetail(null, person);
              }}
            />
          ))}
        </Canvas>
      )}
    </div>
  );
}

function PersonNode({
  node,
  accent,
  muted,
  hovered,
  focused,
  selected,
  reducedMotion,
  onHover,
  onFocusChange,
  onOpen,
}: {
  node: GraphNode;
  accent: string;
  muted: string;
  hovered: boolean;
  focused: boolean;
  selected: boolean;
  reducedMotion: boolean;
  onHover: (id: string | null) => void;
  onFocusChange: (id: string | null) => void;
  onOpen?: () => void;
}) {
  const you = isYou(node);
  const bridge = isBridge(node);
  // "active" — hovered, keyboard-focused, or the selected (detail-open)
  // person — lights up the disc and, below, shows the name pill.
  const active = hovered || focused || selected;
  const color = you || bridge || active ? accent : muted;
  const pillVisible = shouldShowLabel({ isYou: you, hovered, focused, selected });
  // Sphere size, in world units — named apart from the corner-radius token.
  const nodeRadius = you ? 0.32 : bridge ? 0.22 : 0.16;
  if (you) {
    return (
      <group position={[node.x, node.y, node.z]}>
        <mesh
          onPointerOver={e => { e.stopPropagation(); onHover(node.id); }}
          onPointerOut={() => onHover(null)}
        >
          <sphereGeometry args={[nodeRadius, 24, 24]} />
          <meshStandardMaterial color={color} emissive={color} emissiveIntensity={0.55} />
        </mesh>
        <Html center sprite style={{ pointerEvents: 'none', transform: 'translateY(-18px)' }}>
          <div style={{
            fontFamily: font.body,
            fontSize: textSize.micro,
            fontWeight: 600,
            color: '#fff',
            background: 'rgba(8,10,16,0.78)',
            borderRadius: radius.xs,
            padding: '2px 6px',
            whiteSpace: 'nowrap',
          }}>
            {node.name}
          </div>
        </Html>
      </group>
    );
  }
  const size = active || bridge ? 48 : 40;
  return (
    <group position={[node.x, node.y, node.z]}>
      <Html
        center
        sprite
        zIndexRange={[100, 0]}
        style={{ pointerEvents: 'auto' }}
      >
        <div
          onPointerOver={e => { e.stopPropagation(); onHover(node.id); }}
          onPointerOut={() => onHover(null)}
          style={{ position: 'relative' }}
        >
          {/* The name pill rides INSIDE the face's <Html>, not in a second one.
              Always mounted so the opacity change is a real ~120ms fade rather
              than a mount pop-in — and one <Html> per node instead of two,
              which matters: drei re-projects every <Html> each frame, so a
              second one per person doubled that cost for a directory of any
              size. Reduced motion keeps this fade; it is opacity, not motion. */}
          <div style={{
            position: 'absolute',
            left: '50%',
            bottom: '100%',
            transform: 'translate(-50%, -8px)',
            fontFamily: font.body,
            fontSize: textSize.micro,
            fontWeight: 600,
            color: '#fff',
            background: 'rgba(8,10,16,0.78)',
            borderRadius: radius.xs,
            padding: '2px 6px',
            whiteSpace: 'nowrap',
            pointerEvents: 'none',
            opacity: pillVisible ? 1 : 0,
            transition: 'opacity 120ms ease',
          }}>
            {node.name}
          </div>
          <PersonFace
            name={node.name}
            photoUrl={node.photoUrl}
            size={size}
            accent={color}
            dimmed={isQuiet(node.lastContactAt)}
            active={active}
            reducedMotion={reducedMotion}
            onClick={onOpen}
            onFocusChange={isFocused => onFocusChange(isFocused ? node.id : null)}
          />
        </div>
      </Html>
    </group>
  );
}
