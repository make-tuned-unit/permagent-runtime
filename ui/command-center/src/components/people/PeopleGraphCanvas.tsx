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
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import type { DirectoryPerson } from '../projects/types';
import { isBridge, isYou, layoutPeopleGraph, type GraphNode } from './peopleGraph';
import { PersonFace } from './PersonFace';
import { isQuiet } from './contactAge';

type Status = 'loading' | 'error' | 'ready';

export function PeopleGraph() {
  const { colors, gradient } = useTheme();
  const openPersonDetail = useCommandCenter(s => s.openPersonDetail);
  const peopleRev = useCommandCenter(s => s.peopleRev);
  const [people, setPeople] = useState<DirectoryPerson[]>([]);
  const [status, setStatus] = useState<Status>('loading');
  const [query, setQuery] = useState('');
  const [hovered, setHovered] = useState<string | null>(null);

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
          fontSize: 12,
          fontFamily: font.body,
          padding: '6px 10px',
          borderRadius: 6,
          border: `1px solid ${colors.border}`,
          background: colors.inputBg,
          color: colors.text,
          outline: 'none',
        }}
      />
      {status === 'loading' && (
        <div style={{ position: 'absolute', inset: 0, display: 'grid', placeItems: 'center', color: colors.textDim, fontFamily: font.body, fontSize: 12 }}>
          Loading people…
        </div>
      )}
      {status === 'error' && (
        <div style={{ position: 'absolute', inset: 0, display: 'grid', placeItems: 'center', color: colors.danger, fontFamily: font.body, fontSize: 12 }}>
          Couldn't load people.
        </div>
      )}
      {status === 'ready' && filtered.length === 0 && query.trim() !== '' && (
        <div style={{
          position: 'absolute', top: 48, left: 24, zIndex: 2,
          color: colors.textDim, fontFamily: font.body, fontSize: 12,
        }}>
          No people match that search.
        </div>
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
            return (
              <Line
                key={`${edge.from}|${edge.to}|${edge.via}`}
                points={[[from.x, from.y, from.z], [to.x, to.y, to.z]]}
                color={ego ? colors.cyan : colors.textMuted}
                transparent
                opacity={ego ? 0.4 : 0.18}
                lineWidth={ego ? 1.4 : 1}
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
              onHover={setHovered}
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
  onHover,
  onOpen,
}: {
  node: GraphNode;
  accent: string;
  muted: string;
  hovered: boolean;
  onHover: (id: string | null) => void;
  onOpen?: () => void;
}) {
  const you = isYou(node);
  const bridge = isBridge(node);
  const color = you || bridge || hovered ? accent : muted;
  const showLabel = you || hovered || bridge;
  const radius = you ? 0.32 : bridge ? 0.22 : 0.16;
  if (you) {
    return (
      <group position={[node.x, node.y, node.z]}>
        <mesh
          onPointerOver={e => { e.stopPropagation(); onHover(node.id); }}
          onPointerOut={() => onHover(null)}
        >
          <sphereGeometry args={[radius, 24, 24]} />
          <meshStandardMaterial color={color} emissive={color} emissiveIntensity={0.55} />
        </mesh>
        <Html center sprite style={{ pointerEvents: 'none', transform: 'translateY(-18px)' }}>
          <div style={{
            fontFamily: font.body,
            fontSize: 11,
            fontWeight: 600,
            color: '#fff',
            background: 'rgba(8,10,16,0.78)',
            borderRadius: 4,
            padding: '2px 6px',
            whiteSpace: 'nowrap',
          }}>
            {node.name}
          </div>
        </Html>
      </group>
    );
  }
  const size = hovered || bridge ? 48 : 40;
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
        >
          <PersonFace
            name={node.name}
            photoUrl={node.photoUrl}
            size={size}
            accent={color}
            dimmed={isQuiet(node.lastContactAt)}
            onClick={onOpen}
          />
        </div>
      </Html>
      {showLabel && (
        <Html center sprite style={{ pointerEvents: 'none', transform: 'translateY(-36px)' }}>
          <div style={{
            fontFamily: font.body,
            fontSize: 11,
            fontWeight: 600,
            color: '#fff',
            background: 'rgba(8,10,16,0.78)',
            borderRadius: 4,
            padding: '2px 6px',
            whiteSpace: 'nowrap',
          }}>
            {node.name}
          </div>
        </Html>
      )}
    </group>
  );
}
