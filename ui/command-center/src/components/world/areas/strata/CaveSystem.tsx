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

import { Suspense, lazy, useEffect, useRef, useState } from 'react';
import { useFrame, useThree } from '@react-three/fiber';
import { blockoutMat } from '../blockout';
import { THROAT } from './strata';
import { setDescentActive } from '../../camera/descentState';

const CaveInterior = lazy(() => import('./CaveInterior'));
const MouthChunk = lazy(() => import('./MouthChunk'));

// ── Explicit cave-load trigger (Carved Cave correction, folds in nav-bug #1) ──
// The descent affordances (the ↓ Descend HUD control + the throat-collar click) must
// force the lazy interior to mount, not wait for the proximity heuristic. A tiny
// module flag + listener does it without prop-drilling through WorldScene.
let caveLoadRequested = false;
const caveLoadListeners = new Set<() => void>();
/** Force the lazy cave interior chunk to mount (called by descent entry points). */
export function requestCaveLoad(): void {
  if (caveLoadRequested) return;
  caveLoadRequested = true;
  caveLoadListeners.forEach((l) => l());
}
function isCaveLoadRequested(): boolean {
  return caveLoadRequested;
}

// Throat-collar imposter: a thin gray ring at the crown floor marking the abyss
// lip, always loaded (≤3 meshes) so the descent reads before the chunk loads.
// Carved Cave: clicking the collar enters descent (entry point B) AND forces the
// interior to load — the real trigger replacing the incidental proximity load.
function ThroatCollar() {
  const [cx, , cz] = THROAT.center;
  return (
    <group position={[cx, -1.6, cz]}>
      <mesh
        rotation-x={-Math.PI / 2}
        material={blockoutMat}
        onClick={(e) => {
          e.stopPropagation();
          requestCaveLoad();
          setDescentActive(true);
        }}
        onPointerOver={() => {
          document.body.style.cursor = 'pointer';
        }}
        onPointerOut={() => {
          document.body.style.cursor = 'default';
        }}
      >
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
  // Explicit request (descent affordances) loads immediately — the real trigger.
  useEffect(() => {
    if (isCaveLoadRequested()) {
      loaded.current = true;
      setLoad(true);
      return;
    }
    const onReq = () => {
      loaded.current = true;
      setLoad(true);
    };
    caveLoadListeners.add(onReq);
    return () => {
      caveLoadListeners.delete(onReq);
    };
  }, []);
  // Proximity heuristic stays as a secondary (incidental) load path.
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

// DEV-ONLY A/B toggle for evidence capture (mirrors atmosphere's __worldFog).
// Lets the CDP harness measure the exact cave cost at an identical pose by
// hiding the whole cave. No effect in production builds.
let caveEnabled = true;
declare global {
  interface Window {
    __worldCave?: { setEnabled: (on: boolean) => void; getEnabled: () => boolean };
  }
}
if (import.meta.env.DEV && typeof window !== 'undefined') {
  window.__worldCave = {
    setEnabled: (on: boolean) => {
      caveEnabled = on;
      window.dispatchEvent(new Event('worldcave-toggle'));
    },
    getEnabled: () => caveEnabled,
  };
}

export function CaveSystem() {
  const loadDescent = useDescentTrigger();
  const loadMouth = useMouthTrigger();
  const [, force] = useState(0);
  // Dev A/B toggle listener (no-op in prod — the event never fires there).
  useEffect(() => {
    if (!import.meta.env.DEV) return;
    const h = () => force((n) => n + 1);
    window.addEventListener('worldcave-toggle', h);
    return () => window.removeEventListener('worldcave-toggle', h);
  }, []);

  if (import.meta.env.DEV && !caveEnabled) return null;

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
