/**
 * Fit the People graph to the viewport it is actually drawn in.
 *
 * QA 2026-09-24, graph area ~660x560 at the real window size: faces at the
 * ring's ends were half off the left edge ("BT", "TB", "TA"), the chip
 * "HARBOURVIEW RESIDENTS' A…" was cut at the right, and the "No project"
 * cluster's faces sat on top of one another and of the front project's label.
 *
 * Root cause: the camera was a constant (`[0, 7.5, 14]`, fov 50) chosen for a
 * wide window, and nothing accounted for the fact that the faces and chips are
 * fixed-PIXEL DOM overlays (drei `<Html>`) on world-space anchors. A narrower
 * canvas narrows the horizontal field of view, so the ±7 world-unit ring spills
 * out; and a 40px face does not shrink with distance, so jittered anchors a
 * fraction of a unit apart overlap on screen. The "No project" cluster sits
 * below you and behind the front of the ring, which projects to almost the
 * same screen row — overlap in 2D that no 3D spacing check would ever see.
 *
 * So this works in screen space, with the real camera and the real pixel sizes:
 *   1. pick the camera distance (along the default view direction) so every
 *      body — face, chip, you — lands inside the viewport minus its own
 *      half-extent and the chrome insets;
 *   2. push overlapping bodies apart on screen, moving each in the camera's
 *      right/up plane at its own depth (so it stays where it projects);
 *   3. refit, never zooming back in (that would re-create the overlap), and
 *      repeat until nothing moves.
 *
 * Follow-up (same day): the free rectangle excludes the Key's MEASURED rect
 * (`insetsAvoiding`), the drawing is centred in that rectangle (the camera's
 * principal point sits at its centre — `CameraPose.center`), and a wide ring
 * is spread vertically (capped) so it uses the rectangle's height as well as
 * its width; a few camera elevations are tried and the best-filling one kept.
 *
 * Pure: plain tuples, no three.js, no React.
 */

import type { GraphCluster, GraphNode } from './peopleGraph';
import { isBridge, isYou, UNASSIGNED_CLUSTER_ID } from './peopleGraph';

export type Vec3 = [number, number, number];

export interface Viewport {
  width: number;
  height: number;
}

export interface Insets {
  top: number;
  right: number;
  bottom: number;
  left: number;
}

/** A thing drawn at a fixed pixel size around a world anchor. */
export interface Body {
  id: string;
  p: Vec3;
  /** Half width / half height on screen, px. */
  hw: number;
  hh: number;
  /** 0 = pinned (you), 1 = moves freely. */
  mobility: number;
}

export interface CameraPose {
  position: Vec3;
  target: Vec3;
  /** Vertical field of view, degrees. */
  fov: number;
  /** Where `target` lands on screen, px — the centre of the free rectangle.
   *  Omitted = the canvas centre. The renderer applies it with
   *  `camera.setViewOffset`, so orbiting still pivots on the graph. */
  center?: { x: number; y: number };
}

export interface FittedGraph {
  camera: CameraPose;
  /** Adjusted anchor per node id. */
  nodes: Map<string, Vec3>;
  /** Adjusted chip anchor per cluster id. */
  labels: Map<string, Vec3>;
}

/** The view direction the graph was designed for (camera at [0, 7.5, 14]). */
export const GRAPH_VIEW_DIR: Vec3 = normalize([0, 7.5, 14]);
export const GRAPH_FOV = 50;
/** The cluster chip rides this far above its centroid. */
export const LABEL_LIFT = 1.6;
/** Space between two bodies on screen, px. */
const GAP = 6;

// ── vector helpers ──────────────────────────────────────────────────────────
const add = (a: Vec3, b: Vec3): Vec3 => [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
const sub = (a: Vec3, b: Vec3): Vec3 => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const scale = (a: Vec3, s: number): Vec3 => [a[0] * s, a[1] * s, a[2] * s];
const dot = (a: Vec3, b: Vec3): number => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const cross = (a: Vec3, b: Vec3): Vec3 => [
  a[1] * b[2] - a[2] * b[1],
  a[2] * b[0] - a[0] * b[2],
  a[0] * b[1] - a[1] * b[0],
];
function normalize(a: Vec3): Vec3 {
  const l = Math.hypot(a[0], a[1], a[2]) || 1;
  return [a[0] / l, a[1] / l, a[2] / l];
}

/** Camera basis for a camera looking back along `dir` (world up = +Y). */
export function cameraBasis(dir: Vec3): { forward: Vec3; right: Vec3; up: Vec3 } {
  const forward = scale(normalize(dir), -1);
  const right = normalize(cross(forward, [0, 1, 0]));
  const up = cross(right, forward);
  return { forward, right, up };
}

/** Focal length in px for a vertical fov. */
export function focalPx(viewport: Viewport, fovDeg: number): number {
  return (viewport.height / 2) / Math.tan((fovDeg * Math.PI) / 360);
}

/** Where a world point lands on screen (px from top-left), plus its depth. */
export function projectPoint(p: Vec3, cam: CameraPose, viewport: Viewport): { x: number; y: number; depth: number } {
  const { forward, right, up } = cameraBasis(sub(cam.position, cam.target));
  const f = focalPx(viewport, cam.fov);
  const q = sub(p, cam.position);
  const depth = dot(q, forward);
  const c = cam.center ?? { x: viewport.width / 2, y: viewport.height / 2 };
  return {
    x: c.x + (f * dot(q, right)) / depth,
    y: c.y - (f * dot(q, up)) / depth,
    depth,
  };
}

/**
 * The smallest camera distance from `target` (along `dir`) at which every body,
 * with its pixel half-extent, sits inside the viewport minus `insets`.
 */
export function requiredDistance(
  bodies: Body[],
  target: Vec3,
  dir: Vec3,
  viewport: Viewport,
  insets: Insets,
  fovDeg = GRAPH_FOV,
): number {
  const { right, up } = cameraBasis(dir);
  const d = normalize(dir);
  const f = focalPx(viewport, fovDeg);
  // Measured from the free rectangle's centre, which is where `target`
  // projects (CameraPose.center) — so every side has room, even when the
  // rectangle is off to one side of the canvas (the Key is open).
  const cx = (insets.left + viewport.width - insets.right) / 2;
  const cy = (insets.top + viewport.height - insets.bottom) / 2;
  let need = 1;
  for (const b of bodies) {
    const q = sub(b.p, target);
    const toward = dot(q, d);
    const lx = dot(q, right);
    const ly = dot(q, up);
    // Room from the screen centre to the edge on the side the body is on,
    // less its own half-extent. Floor at 1px so a body wider than the whole
    // viewport degrades to "as far as it takes" instead of dividing by zero.
    const roomX = Math.max(1, (lx >= 0 ? viewport.width - insets.right - cx : cx - insets.left) - b.hw);
    const roomY = Math.max(1, (ly >= 0 ? cy - insets.top : viewport.height - insets.bottom - cy) - b.hh);
    const depth = Math.max(1, (f * Math.abs(lx)) / roomX, (f * Math.abs(ly)) / roomY);
    need = Math.max(need, toward + depth);
  }
  return need;
}

/**
 * Push overlapping bodies apart on screen. Mutates `bodies[i].p`. Returns the
 * largest remaining overlap in px (0 when clear).
 */
export function relaxOnScreen(bodies: Body[], cam: CameraPose, viewport: Viewport, iterations = 80): number {
  const { right, up } = cameraBasis(sub(cam.position, cam.target));
  const f = focalPx(viewport, cam.fov);
  let worst = 0;
  for (let it = 0; it < iterations; it++) {
    const scr = bodies.map(b => projectPoint(b.p, cam, viewport));
    const shift = bodies.map(() => [0, 0] as [number, number]);
    worst = 0;
    for (let i = 0; i < bodies.length; i++) {
      for (let j = i + 1; j < bodies.length; j++) {
        const a = bodies[i];
        const b = bodies[j];
        const mob = a.mobility + b.mobility;
        if (mob === 0) continue;
        const dx = scr[j].x - scr[i].x;
        const dy = scr[j].y - scr[i].y;
        const ox = a.hw + b.hw + GAP - Math.abs(dx);
        const oy = a.hh + b.hh + GAP - Math.abs(dy);
        if (ox <= 0 || oy <= 0) continue;
        worst = Math.max(worst, Math.min(ox, oy));
        // A zero offset has no sign: use a stable, id-derived one so two
        // anchors on the same pixel still separate (deterministically).
        const tie = a.id < b.id ? 1 : -1;
        const sx = dx !== 0 ? Math.sign(dx) : tie;
        const sy = dy !== 0 ? Math.sign(dy) : tie;
        // Separate along the axis that needs the least movement.
        const push: [number, number] = ox < oy ? [sx * ox, 0] : [0, sy * oy];
        const wa = a.mobility / mob;
        const wb = b.mobility / mob;
        shift[i][0] -= push[0] * wa; shift[i][1] -= push[1] * wa;
        shift[j][0] += push[0] * wb; shift[j][1] += push[1] * wb;
      }
    }
    if (worst === 0) break;
    for (let i = 0; i < bodies.length; i++) {
      const [sx, sy] = shift[i];
      if (sx === 0 && sy === 0) continue;
      // Half-step: pairwise pushes add up, a full step overshoots and jitters.
      const k = scr[i].depth / f / 2;
      bodies[i].p = add(bodies[i].p, add(scale(right, sx * k), scale(up, -sy * k)));
    }
  }
  return worst;
}

/** Pixel size of a cluster chip: 10px mono, uppercase, 0.06em tracking, 6px side padding. */
export function chipHalfExtent(name: string): { hw: number; hh: number } {
  return { hw: (name.length * 6.6 + 14) / 2, hh: 9 };
}

/** Pixel size of a person face (see PeopleGraphCanvas: 40px, 48 for a bridge). */
export function faceHalfExtent(node: GraphNode): { hw: number; hh: number } {
  if (isYou(node)) return { hw: 18, hh: 20 };
  const half = (isBridge(node) ? 48 : 40) / 2;
  return { hw: half, hh: half };
}

/** A screen rectangle, px from the canvas's top-left. */
export interface Rect {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

/**
 * The insets that leave the biggest clear rectangle once `obstacle` (the open
 * Key panel, measured from the DOM) is taken out of the viewport. The graph is
 * then fitted into that rectangle, so no face or chip can land on the key.
 */
export function insetsAvoiding(viewport: Viewport, base: Insets, obstacle: Rect | null, gap = 8): Insets {
  if (!obstacle) return base;
  const W = viewport.width;
  const H = viewport.height;
  const inner: Rect = { left: base.left, top: base.top, right: W - base.right, bottom: H - base.bottom };
  // Does the obstacle reach into the area at all?
  if (obstacle.right <= inner.left || obstacle.left >= inner.right
    || obstacle.bottom <= inner.top || obstacle.top >= inner.bottom) return base;
  const options: Insets[] = [
    { ...base, left: Math.max(base.left, obstacle.right + gap) },       // right of it
    { ...base, right: Math.max(base.right, W - obstacle.left + gap) },  // left of it
    { ...base, top: Math.max(base.top, obstacle.bottom + gap) },        // below it
    { ...base, bottom: Math.max(base.bottom, H - obstacle.top + gap) }, // above it
  ];
  const area = (i: Insets) => Math.max(0, W - i.left - i.right) * Math.max(0, H - i.top - i.bottom);
  return options.reduce((best, o) => (area(o) > area(best) ? o : best));
}

/** Centre of the free rectangle, px. */
export function areaCenter(viewport: Viewport, insets: Insets): { x: number; y: number } {
  return {
    x: (insets.left + viewport.width - insets.right) / 2,
    y: (insets.top + viewport.height - insets.bottom) / 2,
  };
}

function makeBodies(nodes: GraphNode[], clusters: GraphCluster[]): Body[] {
  return [
    ...nodes.map(n => ({ id: `n:${n.id}`, p: [n.x, n.y, n.z] as Vec3, ...faceHalfExtent(n), mobility: isYou(n) ? 0 : 1 })),
    // A chip may slide a little, but less than a face: it names a place.
    ...clusters.map(c => ({ id: `c:${c.id}`, p: [c.x, c.y + LABEL_LIFT, c.z] as Vec3, ...chipHalfExtent(c.name), mobility: 0.5 })),
  ];
}

/** Screen bounding box of the bodies, half-extents included. */
export function screenBounds(bodies: Body[], cam: CameraPose, viewport: Viewport): Rect {
  const r: Rect = { left: Infinity, top: Infinity, right: -Infinity, bottom: -Infinity };
  for (const b of bodies) {
    const s = projectPoint(b.p, cam, viewport);
    r.left = Math.min(r.left, s.x - b.hw);
    r.right = Math.max(r.right, s.x + b.hw);
    r.top = Math.min(r.top, s.y - b.hh);
    r.bottom = Math.max(r.bottom, s.y + b.hh);
  }
  return r;
}

/**
 * Closest camera (along `dir`) that fits, with the look-at point slid so the
 * drawing's box is CENTRED in the available rectangle rather than hugging the
 * side that happened to bind first. `minDist` keeps a later round from zooming
 * back in on faces that were just pushed apart.
 */
function fitCamera(
  bodies: Body[], dir: Vec3, viewport: Viewport, insets: Insets, fovDeg: number, minDist: number, start: Vec3,
): { target: Vec3; dist: number } {
  const { right, up } = cameraBasis(dir);
  const f = focalPx(viewport, fovDeg);
  const center = areaCenter(viewport, insets);
  const { x: cx, y: cy } = center;
  let target = start;
  let dist = minDist;
  for (let k = 0; k < 12; k++) {
    dist = Math.max(minDist, requiredDistance(bodies, target, dir, viewport, insets, fovDeg));
    const cam: CameraPose = { position: add(target, scale(dir, dist)), target, fov: fovDeg, center };
    const b = screenBounds(bodies, cam, viewport);
    const dx = (b.left + b.right) / 2 - cx;
    const dy = (b.top + b.bottom) / 2 - cy;
    if (Math.abs(dx) < 0.5 && Math.abs(dy) < 0.5) break;
    // Content right of centre → move the camera right; below → move it down.
    const k2 = dist / f;
    target = add(target, add(scale(right, dx * k2), scale(up, -dy * k2)));
  }
  dist = Math.max(minDist, requiredDistance(bodies, target, dir, viewport, insets, fovDeg));
  return { target, dist };
}

/** Camera elevations tried, degrees above the ground plane. 28° is the
 *  original design angle; steeper ones spread the ring's depth down the
 *  screen, which is what fills a tall rectangle. */
const ELEVATIONS = [28, 36, 44, 52, 60, 68];
/** How far the drawing may be spread vertically to fill a tall rectangle. */
const MAX_STRETCH = 1.8;

function fitAlong(
  nodes: GraphNode[], clusters: GraphCluster[], viewport: Viewport, insets: Insets, dir: Vec3, fovDeg: number,
): { fitted: FittedGraph; fill: number } {
  const bodies = makeBodies(nodes, clusters);
  const lo: Vec3 = [Infinity, Infinity, Infinity];
  const hi: Vec3 = [-Infinity, -Infinity, -Infinity];
  for (const b of bodies) {
    for (let k = 0; k < 3; k++) {
      lo[k] = Math.min(lo[k], b.p[k]);
      hi[k] = Math.max(hi[k], b.p[k]);
    }
  }
  let target: Vec3 = bodies.length > 0 ? scale(add(lo, hi), 0.5) : [0, 0, 0];
  const center = areaCenter(viewport, insets);
  let dist = 0;

  // Use the height too. The ring is a wide shape (projects fan left and
  // right of you, and the chips are long), so a uniform zoom fills the width
  // and leaves the rectangle's bottom third empty (QA 2026-09-24). Spread the
  // drawing along the camera's up axis until its aspect matches the free
  // rectangle — capped, so the ring still reads as a ring — then fit again.
  ({ target, dist } = fitCamera(bodies, dir, viewport, insets, fovDeg, 0, target));
  {
    const cam: CameraPose = { position: add(target, scale(dir, dist)), target, fov: fovDeg, center };
    const b = screenBounds(bodies, cam, viewport);
    const aw = viewport.width - insets.left - insets.right;
    const ah = viewport.height - insets.top - insets.bottom;
    const bw = Math.max(1, b.right - b.left);
    const bh = Math.max(1, b.bottom - b.top);
    const stretch = Math.min(MAX_STRETCH, Math.max(1, (ah / aw) / (bh / bw)));
    if (stretch > 1.01) {
      const { up } = cameraBasis(dir);
      const f = focalPx(viewport, fovDeg);
      const midY = (b.top + b.bottom) / 2;
      for (const body of bodies) {
        if (body.mobility === 0) continue; // you stay the fixed hub
        const sp = projectPoint(body.p, cam, viewport);
        const dy = (sp.y - midY) * (stretch - 1);
        body.p = add(body.p, scale(up, (-dy * sp.depth) / f));
      }
    }
    dist = 0; // re-fit from scratch after the spread
  }

  for (let round = 0; round < 8; round++) {
    ({ target, dist } = fitCamera(bodies, dir, viewport, insets, fovDeg, dist, target));
    const cam: CameraPose = { position: add(target, scale(dir, dist)), target, fov: fovDeg, center };
    const worst = relaxOnScreen(bodies, cam, viewport);
    const refit = requiredDistance(bodies, target, dir, viewport, insets, fovDeg);
    if (worst === 0 && refit <= dist) break;
  }
  ({ target, dist } = fitCamera(bodies, dir, viewport, insets, fovDeg, dist, target));

  // The chips were relaxed as free bodies alongside the faces, so a big
  // cluster's faces spread out while its chip stayed at the ring centroid —
  // QA 2026-09-24 at 1456px: "NO PROJECT" and "HARBOURVIEW RESIDENTS'
  // ASSOCIATION" floated far from their people. Re-anchor every chip on its
  // own members at the final camera; only chips move, the camera does not.
  anchorClusterLabels(nodes, bodies, {
    position: add(target, scale(dir, dist)), target, fov: fovDeg, center,
  }, viewport, insets);
  // Moved chips shift the drawing's box: re-centre by panning only. Same
  // distance, so the on-screen spacing between faces is unchanged.
  {
    const { right, up } = cameraBasis(dir);
    const f = focalPx(viewport, fovDeg);
    for (let k = 0; k < 8; k++) {
      const cam: CameraPose = { position: add(target, scale(dir, dist)), target, fov: fovDeg, center };
      const b = screenBounds(bodies, cam, viewport);
      const dx = (b.left + b.right) / 2 - center.x;
      const dy = (b.top + b.bottom) / 2 - center.y;
      if (Math.abs(dx) < 0.5 && Math.abs(dy) < 0.5) break;
      target = add(target, add(scale(right, dx * dist / f), scale(up, -dy * dist / f)));
    }
  }

  const camera: CameraPose = { position: add(target, scale(dir, dist)), target, fov: fovDeg, center };
  const b = screenBounds(bodies, camera, viewport);
  const aw = viewport.width - insets.left - insets.right;
  const ah = viewport.height - insets.top - insets.bottom;
  const fill = ((b.right - b.left) * (b.bottom - b.top)) / Math.max(1, aw * ah);
  const fitted: FittedGraph = { camera, nodes: new Map(), labels: new Map() };
  for (const body of bodies) {
    if (body.id.startsWith('n:')) fitted.nodes.set(body.id.slice(2), body.p);
    else fitted.labels.set(body.id.slice(2), body.p);
  }
  return { fitted, fill };
}

/**
 * The people a cluster's chip should name: its single-project members when it
 * has any (a bridge person sits BETWEEN clusters, so averaging them in drags
 * the chip toward the neighbour), else everyone in it.
 */
export function clusterMembers(nodes: GraphNode[], clusterId: string): GraphNode[] {
  const inIt = nodes.filter(n => !isYou(n) && (n.projectIds.length === 0
    ? clusterId === UNASSIGNED_CLUSTER_ID
    : n.projectIds.includes(clusterId)));
  const solo = inIt.filter(n => n.projectIds.length <= 1);
  return solo.length > 0 ? solo : inIt;
}

/**
 * Put each cluster chip right next to its members on screen: at their screen
 * centroid, one gap above the top-most face — or, when that spot is taken or
 * outside the free rectangle, the nearest clear spot searched outward (below
 * the lowest face, sideways, further up/down). Faces never move and the camera
 * never changes here: zooming out to make room would shrink the spacing
 * between fixed-pixel faces and re-stack them. Mutates chip bodies.
 */
export function anchorClusterLabels(
  nodes: GraphNode[], bodies: Body[], cam: CameraPose, viewport: Viewport, insets: Insets,
): void {
  const { right, up } = cameraBasis(sub(cam.position, cam.target));
  const f = focalPx(viewport, cam.fov);
  const byId = new Map(bodies.map(b => [b.id, b]));
  // Everything already on screen that a chip must not land on.
  const placed: { x: number; y: number; hw: number; hh: number }[] = bodies
    .filter(b => b.id.startsWith('n:'))
    .map(b => ({ ...projectPoint(b.p, cam, viewport), hw: b.hw, hh: b.hh }));
  const inside = (x: number, y: number, hw: number, hh: number) =>
    x - hw >= insets.left && x + hw <= viewport.width - insets.right
    && y - hh >= insets.top && y + hh <= viewport.height - insets.bottom;
  const clear = (x: number, y: number, hw: number, hh: number) => placed.every(o =>
    Math.abs(o.x - x) >= o.hw + hw + GAP - 0.01 || Math.abs(o.y - y) >= o.hh + hh + GAP - 0.01);

  for (const chip of bodies) {
    if (!chip.id.startsWith('c:')) continue;
    const members = clusterMembers(nodes, chip.id.slice(2))
      .map(n => byId.get(`n:${n.id}`))
      .filter((b): b is Body => !!b);
    if (members.length === 0) continue;
    let world: Vec3 = [0, 0, 0];
    for (const m of members) world = add(world, m.p);
    world = scale(world, 1 / members.length);
    let sx = 0;
    let top = Infinity;
    let bottom = -Infinity;
    for (const m of members) {
      const s = projectPoint(m.p, cam, viewport);
      sx += s.x;
      top = Math.min(top, s.y - m.hh);
      bottom = Math.max(bottom, s.y + m.hh);
    }
    sx /= members.length;
    const { hw, hh } = chip;
    const rowStep = 2 * hh + GAP;
    // Candidate spots, nearest first: above / below the members, then sideways
    // and one row further out. First one that is inside and clear wins.
    const ys: number[] = [];
    for (let k = 0; k < 4; k++) ys.push(top - GAP - hh - k * rowStep, bottom + GAP + hh + k * rowStep);
    const xs: number[] = [sx];
    for (let k = 1; k <= 6; k++) xs.push(sx - k * hw * 0.5, sx + k * hw * 0.5);
    let best: { x: number; y: number } | null = null;
    let fallback: { x: number; y: number } | null = null;
    for (const y of ys) {
      for (const x0 of xs) {
        // Keep the chip in the free rectangle horizontally.
        const x = Math.min(viewport.width - insets.right - hw, Math.max(insets.left + hw, x0));
        if (!inside(x, y, hw, hh)) continue;
        if (!fallback) fallback = { x, y };
        if (clear(x, y, hw, hh)) { best = { x, y }; break; }
      }
      if (best) break;
    }
    const spot = best ?? fallback ?? { x: sx, y: top - GAP - hh };
    const at = projectPoint(world, cam, viewport);
    const k = at.depth / f;
    chip.p = add(world, add(scale(right, (spot.x - at.x) * k), scale(up, -(spot.y - at.y) * k)));
    placed.push({ x: spot.x, y: spot.y, hw, hh });
  }
}

/**
 * Fit the whole graph into the available rectangle: centred, as large as it
 * goes, nothing clipped and nothing stacked. Tries a few camera elevations and
 * keeps the one whose drawing fills the rectangle best (a steeper view uses
 * the height a wide, shallow ring leaves empty). Deterministic.
 */
export function fitPeopleGraph(
  nodes: GraphNode[],
  clusters: GraphCluster[],
  viewport: Viewport,
  insets: Insets,
  dir?: Vec3,
  fovDeg = GRAPH_FOV,
): FittedGraph {
  // A big directory skips the search: each try is O(n²) per relax step.
  const dirs: Vec3[] = dir
    ? [normalize(dir)]
    : (nodes.length > 120 ? [GRAPH_VIEW_DIR] : ELEVATIONS.map(deg => {
      const a = (deg * Math.PI) / 180;
      return [0, Math.sin(a), Math.cos(a)] as Vec3;
    }));
  let best: { fitted: FittedGraph; fill: number } | null = null;
  for (const d of dirs) {
    const r = fitAlong(nodes, clusters, viewport, insets, d, fovDeg);
    // Only switch away from a shallower view for a clear gain (5%).
    if (!best || r.fill > best.fill * 1.05) best = r;
  }
  return best!.fitted;
}
