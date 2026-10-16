/**
 * 3D people graph — you at the center, project clusters around you.
 *
 * Layout is a pure function (`layoutPeopleGraph` in peopleGraph.ts); this file
 * only draws it. Named *Canvas so it cannot collide with peopleGraph.ts on a
 * case-insensitive filesystem. Click a person to open the same detail panel
 * the list uses. You sit at the origin and are not clickable.
 */

import { useEffect, useMemo, useRef, useState } from 'react';
import { Canvas, useThree } from '@react-three/fiber';
import { Html, Line, OrbitControls } from '@react-three/drei';
import { apiFetch } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';
import { font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import type { DirectoryPerson } from '../projects/types';
import { ErrorBoundary } from '../common/ErrorBoundary';
import { CanvasLegend } from '../common/CanvasLegend';
import { PEOPLE_GESTURES, PEOPLE_VOCABULARY } from './peopleLegend';
import { isBridge, isYou, layoutPeopleGraph, type GraphNode } from './peopleGraph';
import { PersonFace } from './PersonFace';
import { shouldShowLabel } from './peopleFace';
import { isQuiet } from './contactAge';
import { fitPeopleGraph, GRAPH_FOV, insetsAvoiding, LABEL_LIFT, type CameraPose, type Insets, type Rect, type Viewport } from './graphFit';

/** Chrome over the canvas the graph must stay clear of: the filter field at
 *  the top (placed at `space.xl`, ~30px tall, plus a gap). The Key — chip or
 *  open panel — is measured from the DOM and carved out separately, since its
 *  size depends on whether it is open. Pixel clearances, not layout spacing. */
const FILTER_CLEARANCE_PX = 56;
const GRAPH_INSETS: Insets = {
  top: FILTER_CLEARANCE_PX, right: space.xxl, bottom: space.xxl, left: space.xxl,
};

type Status = 'loading' | 'error' | 'ready';

export function PeopleGraph() {
  const { colors, gradient, reduceMotion } = useTheme();
  const openPersonDetail = useCommandCenter(s => s.openPersonDetail);
  const personDetail = useCommandCenter(s => s.personDetail);
  const peopleRev = useCommandCenter(s => s.peopleRev);
  const [people, setPeople] = useState<DirectoryPerson[]>([]);
  const [status, setStatus] = useState<Status>('loading');
  const [query, setQuery] = useState('');
  /** The GL context was lost (or never existed): nothing can be drawn, and a
   *  blank canvas must not pass for "no people" (QA 2026-10-07 Q12). */
  const [glLost, setGlLost] = useState(false);
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

  // Filtering reveals matches in place; it never recomputes the network or camera.
  const layout = useMemo(() => layoutPeopleGraph(people), [people]);
  const visibleIds = useMemo(() => new Set(filtered.map(p => p.entity_uuid)), [filtered]);

  // The canvas's real size, so the graph is fitted to what is on screen
  // (graphFit.ts: a fixed camera clipped and stacked faces at 660x560).
  const hostRef = useRef<HTMLDivElement>(null);
  const [viewport, setViewport] = useState<Viewport | null>(null);
  useEffect(() => {
    const el = hostRef.current;
    if (!el || typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(([entry]) => {
      const { width, height } = entry.contentRect;
      if (width > 0 && height > 0) {
        setViewport(prev => (prev && prev.width === Math.round(width) && prev.height === Math.round(height))
          ? prev
          : { width: Math.round(width), height: Math.round(height) });
      }
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // The Key's real rect, measured AFTER it renders and again whenever it
  // opens, closes or resizes (opening swaps the chip for the panel, which is a
  // new element, so `onElement` fires and this re-observes). The graph is
  // fitted into what the key leaves free — nothing is drawn under it.
  const [legendEl, setLegendEl] = useState<HTMLElement | null>(null);
  const [legendRect, setLegendRect] = useState<Rect | null>(null);
  useEffect(() => {
    const host = hostRef.current;
    if (!legendEl || !host || typeof ResizeObserver === 'undefined') { setLegendRect(null); return; }
    const measure = () => {
      const h = host.getBoundingClientRect();
      const r = legendEl.getBoundingClientRect();
      const next = {
        left: Math.round(r.left - h.left), top: Math.round(r.top - h.top),
        right: Math.round(r.right - h.left), bottom: Math.round(r.bottom - h.top),
      };
      setLegendRect(prev => (prev && prev.left === next.left && prev.top === next.top
        && prev.right === next.right && prev.bottom === next.bottom) ? prev : next);
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(legendEl);
    ro.observe(host);
    return () => ro.disconnect();
  }, [legendEl]);

  const fitted = useMemo(
    () => (viewport
      ? fitPeopleGraph(layout.nodes, layout.clusters, viewport, insetsAvoiding(viewport, GRAPH_INSETS, legendRect))
      : null),
    [layout, viewport, legendRect],
  );
  const nodes = useMemo(() => layout.nodes.map(n => {
    const p = fitted?.nodes.get(n.id);
    return p ? { ...n, x: p[0], y: p[1], z: p[2] } : n;
  }), [layout.nodes, fitted]);
  const byId = useMemo(() => new Map(nodes.map(n => [n.id, n])), [nodes]);

  return (
    <div ref={hostRef} style={{ position: 'relative', width: '100%', height: '100%', background: gradient.workspace }}>
      <input
        value={query}
        onChange={e => setQuery(e.target.value)}
        placeholder="Filter people…"
        aria-label="Filter people"
        style={{
          position: 'absolute',
          top: space.xl,
          left: space.huge,
          zIndex: 2,
          width: 'min(240px, calc(100% - 64px))',
          fontSize: textSize.caption,
          fontFamily: font.body,
          padding: `${space.sm}px ${space.lg}px`,
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
        <div role="alert" style={{ position: 'absolute', inset: 0, display: 'grid', placeItems: 'center', color: colors.danger, fontFamily: font.body, fontSize: textSize.caption }}>
          Couldn't load people. Switch to List or reopen this tab to try again.
        </div>
      )}
      {status === 'ready' && people.length === 0 && (
        <div role="status" style={{ position: 'absolute', inset: 0, display: 'grid', placeItems: 'center', textAlign: 'center', color: colors.textDim, fontFamily: font.body, fontSize: textSize.caption, padding: space.xxl }}>
          No people in your directory yet. They appear here as they come up in your work.
        </div>
      )}
      {status === 'ready' && glLost && (
        <div role="alert" style={{ position: 'absolute', inset: 0, zIndex: 1, display: 'grid', placeItems: 'center', textAlign: 'center', color: colors.warning, fontFamily: font.body, fontSize: textSize.caption, padding: space.xxl }}>
          The graph can't draw here (3D graphics are unavailable or were reset). {people.length} {people.length === 1 ? 'person is' : 'people are'} in your directory; switch to List to see them.
        </div>
      )}
      {status === 'ready' && filtered.length === 0 && query.trim() !== '' && (
        <div style={{
          position: 'absolute', top: 48, left: space.huge, zIndex: 2,
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
          // Open, the key covers ~45% of a 660x560 graph: start collapsed.
          defaultOpen={false}
          onElement={setLegendEl}
          gestures={PEOPLE_GESTURES}
          vocabulary={PEOPLE_VOCABULARY}
        />
      )}
      {status === 'ready' && people.length > 0 && (
        <ErrorBoundary surface="the people graph">
        <Canvas
          onCreated={({ gl }) => {
            const el = gl.domElement;
            el.addEventListener('webglcontextlost', e => { e.preventDefault(); setGlLost(true); });
            el.addEventListener('webglcontextrestored', () => setGlLost(false));
          }}
          camera={{ position: [0, 7.5, 14], fov: GRAPH_FOV }}
          gl={{ alpha: true, antialias: true }}
          style={{ width: '100%', height: '100%', background: 'transparent' }}
          onPointerMissed={() => setHovered(null)}
        >
          <ambientLight intensity={0.55} />
          <pointLight position={[8, 10, 6]} intensity={1.1} />
          <OrbitControls enableDamping makeDefault />
          <FitCamera pose={fitted?.camera ?? null} />
          {layout.edges.map(edge => {
            const from = byId.get(edge.from);
            const to = byId.get(edge.to);
            if (!from || !to || (from.kind !== 'you' && !visibleIds.has(from.id)) || (to.kind !== 'you' && !visibleIds.has(to.id))) return null;
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
            <Html
              key={cluster.id}
              position={fitted?.labels.get(cluster.id) ?? [cluster.x, cluster.y + LABEL_LIFT, cluster.z]}
              center
              style={{ pointerEvents: 'none' }}
            >
              {/* A chip, not bare text: these labels sit in a 3D scene and
                  always draw on top, so over a face or a dense cluster the
                  letters used to land on the nodes themselves (QA 2026-09-20,
                  "HARBOURVIEW RESIDENTS' ASSOCIATION" across an avatar). The
                  World's place labels already wear the same chip. */}
              <div style={{
                fontFamily: font.mono,
                fontSize: 10,
                letterSpacing: '0.06em',
                textTransform: 'uppercase',
                color: colors.textMuted,
                whiteSpace: 'nowrap',
                padding: `${space.xxs}px ${space.sm}px`,
                borderRadius: radius.sm,
                background: colors.surface,
                border: `1px solid ${colors.border}`,
              }}>
                {cluster.name}
              </div>
            </Html>
          ))}
          {nodes.filter(node => node.kind === 'you' || visibleIds.has(node.id)).map(node => (
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
        </ErrorBoundary>
      )}
    </div>
  );
}

/** Moves the camera (and the orbit pivot) to the fitted pose when the layout
 *  or the canvas size changes. A user's own orbit stays until then. */
function FitCamera({ pose }: { pose: CameraPose | null }) {
  const camera = useThree(s => s.camera);
  const size = useThree(s => s.size);
  const controls = useThree(s => s.controls) as unknown as
    | { target: { set: (x: number, y: number, z: number) => void }; update: () => void }
    | null;
  useEffect(() => {
    if (!pose) return;
    camera.position.set(...pose.position);
    camera.lookAt(...pose.target);
    // The fit centres the graph in the rectangle the Key leaves free, which is
    // off-centre when the Key is open. Shift the projection's centre there
    // (not the camera), so orbiting still pivots on the graph itself.
    if ('setViewOffset' in camera && pose.center && size.width > 0 && size.height > 0) {
      const { width: w, height: h } = size;
      camera.setViewOffset(w, h, w / 2 - pose.center.x, h / 2 - pose.center.y, w, h);
      camera.updateProjectionMatrix();
    }
    if (controls) {
      controls.target.set(...pose.target);
      controls.update();
    }
  }, [pose, camera, controls, size]);
  return null;
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
  const nodeRadius = you ? 0.32 : node.relationshipScore !== undefined
    ? 0.16 + 0.08 * Math.max(0, Math.min(100, node.relationshipScore)) / 100
    : bridge ? 0.22 : 0.16;
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
            padding: `${space.xxs}px ${space.sm}px`,
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
            padding: `${space.xxs}px ${space.sm}px`,
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
