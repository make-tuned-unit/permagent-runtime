// Scaffold mount blockout — gray stand-in geometry for each construction mount.
// THE_CAVE_vision_bible.md §4 ("Scaffolding is promise"). The real kit (lattice,
// crane, banked-stone piles, wings) is W2's detail pass off ./anchors; W1 ships a
// legible gray placeholder per scaffold state so the blockout review reads the
// construction story (built wings high, bare survey deep).
//
// Each repeated element (lattice uprights, banked stones) is one InstancedMesh
// across ALL mounts (bible §8: instancing MANDATORY > 2×). Real scale.

import { useMemo } from 'react';
import * as THREE from 'three';
import { blockoutMat, unitBox } from '../blockout';
import { InstancedProp, type InstanceTransform } from '../../shared/instancing';
import { SCAFFOLD_MOUNTS } from './anchors';

// One shared upright geometry for scaffold/crane lattice (1×1 column, scaled).
const POST = unitBox;
// One shared rough stone for banked piles.
const stone = new THREE.DodecahedronGeometry(0.5, 0);

/** Scaffold lattice uprights + crane masts — all mounts' posts in one InstancedMesh. */
function LatticePosts() {
  const transforms = useMemo<InstanceTransform[]>(() => {
    const t: InstanceTransform[] = [];
    for (const m of SCAFFOLD_MOUNTS) {
      if (m.kind !== 'scaffold' && m.kind !== 'crane') continue;
      const [x, y, z] = m.position;
      const [fx, fz] = m.footprint;
      const hx = fx / 2;
      const hz = fz / 2;
      const tall = m.kind === 'crane' ? 6 : 2 + m.progress * 4;
      // Four corner uprights.
      const corners: Array<[number, number]> = [
        [-hx, -hz],
        [hx, -hz],
        [hx, hz],
        [-hx, hz],
      ];
      for (const [dx, dz] of corners) {
        t.push({ position: [x + dx, y + tall / 2, z + dz], scale: [0.18, tall, 0.18] });
      }
      // Two cross-braces at mid + top.
      t.push({ position: [x, y + tall * 0.5, z - hz], scale: [fx, 0.15, 0.15] });
      t.push({ position: [x, y + tall, z], scale: [fx, 0.15, 0.15] });
      // Crane gets a jib reaching over the cavern centre.
      if (m.kind === 'crane') {
        t.push({ position: [x * 0.6, y + tall, (z + 9) / 2], scale: [4.5, 0.2, 0.2] });
      }
    }
    return t;
  }, []);

  return (
    <InstancedProp
      name="strata.scaffold.posts"
      geometry={POST}
      material={blockoutMat}
      transforms={transforms}
      castShadow
    />
  );
}

/** Banked quarried-stone piles — all banked mounts' stones in one InstancedMesh. */
function BankedStones() {
  const transforms = useMemo<InstanceTransform[]>(() => {
    const t: InstanceTransform[] = [];
    for (const m of SCAFFOLD_MOUNTS) {
      if (m.kind !== 'banked') continue;
      const [x, y, z] = m.position;
      const [fx, fz] = m.footprint;
      let seed = x * 7.1 + z * 3.3 + 11;
      const rnd = () => {
        seed = (seed * 9301 + 49297) % 233280;
        return seed / 233280;
      };
      const count = Math.round(5 + m.progress * 8);
      for (let i = 0; i < count; i++) {
        const px = x + (rnd() - 0.5) * fx * 0.8;
        const pz = z + (rnd() - 0.5) * fz * 0.8;
        const s = 0.5 + rnd() * 0.5;
        // Stack them a little: lower pile, fewer on top.
        const py = y + s * 0.4 + (i < count / 2 ? 0 : 0.5);
        t.push({
          position: [px, py, pz],
          rotation: [rnd() * Math.PI, rnd() * Math.PI, rnd() * Math.PI],
          scale: s,
        });
      }
    }
    return t;
  }, []);

  return (
    <InstancedProp
      name="strata.banked.stones"
      geometry={stone}
      material={blockoutMat}
      transforms={transforms}
      castShadow
      receiveShadow
    />
  );
}

/** Finished/built wing masses (crown-adjacent) — solid gray blocks, instanced. */
function BuiltWings() {
  const transforms = useMemo<InstanceTransform[]>(() => {
    const t: InstanceTransform[] = [];
    for (const m of SCAFFOLD_MOUNTS) {
      if (m.kind !== 'built') continue;
      const [x, y, z] = m.position;
      const [fx, fz] = m.footprint;
      // A low wing wall + a back mass — reads as finished stonework.
      t.push({ position: [x, y + 1.4, z], scale: [fx, 2.8, fz] });
      t.push({ position: [x, y + 0.3, z], scale: [fx + 0.6, 0.6, fz + 0.6] }); // plinth
    }
    return t;
  }, []);

  return (
    <InstancedProp
      name="strata.built.wings"
      geometry={unitBox}
      material={blockoutMat}
      transforms={transforms}
      castShadow
      receiveShadow
    />
  );
}

export function ScaffoldMounts() {
  return (
    <group>
      <LatticePosts />
      <BankedStones />
      <BuiltWings />
    </group>
  );
}
