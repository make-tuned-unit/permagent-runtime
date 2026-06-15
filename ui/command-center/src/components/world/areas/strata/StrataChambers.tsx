// Strata chambers — the vertical cavern walls beneath the crown.
// THE_CAVE_vision_bible.md §2 "depth is time", §9 W1 scope. Gray blockout only;
// the material gradient (mature stone high → feral raw rock low) is the W2/W4
// detail pass, gated on Jesse's blockout review. Real scale, 1u = 1m.
//
// Geometry budget: each stratum's cavern wall is one open cylinder shell (the
// rock face). The ragged rock chunks studding the walls are placed many times per
// stratum, so they are a single InstancedMesh (bible §8: instancing MANDATORY > 2×).

import { useMemo } from 'react';
import * as THREE from 'three';
import { blockoutMat } from '../blockout';
import { InstancedProp, type InstanceTransform } from '../../shared/instancing';
import { STRATA, type StratumDef } from './strata';

// A rough rock-chunk geometry shared by every stratum's wall studs (low-poly,
// chamfered mass per bible §1 silhouette law). Module singleton — one geometry.
const rockChunk = new THREE.IcosahedronGeometry(1, 0);

/** Open cylinder shell = the cavern rock wall of one stratum (1 draw call). */
function StratumWall({ s }: { s: StratumDef }) {
  const height = s.top - s.floor;
  const midY = (s.top + s.floor) / 2;
  // Throat centre is at z=9; the cavern is centred there too so the walls wrap
  // the descending abyss.
  return (
    <mesh position={[0, midY, 9]} material={blockoutMat} receiveShadow>
      <cylinderGeometry args={[s.radius, s.radius + 1, height, 24, 1, true]} />
    </mesh>
  );
}

/**
 * Rough rock studs around a stratum wall — denser and more chaotic the deeper
 * (rawer) the stratum. One InstancedMesh per stratum. Seeded deterministically so
 * the blockout is stable across reloads (no per-frame work, all built in useMemo).
 */
function StratumStuds({ s }: { s: StratumDef }) {
  const transforms = useMemo<InstanceTransform[]>(() => {
    const t: InstanceTransform[] = [];
    // Rawer strata (low maturity) get more, larger, more jagged studs.
    const count = Math.round(10 + (1 - s.maturity) * 22);
    const height = s.top - s.floor;
    let seed = s.floor * 13.37 + s.radius; // deterministic pseudo-random
    const rnd = () => {
      seed = (seed * 9301 + 49297) % 233280;
      return seed / 233280;
    };
    for (let i = 0; i < count; i++) {
      const ang = rnd() * Math.PI * 2;
      const y = s.floor + rnd() * height;
      const r = s.radius - 0.3;
      const size = 0.5 + rnd() * (0.8 + (1 - s.maturity) * 1.6);
      t.push({
        position: [Math.cos(ang) * r, y, 9 + Math.sin(ang) * r],
        rotation: [rnd() * Math.PI, rnd() * Math.PI, rnd() * Math.PI],
        scale: [size, size * (0.7 + rnd() * 0.6), size],
      });
    }
    return t;
  }, [s]);

  return (
    <InstancedProp
      name={`strata.${s.id}.studs`}
      geometry={rockChunk}
      material={blockoutMat}
      transforms={transforms}
      receiveShadow
    />
  );
}

/** All five strata walls + their instanced rock studs. */
export function StrataChambers() {
  return (
    <group>
      {STRATA.map((s) => (
        <group key={s.id}>
          <StratumWall s={s} />
          <StratumStuds s={s} />
        </group>
      ))}
    </group>
  );
}
