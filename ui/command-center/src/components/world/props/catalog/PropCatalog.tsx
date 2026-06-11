// W2 — dev-only prop catalog. NOT part of the product build: gated on
// import.meta.env.DEV and only reachable via /propcatalog.html, which vite
// serves in dev but never emits in `vite build` (index.html is the only entry).
//
// Lays out every W2 prop family on a stage with the shared PerfSampler and an
// overlay reading the instance ledger — this is the evidence surface for the
// lane PR (instances-per-draw-call per family, fps, calls, triangles).

import { useEffect, useState } from 'react';
import { Canvas, useFrame, useThree } from '@react-three/fiber';
import { Html, OrbitControls } from '@react-three/drei';
import { ENV } from '../../shared/palette';
import { getInstanceLedger } from '../../shared/instancing';
import { PerfSampler, getPerfSnapshot, type PerfSnapshot } from '../../shared/perf';
import { unitPlane } from '../geometries';
import { stoneDark } from '../materials';
import { WorkstationCluster } from '../WorkstationCluster';
import { MezzanineBookWall } from '../MezzanineBookWall';
import { BrainShelves } from '../BrainShelves';
import { LabProps } from '../LabProps';
import { AutomateSteles } from '../AutomateSteles';
import { AmbientProps } from '../AmbientProps';

type Vec3 = [number, number, number];

/**
 * PerfSampler's priority-1000 subscription switches R3F to manual rendering;
 * the product scene's EffectComposer drives the render there, but the catalog
 * has no post chain — so drive it here at priority 1. Also pins
 * gl.info.autoReset=false + manual reset so the sampler reads full-frame
 * calls/triangles (the bible §10 measurement method).
 */
function CatalogRenderDriver() {
  const gl = useThree((s) => s.gl);
  useEffect(() => {
    gl.info.autoReset = false;
    return () => {
      gl.info.autoReset = true;
    };
  }, [gl]);
  useFrame(({ scene, camera }) => {
    gl.info.reset();
    gl.render(scene, camera);
  }, 1);
  return null;
}

/** Evidence support: #cam=x,y,z&t=x,y,z selects a deterministic camera view. */
function hashCam(): { pos: Vec3; target: Vec3 } {
  const params = new URLSearchParams(window.location.hash.slice(1));
  const parse = (s: string | null, fallback: Vec3): Vec3 => {
    if (!s) return fallback;
    const v = s.split(',').map(Number);
    return v.length === 3 && v.every(Number.isFinite) ? (v as Vec3) : fallback;
  };
  return { pos: parse(params.get('cam'), [16, 14, 30]), target: parse(params.get('t'), [0, 1.5, 0]) };
}

function Label({ position, text }: { position: [number, number, number]; text: string }) {
  return (
    <Html position={position} center style={{ pointerEvents: 'none' }}>
      <div
        style={{
          color: ENV.neonCyan,
          fontFamily: 'monospace',
          fontSize: 12,
          whiteSpace: 'nowrap',
          textShadow: '0 0 6px #000',
        }}
      >
        {text}
      </div>
    </Html>
  );
}

function CatalogOverlay() {
  const [snap, setSnap] = useState<PerfSnapshot | null>(null);
  const [ledger, setLedger] = useState<Record<string, number>>({});

  useEffect(() => {
    const t = setInterval(() => {
      setSnap(getPerfSnapshot());
      setLedger(getInstanceLedger());
    }, 1000);
    return () => clearInterval(t);
  }, []);

  // Group instanced draw calls by family prefix (first ledger-name segment).
  const families = new Map<string, { calls: number; instances: number }>();
  for (const [name, count] of Object.entries(ledger)) {
    const family = name.split('.')[0];
    const f = families.get(family) ?? { calls: 0, instances: 0 };
    f.calls += 1;
    f.instances += count;
    families.set(family, f);
  }
  const totalInstances = [...families.values()].reduce((s, f) => s + f.instances, 0);
  const totalInstancedCalls = [...families.values()].reduce((s, f) => s + f.calls, 0);

  return (
    <div
      style={{
        position: 'absolute',
        top: 12,
        left: 12,
        padding: '10px 14px',
        background: 'rgba(10,14,26,0.85)',
        border: `1px solid ${ENV.neonCyan}40`,
        color: ENV.marble,
        fontFamily: 'monospace',
        fontSize: 12,
        lineHeight: 1.5,
        pointerEvents: 'none',
      }}
    >
      <div style={{ color: ENV.neonCyan }}>W2 PROP CATALOG (dev)</div>
      {snap && (
        <div>
          {snap.fps} fps · {snap.calls} calls · {snap.triangles.toLocaleString()} tris ·{' '}
          {snap.geometries} geom · dpr {snap.dpr}
        </div>
      )}
      <div style={{ marginTop: 6, color: ENV.violet }}>
        instance ledger — {totalInstances} instances in {totalInstancedCalls} draw calls
      </div>
      {[...families.entries()]
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([family, f]) => (
          <div key={family}>
            {family}: {f.instances} instances / {f.calls} instanced DC
          </div>
        ))}
    </div>
  );
}

export function PropCatalog() {
  if (!import.meta.env.DEV) return null;
  const { pos, target } = hashCam();

  return (
    <div style={{ width: '100%', height: '100%', position: 'relative', background: ENV.deepVoid }}>
      <Canvas
        shadows
        dpr={[1, 1.5]}
        camera={{ position: pos, fov: 50, near: 0.1, far: 300 }}
      >
        <color attach="background" args={[ENV.deepVoid]} />
        {/* Bible §1 light formula (W4 owns the in-scene rig; dev harness copy). */}
        <ambientLight intensity={0.08} />
        <directionalLight
          position={[18, 26, 12]}
          intensity={1.15}
          color="#FFF0D4"
          castShadow
          shadow-mapSize={[1024, 1024]}
          shadow-camera-left={-40}
          shadow-camera-right={40}
          shadow-camera-top={40}
          shadow-camera-bottom={-40}
        />
        <directionalLight position={[-16, 10, -14]} intensity={0.4} color="#8EC8E8" />

        {/* Stage floor */}
        <mesh
          rotation-x={-Math.PI / 2}
          position-y={-0.01}
          scale={[140, 140, 1]}
          geometry={unitPlane}
          material={stoneDark}
          receiveShadow
        />

        <WorkstationCluster position={[0, 0, -8]} />
        <Label position={[0, 4, -8]} text="WorkstationCluster — 8 DC · 6 seat anchors" />

        <BrainShelves position={[20, 0, -8]} />
        <Label position={[20, 4.5, -8]} text="BrainShelves — 9 DC · 2 seat + 4 stand anchors" />

        <LabProps position={[-20, 0, -8]} />
        <Label position={[-20, 5, -8]} text="LabProps — 8 DC · 4 stand anchors" />

        <AutomateSteles position={[0, 0, 10]} />
        <Label position={[0, 4.5, 10]} text="AutomateSteles — 7 DC · 4 lean + 2 stand anchors" />

        {/* Hall families keep their hall-space layout; offset for the stage. */}
        <group position={[0, 0, 44]}>
          <AmbientProps />
          <Label position={[0, 3.5, 0]} text="AmbientProps — 4 DC · 6 seat anchors" />
        </group>
        <group position={[0, -10.01, 44]}>
          <MezzanineBookWall />
          <Label position={[0, 15.5, 0]} text="MezzanineBookWall — 12 DC (replaces ~443 meshes)" />
        </group>

        <OrbitControls target={target} enableDamping />
        <CatalogRenderDriver />
        <PerfSampler />
      </Canvas>
      <CatalogOverlay />
    </div>
  );
}
