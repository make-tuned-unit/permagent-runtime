// CaveSystem — the vertical cave beneath the crown (THE_CAVE_vision_bible.md §9 W1).
// Composition + lazy depth-chunk mounting. The cave hangs below the rotunda floor
// (y=0) and descends to the bedrock near the void grid (y=-29); the Mouth aperture
// sits far above the dome (y≈64). The crown geometry itself is HallStructure — this
// module only adds what's beneath it and the distant aperture above.
//
// Lazy depth chunks (bible §8: "the chunk architecture maps onto depth"):
//  - The whole cave interior is one React.lazy chunk that mounts only when the
//    camera descends toward / approaches the throat lip. Until then the crown shows
//    just the always-cheap throat-collar imposter (≤3 meshes) so the abyss reads
//    from the Brain bridge without paying for the full descent.
//  - The Mouth aperture is a second, independent lazy chunk that mounts when the
//    camera looks/climbs up the oculus sightline.
// Once a chunk mounts it stays (matches ZoneMount semantics).
//
// LAW: zero per-frame allocations — scalar distance math only.

import { Suspense, lazy, useRef, useState } from 'react';
import { useFrame, useThree } from '@react-three/fiber';
import { blockoutMat } from '../blockout';
import { THROAT } from './strata';

const CaveInterior = lazy(() => import('./CaveInterior'));
const MouthChunk = lazy(() => import('./MouthChunk'));

// Throat-collar imposter: a thin gray ring at the crown floor marking the abyss
// lip, always loaded (≤3 meshes) so the descent reads before the chunk loads.
function ThroatCollar() {
  const [cx, , cz] = THROAT.center;
  return (
    <group position={[cx, -1.6, cz]}>
      <mesh rotation-x={-Math.PI / 2} material={blockoutMat}>
        <ringGeometry args={[THROAT.radiusTop, THROAT.radiusTop + 1.2, 24]} />
      </mesh>
    </group>
  );
}

// Trigger the descent chunk when the camera dips toward the cave or nears the
// throat lip horizontally (so peering over the Brain bridge loads it too).
const DESCENT_LOAD_Y = 4; // camera below this y starts loading
const THROAT_LOAD_DIST2 = 16 * 16; // or within 16m of the throat lip

function useDescentTrigger(): boolean {
  const camera = useThree((s) => s.camera);
  const [load, setLoad] = useState(false);
  const loaded = useRef(false);
  useFrame(() => {
    if (loaded.current) return;
    const [cx, , cz] = THROAT.center;
    const dx = camera.position.x - cx;
    const dz = camera.position.z - cz;
    if (camera.position.y < DESCENT_LOAD_Y || dx * dx + dz * dz < THROAT_LOAD_DIST2) {
      loaded.current = true;
      setLoad(true);
    }
  });
  return load;
}

// Trigger the Mouth chunk when the camera climbs / the sightline opens upward.
const MOUTH_LOAD_Y = 12; // looking up out of the crown

function useMouthTrigger(): boolean {
  const camera = useThree((s) => s.camera);
  const [load, setLoad] = useState(false);
  const loaded = useRef(false);
  useFrame(() => {
    if (loaded.current) return;
    // Near the world axis (under the oculus) and looking/rising up the shaft.
    const horiz2 = camera.position.x * camera.position.x + camera.position.z * camera.position.z;
    if (camera.position.y > MOUTH_LOAD_Y || (horiz2 < 18 * 18 && camera.position.y > 2)) {
      loaded.current = true;
      setLoad(true);
    }
  });
  return load;
}

export function CaveSystem() {
  const loadDescent = useDescentTrigger();
  const loadMouth = useMouthTrigger();

  return (
    <group>
      <ThroatCollar />
      {loadDescent && (
        <Suspense fallback={null}>
          <CaveInterior />
        </Suspense>
      )}
      {loadMouth && (
        <Suspense fallback={null}>
          <MouthChunk />
        </Suspense>
      )}
    </group>
  );
}
