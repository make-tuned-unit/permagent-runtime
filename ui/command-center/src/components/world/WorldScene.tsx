// WorldScene — thin composition file (W1-owned per WORLD_VIEW_BIBLE.md §5).
// Hall geometry lives in areas/hall/, zones in areas/<zone>/ (lazy-loaded),
// lighting/fog/particles in atmosphere/ (W4), legacy furniture in
// props/legacy/ (W2), agents in agents/ (W3).

import { useEffect, useRef } from 'react';
import { useFrame, useThree } from '@react-three/fiber';
import * as THREE from 'three';
import { HallStructure } from './areas/hall/HallStructure';
import { MezzanineLibrary } from './areas/hall/MezzanineLibrary';
import { WorkstationCluster } from './props/WorkstationCluster';
import { GoalPlaques } from './props/GoalPlaques';
import { ShelfBook } from './props/ShelfBook';
import { BenchArtifacts } from './props/BenchArtifacts';
import { Horologium } from './props/Horologium';
import { PetitionBasin } from './props/PetitionBasin';
import { WatcherBeacon } from './props/WatcherBeacon';
import { ColonnadeLanterns } from './props/ColonnadeLanterns';
import { Zones } from './areas/WorldZones';
import { Agora, PortalStream } from './areas/forum/Agora';
import { Atmosphere, DistantGrid } from './atmosphere/Atmosphere';
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
      {/* Atmosphere (W4): lighting, fog, starfield, dust, light shaft,
          orbital arcs, reactive ambience — see atmosphere/Atmosphere.tsx */}
      <Atmosphere />

      {/* Environment */}
      <DistantGrid />

      {/* Main hall — rotunda + mezzanine library */}
      <HallStructure onHoverStation={onHoverStation} onClickStation={onClickStation} />
      <MezzanineLibrary />

      {/* The working bay (W2 WorkstationCluster, previously unmounted): six
          real seat anchors by the Lab pedestal, with holo plaques bound to
          REAL active goals — the Kanban made physical. The Librarian's shelf
          book animates off the mezzanine wall on real describe events. Bench
          artifacts: LIVE task completions leave cooling work-stones. */}
      <WorkstationCluster
        position={[0, 0, -11.4]}
        rotationY={Math.PI}
        areaId="hall"
        idPrefix="hall.bay"
      />
      <GoalPlaques origin={[0, 0, -11.4]} rotationY={Math.PI} />
      <ShelfBook />
      <BenchArtifacts />

      {/* Environment truth props (this pass) — each bound to one real feed:
            Horologium     — /api/runs schedules (tick per REAL job; the
                             escapement turns only while one really runs)
            PetitionBasin  — /api/decisions (a sealed scroll per REAL pending
                             decision; created drops in, resolved ascends)
            WatcherBeacon  — the Watcher's vigil tower; flares + plaque only
                             on a REAL proactive_nudge event
            ColonnadeLanterns — decorative Light-tier craft on the real clock */}
      <Horologium />
      <PetitionBasin />
      <WatcherBeacon />
      <ColonnadeLanterns />

      {/* The relocated Mesh Stargate in the colonnade opening (clickable → the
          Agora arc, #306). */}
      <Zones onClickStation={onClickStation} />

      {/* THE AGORA — the collective mind of the mesh, beyond the Stargate. A dark
          constellation of sovereign glyph-stars (honest-empty today). The portal
          dissolve/reconstitute stream lives in world space (crosses the boundary
          between the warm Rotunda and the weightless data-volume). */}
      <Agora />
      <PortalStream />

      {/* Legacy station-corner furniture (W2 takes over in props/) */}
      <LegacyFurniture />

      {/* Atmosphere */}

      {/* Shared perf probe (bible §6) + dev-only camera hook for evidence */}
      <PerfSampler />
      {import.meta.env.DEV && <WorldDevHooks />}
    </>
  );
}
