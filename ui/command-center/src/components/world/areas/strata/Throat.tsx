// The Throat — the Brain Archive's descending abyss.
// THE_CAVE_vision_bible.md §2/§5: "entered on a bridge over the descending abyss …
// memory-stacks giving way to raw rock as you descend: geological memory." The bore
// is clean near the crown and degrades into raw rock at THROAT.rawRockY.
// W1 owns geometry only — the violet memory glow + recall motes are W4/W2/W3.
// Gray blockout. Real scale.

import { useMemo } from 'react';
import * as THREE from 'three';
import { ENV } from '../../shared/palette';
import { blockoutMat } from '../blockout';
import { InstancedProp, type InstanceTransform } from '../../shared/instancing';
import { THROAT, CAVE_FLOOR } from './strata';

// DoubleSide shaft wall material so the bored throat reads from inside the abyss
// and from the bridge looking down. Stone tier, matte. Module singleton.
const shaftMat = new THREE.MeshStandardMaterial({
  color: ENV.darkStone,
  roughness: 0.95,
  metalness: 0.03,
  side: THREE.DoubleSide,
});

// Shared ledge geometry — shelf-rings descending the bored upper shaft (bible §5
// "newest memories near the surface, oldest fading into raw rock below"). Placed
// many times → one InstancedMesh.
const ledgeGeo = new THREE.BoxGeometry(1, 0.3, 2.4);
// Raw-rock teeth for the lower abyss.
const toothGeo = new THREE.ConeGeometry(1, 2.6, 5);

/** The bored shaft wall (clean upper) + raw-rock funnel (lower). */
function ThroatShaft() {
  const [cx, , cz] = THROAT.center;
  const topY = -2;
  const boreHeight = topY - THROAT.rawRockY;
  const rawHeight = THROAT.rawRockY - CAVE_FLOOR;
  return (
    <group position={[cx, 0, cz]}>
      {/* Clean bored shaft (crown → rawRockY): open cylinder, smooth radius. */}
      <mesh position={[0, (topY + THROAT.rawRockY) / 2, 0]} material={shaftMat} receiveShadow>
        <cylinderGeometry
          args={[THROAT.radiusTop, THROAT.radiusTop + 0.5, boreHeight, 20, 1, true]}
        />
      </mesh>
      {/* Raw-rock funnel (rawRockY → floor): widens, jagged segment count. */}
      <mesh
        position={[0, (THROAT.rawRockY + CAVE_FLOOR) / 2, 0]}
        material={shaftMat}
        receiveShadow
      >
        <cylinderGeometry
          args={[THROAT.radiusBottom, THROAT.radiusTop, rawHeight, 9, 1, true]}
        />
      </mesh>
    </group>
  );
}

/** Descending shelf-rings of memory-stacks in the clean upper bore (instanced). */
function ShelfRings() {
  const [cx, , cz] = THROAT.center;
  const transforms = useMemo<InstanceTransform[]>(() => {
    const t: InstanceTransform[] = [];
    const topY = -3;
    const perRing = 12;
    // Rings every ~1.5m from crown down to where raw rock begins.
    for (let y = topY; y > THROAT.rawRockY; y -= 1.5) {
      const ringR = THROAT.radiusTop - 0.4;
      for (let i = 0; i < perRing; i++) {
        const ang = (i / perRing) * Math.PI * 2;
        t.push({
          position: [cx + Math.cos(ang) * ringR, y, cz + Math.sin(ang) * ringR],
          rotation: [0, -ang, 0],
          scale: [0.9, 1, 1],
        });
      }
    }
    return t;
  }, [cx, cz]);

  return (
    <InstancedProp
      name="throat.shelfrings"
      geometry={ledgeGeo}
      material={blockoutMat}
      transforms={transforms}
      castShadow
      receiveShadow
    />
  );
}

/** Raw-rock teeth ringing the funnel where the bore gives way to raw rock. */
function RawRockTeeth() {
  const [cx, , cz] = THROAT.center;
  const transforms = useMemo<InstanceTransform[]>(() => {
    const t: InstanceTransform[] = [];
    let seed = 71.3;
    const rnd = () => {
      seed = (seed * 9301 + 49297) % 233280;
      return seed / 233280;
    };
    const count = 18;
    for (let i = 0; i < count; i++) {
      const ang = (i / count) * Math.PI * 2 + rnd() * 0.2;
      const y = THROAT.rawRockY - rnd() * (THROAT.rawRockY - CAVE_FLOOR) * 0.8;
      const r = THROAT.radiusTop + rnd() * 1.5;
      const size = 0.8 + rnd() * 1.4;
      t.push({
        position: [cx + Math.cos(ang) * r, y, cz + Math.sin(ang) * r],
        rotation: [Math.PI + (rnd() - 0.5) * 0.6, ang, (rnd() - 0.5) * 0.4],
        scale: [size, size * (1 + rnd()), size],
      });
    }
    return t;
  }, [cx, cz]);

  return (
    <InstancedProp
      name="throat.rawteeth"
      geometry={toothGeo}
      material={blockoutMat}
      transforms={transforms}
      receiveShadow
    />
  );
}

/**
 * The entry bridge over the abyss (bible §5). A simple gray span from the Brain
 * threshold side (south, +z) out over the throat lip. Detail/railings = W2.
 */
function AbyssBridge() {
  const [cx, , cz] = THROAT.center;
  return (
    <mesh position={[cx, -2.4, cz + THROAT.radiusTop + 1]} material={blockoutMat} castShadow receiveShadow>
      <boxGeometry args={[3, 0.4, 5]} />
    </mesh>
  );
}

export function Throat() {
  return (
    <group>
      <ThroatShaft />
      <ShelfRings />
      <RawRockTeeth />
      <AbyssBridge />
    </group>
  );
}
