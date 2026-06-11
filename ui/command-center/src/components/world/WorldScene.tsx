// WorldScene — thin composition file (W1-owned per WORLD_VIEW_BIBLE.md §5).
// Hall geometry lives in areas/hall/, zones in areas/<zone>/ (lazy-loaded),
// lighting/fog/particles in atmosphere/ (W4), legacy furniture in
// props/legacy/ (W2), agents in agents/ (W3).

import { useEffect, useRef } from 'react';
import { useFrame, useThree } from '@react-three/fiber';
import * as THREE from 'three';
import { HallStructure } from './areas/hall/HallStructure';
import { MezzanineLibrary } from './areas/hall/MezzanineLibrary';
import { Zones } from './areas/WorldZones';
import { HallLighting, Starfield, DistantGrid, DustMotes, WorldFog } from './atmosphere/Atmosphere';
import { LegacyFurniture } from './props/legacy/WorldFurniture';
import { PerfSampler } from './shared/perf';

declare global {
  interface Window {
    __worldDebug?: {
      setCam: (pos: [number, number, number], look: [number, number, number]) => void;
      clearCam: () => void;
    };
  }
}

// Dev-only evidence hook: lets the CDP harness park the camera at exact poses
// for fly-throughs and budget screenshots. gl.info management (autoReset +
// per-frame reset) is owned by the FROZEN-amended shared/perf.ts PerfSampler —
// this hook must NOT touch gl.info or it would zero counters before the
// sampler reads them. No effect in production builds.
function WorldDevHooks() {
  const camera = useThree((s) => s.camera);
  const override = useRef<{ active: boolean; pos: THREE.Vector3; look: THREE.Vector3 }>({
    active: false,
    pos: new THREE.Vector3(),
    look: new THREE.Vector3(),
  });

  useEffect(() => {
    const o = override.current;
    window.__worldDebug = {
      setCam: (pos, look) => {
        o.pos.set(pos[0], pos[1], pos[2]);
        o.look.set(look[0], look[1], look[2]);
        o.active = true;
      },
      clearCam: () => {
        o.active = false;
      },
    };
    return () => {
      delete window.__worldDebug;
    };
  }, []);

  // Runs after OrbitControls (priority -1) so the override wins.
  // No per-frame allocations.
  useFrame(() => {
    const o = override.current;
    if (!o.active) return;
    camera.position.copy(o.pos);
    camera.lookAt(o.look);
  });

  return null;
}

// Main scene composition
export function WorldSceneContent({
  onHoverStation,
  onClickStation,
}: {
  onHoverStation: (id: string | null) => void;
  onClickStation: (id: string) => void;
}) {
  return (
    <>
      {/* Lighting */}
      <HallLighting />

      {/* Environment */}
      <Starfield />
      <DistantGrid />

      {/* Main hall — rotunda + mezzanine library */}
      <HallStructure onHoverStation={onHoverStation} onClickStation={onClickStation} />
      <MezzanineLibrary />

      {/* Five zones off the rotunda — thresholds always on, interiors lazy (§3) */}
      <Zones />

      {/* Legacy station-corner furniture (W2 takes over in props/) */}
      <LegacyFurniture />

      {/* Atmosphere */}
      <DustMotes />
      <WorldFog />

      {/* Shared perf probe (bible §6) + dev-only camera hook for evidence */}
      <PerfSampler />
      {import.meta.env.DEV && <WorldDevHooks />}
    </>
  );
}
