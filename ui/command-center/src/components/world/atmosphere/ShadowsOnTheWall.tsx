// W4 atmosphere — SHADOWS ON THE WALL, the day-one state (THE CAVE bible §2).
//
// "The Turn. A new user's world opens *facing the cave wall*: darkness, wet
//  stone, and their first few memories cast as faint moving shadow-silhouettes
//  (an honest render of a near-empty Brain — these are real entities,
//  shadow-projected). The light casting the shadows comes from the Mouth, behind
//  the camera, not yet visible." (§2)
//
// HONESTY (bible §8): the silhouettes are REAL entities from the Brain graph
// (atmosphere/worldSignals → /api/brain/graph). A near-empty Brain casts few
// faint shapes; an empty Brain casts NONE (honest darkness — we do NOT invent
// shadows). Silhouette shape is derived from the entity TYPE so the projection
// is legibly the user's own first fragments, not decoration.
//
// THE TURN (camera): this also exposes a day-one "turn" framing accessor — a
// camera pose facing the wall (the Mouth behind the lens). The camera lane
// (camera/) consumes the pose; this module owns the wall + the projection.
//
// W1 DEPENDENCY (noted): the literal cave wall is W1's deepest-stratum geometry,
// not yet merged. STAGED against bible geography: until the strata land, the wall
// renders as a tall dark slab on the far -z side of the hall (the throat drops
// behind it). When W1's wall lands, WALL_POS/normal plug into its surface.
//
// LAW: bible §8 — zero per-frame allocations; reduceMotion → static silhouettes
// (no drift, no flicker). InstancedMesh would force one shared silhouette shape,
// but type-driven shapes need per-entity geometry; the cap (≤ MAX_SILHOUETTES)
// keeps this well inside the draw-call budget without instancing.

import { useMemo, useRef } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import { ENV } from '../shared/palette';
import { getReduceMotion } from '../../../styles/tokens';
import { useWorldSignals, type SignalEntity } from './worldSignals';

const MAX_SILHOUETTES = 8; // a near-empty Brain shows few; budget-safe without instancing

// Staged wall geography: far -z, tall and dark (the §2 "wet stone" the camera
// faces on day one). A light temperature region, not a palette accent.
const WALL_POS: [number, number, number] = [0, 6, -16];
const WALL_W = 28;
const WALL_H = 14;
const SHADOW_COLOR = '#05070C'; // darker than the deepVoid bg — a cast shadow

// THE TURN — the day-one camera pose: low, facing the wall (-z), the Mouth
// (and its blade) behind the lens (+z, high). The camera lane reads this.
export const TURN_CAMERA = {
  position: [0, 3.5, 6] as [number, number, number],
  lookAt: [0, 5, -16] as [number, number, number],
};

/** Map an entity type to a primitive silhouette shape. Real shape, not decor. */
function silhouetteGeometry(type: string): THREE.BufferGeometry {
  switch ((type || '').toLowerCase()) {
    case 'person':
    case 'user':
    case 'agent':
      // Upright figure — a rounded capsule.
      return new THREE.CapsuleGeometry(0.55, 1.6, 4, 12);
    case 'project':
    case 'goal':
    case 'task':
      // Standing slab — work.
      return new THREE.BoxGeometry(1.3, 2.0, 0.2);
    case 'tool':
    case 'skill':
      // Angular instrument — a tilted prism.
      return new THREE.ConeGeometry(0.8, 2.0, 5);
    case 'place':
    case 'location':
      return new THREE.CylinderGeometry(0.9, 1.1, 1.8, 8);
    default:
      // Unknown fragment — a soft blob.
      return new THREE.SphereGeometry(0.9, 12, 10);
  }
}

interface SilhouetteProps {
  entity: SignalEntity;
  index: number;
  total: number;
  reduceMotion: boolean;
}

/**
 * One real entity, projected as a flat dark silhouette in front of the wall, with
 * a faint slow sway (a flicker on the wall, §2) — static under reduceMotion. The
 * silhouette sits just off the wall and is flattened in z so it reads as a cast
 * shadow, not a floating object.
 */
function Silhouette({ entity, index, total, reduceMotion }: SilhouetteProps) {
  const ref = useRef<THREE.Mesh>(null);
  const geo = useMemo(() => silhouetteGeometry(entity.type), [entity.type]);

  // Spread silhouettes across the wall width; deterministic from index.
  const baseX = useMemo(() => {
    if (total <= 1) return 0;
    const t = index / (total - 1); // 0..1
    return (t - 0.5) * (WALL_W - 8);
  }, [index, total]);
  const baseY = useMemo(() => 4.5 + ((index * 37) % 5) - 2, [index]);
  const phase = useMemo(() => (index * 1.7) % (Math.PI * 2), [index]);

  useFrame(() => {
    const mesh = ref.current;
    if (!mesh) return;
    if (reduceMotion) {
      mesh.position.x = baseX;
      return;
    }
    // Faint moving shadow — a slow lateral drift + breath, like firelight.
    const t = performance.now() * 0.0004;
    mesh.position.x = baseX + Math.sin(t + phase) * 0.4;
    mesh.position.y = baseY + Math.sin(t * 0.7 + phase) * 0.15;
  });

  return (
    <mesh
      ref={ref}
      geometry={geo}
      position={[baseX, baseY, WALL_POS[2] + 0.3]}
      scale={[1, 1, 0.05]} // flattened toward the wall → reads as a shadow
    >
      <meshBasicMaterial
        color={SHADOW_COLOR}
        transparent
        opacity={0.82}
        depthWrite={false}
      />
    </mesh>
  );
}

/** The dark cave wall the silhouettes fall on (staged; W1 wall plugs in later). */
function CaveWall() {
  return (
    <mesh position={WALL_POS} receiveShadow>
      <planeGeometry args={[WALL_W, WALL_H]} />
      <meshStandardMaterial color={ENV.darkStone} roughness={0.95} metalness={0.02} />
    </mesh>
  );
}

/**
 * Shadows-on-the-wall: the dark wall + a faint silhouette per real Brain entity
 * (capped). An empty Brain shows the bare wall and nothing else (honest day-one
 * darkness). Mounted from Atmosphere.tsx. Re-renders only when the real entity
 * set changes (slow graph poll).
 */
export function ShadowsOnTheWall() {
  const reduceMotion = useMemo(() => getReduceMotion(), []);
  const { entities } = useWorldSignals();
  const shown = entities.slice(0, MAX_SILHOUETTES);

  return (
    <>
      <CaveWall />
      {shown.map((e, i) => (
        <Silhouette
          key={e.id}
          entity={e}
          index={i}
          total={shown.length}
          reduceMotion={reduceMotion}
        />
      ))}
    </>
  );
}
