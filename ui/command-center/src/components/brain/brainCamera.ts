/**
 * Framing and label-size math for the Brain graph (BrainScene).
 *
 * QA 2026-09-24 at 1456px: Graph opened with the camera INSIDE the layout — a
 * giant blurred "market scanner" label spanned half the screen and a diamond
 * node filled the top. Root cause: the orbit radius was the constant 32 while
 * the force layout of ~80 entities and their memories settles well past that
 * radius, so nodes sat beside (or behind) the lens. Labels were world-sized
 * canvas sprites (1.05 units tall), so a node two units away drew its 40px
 * bitmap stretched to hundreds of pixels — hence the blur.
 *
 * The fix, as pure functions so it can be tested without WebGL:
 *   - frame the whole node set: bounding sphere → camera distance that fits
 *     it inside BOTH the vertical and horizontal field of view, with margin;
 *   - size every label in screen pixels, clamped to a max (and never above the
 *     bitmap's own resolution, so it is only ever downsampled — crisp);
 *   - show only a budget of labels: forced ones (you, hover ring, search), then
 *     the most important/nearest, faded by distance.
 *
 * Pure: plain tuples, no three.js.
 */

export type Vec3 = [number, number, number];

export interface Sphere {
  center: Vec3;
  radius: number;
}

/**
 * Bounding sphere of a point set: centre of the axis-aligned box, radius to
 * the farthest point (plus `pad` for the node bodies themselves). An empty set
 * is a unit sphere at the origin so the camera still has somewhere to be.
 */
export function boundingSphere(points: Vec3[], pad = 0): Sphere {
  if (points.length === 0) return { center: [0, 0, 0], radius: 1 };
  const lo: Vec3 = [Infinity, Infinity, Infinity];
  const hi: Vec3 = [-Infinity, -Infinity, -Infinity];
  for (const p of points) {
    for (let k = 0; k < 3; k++) {
      if (p[k] < lo[k]) lo[k] = p[k];
      if (p[k] > hi[k]) hi[k] = p[k];
    }
  }
  const center: Vec3 = [(lo[0] + hi[0]) / 2, (lo[1] + hi[1]) / 2, (lo[2] + hi[2]) / 2];
  let r2 = 0;
  for (const p of points) {
    const dx = p[0] - center[0];
    const dy = p[1] - center[1];
    const dz = p[2] - center[2];
    r2 = Math.max(r2, dx * dx + dy * dy + dz * dz);
  }
  return { center, radius: Math.max(1, Math.sqrt(r2) + pad) };
}

/** Horizontal field of view (radians) for a vertical fov and an aspect ratio. */
export function horizontalFov(fovYDeg: number, aspect: number): number {
  const fy = (fovYDeg * Math.PI) / 180;
  return 2 * Math.atan(Math.tan(fy / 2) * Math.max(0.01, aspect));
}

/**
 * Camera distance from the sphere centre at which the whole sphere fits the
 * narrower of the two fields of view, times `margin` (1.15 = 15% breathing
 * room). The sphere's silhouette is tangent to the frustum at r / sin(fov/2).
 */
export function fitDistance(radius: number, fovYDeg: number, aspect: number, margin = 1.15): number {
  const fy = (fovYDeg * Math.PI) / 180;
  const fx = horizontalFov(fovYDeg, aspect);
  const half = Math.min(fy, fx) / 2;
  return (radius * margin) / Math.sin(half);
}

/** World units covered by one screen pixel at `distance` from the camera. */
export function worldPerPixel(distance: number, fovYDeg: number, viewportHeightPx: number): number {
  const fy = (fovYDeg * Math.PI) / 180;
  return (2 * Math.max(0.001, distance) * Math.tan(fy / 2)) / Math.max(1, viewportHeightPx);
}

/** Pixels of chrome overlapping each edge of a fixed-FOV canvas — a header, a
 *  bottom bar, an open side panel. The canvas itself never crops or resizes;
 *  only the camera's framing accounts for what these cover. */
export interface ContentInsets {
  top: number;
  right: number;
  bottom: number;
  left: number;
}

/**
 * Fit a bounding sphere into the sub-rectangle of a `viewportW × viewportH`
 * canvas left over after `insets`, for a camera that always renders the FULL
 * canvas (no cropping/`setViewOffset`).
 *
 * QA 2026-09-25 at 1440×900: the fit used the whole viewport, so the bottom
 * of the graph sat under the time-slider bar. Two things change relative to
 * `fitDistance`: the camera backs off further, sized to the SMALLER effective
 * field of view the visible band actually subtends (computed from real pixel
 * geometry, not a small-angle guess); and the orbit pivot pans in the current
 * camera's own right/up plane so the sphere's on-screen centre lands in the
 * middle of the visible rect rather than the full canvas's centre — derived
 * so that a point on the camera's optical axis (screen centre) reprojects to
 * exactly the visible rect's centre pixel.
 */
export function fitToInsetRect(
  radius: number, fovYDeg: number, viewportW: number, viewportH: number,
  insets: ContentInsets, margin = 1.15,
): { distance: number; panRight: number; panUp: number } {
  const visW = Math.max(1, viewportW - insets.left - insets.right);
  const visH = Math.max(1, viewportH - insets.top - insets.bottom);
  const fy = (fovYDeg * Math.PI) / 180;
  const aspect = viewportW / Math.max(1, viewportH);
  const fx = horizontalFov(fovYDeg, aspect);
  const halfY = Math.atan((visH / viewportH) * Math.tan(fy / 2));
  const halfX = Math.atan((visW / viewportW) * Math.tan(fx / 2));
  const distance = (radius * margin) / Math.sin(Math.min(halfY, halfX));
  // The visible rect's centre, in pixels offset from the full canvas's own
  // centre (+x right, +y down — standard screen coordinates).
  const offsetPxX = (insets.left - insets.right) / 2;
  const offsetPxY = (insets.top - insets.bottom) / 2;
  const upp = worldPerPixel(distance, fovYDeg, viewportH);
  return { distance, panRight: -offsetPxX * upp, panUp: offsetPxY * upp };
}

export interface LabelSizing {
  /** Height the label would have in the world if it were not clamped. */
  worldHeight: number;
  /** On-screen ceiling, CSS px. */
  maxPx: number;
  /** On-screen floor, CSS px — below this a label is unreadable noise. */
  minPx: number;
  /** The bitmap's own height in CSS px (canvas px / its supersample).
   *  The label is never drawn taller than this, so it is never upscaled. */
  nativePx: number;
}

/**
 * On-screen height (CSS px) a label should be drawn at: its natural projected
 * size, clamped to [minPx, min(maxPx, nativePx)].
 */
export function labelScreenPx(distance: number, fovYDeg: number, viewportHeightPx: number, s: LabelSizing): number {
  const upp = worldPerPixel(distance, fovYDeg, viewportHeightPx);
  const natural = s.worldHeight / upp;
  const ceiling = Math.min(s.maxPx, s.nativePx);
  return Math.max(Math.min(s.minPx, ceiling), Math.min(ceiling, natural));
}

/** World height to give a sprite so it lands at `labelScreenPx`. */
export function labelWorldHeight(distance: number, fovYDeg: number, viewportHeightPx: number, s: LabelSizing): number {
  return labelScreenPx(distance, fovYDeg, viewportHeightPx, s) * worldPerPixel(distance, fovYDeg, viewportHeightPx);
}

export interface LabelCandidate {
  id: string;
  /** Distance from the camera. */
  distance: number;
  /** Larger = more deserving of a name (hub degree, node size). 0 = only when forced. */
  importance: number;
  /** Always shown (you, hovered neighbourhood, search focus). Not counted against the budget. */
  forced: boolean;
}

/**
 * Which labels to draw. Forced ones always; then up to `budget` of the rest,
 * most important first, nearer breaking ties. Candidates with importance 0
 * (memories) are only ever shown when forced.
 */
export function pickLabels(cands: LabelCandidate[], budget: number): Set<string> {
  const out = new Set<string>();
  const rest: LabelCandidate[] = [];
  for (const c of cands) {
    if (c.forced) out.add(c.id);
    else if (c.importance > 0) rest.push(c);
  }
  rest.sort((a, b) => (b.importance - a.importance) || (a.distance - b.distance) || (a.id < b.id ? -1 : 1));
  for (let i = 0; i < rest.length && i < budget; i++) out.add(rest[i].id);
  return out;
}

/** A label's on-screen box, in pixels, for collision culling. */
export interface LabelBox {
  id: string;
  /** Centre, screen px. */
  x: number;
  y: number;
  width: number;
  height: number;
  /** Higher wins a collision. Caller's scale (e.g. hovered > selected > degree). */
  priority: number;
}

/**
 * Screen-space label collision culling (Obsidian critique again: a dense
 * centre with several similar-sized nodes drew overlapping names — "Nadia
 * Petrova" over "Priya Chandrasekaran" — illegible, and neither one more
 * readable than the other). Greedy by priority: highest first, each box kept
 * unless it overlaps one already kept. Ties break on `id` so the result is
 * deterministic frame to frame (no flicker from an unstable sort).
 */
export function cullOverlappingLabels(boxes: LabelBox[]): Set<string> {
  const sorted = [...boxes].sort((a, b) => (b.priority - a.priority) || (a.id < b.id ? -1 : 1));
  const kept: LabelBox[] = [];
  const keptIds = new Set<string>();
  for (const b of sorted) {
    const overlapsKept = kept.some(k =>
      Math.abs(b.x - k.x) * 2 < (b.width + k.width) && Math.abs(b.y - k.y) * 2 < (b.height + k.height));
    if (!overlapsKept) { kept.push(b); keptIds.add(b.id); }
  }
  return keptIds;
}

/**
 * Distance fade for a shown label: full strength up to the framed centre
 * distance, fading to `floor` at the far side of the framed sphere. Nearer
 * labels read first; the back of the graph recedes instead of competing.
 */
export function labelFade(distance: number, fitDist: number, radius: number, floor = 0.35): number {
  const near = fitDist;
  const far = fitDist + Math.max(0.001, radius);
  if (distance <= near) return 1;
  if (distance >= far) return floor;
  return 1 - ((distance - near) / (far - near)) * (1 - floor);
}
