// Instancing helper — WORLD_VIEW_BIBLE.md §6, §8 (InstancedMesh MANDATORY for any
// prop placed more than twice). FROZEN after Phase 0.

import { useLayoutEffect, useRef } from 'react';
import * as THREE from 'three';

export interface InstanceTransform {
  position: [number, number, number];
  /** Euler radians. */
  rotation?: [number, number, number];
  scale?: number | [number, number, number];
}

const tmpObj = new THREE.Object3D();

// Evidence support: instances-per-draw-call bookkeeping for lane PR screenshots.
const instanceLedger = new Map<string, number>();
export function getInstanceLedger(): Record<string, number> {
  return Object.fromEntries(instanceLedger);
}

interface InstancedPropProps {
  /** Ledger name for evidence, e.g. 'brain.book'. One InstancedProp = one draw call. */
  name: string;
  geometry: THREE.BufferGeometry;
  material: THREE.Material;
  transforms: InstanceTransform[];
  castShadow?: boolean;
  receiveShadow?: boolean;
}

export function InstancedProp({
  name,
  geometry,
  material,
  transforms,
  castShadow,
  receiveShadow,
}: InstancedPropProps) {
  const ref = useRef<THREE.InstancedMesh>(null);

  useLayoutEffect(() => {
    const mesh = ref.current;
    if (!mesh) return;
    transforms.forEach((t, i) => {
      tmpObj.position.set(...t.position);
      tmpObj.rotation.set(...(t.rotation ?? [0, 0, 0]));
      const s = t.scale ?? 1;
      if (typeof s === 'number') tmpObj.scale.setScalar(s);
      else tmpObj.scale.set(...s);
      tmpObj.updateMatrix();
      mesh.setMatrixAt(i, tmpObj.matrix);
    });
    mesh.count = transforms.length;
    mesh.instanceMatrix.needsUpdate = true;
    instanceLedger.set(name, transforms.length);
    return () => {
      instanceLedger.delete(name);
    };
  }, [name, transforms]);

  return (
    <instancedMesh
      ref={ref}
      args={[geometry, material, transforms.length]}
      castShadow={castShadow}
      receiveShadow={receiveShadow}
    />
  );
}
