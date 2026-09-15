/** @vitest-environment jsdom */
// What the default vantage has to show, measured rather than eyeballed.
//
// Job 19 (`docs/design/solar-forum/jobs/19-night-recheck.md`, bug 4) found the
// old commons camera framed the paving and nothing else: 0 of 4,056 tower
// window-strip vertices projected into it. These tests project the landmark
// table in `vistaLandmarks.json` — derived from the Blender authoring scripts
// named in each row's `source` — through the shipped camera and assert what
// lands inside the canvas.
import { expect, it } from 'vitest';
import { FORUM_VISTA, FORUM_BASE_FOV, FORUM_WALK_FOV, VISTA_LANDMARKS, frameLandmarks, projectPoint } from './vistaCamera';

/** The forum canvas inside the Command Center at a 1280 x 1000 viewport, as
 *  job 19 measured it and as `capture-vista-v4.mjs` re-measures it live. */
const CANVAS = { width: 702, height: 972 };
const ASPECT = CANVAS.width / CANVAS.height;
const rows = frameLandmarks(FORUM_VISTA, ASPECT);
const row = (id: string) => rows.find(r => r.id === id)!;

it('reads as a low, wide vista rather than an orbit of the paving', () => {
  const forward = [
    FORUM_VISTA.target[0] - FORUM_VISTA.position[0],
    FORUM_VISTA.target[1] - FORUM_VISTA.position[1],
    FORUM_VISTA.target[2] - FORUM_VISTA.position[2],
  ];
  const horizontal = Math.hypot(forward[0], forward[2]);
  const pitch = (Math.atan2(forward[1], horizontal) * 180) / Math.PI;
  // Low: the view axis is within a few degrees of level, not the 39-degree
  // downward stare of the old (29, 29, 36) -> (0, 1, 0) orbit.
  expect(pitch).toBeLessThan(0);
  expect(pitch).toBeGreaterThan(-8);
  // ...and the eye is over the terrace, south of the balustrade (Blender
  // y = -18.8, i.e. three.js z = 18.8), not up on an orbit.
  expect(FORUM_VISTA.position[1]).toBeLessThanOrEqual(12);
  expect(FORUM_VISTA.position[2]).toBeGreaterThan(18.8);
  // Wide, and wider than every other pose in the world.
  expect(FORUM_VISTA.fov).toBeGreaterThan(FORUM_BASE_FOV);
  expect(FORUM_WALK_FOV).toBe(FORUM_BASE_FOV);
  // OrbitControls clamps the polar angle to Math.PI / 2.05 on every update(),
  // so a target above the eye would be pulled back down and this pose would
  // not survive its first frame.
  const toCamera = forward.map(c => -c);
  const polar = (Math.acos(toCamera[1] / Math.hypot(...toCamera)) * 180) / Math.PI;
  expect(polar).toBeLessThan((180 / 2.05));
});

it('frames the foreground, the middle distance and the far landmarks at once', () => {
  // Foreground: the hologram globe, in the lower half and inside the canvas.
  const globe = row('hologram_globe');
  expect(globe.visible).toBe(true);
  expect(globe.nearestToCentre!.y).toBeLessThan(0);
  // Middle: the lagoon and its northern island.
  expect(row('lagoon').visible).toBe(true);
  expect(row('council_island').visible).toBe(true);
  // Far: the things job 19 could not see from the commons at all.
  for (const id of ['ne_spires', 'ridge_crest', 'ridge_waterfalls', 'orbital_ring_1', 'orbital_ring_2', 'floating_islands']) {
    expect(row(id), `${id} is out of frame`).toMatchObject({ visible: true });
  }
  // The spires and the ridge are the two that fix the field: they sit on
  // opposite sides of the frame and both must stay inside it.
  expect(row('ne_spires').nearestToCentre!.x).toBeGreaterThan(0);
  expect(row('ridge_crest').nearestToCentre!.x).toBeLessThan(0);
});

it('records honestly that the western habitats and the south-east dome are behind the viewer', () => {
  // A single vantage on the south edge cannot also show the market habitats
  // (azimuth -93 deg) or the conservatory dome (+132 deg). They are in the
  // table so the report says so instead of leaving them unmentioned.
  for (const id of ['west_habitats', 'se_conservatory_dome']) {
    expect(row(id).visible).toBe(false);
    expect(row(id).nearestToCentre).toBeNull();
  }
});

it('keeps every landmark sample point in front of the camera or explicitly behind it', () => {
  expect(VISTA_LANDMARKS.length).toBeGreaterThan(8);
  for (const landmark of VISTA_LANDMARKS) {
    expect(landmark.points.length).toBeGreaterThan(0);
    for (const point of landmark.points) expect(point).toHaveLength(3);
  }
  // A sanity check on the projection itself: a point straight down the view
  // axis lands in the middle of the frame.
  const axis = FORUM_VISTA.target;
  const centre = projectPoint(axis, FORUM_VISTA, ASPECT)!;
  expect(Math.abs(centre.x)).toBeLessThan(1e-9);
  expect(Math.abs(centre.y)).toBeLessThan(1e-9);
});
