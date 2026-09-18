import { useEffect, useMemo, useRef } from 'react';
import { useFrame } from '@react-three/fiber';
import { useGLTF } from '@react-three/drei';
import * as THREE from 'three';
import { getReduceMotion } from '../../../styles/tokens';
import { FORUM_ASSET_REVISION } from './assetRevision';
import { extendForumLoader } from './forumGltf';
import { SHIP_COUNT, shipCourse } from './shipMotion';

/**
 * Ships that actually sail.
 *
 * The four boats already in `solar-forum.glb` are baked into the island mesh
 * (`scripts/blender/forum_landform.py`), so they can never move — they are
 * terrain. This loads the separate `forum-ship.glb`
 * (`scripts/blender/build_forum_ship.py`), clones it a few times and drives
 * each copy along its own course, the same pattern `ForumSky`'s birds use.
 *
 * Cost is deliberately small: one 9 KB model, five clones, no shadows cast (at
 * this range a hull's shadow is a smudge that costs a whole shadow-map pass),
 * and `getReduceMotion()` freezes them in place rather than removing them, so
 * the lagoon still has ships for someone who has asked the OS for less motion.
 */
export function ForumShips() {
  const url = `${import.meta.env.BASE_URL}world/forum-ship.glb?v=${FORUM_ASSET_REVISION}`;
  const { scene } = useGLTF(url, false, false, extendForumLoader);
  const group = useRef<THREE.Group>(null);

  // One clone per ship. `SkeletonUtils` is unnecessary — nothing here is
  // skinned — so a plain clone keeps the shared geometry and materials.
  const ships = useMemo(() => {
    return Array.from({ length: SHIP_COUNT }, (_, index) => {
      const copy = scene.clone(true);
      copy.name = `ForumShip${index}`;
      const scale = 0.85 + ((index * 0.13) % 0.4);
      copy.scale.setScalar(scale);
      copy.traverse((child) => {
        if ((child as THREE.Mesh).isMesh) {
          const mesh = child as THREE.Mesh;
          mesh.castShadow = false;
          mesh.receiveShadow = false;
          // The wake is a flat plane sitting just above the water; without
          // this it z-fights the lagoon surface from a low eye.
          if (mesh.name.startsWith('Wake')) {
            const material = mesh.material as THREE.MeshStandardMaterial;
            material.transparent = true;
            material.depthWrite = false;
            material.polygonOffset = true;
            material.polygonOffsetFactor = -2;
          }
        }
      });
      return copy;
    });
  }, [scene]);

  useEffect(() => {
    const root = group.current;
    if (!root) return;
    for (const ship of ships) root.add(ship);
    return () => {
      for (const ship of ships) root.remove(ship);
    };
  }, [ships]);

  useFrame(({ clock }) => {
    if (getReduceMotion()) return;
    const seconds = clock.getElapsedTime();
    for (let index = 0; index < ships.length; index += 1) {
      const ship = ships[index];
      const pose = shipCourse(seconds, index);
      ship.position.set(pose.x, pose.y, pose.z);
      ship.rotation.set(pose.pitch, pose.yaw, pose.roll, 'YXZ');
      const wake = ship.getObjectByName('Wake') as THREE.Mesh | undefined;
      if (wake) {
        const material = wake.material as THREE.MeshStandardMaterial;
        // A drifting ship leaves nothing; a running one leaves a visible V.
        material.opacity = 0.1 + pose.speed * 0.5;
      }
    }
  });

  return <group ref={group} name="ForumShips" />;
}
