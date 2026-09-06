// Cyborg rig builder — WORLD_VIEW_BIBLE.md §4 + §8.
// The previous CyborgCharacterModel drew ~65 meshes per agent. This builder merges
// the same silhouette into 5 skinned draw calls per agent (6 for Henry's crown):
//   1. metal  — body/joints/accents, vertex-colored (gunmetal / dark / bronze)
//   2. trim   — IDENTITY emissive channels (toga trim family; never state-colored)
//   3. state  — state channels: joint glow rings + cape circuit lines + feet aura (+ crown gems)
//   4. visor  — state channel with independent intensity (dim/bright/full/flicker)
//   5. cape   — translucent fabric, vertex-alpha fade
// Articulation: 12 rigidly-skinned bones (root/spine/head/arms/forearms/thighs/calves/aura)
// posed per poses.ts. Geometry is built once per variant and cached for the app's
// lifetime (no per-mount allocation, no leak).

import * as THREE from 'three';
import { mergeGeometries } from 'three/examples/jsm/utils/BufferGeometryUtils.js';
import { ENV } from '../shared/palette';
import { BONE_NAMES, type BoneName } from './poses';

// ── Bones ────────────────────────────────────────────────────────────

const BONE_INDEX: Record<BoneName, number> = Object.fromEntries(
  BONE_NAMES.map((n, i) => [n, i]),
) as Record<BoneName, number>;

/** Absolute bind positions (model space, 1u = 1m, agent ≈ 2.4u tall). */
const BONE_BIND: Record<BoneName, [number, number, number]> = {
  root: [0, 0, 0],
  spine: [0, 0.85, 0],
  head: [0, 1.78, 0],
  armL: [0.3, 1.5, 0],
  foreL: [0.42, 1.04, 0],
  armR: [-0.3, 1.5, 0],
  foreR: [-0.42, 1.04, 0],
  thighL: [0.12, 0.66, 0],
  calfL: [0.12, 0.18, 0],
  thighR: [-0.12, 0.66, 0],
  calfR: [-0.12, 0.18, 0],
  aura: [0, 0.02, 0],
};

const BONE_PARENT: Record<BoneName, BoneName | null> = {
  root: null,
  spine: 'root',
  head: 'spine',
  armL: 'spine',
  foreL: 'armL',
  armR: 'spine',
  foreR: 'armR',
  thighL: 'root',
  calfL: 'thighL',
  thighR: 'root',
  calfR: 'thighR',
  aura: 'root',
};

export interface RigBones {
  list: THREE.Bone[];
  byName: Record<BoneName, THREE.Bone>;
}

export function buildBones(): RigBones {
  const byName = {} as Record<BoneName, THREE.Bone>;
  const list: THREE.Bone[] = [];
  for (const name of BONE_NAMES) {
    const b = new THREE.Bone();
    b.name = name;
    byName[name] = b;
    list.push(b);
  }
  for (const name of BONE_NAMES) {
    const parent = BONE_PARENT[name];
    const abs = BONE_BIND[name];
    if (parent) {
      const p = BONE_BIND[parent];
      byName[name].position.set(abs[0] - p[0], abs[1] - p[1], abs[2] - p[2]);
      byName[parent].add(byName[name]);
    } else {
      byName[name].position.set(abs[0], abs[1], abs[2]);
    }
  }
  byName.root.updateMatrixWorld(true);
  return { list, byName };
}

/** Standing root-bone bind Y — pose rootY offsets are relative to this. */
export const ROOT_BIND_Y = 0;

// ── Geometry chunks ──────────────────────────────────────────────────

interface ChunkSpec {
  geo: THREE.BufferGeometry;
  p?: [number, number, number];
  r?: [number, number, number];
  s?: [number, number, number];
  bone: BoneName;
  color?: THREE.Color;
}

const tmpMat4 = new THREE.Matrix4();
const tmpQuat = new THREE.Quaternion();
const tmpEuler = new THREE.Euler();
const tmpPos = new THREE.Vector3();
const tmpScale = new THREE.Vector3();

function prepChunk(spec: ChunkSpec, withColor: boolean): THREE.BufferGeometry {
  const geo = spec.geo;
  tmpPos.set(...(spec.p ?? [0, 0, 0]));
  tmpEuler.set(...(spec.r ?? [0, 0, 0]));
  tmpQuat.setFromEuler(tmpEuler);
  tmpScale.set(...(spec.s ?? [1, 1, 1]));
  tmpMat4.compose(tmpPos, tmpQuat, tmpScale);
  geo.applyMatrix4(tmpMat4);

  const n = geo.attributes.position.count;
  const boneIdx = BONE_INDEX[spec.bone];
  const si = new Uint16Array(n * 4);
  const sw = new Float32Array(n * 4);
  for (let i = 0; i < n; i++) {
    si[i * 4] = boneIdx;
    sw[i * 4] = 1;
  }
  geo.setAttribute('skinIndex', new THREE.BufferAttribute(si, 4));
  geo.setAttribute('skinWeight', new THREE.BufferAttribute(sw, 4));

  if (withColor) {
    const c = spec.color ?? new THREE.Color(1, 1, 1);
    const col = new Float32Array(n * 3);
    for (let i = 0; i < n; i++) {
      col[i * 3] = c.r;
      col[i * 3 + 1] = c.g;
      col[i * 3 + 2] = c.b;
    }
    geo.setAttribute('color', new THREE.BufferAttribute(col, 3));
  }
  return geo;
}

function mergeChunks(specs: ChunkSpec[], withColor: boolean): THREE.BufferGeometry {
  const merged = mergeGeometries(
    specs.map((s) => prepChunk(s, withColor)),
    false,
  );
  if (!merged) throw new Error('cyborg rig: geometry merge failed');
  specs.forEach((s) => s.geo.dispose());
  merged.computeBoundingSphere();
  return merged;
}

// ── Cape drape (ported from the original CyborgCharacter) ────────────

function bodyBackZ(absY: number): number {
  const relY = absY - 0.68;
  const prof: [number, number][] = [
    [0.0, 0.18],
    [0.05, 0.17],
    [0.18, 0.14],
    [0.3, 0.16],
    [0.55, 0.23],
    [0.68, 0.25],
    [0.78, 0.24],
    [0.88, 0.2],
    [0.98, 0.14],
    [1.05, 0.1],
  ];
  if (relY <= 0) return -prof[0][1];
  if (relY >= prof[prof.length - 1][0]) return -prof[prof.length - 1][1];
  for (let i = 0; i < prof.length - 1; i++) {
    if (relY >= prof[i][0] && relY <= prof[i + 1][0]) {
      const t = (relY - prof[i][0]) / (prof[i + 1][0] - prof[i][0]);
      return -(prof[i][1] + t * (prof[i + 1][1] - prof[i][1]));
    }
  }
  return -0.18;
}

const CAPE_TOP = 1.75;
const CAPE_BOT = 0.3;
const CAPE_LEN = CAPE_TOP - CAPE_BOT;

function capePos(u: number, v: number): [number, number, number] {
  const y = CAPE_TOP - v * CAPE_LEN;
  const neckW = 0.22;
  const shoulderW = 0.7;
  const hemW = 0.85;
  let width: number;
  if (v < 0.08) {
    const t = v / 0.08;
    width = neckW + (shoulderW - neckW) * t * t;
  } else {
    width = shoulderW + (hemW - shoulderW) * ((v - 0.08) / 0.92);
  }
  const x = (u - 0.5) * width;

  const bodyFollowZ = bodyBackZ(y) - 0.03;
  const freeHangZ = -0.28;
  const waistV = 0.55;
  let z: number;
  if (v < waistV) z = Math.min(bodyFollowZ, freeHangZ);
  else z = freeHangZ;

  const edgeness = Math.abs(u - 0.5) * 2;
  z += Math.max(0, 1 - v * 8) * edgeness * 0.12;
  return [x, y, z];
}

function buildCapeGeometry(): THREE.BufferGeometry {
  const widthSegs = 14;
  const heightSegs = 20;
  const geo = new THREE.PlaneGeometry(1, 1, widthSegs, heightSegs);
  const pos = geo.attributes.position;
  const colors = new Float32Array(pos.count * 3);
  for (let i = 0; i < pos.count; i++) {
    const u = pos.getX(i) + 0.5;
    const v = pos.getY(i) + 0.5;
    const [cx, cy, cz] = capePos(u, v);
    pos.setXYZ(i, cx, cy, cz);
    const alpha = 1.0 - v * 0.55;
    colors[i * 3] = alpha;
    colors[i * 3 + 1] = alpha;
    colors[i * 3 + 2] = alpha;
  }
  pos.needsUpdate = true;
  geo.setAttribute('color', new THREE.BufferAttribute(colors, 3));
  geo.computeVertexNormals();
  return geo;
}

function capeCurveTube(
  points: [number, number][], // [u, v] samples
  radius: number,
  tubularSegs: number,
): THREE.BufferGeometry {
  const pts = points.map(([u, v]) => new THREE.Vector3(...capePos(u, v)));
  return new THREE.TubeGeometry(new THREE.CatmullRomCurve3(pts), tubularSegs, radius, 4, false);
}

function sampleLine(fixed: number, along: 'u' | 'v', segs: number): [number, number][] {
  const out: [number, number][] = [];
  for (let i = 0; i <= segs; i++) {
    const t = i / segs;
    out.push(along === 'u' ? [t, fixed] : [fixed, t]);
  }
  return out;
}

// ── Chunk tables ─────────────────────────────────────────────────────

const cyl = (rt: number, rb: number, h: number, seg: number) =>
  new THREE.CylinderGeometry(rt, rb, h, seg);
const torus = (r: number, t: number, rs: number, ts: number) =>
  new THREE.TorusGeometry(r, t, rs, ts);
const box = (x: number, y: number, z: number) => new THREE.BoxGeometry(x, y, z);
const sphere = (r: number, w: number, h: number) => new THREE.SphereGeometry(r, w, h);

function torsoLathe(): THREE.BufferGeometry {
  const profile: [number, number][] = [
    [0.18, 0.0],
    [0.17, 0.05],
    [0.14, 0.18],
    [0.16, 0.3],
    [0.23, 0.55],
    [0.25, 0.68],
    [0.24, 0.78],
    [0.2, 0.88],
    [0.14, 0.98],
    [0.1, 1.05],
  ];
  return new THREE.LatheGeometry(
    profile.map(([x, y]) => new THREE.Vector2(x, y)),
    16,
  );
}

/** Rig body variant — each real agent gets a signature silhouette. */
export type RigVariant =
  | 'henry'
  | 'librarian'
  | 'reader'
  | 'watcher'
  | 'steward'
  | 'strix'
  | 'financier'
  | null;

/**
 * Signature gear per agent (visual overhaul 2026-07-28): every agent shared an
 * identical body, so the roster read as clones. Each variant now carries
 * recognizable equipment merged into the SAME channel draw calls (zero extra
 * cost): Henry a presiding collar + chest core, the Librarian a satchel and
 * spine-rack of books, the Reader a scanning arc + cyclops lens, the Watcher a
 * vigil antenna + extra eyes, the Steward a groundskeeper yoke + key ring.
 */
function gearChunks(
  variant: RigVariant,
  GUN: THREE.Color,
  DARK: THREE.Color,
  BRONZE: THREE.Color,
): { metal: ChunkSpec[]; trim: ChunkSpec[]; state: ChunkSpec[]; visor: ChunkSpec[] } {
  const metal: ChunkSpec[] = [];
  const trim: ChunkSpec[] = [];
  const state: ChunkSpec[] = [];
  const visor: ChunkSpec[] = [];

  switch (variant) {
    case 'henry':
      // Presiding high-collar fins behind the neck + gold epaulette bars.
      metal.push(
        { geo: box(0.05, 0.2, 0.04), p: [0.15, 1.7, -0.13], r: [0.15, 0, -0.3], bone: 'spine', color: DARK },
        { geo: box(0.05, 0.2, 0.04), p: [-0.15, 1.7, -0.13], r: [0.15, 0, 0.3], bone: 'spine', color: DARK },
        { geo: box(0.16, 0.025, 0.09), p: [0.32, 1.62, 0], r: [0, 0, -0.12], bone: 'armL', color: BRONZE },
        { geo: box(0.16, 0.025, 0.09), p: [-0.32, 1.62, 0], r: [0, 0, 0.12], bone: 'armR', color: BRONZE },
      );
      // Chest core — reads his state across the room (crown-gem crossover family).
      state.push({ geo: sphere(0.045, 12, 10), p: [0, 1.36, 0.235], bone: 'spine' });
      trim.push({ geo: torus(0.065, 0.01, 4, 16), p: [0, 1.36, 0.235], bone: 'spine' });
      break;

    case 'librarian': {
      // Hip satchel + chest strap, and a spine-rack of three book slabs.
      metal.push(
        { geo: box(0.17, 0.13, 0.07), p: [0.24, 0.8, -0.08], r: [0, 0.25, -0.08], bone: 'spine', color: DARK },
        { geo: box(0.18, 0.035, 0.075), p: [0.24, 0.875, -0.08], r: [0, 0.25, -0.08], bone: 'spine', color: BRONZE },
      );
      trim.push({ geo: cyl(0.012, 0.012, 0.78, 5), p: [-0.02, 1.2, 0.21], r: [0, 0, 0.5], bone: 'spine' });
      const bookCols = [BRONZE, GUN, DARK];
      for (let i = 0; i < 3; i++) {
        metal.push({
          geo: box(0.07, 0.24 - i * 0.03, 0.05),
          p: [(i - 1) * 0.085, 1.02, -0.24],
          r: [0, 0, (i - 1) * 0.08],
          bone: 'spine',
          color: bookCols[i],
        });
      }
      // Reading-lamp antenna, tip on the state channel (glows while mining).
      metal.push({ geo: cyl(0.008, 0.008, 0.16, 5), p: [0.14, 2.16, 0.02], r: [0, 0, -0.25], bone: 'head', color: DARK });
      state.push({ geo: sphere(0.02, 8, 8), p: [0.16, 2.24, 0.02], bone: 'head' });
      break;
    }

    case 'reader': {
      // Scanning arc over the skull + a cyclops lens centered on the visor.
      trim.push({
        geo: new THREE.TorusGeometry(0.3, 0.012, 5, 20, Math.PI),
        p: [0, 1.92, 0.02],
        r: [0, Math.PI / 2, 0],
        bone: 'head',
      });
      visor.push({ geo: cyl(0.062, 0.062, 0.025, 16), p: [0, 1.9, 0.24], r: [Math.PI / 2, 0, 0], bone: 'head' });
      metal.push({ geo: torus(0.075, 0.012, 5, 16), p: [0, 1.9, 0.245], bone: 'head', color: DARK });
      // Document clip at the hip — the ingest tray.
      metal.push({ geo: box(0.14, 0.18, 0.02), p: [-0.23, 0.82, 0.02], r: [0, 0.3, 0.1], bone: 'spine', color: GUN });
      trim.push({ geo: box(0.15, 0.012, 0.024), p: [-0.23, 0.9, 0.02], r: [0, 0.3, 0.1], bone: 'spine' });
      break;
    }

    case 'watcher': {
      // Tall vigil antenna off the back with a beacon tip (state channel) +
      // two extra watchful eye-dots above the visor.
      metal.push({ geo: cyl(0.01, 0.006, 0.55, 5), p: [-0.1, 2.1, -0.14], r: [0.22, 0, 0.12], bone: 'head', color: DARK });
      state.push({ geo: sphere(0.026, 8, 8), p: [-0.165, 2.36, -0.2], bone: 'head' });
      visor.push(
        { geo: box(0.05, 0.03, 0.04), p: [0.08, 1.99, 0.2], bone: 'head' },
        { geo: box(0.05, 0.03, 0.04), p: [-0.08, 1.99, 0.2], bone: 'head' },
      );
      // Layered shoulder cowl slabs — the sentinel hunch.
      metal.push(
        { geo: box(0.46, 0.035, 0.2), p: [0, 1.6, -0.06], r: [0.12, 0, 0], bone: 'spine', color: DARK },
        { geo: box(0.38, 0.03, 0.16), p: [0, 1.66, -0.08], r: [0.16, 0, 0], bone: 'spine', color: GUN },
      );
      break;
    }

    case 'steward': {
      // Groundskeeper yoke across the shoulders + belt key-ring with keys.
      metal.push(
        { geo: box(0.52, 0.04, 0.13), p: [0, 1.6, -0.1], r: [0.1, 0, 0], bone: 'spine', color: DARK },
        { geo: torus(0.05, 0.009, 5, 14), p: [0.21, 0.7, 0.12], r: [0, 0.4, 0], bone: 'spine', color: BRONZE },
        { geo: box(0.015, 0.06, 0.008), p: [0.21, 0.64, 0.13], bone: 'spine', color: BRONZE },
        { geo: box(0.015, 0.05, 0.008), p: [0.235, 0.645, 0.115], r: [0, 0, 0.2], bone: 'spine', color: GUN },
        // Crossed shears on the left hip.
        { geo: box(0.02, 0.2, 0.015), p: [-0.22, 0.72, -0.05], r: [0, 0, 0.5], bone: 'spine', color: DARK },
        { geo: box(0.02, 0.2, 0.015), p: [-0.22, 0.72, -0.05], r: [0, 0, -0.5], bone: 'spine', color: DARK },
      );
      trim.push({ geo: torus(0.03, 0.007, 4, 10), p: [0.21, 0.7, 0.12], r: [0, 0.4, 0], bone: 'spine' });
      break;
    }
    case 'strix': {
      // The owl: a hooded brow over the visor, a scanning lens on the left
      // forearm, and a pair of probe picks at the hip. The lens goes on the
      // STATE channel so it glows with the HUD state — dark while idle, lit
      // amber while a sweep is actually running.
      metal.push(
        // Hood brow, swept back over the head.
        { geo: box(0.34, 0.05, 0.2), p: [0, 1.86, -0.02], r: [0.22, 0, 0], bone: 'head', color: DARK },
        { geo: box(0.08, 0.05, 0.16), p: [-0.15, 1.82, 0.02], r: [0.18, 0, -0.35], bone: 'head', color: DARK },
        { geo: box(0.08, 0.05, 0.16), p: [0.15, 1.82, 0.02], r: [0.18, 0, 0.35], bone: 'head', color: DARK },
        // Forearm scanner housing.
        { geo: box(0.1, 0.07, 0.14), p: [0, -0.28, 0.04], bone: 'armL', color: GUN },
        // Probe picks at the right hip.
        { geo: cyl(0.008, 0.008, 0.22, 6), p: [0.2, 0.68, -0.06], r: [0, 0, 0.28], bone: 'spine', color: GUN },
        { geo: cyl(0.008, 0.008, 0.2, 6), p: [0.23, 0.68, -0.06], r: [0, 0, 0.16], bone: 'spine', color: BRONZE },
      );
      // Identity trim: a torc at the throat, the owl's collar.
      trim.push({ geo: torus(0.11, 0.012, 5, 16), p: [0, 1.5, 0], r: [1.57, 0, 0], bone: 'spine' });
      // The scanning lens — emissive, driven by state.
      state.push({ geo: sphere(0.035, 8, 6), p: [0, -0.28, 0.11], bone: 'armL' });
      break;
    }
    case 'financier': {
      // Ledger tablet at the hip + a coin on a chain. The tablet's face sits
      // on the STATE channel so a live quote fetch actually lights it.
      metal.push(
        { geo: box(0.12, 0.16, 0.02), p: [0.22, 0.78, 0.08], r: [0.1, 0.3, 0.15], bone: 'spine', color: DARK },
        { geo: torus(0.035, 0.008, 6, 14), p: [-0.2, 0.72, 0.1], r: [1.2, 0, 0], bone: 'spine', color: BRONZE },
      );
      trim.push({ geo: torus(0.1, 0.01, 5, 16), p: [0, 1.5, 0], r: [1.57, 0, 0], bone: 'spine' });
      state.push({ geo: sphere(0.028, 8, 6), p: [0.22, 0.78, 0.1], bone: 'spine' });
      break;
    }
  }
  return { metal, trim, state, visor };
}

function metalChunks(weathering: number, extra: ChunkSpec[] = []): ChunkSpec[] {
  const tint = 1 - weathering * 0.25;
  const GUN = new THREE.Color(ENV.gunmetal).multiplyScalar(tint);
  const DARK = new THREE.Color(ENV.gunmetal).multiplyScalar(0.55 * tint);
  const BRONZE = new THREE.Color(ENV.bronze).multiplyScalar(tint);

  const chunks: ChunkSpec[] = [
    // Head
    { geo: sphere(0.28, 16, 12), p: [0, 1.88, 0], s: [0.85, 1.1, 0.95], bone: 'head', color: GUN },
    { geo: box(0.16, 0.06, 0.12), p: [0, 1.74, 0.18], bone: 'head', color: DARK },
    { geo: cyl(0.07, 0.09, 0.14, 12), p: [0, 1.72, 0], bone: 'head', color: DARK },
    // Brow ridge over the visor + jaw guard (detail pass 2026-07-28)
    { geo: box(0.26, 0.03, 0.06), p: [0, 1.95, 0.2], bone: 'head', color: DARK },
    { geo: box(0.14, 0.05, 0.08), p: [0, 1.68, 0.16], bone: 'head', color: DARK },
    // Torso
    { geo: torsoLathe(), p: [0, 0.68, 0], bone: 'spine', color: GUN },
    // Neck collar guard
    { geo: cyl(0.12, 0.15, 0.09, 12), p: [0, 1.64, 0], bone: 'spine', color: DARK },
    // Chest plate + sternum spine
    { geo: box(0.3, 0.24, 0.05), p: [0, 1.34, 0.2], bone: 'spine', color: GUN },
    { geo: box(0.07, 0.34, 0.04), p: [0, 1.18, 0.23], bone: 'spine', color: DARK },
    // Segmented abdominal plates
    { geo: box(0.22, 0.05, 0.05), p: [0, 0.98, 0.19], bone: 'spine', color: DARK },
    { geo: box(0.2, 0.05, 0.05), p: [0, 0.9, 0.18], bone: 'spine', color: DARK },
    // Utility belt + buckle
    { geo: torus(0.19, 0.032, 6, 20), p: [0, 0.76, 0], r: [Math.PI / 2, 0, 0], bone: 'spine', color: DARK },
    { geo: box(0.08, 0.07, 0.04), p: [0, 0.76, 0.2], bone: 'spine', color: BRONZE },
    // Dorsal spine ridge
    { geo: box(0.05, 0.55, 0.04), p: [0, 1.18, -0.23], bone: 'spine', color: DARK },
    // Universal detail (overhaul 2026-07-28): angled side intake vents + hip guards.
    { geo: box(0.02, 0.1, 0.06), p: [0.235, 1.22, 0.04], r: [0, 0, -0.25], bone: 'spine', color: DARK },
    { geo: box(0.02, 0.1, 0.06), p: [-0.235, 1.22, 0.04], r: [0, 0, 0.25], bone: 'spine', color: DARK },
    { geo: box(0.02, 0.08, 0.05), p: [0.245, 1.08, 0.02], r: [0, 0, -0.25], bone: 'spine', color: DARK },
    { geo: box(0.02, 0.08, 0.05), p: [-0.245, 1.08, 0.02], r: [0, 0, 0.25], bone: 'spine', color: DARK },
    { geo: box(0.055, 0.15, 0.11), p: [0.19, 0.72, 0], bone: 'spine', color: GUN },
    { geo: box(0.055, 0.15, 0.11), p: [-0.19, 0.72, 0], bone: 'spine', color: GUN },
  ];

  for (const side of [1, -1] as const) {
    const arm: BoneName = side === 1 ? 'armL' : 'armR';
    const fore: BoneName = side === 1 ? 'foreL' : 'foreR';
    const thigh: BoneName = side === 1 ? 'thighL' : 'thighR';
    const calf: BoneName = side === 1 ? 'calfL' : 'calfR';
    chunks.push(
      // Pauldron shell over the shoulder (detail pass 2026-07-28)
      { geo: sphere(0.115, 12, 8), p: [side * 0.33, 1.55, 0], s: [1, 0.75, 1], bone: arm, color: GUN },
      // Thigh armor plate, hand knuckle cap, shin guard
      { geo: box(0.09, 0.22, 0.04), p: [side * 0.13, 0.46, 0.075], bone: thigh, color: GUN },
      { geo: box(0.055, 0.055, 0.06), p: [side * 0.45, 0.5, 0.05], bone: fore, color: DARK },
      { geo: box(0.075, 0.2, 0.03), p: [side * 0.12, 0.12, 0.075], bone: calf, color: DARK },
      // Shoulder + arm
      { geo: new THREE.CapsuleGeometry(0.07, 0.08, 4, 8), p: [side * 0.3, 1.5, 0], r: [0, 0, side * 0.3], bone: arm, color: BRONZE },
      { geo: cyl(0.055, 0.042, 0.42, 10), p: [side * 0.38, 1.28, 0], r: [0, 0, side * 0.12], bone: arm, color: GUN },
      { geo: sphere(0.038, 10, 10), p: [side * 0.42, 1.04, 0], bone: fore, color: DARK },
      { geo: cyl(0.042, 0.032, 0.38, 10), p: [side * 0.44, 0.84, 0.02], r: [0, 0, side * 0.05], bone: fore, color: GUN },
      { geo: sphere(0.028, 8, 8), p: [side * 0.45, 0.63, 0.03], bone: fore, color: DARK },
      { geo: cyl(0.022, 0.035, 0.12, 6), p: [side * 0.45, 0.55, 0.04], bone: fore, color: GUN },
      // Leg
      { geo: cyl(0.07, 0.055, 0.5, 10), p: [side * 0.12, 0.42, 0], bone: thigh, color: GUN },
      { geo: sphere(0.05, 10, 10), p: [side * 0.12, 0.18, 0], bone: calf, color: BRONZE },
      { geo: cyl(0.05, 0.04, 0.28, 10), p: [side * 0.12, 0.05, 0.01], bone: calf, color: GUN },
      // Boot
      { geo: cyl(0.03, 0.045, 0.18, 8), p: [side * 0.12, -0.05, 0.06], r: [Math.PI / 2, 0, 0], bone: calf, color: GUN },
      { geo: box(0.06, 0.02, 0.06), p: [side * 0.12, -0.07, -0.02], bone: calf, color: DARK },
    );
  }
  chunks.push(...extra);
  return chunks;
}

/** IDENTITY trim channels — toga-trim family. Never state-colored (bible §4). */
function trimChunks(extra: ChunkSpec[] = []): ChunkSpec[] {
  const chunks: ChunkSpec[] = [
    // Head channels
    { geo: cyl(0.02, 0.02, 0.4, 4), p: [0, 2.06, 0], r: [Math.PI / 2, 0, 0], bone: 'head' },
    { geo: box(0.015, 0.16, 0.02), p: [0, 1.82, 0.24], bone: 'head' },
    { geo: torus(0.08, 0.015, 6, 16), p: [0, 1.78, 0], bone: 'head' },
    // Chest / spine channels
    { geo: cyl(0.015, 0.015, 0.6, 4), p: [0.12, 1.2, 0.18], bone: 'spine' },
    { geo: cyl(0.015, 0.015, 0.6, 4), p: [-0.12, 1.2, 0.18], bone: 'spine' },
    { geo: cyl(0.012, 0.012, 0.7, 4), p: [0, 1.15, 0.2], bone: 'spine' },
    { geo: cyl(0.015, 0.015, 0.65, 4), p: [0, 1.15, -0.16], bone: 'spine' },
    // Belt-line identity channel (detail pass 2026-07-28)
    { geo: torus(0.2, 0.01, 4, 20), p: [0, 0.8, 0], r: [Math.PI / 2, 0, 0], bone: 'spine' },
    // Torso panel seam rings + inner under-glow (identity warm glow through gaps)
    { geo: torus(0.15, 0.012, 4, 20), p: [0, 0.88, 0], r: [Math.PI / 2, 0, 0], bone: 'spine' },
    { geo: torus(0.19, 0.012, 4, 20), p: [0, 1.08, 0], r: [Math.PI / 2, 0, 0], bone: 'spine' },
    { geo: torus(0.24, 0.012, 4, 20), p: [0, 1.3, 0], r: [Math.PI / 2, 0, 0], bone: 'spine' },
    { geo: torus(0.22, 0.012, 4, 20), p: [0, 1.48, 0], r: [Math.PI / 2, 0, 0], bone: 'spine' },
    { geo: cyl(0.12, 0.1, 0.85, 12), p: [0, 0.68, 0], bone: 'spine' },
    // Cape edge trim (toga edge — THE identity channel)
    { geo: capeCurveTube(sampleLine(1.0, 'u', 20), 0.012, 20), bone: 'spine' },
    { geo: capeCurveTube(sampleLine(0.0, 'u', 12), 0.01, 12), bone: 'spine' },
    { geo: capeCurveTube(sampleLine(0.0, 'v', 16), 0.008, 12), bone: 'spine' },
    { geo: capeCurveTube(sampleLine(1.0, 'v', 16), 0.008, 12), bone: 'spine' },
  ];
  for (const side of [1, -1] as const) {
    const arm: BoneName = side === 1 ? 'armL' : 'armR';
    const fore: BoneName = side === 1 ? 'foreL' : 'foreR';
    const thigh: BoneName = side === 1 ? 'thighL' : 'thighR';
    const calf: BoneName = side === 1 ? 'calfL' : 'calfR';
    chunks.push(
      // Temple + upper skull channels
      { geo: cyl(0.015, 0.015, 0.25, 4), p: [side * 0.2, 1.94, 0.08], r: [0, side * 0.4, side * -0.15], bone: 'head' },
      { geo: cyl(0.012, 0.012, 0.22, 4), p: [side * 0.1, 2.08, -0.05], r: [0.4, 0, side * -0.1], bone: 'head' },
      // Side + rear-diagonal torso channels
      { geo: cyl(0.012, 0.012, 0.55, 4), p: [side * 0.2, 1.15, 0.08], bone: 'spine' },
      { geo: cyl(0.01, 0.01, 0.5, 4), p: [side * 0.1, 1.2, -0.14], r: [0, 0, side * 0.15], bone: 'spine' },
      // Pauldron edge channel (detail pass 2026-07-28)
      { geo: torus(0.1, 0.008, 4, 14), p: [side * 0.33, 1.5, 0], r: [Math.PI / 2, 0, 0], bone: arm },
      // Shin channel
      { geo: cyl(0.008, 0.008, 0.16, 4), p: [side * 0.12, 0.12, 0.092], bone: calf },
      // Shoulder ring + arm channels
      { geo: torus(0.06, 0.012, 4, 8), p: [side * 0.3, 1.44, 0], bone: arm },
      { geo: cyl(0.01, 0.01, 0.34, 4), p: [side * 0.38, 1.28, 0.055], r: [0, 0, side * 0.12], bone: arm },
      { geo: cyl(0.012, 0.012, 0.3, 4), p: [side * 0.44, 0.84, 0.065], r: [0, 0, side * 0.05], bone: fore },
      // Leg channel + boot trim ring
      { geo: cyl(0.012, 0.012, 0.3, 4), p: [side * 0.12, 0.42, 0.068], bone: thigh },
      { geo: torus(0.04, 0.008, 4, 8), p: [side * 0.12, -0.08, 0.02], r: [Math.PI / 2, 0, 0], bone: calf },
    );
  }
  chunks.push(...extra);
  return chunks;
}

/** STATE channels: joint glow rings + cape circuit lines + feet aura (+ crown gems). */
function stateChunks(withGems: boolean, extra: ChunkSpec[] = []): ChunkSpec[] {
  const chunks: ChunkSpec[] = [
    // Feet aura ring (breathes when available)
    { geo: new THREE.RingGeometry(0.4, 0.55, 24), p: [0, 0, 0], r: [-Math.PI / 2, 0, 0], bone: 'aura' },
  ];
  // Cape circuit lines (vertical) + panel seams (horizontal)
  for (const u of [0.2, 0.35, 0.5, 0.65, 0.8]) {
    chunks.push({ geo: capeCurveTube(sampleLine(u, 'v', 12), 0.005, 10), bone: 'spine' });
  }
  for (const v of [0.25, 0.5, 0.75]) {
    chunks.push({ geo: capeCurveTube(sampleLine(v, 'u', 16), 0.006, 14), bone: 'spine' });
  }
  for (const side of [1, -1] as const) {
    const fore: BoneName = side === 1 ? 'foreL' : 'foreR';
    const calf: BoneName = side === 1 ? 'calfL' : 'calfR';
    chunks.push(
      { geo: torus(0.032, 0.006, 4, 12), p: [side * 0.42, 1.04, 0], r: [Math.PI / 2, 0, 0], bone: fore },
      { geo: torus(0.024, 0.005, 4, 10), p: [side * 0.45, 0.63, 0.03], bone: fore },
      { geo: torus(0.042, 0.006, 4, 12), p: [side * 0.12, 0.18, 0], r: [Math.PI / 2, 0, 0], bone: calf },
    );
  }
  if (withGems) {
    // Crown gems — the ONE sanctioned identity/state crossover (bible §4):
    // Henry's gems pick up his current state color so it reads across the room.
    // Seated on the FRONT of the circlet band (y≈2.0), flanking the centre point.
    chunks.push(
      { geo: sphere(0.03, 10, 10), p: [0.085, 2.02, 0.215], bone: 'head' },
      { geo: sphere(0.03, 10, 10), p: [-0.085, 2.02, 0.215], bone: 'head' },
    );
  }
  // Back power core — every agent's reactor, glowing with the live state color.
  chunks.push(
    { geo: cyl(0.05, 0.05, 0.015, 14), p: [0, 1.22, -0.255], r: [Math.PI / 2, 0, 0], bone: 'spine' },
    { geo: torus(0.075, 0.008, 4, 16), p: [0, 1.22, -0.255], bone: 'spine' },
  );
  chunks.push(...extra);
  return chunks;
}

/** Visor — its own draw call so flicker/intensity is independent of other channels. */
function visorChunks(extra: ChunkSpec[] = []): ChunkSpec[] {
  const chunks: ChunkSpec[] = [
    { geo: box(0.22, 0.045, 0.06), p: [0, 1.9, 0.22], bone: 'head' },
  ];
  for (const side of [1, -1] as const) {
    chunks.push(
      { geo: box(0.06, 0.04, 0.04), p: [side * 0.13, 1.9, 0.18], r: [0, side * 0.5, 0], bone: 'head' },
      { geo: cyl(0.04, 0.04, 0.01, 8), p: [side * 0.24, 1.86, 0], r: [0, side * (Math.PI / 2), 0], bone: 'head' },
    );
  }
  chunks.push(...extra);
  return chunks;
}

function crownChunks(): ChunkSpec[] {
  // A refined circlet seated ON the upper skull (top ≈ 2.19), not a spiky halo
  // floating above it: a smooth horizontal band + a fine upper rim, ringed by
  // alternating tall/short merlon points (the classic crown silhouette).
  const R = 0.235; // wraps the skull, sitting slightly proud
  const yBand = 2.0;
  const chunks: ChunkSpec[] = [
    { geo: torus(R, 0.02, 10, 32), p: [0, yBand, 0], r: [Math.PI / 2, 0, 0], bone: 'head' },
    { geo: torus(R - 0.004, 0.009, 8, 32), p: [0, yBand + 0.05, 0], r: [Math.PI / 2, 0, 0], bone: 'head' },
  ];
  const N = 8;
  for (let i = 0; i < N; i++) {
    const angle = (i / N) * Math.PI * 2;
    const h = i % 2 === 0 ? 0.17 : 0.1; // alternating merlons
    chunks.push({
      geo: new THREE.ConeGeometry(0.025, h, 6),
      p: [Math.cos(angle) * R, yBand + 0.03 + h / 2, Math.sin(angle) * R],
      bone: 'head',
    });
  }
  return chunks;
}

// ── Geometry cache (built once per variant, app-lifetime — no leak) ──

export interface RigGeometries {
  metal: THREE.BufferGeometry;
  trim: THREE.BufferGeometry;
  state: THREE.BufferGeometry;
  visor: THREE.BufferGeometry;
  cape: THREE.BufferGeometry;
  crown: THREE.BufferGeometry | null;
}

const geoCache = new Map<string, RigGeometries>();

export function getRigGeometries(opts: {
  weathering: number;
  crown: boolean;
  variant?: RigVariant;
}): RigGeometries {
  const variant = opts.variant ?? null;
  const key = `${opts.weathering}|${opts.crown}|${variant}`;
  let g = geoCache.get(key);
  if (!g) {
    // Signature gear rides the same four channel draw calls — no extra cost.
    const tint = 1 - opts.weathering * 0.25;
    const gear = gearChunks(
      variant,
      new THREE.Color(ENV.gunmetal).multiplyScalar(tint),
      new THREE.Color(ENV.gunmetal).multiplyScalar(0.55 * tint),
      new THREE.Color(ENV.bronze).multiplyScalar(tint),
    );
    g = {
      metal: mergeChunks(metalChunks(opts.weathering, gear.metal), true),
      trim: mergeChunks(trimChunks(gear.trim), false),
      state: mergeChunks(stateChunks(opts.crown, gear.state), false),
      visor: mergeChunks(visorChunks(gear.visor), false),
      cape: (() => {
        const cape = buildCapeGeometry();
        return prepChunk({ geo: cape, bone: 'spine' }, false);
      })(),
      crown: opts.crown ? mergeChunks(crownChunks(), false) : null,
    };
    geoCache.set(key, g);
  }
  return g;
}

// ── Materials ────────────────────────────────────────────────────────

// Shared across all agents (no identity/state color baked in).
let sharedMetalMat: THREE.MeshStandardMaterial | null = null;
let sharedCapeMat: THREE.MeshStandardMaterial | null = null;
let sharedGoldMat: THREE.MeshStandardMaterial | null = null;

function getSharedMats() {
  if (!sharedMetalMat) {
    // Punchier metal (overhaul 2026-07-28): lower roughness + higher metalness
    // gives the plates a specular read instead of matte gray.
    sharedMetalMat = new THREE.MeshStandardMaterial({
      vertexColors: true,
      roughness: 0.38,
      metalness: 0.55,
    });
    sharedCapeMat = new THREE.MeshStandardMaterial({
      color: new THREE.Color(ENV.deepVoid).multiplyScalar(2.2),
      roughness: 0.7,
      metalness: 0.1,
      transparent: true,
      opacity: 0.4,
      side: THREE.DoubleSide,
      vertexColors: true,
    });
    // Crown gold — pre-existing crown identity color (not a state/trim semantic).
    sharedGoldMat = new THREE.MeshStandardMaterial({
      color: '#FFD700',
      roughness: 0.2,
      metalness: 0.8,
      emissive: '#FFD700',
      emissiveIntensity: 0.3,
    });
  }
  return {
    metal: sharedMetalMat,
    cape: sharedCapeMat as THREE.MeshStandardMaterial,
    gold: sharedGoldMat as THREE.MeshStandardMaterial,
  };
}

/** A glowing tablet the Librarian pulls/reshelves during a real describe. */
export interface TabletMesh extends THREE.Mesh {
  material: THREE.MeshStandardMaterial;
}

export interface AgentRig {
  root: THREE.Group;
  bones: Record<BoneName, THREE.Bone>;
  /** Per-agent animated materials. */
  trimMat: THREE.MeshStandardMaterial;
  stateMat: THREE.MeshStandardMaterial;
  visorMat: THREE.MeshStandardMaterial;
  /** Librarian-only: the violet describe tablet (null for everyone else). */
  tablet: TabletMesh | null;
  /** Henry-only: the soft pool of light that gathers at his feet (null otherwise). */
  presenceLight: THREE.Mesh | null;
  /** The work halo — orbiting data ring + glyph helix shown while the agent
   *  processes a task standing (agents have no desks; the work happens AROUND
   *  them). AgentCharacterV2 drives visibility/rotation/color per frame. */
  workHalo: { group: THREE.Group; mat: THREE.MeshBasicMaterial } | null;
  /** Draw calls this rig contributes. */
  drawCalls: number;
  dispose(): void;
}

export function createAgentRig(opts: {
  trimColor: string;
  weathering: number;
  crown: boolean;
  /** Signature-gear variant; 'librarian' also gets the describe tablet and
   *  'henry' the presence light. */
  variant?: RigVariant;
  /** Authored armor replaces three geometry channels, not live state/poses. */
  armor?: Pick<RigGeometries, 'metal' | 'trim' | 'visor'> | null;
}): AgentRig {
  const geos = { ...getRigGeometries({
    weathering: opts.weathering,
    crown: opts.crown,
    variant: opts.variant ?? null,
  }), ...opts.armor };
  const shared = getSharedMats();
  const bones = buildBones();
  const skeleton = new THREE.Skeleton(bones.list);

  const trimMat = new THREE.MeshStandardMaterial({
    color: opts.trimColor,
    emissive: opts.trimColor,
    emissiveIntensity: 1.6,
    roughness: 0.2,
    metalness: 0.1,
  });
  const stateMat = new THREE.MeshStandardMaterial({
    color: '#FFFFFF',
    emissive: '#FFFFFF',
    emissiveIntensity: 1.0,
    roughness: 0.2,
    metalness: 0.1,
    transparent: true,
    opacity: 0.9,
  });
  const visorMat = new THREE.MeshStandardMaterial({
    color: '#FFFFFF',
    emissive: '#FFFFFF',
    emissiveIntensity: 2.0,
    roughness: 0.05,
    metalness: 0.1,
  });

  const root = new THREE.Group();
  root.userData.blenderArmor = !!opts.armor;
  root.add(bones.byName.root);

  const makeMesh = (geo: THREE.BufferGeometry, mat: THREE.Material, castShadow = false) => {
    const mesh = new THREE.SkinnedMesh(geo, mat);
    mesh.castShadow = castShadow;
    mesh.frustumCulled = false; // rigid-skinned; bounds move with bones
    root.add(mesh);
    mesh.bind(skeleton);
    return mesh;
  };

  makeMesh(geos.metal, shared.metal, true);
  makeMesh(geos.trim, trimMat);
  makeMesh(geos.state, stateMat);
  makeMesh(geos.visor, visorMat);
  makeMesh(geos.cape, shared.cape);
  let drawCalls = 5;
  if (geos.crown) {
    makeMesh(geos.crown, shared.gold);
    drawCalls += 1;
  }

  // ── Librarian-only describe tablet (bible §5): a small violet plate that brightens
  // in the hands during a real describe. Parented to the spine bone so it follows the
  // body; AgentCharacterV2 moves it shelf→hands and ramps emissive from the mining loop.
  let tablet: TabletMesh | null = null;
  let tabletMat: THREE.MeshStandardMaterial | null = null;
  if (opts.variant === 'librarian') {
    const tabletGeo = new THREE.BoxGeometry(0.22, 0.3, 0.03);
    tabletMat = new THREE.MeshStandardMaterial({
      color: ENV.violet,
      emissive: ENV.violet,
      emissiveIntensity: 0,
      roughness: 0.3,
      metalness: 0.1,
    });
    const t = new THREE.Mesh(tabletGeo, tabletMat) as TabletMesh;
    t.visible = false;
    t.frustumCulled = false;
    bones.byName.spine.add(t);
    tablet = t;
    drawCalls += 1;
  }

  // ── Henry-only presence light (bible §4): a soft warm pool that gathers at his feet.
  // He never lifts; the light is the only thing his presiding adds to the floor.
  let presenceLight: THREE.Mesh | null = null;
  let presenceMat: THREE.MeshBasicMaterial | null = null;
  let presenceGeo: THREE.RingGeometry | null = null;
  if (opts.variant === 'henry') {
    presenceGeo = new THREE.RingGeometry(0.2, 1.6, 24);
    presenceMat = new THREE.MeshBasicMaterial({
      color: '#FFF0D4',
      transparent: true,
      opacity: 0.16,
      depthWrite: false,
      side: THREE.DoubleSide,
    });
    const pl = new THREE.Mesh(presenceGeo, presenceMat);
    pl.rotation.x = -Math.PI / 2;
    pl.position.y = 0.015;
    pl.frustumCulled = false;
    root.add(pl);
    presenceLight = pl;
    drawCalls += 1;
  }

  // ── Work halo (2026-07-28: agents have no desks) ──────────────────
  // An AI agent doesn't sit at a workstation — while it processes, the work
  // orbits IT: a slow data ring + a helix of glyph quads, tinted live from
  // the state channel (amber while working, per the HUD color law). Two
  // merged meshes (+2 draw calls, visible only while processing).
  const haloMat = new THREE.MeshBasicMaterial({
    color: '#FFFFFF',
    transparent: true,
    opacity: 0,
    depthWrite: false,
    side: THREE.DoubleSide,
    blending: THREE.AdditiveBlending,
    toneMapped: false,
  });
  const haloGroup = new THREE.Group();
  haloGroup.visible = false;
  {
    const ring = new THREE.Mesh(new THREE.TorusGeometry(0.62, 0.014, 6, 40), haloMat);
    ring.rotation.x = Math.PI / 2;
    ring.position.y = 1.25;
    ring.frustumCulled = false;
    haloGroup.add(ring);
    // Glyph helix: 10 small quads spiralling 0.6 → 1.9 around the body.
    const glyphGeos: THREE.BufferGeometry[] = [];
    for (let i = 0; i < 10; i++) {
      const t = i / 10;
      const a = t * Math.PI * 4;
      const quad = new THREE.PlaneGeometry(0.07, 0.1);
      const m4 = new THREE.Matrix4()
        .makeRotationY(-a)
        .setPosition(Math.cos(a) * 0.58, 0.6 + t * 1.3, Math.sin(a) * 0.58);
      quad.applyMatrix4(m4);
      glyphGeos.push(quad);
    }
    const glyphs = new THREE.Mesh(mergeGeometries(glyphGeos, false)!, haloMat);
    glyphGeos.forEach((g) => g.dispose());
    glyphs.frustumCulled = false;
    haloGroup.add(glyphs);
    bones.byName.root.add(haloGroup);
  }
  const workHalo = { group: haloGroup, mat: haloMat };

  return {
    root,
    bones: bones.byName,
    trimMat,
    stateMat,
    visorMat,
    tablet,
    presenceLight,
    workHalo,
    drawCalls,
    dispose() {
      // Geometries + shared materials are cached app-lifetime; only per-agent
      // materials (and the per-variant tablet/light geo) are owned here.
      trimMat.dispose();
      stateMat.dispose();
      visorMat.dispose();
      tabletMat?.dispose();
      tablet?.geometry.dispose();
      presenceMat?.dispose();
      presenceGeo?.dispose();
      haloMat.dispose();
      haloGroup.children.forEach((c) => {
        if (c instanceof THREE.Mesh) c.geometry.dispose();
      });
    },
  };
}
