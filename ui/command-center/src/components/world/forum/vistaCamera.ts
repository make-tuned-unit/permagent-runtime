import landmarks from './vistaLandmarks.json';

/**
 * The World's default vantage.
 *
 * Before this, the app opened — and the "Commons" district focus returned to —
 * a steep orbit above the paving at `(29, 29, 36)` looking at `(0, 1, 0)`
 * (`assets/world/forum/browser/v3-world-night.webp`). That frame is the
 * commons and nothing else: job 19 measured **0 of 4,056** tower window-strip
 * vertices inside it (`docs/design/solar-forum/jobs/19-night-recheck.md`,
 * bug 4), and no ridge, ring or celestial body reads at all.
 *
 * The north star (`docs/design/solar-forum/north-star.png`) is a low, wide
 * vista from a terrace edge: the gathering terrace and its hologram globe in
 * the foreground, the lagoon and its islands in the middle, and the ridge, the
 * orbital rings, the floating gardens and the sky bodies behind. This is that
 * camera, and the numbers are the ones the frame test below pins:
 *
 *   position  (0, 10, 23)  — over the south overlook, 4 m beyond the balustrade
 *                            at Blender y = -18.8
 *   target    (27, 1, -65) — 92.5 m out, yaw +17.06 deg east of north,
 *                            pitch -5.58 deg
 *   fov       72 deg vertical
 *
 * Why the view axis is swung 17 degrees east of due north rather than looking
 * straight up the commons axis: the two landmarks that have to share the frame
 * sit about 51 degrees apart as seen from here — the nearest NE spire is at
 * azimuth +42 deg and the ridge's eastern end (the Blender polar-100 arc, with
 * its waterfall) is at -8.5 deg. Their bisector is +17 deg, and a horizontal
 * half-field of at least 26 deg is the smallest that holds both — which on the
 * app's 702 x 972 canvas is 72 deg vertical, not the 48 the canvas shipped
 * with.
 *
 * Why the eye is at 10 m and not at rail height: the Gallery's raised garden
 * deck crosses the view at z = -20 and tops out near 9 m. From 6-7 m it closes
 * off the whole middle distance; from 10 m the frame clears it and the far
 * shore, the ridge and the sky open up above it. (The lagoon surface itself is
 * still behind that deck — it needs an eye above about 14 m, which is an
 * overlook rather than a terrace edge. Job 20 records that.)
 *
 * Why the target is BELOW the camera: `OrbitControls` clamps the polar angle to
 * `maxPolarAngle` (Math.PI / 2.05 = 87.8 deg) on every `update()`, so a target
 * above the camera is silently pulled back down and the pose would not survive
 * its first frame. Here the polar angle is 84.4 deg, inside the clamp.
 */
export const FORUM_VISTA = {
  position: [0, 10, 23],
  target: [27, 1, -65],
  fov: 72,
} as const satisfies { position: [number, number, number] | readonly number[]; target: readonly number[]; fov: number };

/** The field every other district focus keeps: those camera poses were framed
 *  for it and are deliberately untouched by the vista work. */
export const FORUM_BASE_FOV = 48;
/** Walk mode keeps the same field it always had — a vista lens on a pair of
 *  eyes reads as a fisheye. */
export const FORUM_WALK_FOV = FORUM_BASE_FOV;

export type VistaLandmark = { id: string; label: string; source: string; points: number[][] };
export const VISTA_LANDMARKS: VistaLandmark[] = landmarks.landmarks as VistaLandmark[];

type Vec3 = readonly number[];
const sub = (a: Vec3, b: Vec3): number[] => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const dot = (a: Vec3, b: Vec3) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const cross = (a: Vec3, b: Vec3): number[] => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const unit = (a: Vec3): number[] => { const m = Math.hypot(a[0], a[1], a[2]); return [a[0] / m, a[1] / m, a[2] / m]; };

/** Normalised device coordinates of a world point, or `null` when it is behind
 *  the camera. Plain arithmetic rather than three's `Vector3.project` so the
 *  frame test needs no WebGL context and no scene. */
export function projectPoint(point: Vec3, camera = FORUM_VISTA, aspect = 702 / 972) {
  const forward = unit(sub(camera.target, camera.position));
  const right = unit(cross(forward, [0, 1, 0]));
  const up = cross(right, forward);
  const d = sub(point, camera.position);
  const z = dot(d, forward);
  if (z <= 0.01) return null;
  const t = Math.tan((camera.fov * Math.PI) / 180 / 2);
  return { x: dot(d, right) / z / (t * aspect), y: dot(d, up) / z / t, distance: Math.hypot(d[0], d[1], d[2]) };
}

export type LandmarkFrameRow = {
  id: string; label: string; source: string;
  /** How many sample points the landmark has, and how many land inside. */
  points: number; inFrame: number; visible: boolean;
  /** NDC of the sample point closest to the middle of the frame. */
  nearestToCentre: { x: number; y: number; distanceM: number } | null;
};

/** Which of the north-star landmarks the given camera actually frames. This is
 *  the measurement behind `docs/design/solar-forum/jobs/20-vista-camera.md`;
 *  the same table is recomputed in the browser against the live camera by
 *  `scripts/capture-vista-v4.mjs`. */
export function frameLandmarks(camera = FORUM_VISTA, aspect = 702 / 972): LandmarkFrameRow[] {
  return VISTA_LANDMARKS.map(landmark => {
    let inFrame = 0;
    let nearest: LandmarkFrameRow['nearestToCentre'] = null;
    let nearestScore = Infinity;
    for (const point of landmark.points) {
      const ndc = projectPoint(point, camera, aspect);
      if (!ndc) continue;
      const score = Math.max(Math.abs(ndc.x), Math.abs(ndc.y));
      if (Math.abs(ndc.x) <= 1 && Math.abs(ndc.y) <= 1) inFrame++;
      if (score < nearestScore) {
        nearestScore = score;
        nearest = { x: Math.round(ndc.x * 1000) / 1000, y: Math.round(ndc.y * 1000) / 1000, distanceM: Math.round(ndc.distance) };
      }
    }
    return { id: landmark.id, label: landmark.label, source: landmark.source, points: landmark.points.length, inFrame, visible: inFrame > 0, nearestToCentre: nearest };
  });
}
