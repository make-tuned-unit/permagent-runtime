// Survey-line footprints — THE_CAVE_vision_bible.md §4: "survey lines glow on bare
// stone where future wings will rise. The roadmap is geography." Each unbuilt
// scaffold mount (kind 'survey') gets its footprint traced as a glowing outline on
// the rock floor of its stratum. Survey lines ARE the Light tier by definition
// (they're light, not stone), so a single dim emissive line material is sanctioned
// here even at blockout — analogous to the hall floor circuits.
//
// W1 owns the footprints. Their event-driven brightening (a wing breaking ground)
// is the W2/W3 detail pass. Real scale. Zero per-frame allocations — outlines are
// static line geometry built once in useMemo.

import { useMemo } from 'react';
import * as THREE from 'three';
import { ENV } from '../../shared/palette';
import { SCAFFOLD_MOUNTS } from './anchors';

// One shared dim line material for every survey footprint (one program).
const surveyMat = new THREE.LineBasicMaterial({
  color: ENV.neonCyan,
  transparent: true,
  opacity: 0.32,
});

export function SurveyLines() {
  // Build one merged-vertex line-segments geometry for ALL survey footprints so
  // the whole roadmap is a single draw call.
  const geometry = useMemo(() => {
    const pts: number[] = [];
    for (const m of SCAFFOLD_MOUNTS) {
      if (m.kind !== 'survey') continue;
      const [x, y, z] = m.position;
      const [fx, fz] = m.footprint;
      const hx = fx / 2;
      const hz = fz / 2;
      // Lift slightly off the rock floor of the stratum to avoid z-fighting.
      const yy = y + 0.04;
      // Rectangle corners (closed loop) → 4 line segments.
      const c: Array<[number, number]> = [
        [x - hx, z - hz],
        [x + hx, z - hz],
        [x + hx, z + hz],
        [x - hx, z + hz],
      ];
      for (let i = 0; i < 4; i++) {
        const a = c[i];
        const b = c[(i + 1) % 4];
        pts.push(a[0], yy, a[1], b[0], yy, b[1]);
      }
      // A small cross at the mount origin — the survey peg.
      pts.push(x - 0.4, yy, z, x + 0.4, yy, z);
      pts.push(x, yy, z - 0.4, x, yy, z + 0.4);
    }
    const geo = new THREE.BufferGeometry();
    geo.setAttribute('position', new THREE.Float32BufferAttribute(pts, 3));
    return geo;
  }, []);

  return <lineSegments geometry={geometry} material={surveyMat} />;
}
