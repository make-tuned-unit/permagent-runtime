// Agora constellation sim — the swarm of sovereign minds as a force field.
//
// This is the private Brain graph (BrainScene.ts) turned INSIDE-OUT and made
// scale-native: same visual language (glowing nodes drifting into clusters, light
// pulses flowing the edges between them), but rendered as a COUNT, not a
// collection. BrainScene uses one THREE.Mesh per node — perfect for a private
// graph of dozens, catastrophic for a mesh of thousands. So the Agora reimplements
// the force/cluster/pulse PATTERN over flat typed arrays: O(n) clustered attraction
// (not O(n²) all-pairs), one instanced/points draw call for the whole swarm.
//
// "Agents with shared interest drift into clusters": each glyph belongs to a
// cluster (a shared-interest constellation); it springs toward its cluster's
// slowly-orbiting center. "When two commune, a pulse travels the edge": each
// cluster is a small star topology (members ↔ hub) whose edges carry pulses.
//
// LAW: zero per-frame allocation — all state lives in preallocated Float32Arrays
// mutated in place. Dormant today (population 0) ⇒ ensure()/step() no-op, no cost.

import { AGORA_VISIBLE_CAP, slotHash } from './agoraPeers';

const CAP = AGORA_VISIBLE_CAP;
const CLUSTER_MAX = 20;
const CLUSTER_R = 8.5; // cluster centers orbit on this shell
const MEMBER_R = 2.6; // glyph spread within a cluster
const SPRING = 2.2;
const DAMP = 0.86;

// Per-glyph state (local Agora space; the Agora group sits at AGORA_CENTER).
const pos = new Float32Array(CAP * 3);
const vel = new Float32Array(CAP * 3);
const cluster = new Int16Array(CAP);
// Deterministic per-glyph offset direction within its cluster (unit-ish).
const offX = new Float32Array(CAP);
const offY = new Float32Array(CAP);
const offZ = new Float32Array(CAP);

// Cluster base directions on a Fibonacci sphere (stable), + orbit phase.
const cbx = new Float32Array(CLUSTER_MAX);
const cby = new Float32Array(CLUSTER_MAX);
const cbz = new Float32Array(CLUSTER_MAX);

let builtFor = -1;
let clusterCount = 1;
let simTime = 0;

// Edge list (cluster star topology): member index → its cluster hub index.
const edgeA = new Uint16Array(CAP);
const edgeB = new Uint16Array(CAP);
let edgeCount = 0;

function fib(i: number, n: number, out: Float32Array[], idx: number): void {
  const gold = Math.PI * (3 - Math.sqrt(5));
  const y = 1 - (i / Math.max(1, n - 1)) * 2;
  const r = Math.sqrt(Math.max(0, 1 - y * y));
  const th = gold * i;
  out[0][idx] = Math.cos(th) * r;
  out[1][idx] = y;
  out[2][idx] = Math.sin(th) * r;
}

/** (Re)build the layout for a given live population. Cheap; only on count change. */
export function ensureLayout(count: number): void {
  const n = Math.min(count, CAP);
  if (n === builtFor) return;
  builtFor = n;
  if (n <= 0) {
    edgeCount = 0;
    clusterCount = 1;
    return;
  }
  clusterCount = Math.max(1, Math.min(CLUSTER_MAX, Math.round(Math.sqrt(n / 3))));
  const cb = [cbx, cby, cbz];
  for (let c = 0; c < clusterCount; c++) fib(c, clusterCount, cb, c);

  edgeCount = 0;
  for (let i = 0; i < n; i++) {
    const c = i % clusterCount;
    cluster[i] = c;
    // Deterministic offset direction within the cluster (spherical, hashed).
    const u = slotHash(i * 2 + 1) * 2 - 1;
    const th = slotHash(i * 2 + 2) * Math.PI * 2;
    const rr = Math.sqrt(Math.max(0, 1 - u * u));
    offX[i] = Math.cos(th) * rr;
    offY[i] = u;
    offZ[i] = Math.sin(th) * rr;
    // Seed position at the cluster center + offset so the swarm opens gracefully.
    const scl = MEMBER_R * (0.4 + slotHash(i * 3 + 5) * 0.6);
    pos[i * 3] = cbx[c] * CLUSTER_R + offX[i] * scl;
    pos[i * 3 + 1] = cby[c] * CLUSTER_R + offY[i] * scl;
    pos[i * 3 + 2] = cbz[c] * CLUSTER_R + offZ[i] * scl;
    vel[i * 3] = vel[i * 3 + 1] = vel[i * 3 + 2] = 0;
    // Edge: every non-hub glyph links to its cluster hub (first member of cluster).
    const hub = c; // hub index == cluster index (i in [0..clusterCount) are hubs)
    if (i >= clusterCount && hub < n) {
      edgeA[edgeCount] = i;
      edgeB[edgeCount] = hub;
      edgeCount++;
    }
  }
}

/** Advance the swarm one frame. Zero allocation. No-op when empty. */
export function stepSim(dt: number, count: number): void {
  const n = Math.min(count, CAP);
  if (n <= 0) return;
  simTime += dt;
  const spin = simTime * 0.06; // whole constellation slowly turns
  const cs = Math.cos(spin);
  const sn = Math.sin(spin);
  for (let i = 0; i < n; i++) {
    const c = cluster[i];
    // Cluster center (base dir orbited around Y) × shell radius.
    const bx = cbx[c];
    const bz = cbz[c];
    const cxp = (bx * cs - bz * sn) * CLUSTER_R;
    const czp = (bx * sn + bz * cs) * CLUSTER_R;
    const cyp = cby[c] * CLUSTER_R;
    // Living within-cluster offset (slow per-glyph breathing).
    const br = MEMBER_R * (0.7 + 0.3 * Math.sin(simTime * 0.7 + i));
    const tx = cxp + offX[i] * br;
    const ty = cyp + offY[i] * br;
    const tz = czp + offZ[i] * br;
    const k = i * 3;
    vel[k] = (vel[k] + (tx - pos[k]) * SPRING * dt) * DAMP;
    vel[k + 1] = (vel[k + 1] + (ty - pos[k + 1]) * SPRING * dt) * DAMP;
    vel[k + 2] = (vel[k + 2] + (tz - pos[k + 2]) * SPRING * dt) * DAMP;
    pos[k] += vel[k] * dt;
    pos[k + 1] += vel[k + 1] * dt;
    pos[k + 2] += vel[k + 2] * dt;
  }
}

export function getPositions(): Float32Array {
  return pos;
}
export function getEdges(): { a: Uint16Array; b: Uint16Array; count: number } {
  return { a: edgeA, b: edgeB, count: edgeCount };
}
