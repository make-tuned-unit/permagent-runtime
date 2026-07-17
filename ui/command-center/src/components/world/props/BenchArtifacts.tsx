// BenchArtifacts — real completions leave work-stones on the bay benches.
//
// Driven ONLY by live task_completed / task_failed events (agents/
// taskArtifacts.ts — replayed history filtered at the wire). A completed
// task sets a small stone on a bench slot glowing warm amber; it cools to
// plain stone over minutes and finally leaves. A failed task is a red ember
// that dies quickly. The bench starts every session honestly empty — every
// stone you can see is something that actually got done while you watched.
//
// BUDGET: 3 status buckets (glowing / cooled / ember) × instanced = 3 draw
// calls, no lights. LAW: no per-frame work at all — buckets recompute on a
// slow 2s interval + on real events, as plain React state.

import { useEffect, useState } from 'react';
import { InstancedProp, type InstanceTransform } from '../shared/instancing';
import { unitBox, hash01 } from './geometries';
import { lightAmberWork, lightErrorTick, stoneDark } from './materials';
import {
  artifactVisual,
  getArtifacts,
  subscribeArtifacts,
  sweepArtifacts,
  ARTIFACT_CAP,
} from '../agents/taskArtifacts';

// The bay's world placement (must match WorldScene's WorkstationCluster).
const ORIGIN: [number, number, number] = [0, 0, -11.4];
const ROT_Y = Math.PI;
const BENCH_TOP_Y = 0.98 + 0.08; // bench surface + half stone height

// One stone slot per bay station (2 rows × 6 across = ARTIFACT_CAP), placed
// beside each terminal so stones read as "set down at the workplace".
const SLOT_LOCAL: [number, number][] = (() => {
  const out: [number, number][] = [];
  for (const z of [0, -3.0]) {
    for (const x of [-3.0, -1.8, -0.6, 0.6, 1.8, 3.0]) {
      out.push([x, z + 0.32]);
    }
  }
  return out.slice(0, ARTIFACT_CAP);
})();

function slotWorld(i: number): [number, number, number] {
  const [lx, lz] = SLOT_LOCAL[i % SLOT_LOCAL.length];
  const cos = Math.cos(ROT_Y);
  const sin = Math.sin(ROT_Y);
  return [ORIGIN[0] + lx * cos + lz * sin, BENCH_TOP_Y, ORIGIN[2] - lx * sin + lz * cos];
}

interface Buckets {
  glowing: InstanceTransform[];
  cooled: InstanceTransform[];
  embers: InstanceTransform[];
}

/** Cheap deterministic string seed (for stable slot/rotation per task id). */
function strSeed(id: string): number {
  let h = 0;
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) >>> 0;
  return h % 9973;
}

function computeBuckets(now: number): Buckets {
  const glowing: InstanceTransform[] = [];
  const cooled: InstanceTransform[] = [];
  const embers: InstanceTransform[] = [];
  const artifacts = getArtifacts();
  for (let i = 0; i < artifacts.length; i++) {
    const a = artifacts[i];
    const v = artifactVisual(a, now);
    const seed = strSeed(a.id);
    // Stable slot from the id; index offset walks past collisions.
    const slot = (Math.floor(hash01(seed) * SLOT_LOCAL.length) + i) % SLOT_LOCAL.length;
    const s = 0.13 + hash01(seed + 7) * 0.05;
    const t: InstanceTransform = {
      position: slotWorld(slot),
      rotation: [0, hash01(seed + 13) * Math.PI, 0],
      scale: [s, 0.16 * (1 - v.age01 * 0.25), s],
    };
    if (a.kind === 'failed') embers.push(t);
    else if (v.glow > 0.25) glowing.push(t);
    else cooled.push(t);
  }
  return { glowing, cooled, embers };
}

export function BenchArtifacts() {
  const [buckets, setBuckets] = useState<Buckets>(() => computeBuckets(Date.now()));

  useEffect(() => {
    const recompute = () => setBuckets(computeBuckets(Date.now()));
    const unsub = subscribeArtifacts(recompute);
    // Slow aging tick: re-bucket + expire (glow cooling is minutes-scale).
    const t = setInterval(() => {
      sweepArtifacts(Date.now());
      recompute();
    }, 2000);
    return () => {
      unsub();
      clearInterval(t);
    };
  }, []);

  const { glowing, cooled, embers } = buckets;

  return (
    <group>
      {glowing.length > 0 && (
        <InstancedProp name="bay.stone.glowing" geometry={unitBox} material={lightAmberWork} transforms={glowing} />
      )}
      {cooled.length > 0 && (
        <InstancedProp name="bay.stone.cooled" geometry={unitBox} material={stoneDark} transforms={cooled} />
      )}
      {embers.length > 0 && (
        <InstancedProp name="bay.stone.ember" geometry={unitBox} material={lightErrorTick} transforms={embers} />
      )}
    </group>
  );
}
