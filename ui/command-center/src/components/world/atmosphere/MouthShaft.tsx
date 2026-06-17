// W4 atmosphere — THE MOUTH'S DAYLIGHT SHAFT (THE CAVE bible §2, §3).
//
// "The Mouth = the Mesh portal. Plato's cave has an exit, and it is visible from
//  day one: a distant blade of pale daylight, far above the highest chamber. It
//  is the light source for the entire world." (§2)
//
// This is THE light of the world: cold, pale, sacred, opt-in. It hangs far above
// the crown on the Antechamber sightline (the final carved climb to the Mouth,
// §2). Its grade through the strata is strongest at the Antechamber approach and
// fades with depth — the deepest chambers go nearly dark except agent work-lights
// (W3). We express that grade with emissive/fog/gradient craft, NOT a new shadow
// caster (the 1-caster law stands — see atmosphere/Atmosphere.tsx Lighting) and
// NOT a point-light spree (the integration census must REDUCE, not add; this file
// adds exactly ONE distance-capped cold downlight as the Mouth's key, and removes
// none — net census delta is reported in the PR).
//
// W1 DEPENDENCY (noted): W1 blocks out the vertical strata + the Mouth aperture
// in parallel. Until that lands there is no literal aperture mesh to align to, so
// this is STAGED against the bible geography: the blade hangs on the Antechamber
// diagonal (NW, §3 coordinates: x=-19, z=-19) high above the dome. When W1's
// aperture lands, MOUTH_POS plugs into its world-space opening — one constant.
//
// LAW: bible §8 — zero per-frame allocations; reduceMotion → constant shaft.

import { useMemo, useRef } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import { DOME_HEIGHT } from '../constants';
import { getReduceMotion } from '../../../styles/tokens';
import { getAmbienceLevel } from './ambience';

// Cold pale daylight — NOT a palette accent. Like the §1 light temperatures in
// Atmosphere.tsx, the Mouth's daylight is a light *temperature*, so it lives here
// rather than in the frozen shared/palette.ts. Pale, slightly blue: the world of
// forms seen from inside the cave.
const DAYLIGHT = {
  blade: '#DCE8F2', // the visible blade of daylight (cold off-white)
  glow: '#C4D8EC', // the cold halo around the aperture
  key: '#D6E4F0', // the cold downlight color
} as const;

// Staged Mouth position: high above the Antechamber sightline (bible §2/§3 — the
// final climb ends at cold daylight on the NW diagonal). y is far above the dome
// crown (DOME_HEIGHT=18) so it reads as "impossibly distant" (§7 hero board).
// When W1's aperture lands, this becomes the aperture's world-space center.
const MOUTH_POS = new THREE.Vector3(-19, DOME_HEIGHT + 34, -19);

// The blade points down the throat toward the hall center.
const HALL_CENTER = new THREE.Vector3(0, 4, 0);

/**
 * The visible blade: a thin, tall, soft quad of pale daylight far above, plus a
 * cold halo disc at the aperture. Additive, depth-write off — it reads as light,
 * not geometry. Faint breathing scaled by the ambience level (busy world =
 * marginally brighter daylight as the climb advances), constant under reduceMotion.
 */
function DaylightBlade({ reduceMotion }: { reduceMotion: boolean }) {
  const bladeRef = useRef<THREE.Mesh>(null);

  // Orient the blade so it faces roughly toward the hall (down the throat).
  const bladeQuat = useMemo(() => {
    const m = new THREE.Matrix4();
    const up = new THREE.Vector3(0, 0, 1);
    m.lookAt(MOUTH_POS, HALL_CENTER, up);
    const q = new THREE.Quaternion().setFromRotationMatrix(m);
    return q;
  }, []);

  useFrame(() => {
    if (reduceMotion) return;
    const mesh = bladeRef.current;
    if (!mesh) return;
    const mat = mesh.material as THREE.MeshBasicMaterial;
    // Sacred, near-constant: a very faint breath only. The light is *given*, not
    // performed — the bible's "indifferent, beautiful" register.
    mat.opacity = 0.55 + 0.05 * Math.sin(performance.now() * 0.0003) * getAmbienceLevel();
  });

  return (
    <group position={MOUTH_POS.toArray()}>
      {/* The blade itself — a tall narrow plane of cold daylight. */}
      <mesh ref={bladeRef} quaternion={bladeQuat}>
        <planeGeometry args={[3.2, 14]} />
        <meshBasicMaterial
          color={DAYLIGHT.blade}
          transparent
          opacity={0.55}
          depthWrite={false}
          side={THREE.DoubleSide}
          blending={THREE.AdditiveBlending}
          toneMapped={false}
        />
      </mesh>
      {/* Cold halo around the aperture — soft bloom seed (sacred, focal). */}
      <mesh quaternion={bladeQuat}>
        <circleGeometry args={[5, 32]} />
        <meshBasicMaterial
          color={DAYLIGHT.glow}
          transparent
          opacity={0.18}
          depthWrite={false}
          blending={THREE.AdditiveBlending}
          toneMapped={false}
        />
      </mesh>
    </group>
  );
}

/**
 * A long fake-volumetric throat of daylight descending from the Mouth toward the
 * crown — the shaft the §7 hero shot breaches into. Vertex-coloured so it is
 * BRIGHT near the aperture and fades to nothing before it reaches the hall: the
 * grade-through-the-strata made literal (strongest high/near the Mouth, dark in
 * the depths). Constant under reduceMotion (positional fade, no animation).
 */
function DaylightThroat() {
  const geo = useMemo(() => {
    const length = 40;
    const g = new THREE.CylinderGeometry(2.2, 6.5, length, 24, 8, true);
    // Vertex colours: bright (toward +y / the Mouth) → black (toward -y / depths).
    const pos = g.attributes.position;
    const colors = new Float32Array(pos.count * 3);
    const c = new THREE.Color(DAYLIGHT.glow);
    for (let i = 0; i < pos.count; i++) {
      const y = pos.getY(i); // [-length/2, +length/2]
      const t = (y + length / 2) / length; // 0 at bottom, 1 at top
      // Ease: daylight concentrates near the aperture, depths go dark.
      const b = Math.pow(t, 1.8);
      colors[i * 3] = c.r * b;
      colors[i * 3 + 1] = c.g * b;
      colors[i * 3 + 2] = c.b * b;
    }
    g.setAttribute('color', new THREE.BufferAttribute(colors, 3));
    return g;
  }, []);

  // Aim the throat down the line from the Mouth toward the hall center.
  const { position, quaternion } = useMemo(() => {
    const mid = MOUTH_POS.clone().lerp(HALL_CENTER, 0.5);
    const dir = HALL_CENTER.clone().sub(MOUTH_POS).normalize();
    // Cylinder's axis is +y; rotate +y to point along `dir`.
    const q = new THREE.Quaternion().setFromUnitVectors(new THREE.Vector3(0, 1, 0), dir);
    return { position: mid, quaternion: q };
  }, []);

  return (
    <mesh geometry={geo} position={position.toArray()} quaternion={quaternion}>
      <meshBasicMaterial
        vertexColors
        transparent
        opacity={0.12}
        depthWrite={false}
        side={THREE.DoubleSide}
        blending={THREE.AdditiveBlending}
        toneMapped={false}
      />
    </mesh>
  );
}

/**
 * The Mouth's cold key light: a single distance-capped directional-feeling
 * pointLight high on the Antechamber sightline, washing the crown in pale
 * daylight that falls off toward the depths. This is the world's light source.
 * Census: +1 point light (the only light this lane adds); decay 2, distance
 * capped per §1. NOT a shadow caster (the §1 single caster stays the warm key in
 * Lighting). Net census delta reported in the PR.
 */
function MouthKeyLight() {
  return (
    <pointLight
      position={MOUTH_POS.toArray()}
      color={DAYLIGHT.key}
      intensity={1.1}
      distance={DOME_HEIGHT + 60}
      decay={2}
    />
  );
}

/**
 * Full Mouth daylight system: the visible blade + halo, the graded throat, and
 * the single cold key light. Mounted once from Atmosphere.tsx.
 */
export function MouthShaft() {
  const reduceMotion = useMemo(() => getReduceMotion(), []);
  return (
    <>
      <MouthKeyLight />
      <DaylightThroat />
      <DaylightBlade reduceMotion={reduceMotion} />
    </>
  );
}
