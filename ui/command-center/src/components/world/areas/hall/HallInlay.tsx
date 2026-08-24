// HallInlay — tasteful engraved floor detail + a REACTIVE heart for the rotunda.
//
// Two additions, both budget-safe (WORLD_VIEW_BIBLE.md §8):
//   1. Engraved circuit-node inlay: two rings of small Light-tier nodes set into
//      the marble floor between the existing mandala rings — ONE instanced draw
//      call (§8 instancing law), calm cyan, low emissive.
//   2. The omphalos: a central emissive seal that BREATHES WITH REAL EVENTS. It
//      reads ONLY atmosphere/worldSignals (the honesty boundary — real Brain
//      recall/ingest activity, never faked): flow brightens the seal, and each
//      real memory_added (rainTick) fires a decaying ripple. Dormant/steady when
//      the daemon is quiet. This is the room "breathing with real events" (§4).
//
// LAW: zero per-frame allocation — getWorldSignals() is a plain snapshot getter;
// materials are mutated in place. reduceMotion → steady (no breath, no ripple).

import { useMemo, useRef } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import { ENV } from '../../shared/palette';
import { getReduceMotion } from '../../../../styles/tokens';
import { InstancedProp, type InstanceTransform } from '../../shared/instancing';
import { unitCylinder16 } from '../../props/geometries';
import { getWorldSignals } from '../../atmosphere/worldSignals';

const nodeMat = new THREE.MeshLambertMaterial({
  color: ENV.deepVoid,
  emissive: ENV.neonCyan,
  emissiveIntensity: 0.7,
});

// Two rings of engraved nodes set flush into the floor.
function InlayNodes() {
  const transforms = useMemo<InstanceTransform[]>(() => {
    const out: InstanceTransform[] = [];
    const rings = [
      { r: 7.5, n: 24 },
      { r: 10.5, n: 32 },
    ];
    for (const { r, n } of rings) {
      for (let i = 0; i < n; i++) {
        const a = (i / n) * Math.PI * 2;
        out.push({
          position: [Math.cos(a) * r, 0.14, Math.sin(a) * r],
          scale: [0.12, 0.03, 0.12],
        });
      }
    }
    return out;
  }, []);
  return <InstancedProp name="hall.inlayNodes" geometry={unitCylinder16} material={nodeMat} transforms={transforms} />;
}

// The omphalos — the reactive seal at the rotunda center.
const omphalosMat = new THREE.MeshLambertMaterial({
  color: ENV.deepVoid,
  emissive: ENV.neonCyan,
  emissiveIntensity: 0.6,
});

function Omphalos({ reduceMotion }: { reduceMotion: boolean }) {
  const ringRef = useRef<THREE.Mesh>(null);
  const rippleRef = useRef<THREE.Mesh>(null);
  const lastRain = useRef(-1);
  const rippleT = useRef(1); // 1 = spent

  useFrame((_, dt) => {
    const s = getWorldSignals();
    // Fire a ripple on each REAL memory_added (rainTick increments).
    if (s.rainTick !== lastRain.current) {
      if (lastRain.current >= 0 && !reduceMotion) rippleT.current = 0;
      lastRain.current = s.rainTick;
    }

    if (ringRef.current) {
      const mat = ringRef.current.material as THREE.MeshLambertMaterial;
      // Steady base + real recall/ingest flow. reduceMotion: base only.
      const breath = reduceMotion ? 0 : 0.25 * Math.sin(performance.now() * 0.0009);
      mat.emissiveIntensity = 0.55 + s.flow * 1.1 + breath;
    }

    if (rippleRef.current) {
      if (rippleT.current < 1) {
        rippleT.current = Math.min(1, rippleT.current + dt * 0.8);
        const k = rippleT.current;
        rippleRef.current.visible = true;
        rippleRef.current.scale.setScalar(0.5 + k * 8);
        (rippleRef.current.material as THREE.MeshBasicMaterial).opacity = 0.4 * (1 - k);
      } else if (rippleRef.current.visible) {
        rippleRef.current.visible = false;
      }
    }
  });

  return (
    // Raised onto the task dais surface (TaskDais platform top ≈ 0.34) — the
    // seal is the dais's engraving now, not a floor inlay under it.
    <group position-y={0.37}>
      {/* The seal ring. */}
      <mesh ref={ringRef} rotation-x={-Math.PI / 2} material={omphalosMat}>
        <ringGeometry args={[1.1, 1.5, 48]} />
      </mesh>
      {/* Inner engraved disc. */}
      <mesh rotation-x={-Math.PI / 2} position-y={-0.01} material={omphalosMat}>
        <ringGeometry args={[0.3, 0.5, 32]} />
      </mesh>
      {/* Real-event ripple (hidden until a memory_added fires). */}
      <mesh ref={rippleRef} rotation-x={-Math.PI / 2} visible={false}>
        <ringGeometry args={[1.4, 1.55, 48]} />
        <meshBasicMaterial color={ENV.neonCyan} transparent opacity={0} depthWrite={false} blending={THREE.AdditiveBlending} toneMapped={false} />
      </mesh>
    </group>
  );
}

export function HallInlay() {
  const reduceMotion = useMemo(() => getReduceMotion(), []);
  return (
    <group>
      <InlayNodes />
      <Omphalos reduceMotion={reduceMotion} />
    </group>
  );
}
