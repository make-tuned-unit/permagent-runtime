// ZoneMount — lazy zone mounting per WORLD_VIEW_BIBLE.md §3/§8.
// A zone's interior chunk loads (React.lazy) when the camera approaches its
// room center; until the chunk has actually mounted, only the ≤3-mesh imposter
// silhouette renders. Once loaded, a zone stays mounted.

import { Suspense, useState, useRef, useCallback, type ReactNode, type LazyExoticComponent, type ComponentType } from 'react';
import { useFrame, useThree } from '@react-three/fiber';
import { ZONE_LOAD_DISTANCE, zoneCenter, type ZoneDef } from './zones';

export interface ZoneContentProps {
  /** The zone calls this once its real geometry has mounted. */
  onReady: () => void;
}

interface ZoneMountProps {
  zone: ZoneDef;
  /** Lazy interior chunk. */
  component: LazyExoticComponent<ComponentType<ZoneContentProps>>;
  /** ≤3-mesh distant silhouette of the zone's landmark (always cheap). */
  imposter: ReactNode;
}

export function ZoneMount({ zone, component: Zone, imposter }: ZoneMountProps) {
  const camera = useThree((s) => s.camera);
  const [load, setLoad] = useState(false);
  const [ready, setReady] = useState(false);
  const center = useRef(zoneCenter(zone));

  // No allocations: plain scalar distance math against the camera position.
  useFrame(() => {
    if (load) return;
    const [cx, cy, cz] = center.current;
    const dx = camera.position.x - cx;
    const dy = camera.position.y - cy;
    const dz = camera.position.z - cz;
    if (dx * dx + dy * dy + dz * dz < ZONE_LOAD_DISTANCE * ZONE_LOAD_DISTANCE) {
      setLoad(true);
    }
  });

  const handleReady = useCallback(() => setReady(true), []);

  return (
    <group rotation-y={-zone.angle}>
      {!ready && imposter}
      {load && (
        <Suspense fallback={null}>
          <Zone onReady={handleReady} />
        </Suspense>
      )}
    </group>
  );
}
