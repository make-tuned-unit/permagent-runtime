// ShelfBook — the Librarian physically takes a book off the mezzanine wall.
//
// Timing and truth come from agents/librarianMining.ts (resolveTablet): the
// SAME real librarian_describe_* events that drive the tablet also slide one
// emissive book out of the BookshelfWall beside the Librarian, hold it aglow
// while the describe runs, and reshelve it on completion. One mesh, no lights,
// zero per-frame allocations (bible §8).

import { useRef } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import { resolveTablet } from '../agents/librarianMining';
import { getAgentPosition } from '../agents/motion';
import { ENV } from '../shared/palette';

// Mezzanine shelf wall geometry (mirrors MezzanineLibrary's BookshelfWall).
const WALL_RADIUS = 15.45;
const BOOK_Y = 10.95;
const PULL_DISTANCE = 0.5;

const bookMaterial = new THREE.MeshLambertMaterial({
  color: '#2A2438',
  emissive: new THREE.Color(ENV.violet),
  emissiveIntensity: 0.0,
});

export function ShelfBook() {
  const ref = useRef<THREE.Mesh>(null);

  useFrame(() => {
    const mesh = ref.current;
    if (!mesh) return;
    const tablet = resolveTablet(performance.now());
    if (!tablet.visible) {
      mesh.visible = false;
      return;
    }
    const pos = getAgentPosition('librarian');
    if (!pos) {
      mesh.visible = false;
      return;
    }
    // Nearest shelf slot to the Librarian's current ring angle.
    const angle = Math.atan2(pos.z, pos.x);
    const r = WALL_RADIUS - tablet.reach * PULL_DISTANCE;
    mesh.visible = true;
    mesh.position.set(Math.cos(angle) * r, BOOK_Y, Math.sin(angle) * r);
    mesh.rotation.y = Math.PI / 2 - angle + tablet.reach * 0.35;
    (mesh.material as THREE.MeshLambertMaterial).emissiveIntensity =
      0.15 + tablet.glow * 1.1;
  });

  return (
    <mesh ref={ref} material={bookMaterial} visible={false}>
      <boxGeometry args={[0.1, 0.34, 0.24]} />
    </mesh>
  );
}
