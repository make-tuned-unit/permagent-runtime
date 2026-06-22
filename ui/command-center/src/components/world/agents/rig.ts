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

function metalChunks(weathering: number): ChunkSpec[] {
  const tint = 1 - weathering * 0.25;
  const GUN = new THREE.Color(ENV.gunmetal).multiplyScalar(tint);
  const DARK = new THREE.Color(ENV.gunmetal).multiplyScalar(0.55 * tint);
  const BRONZE = new THREE.Color(ENV.bronze).multiplyScalar(tint);

  const chunks: ChunkSpec[] = [
    // Head
    { geo: sphere(0.28, 16, 12), p: [0, 1.88, 0], s: [0.85, 1.1, 0.95], bone: 'head', color: GUN },
    { geo: box(0.16, 0.06, 0.12), p: [0, 1.74, 0.18], bone: 'head', color: DARK },
    { geo: cyl(0.07, 0.09, 0.14, 12), p: [0, 1.72, 0], bone: 'head', color: DARK },
    // Torso
    { geo: torsoLathe(), p: [0, 0.68, 0], bone: 'spine', color: GUN },
  ];

  for (const side of [1, -1] as const) {
    const arm: BoneName = side === 1 ? 'armL' : 'armR';
    const fore: BoneName = side === 1 ? 'foreL' : 'foreR';
    const thigh: BoneName = side === 1 ? 'thighL' : 'thighR';
    const calf: BoneName = side === 1 ? 'calfL' : 'calfR';
    chunks.push(
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
  return chunks;
}

/** IDENTITY trim channels — toga-trim family. Never state-colored (bible §4). */
function trimChunks(): ChunkSpec[] {
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
      // Shoulder ring + arm channels
      { geo: torus(0.06, 0.012, 4, 8), p: [side * 0.3, 1.44, 0], bone: arm },
      { geo: cyl(0.01, 0.01, 0.34, 4), p: [side * 0.38, 1.28, 0.055], r: [0, 0, side * 0.12], bone: arm },
      { geo: cyl(0.012, 0.012, 0.3, 4), p: [side * 0.44, 0.84, 0.065], r: [0, 0, side * 0.05], bone: fore },
      // Leg channel + boot trim ring
      { geo: cyl(0.012, 0.012, 0.3, 4), p: [side * 0.12, 0.42, 0.068], bone: thigh },
      { geo: torus(0.04, 0.008, 4, 8), p: [side * 0.12, -0.08, 0.02], r: [Math.PI / 2, 0, 0], bone: calf },
    );
  }
  return chunks;
}

/** STATE channels: joint glow rings + cape circuit lines + feet aura (+ crown gems). */
function stateChunks(withGems: boolean): ChunkSpec[] {
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
  return chunks;
}

/** Visor — its own draw call so flicker/intensity is independent of other channels. */
function visorChunks(): ChunkSpec[] {
  const chunks: ChunkSpec[] = [
    { geo: box(0.22, 0.045, 0.06), p: [0, 1.9, 0.22], bone: 'head' },
  ];
  for (const side of [1, -1] as const) {
    chunks.push(
      { geo: box(0.06, 0.04, 0.04), p: [side * 0.13, 1.9, 0.18], r: [0, side * 0.5, 0], bone: 'head' },
      { geo: cyl(0.04, 0.04, 0.01, 8), p: [side * 0.24, 1.86, 0], r: [0, side * (Math.PI / 2), 0], bone: 'head' },
    );
  }
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

export function getRigGeometries(opts: { weathering: number; crown: boolean }): RigGeometries {
  const key = `${opts.weathering}|${opts.crown}`;
  let g = geoCache.get(key);
  if (!g) {
    g = {
      metal: mergeChunks(metalChunks(opts.weathering), true),
      trim: mergeChunks(trimChunks(), false),
      state: mergeChunks(stateChunks(opts.crown), false),
      visor: mergeChunks(visorChunks(), false),
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
    sharedMetalMat = new THREE.MeshStandardMaterial({
      vertexColors: true,
      roughness: 0.45,
      metalness: 0.45,
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

/** A glowing tablet the Librarian pulls/reshelves during a real describe (bible §5). */
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
  /** Draw calls this rig contributes. */
  drawCalls: number;
  dispose(): void;
}

export function createAgentRig(opts: {
  trimColor: string;
  weathering: number;
  crown: boolean;
  /** Librarian gets the describe tablet; Henry gets the presence light. */
  variant?: 'librarian' | 'henry' | null;
}): AgentRig {
  const geos = getRigGeometries({ weathering: opts.weathering, crown: opts.crown });
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

  return {
    root,
    bones: bones.byName,
    trimMat,
    stateMat,
    visorMat,
    tablet,
    presenceLight,
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
    },
  };
}
