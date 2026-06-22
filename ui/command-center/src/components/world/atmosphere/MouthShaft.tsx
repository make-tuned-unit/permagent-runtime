// Atmosphere — THE MESH PORTAL'S DAYLIGHT SHAFT.
//
// The Mesh portal (the Stargate above the rotunda) is the one cold daylight
// source in the world: a distant blade of pale light, far above the colonnade.
// Cold, pale, sacred, opt-in — the light of other minds. (Named "Mouth*" from
// the original cave allegory, which #418 removed; the component is kept because
// it is the Mesh Stargate's daylight, not cave debris — the identifiers are
// retained only to avoid a no-op rename.)
//
// It hangs far above the crown on the Antechamber sightline. We express its
// glow with emissive/fog/gradient craft, NOT a new shadow caster (the 1-caster
// law stands — see atmosphere/Atmosphere.tsx Lighting) and NOT a point-light
// spree (the census must REDUCE, not add; this file adds exactly ONE
// distance-capped cold downlight as the portal's key light, and removes none).
//
// MOUTH_POS is the portal's staged world-space anchor on the NW Antechamber
// diagonal (x=-19, z=-19), high above the dome.
//
// LAW: zero per-frame allocations; reduceMotion → constant shaft.

import { useMemo, useRef } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import { DOME_HEIGHT } from '../constants';
import { getReduceMotion } from '../../../styles/tokens';
import { getAmbienceLevel } from './ambience';

// Cold pale daylight — NOT a palette accent. Like the light temperatures in
// Atmosphere.tsx, the portal's daylight is a light *temperature*, so it lives
// here rather than in the frozen shared/palette.ts. Pale, slightly blue: the
// world of other minds beyond the Mesh.
const DAYLIGHT = {
  blade: '#DCE8F2', // the visible blade of daylight (cold off-white)
  glow: '#C4D8EC', // the cold halo around the aperture
  key: '#D6E4F0', // the cold downlight color
} as const;

// Staged portal position: high above the Antechamber sightline on the NW
// diagonal. y is far above the dome crown (DOME_HEIGHT=18) so it reads as
// "impossibly distant".
const MOUTH_POS = new THREE.Vector3(-19, DOME_HEIGHT + 34, -19);

// The blade points down toward the hall center.
const HALL_CENTER = new THREE.Vector3(0, 4, 0);

/**
 * The visible blade: a thin, tall, soft quad of pale daylight far above, plus a
 * cold halo disc at the aperture. Additive, depth-write off — it reads as light,
 * not geometry. Faint breathing scaled by the ambience level (busy world =
 * marginally brighter daylight as the climb advances), constant under reduceMotion.
 */
function DaylightBlade({ reduceMotion }: { reduceMotion: boolean }) {
  const bladeRef = useRef<THREE.Mesh>(null);

  // Orient the blade so it faces roughly toward the hall (down the shaft).
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
    // performed — an indifferent, beautiful register.
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
 * A long fake-volumetric shaft of daylight descending from the portal toward the
 * crown. Vertex-coloured so it is BRIGHT near the aperture and fades to nothing
 * before it reaches the hall: the grade made literal (strongest high/near the
 * portal, dark below). Constant under reduceMotion (positional fade, no animation).
 */
function DaylightThroat() {
  const geo = useMemo(() => {
    const length = 40;
    const g = new THREE.CylinderGeometry(2.2, 6.5, length, 24, 8, true);
    // Vertex colours: bright (toward +y / the portal) → black (toward -y / below).
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

  // Aim the shaft down the line from the portal toward the hall center.
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
 * The portal's cold key light: a single distance-capped directional-feeling
 * pointLight high on the Antechamber sightline, washing the crown in pale
 * daylight that falls off below. This is the world's light source.
 * Census: +1 point light; decay 2, distance capped. NOT a shadow caster (the
 * single warm caster stays the key in Lighting).
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
 * Full portal daylight system: the visible blade + halo, the graded shaft, and
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
