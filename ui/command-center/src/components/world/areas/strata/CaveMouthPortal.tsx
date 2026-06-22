// CaveMouthPortal — the framed entrance to the cave at the south rotunda rim.
// Walkable-cave rebuild §4/§6: the descent must read as a PURPOSEFUL doorway you walk
// to (like a zone portal), not a hole you stumble into. A marble arch flanking the
// MOUTH_WEDGE notch, with a violet lintel glow (the Brain's archive). Always rendered
// (cheap, ≤5 meshes) so the way down reads before the lazy cave interior mounts.

import * as THREE from 'three';
import { ENV } from '../../shared/palette';
import { THROAT, MOUTH_WEDGE } from './strata';

const marbleMat = new THREE.MeshStandardMaterial({
  color: '#d8d2c6',
  roughness: 0.4,
  metalness: 0.08,
});
const accentMat = new THREE.MeshBasicMaterial({
  color: ENV.violet,
  transparent: true,
  opacity: 0.85,
  toneMapped: false,
});

const POST_H = 4;

export function CaveMouthPortal() {
  const [cx, , cz] = THROAT.center;
  // Posts at the inner corners of the mouth notch, flanking the approach from the hall.
  const postR = MOUTH_WEDGE.rInner + 0.3;
  const corner = (b: number): [number, number, number] => [
    cx + Math.cos(b) * postR,
    0,
    cz + Math.sin(b) * postR,
  ];
  const left = corner(MOUTH_WEDGE.b0);
  const right = corner(MOUTH_WEDGE.b1);
  const midX = (left[0] + right[0]) / 2;
  const midZ = (left[2] + right[2]) / 2;
  const span = Math.hypot(right[0] - left[0], right[2] - left[2]);
  const ang = Math.atan2(right[2] - left[2], right[0] - left[0]);

  return (
    <group>
      {[left, right].map((p, i) => (
        <mesh key={i} position={[p[0], POST_H / 2, p[2]]} material={marbleMat} castShadow>
          <cylinderGeometry args={[0.45, 0.55, POST_H, 12]} />
        </mesh>
      ))}
      {/* Lintel across the posts */}
      <mesh position={[midX, POST_H + 0.3, midZ]} rotation={[0, -ang, 0]} material={marbleMat} castShadow>
        <boxGeometry args={[span + 1.2, 0.6, 1.0]} />
      </mesh>
      {/* Violet archive glow under the lintel */}
      <mesh position={[midX, POST_H - 0.05, midZ]} rotation={[0, -ang, 0]}>
        <boxGeometry args={[span, 0.14, 0.22]} />
        <primitive object={accentMat} attach="material" />
      </mesh>
    </group>
  );
}
