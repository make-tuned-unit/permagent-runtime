// The Mesh Agora — the antechamber beyond the forum's MESH gate
// (WORLD_VIEW_BIBLE.md §3 A5). Bare dark stone, a dormant portal ring, an
// engraved plaque, dense fog, one cold downlight.
//
// HONESTY CONTRACT (bible §6): everything this room says about the Mesh comes
// from `useMeshStatus()` and nothing else. There are no peers to draw, no
// traffic to animate and no endpoint behind it. Offline renders a dormant ring
// and `MESH: NOT CONNECTED`; only a genuinely connected status lights the event
// horizon and prints a peer count.
//
// It is a scene branch of the forum's single Canvas, not a second Canvas and
// not a new tool or tab: `ForumView` hides the forum group and mounts this
// instead, so the GLB is never loaded twice.

import { useEffect, useMemo, useRef } from 'react';
import { useFrame, useThree } from '@react-three/fiber';
import { Html, OrbitControls } from '@react-three/drei';
import * as THREE from 'three';
import { ENV } from '../shared/palette';
import { useMeshStatus, type MeshStatus } from '../shared/meshStatus';
import { InstancedProp, type InstanceTransform } from '../shared/instancing';
import { getReduceMotion } from '../../../styles/tokens';
import { MESH_GATE, gateLocalToWorld, gateYaw } from './meshPortal';

/** The only sentence this room is allowed to say about the Mesh. */
export function meshPlaqueText(status: MeshStatus): string {
  if (status.state === 'connected') {
    return `MESH: CONNECTED · ${status.peerCount} ${status.peerCount === 1 ? 'peer' : 'peers'}`;
  }
  if (status.state === 'connecting') return 'MESH: CONNECTING';
  return 'MESH: NOT CONNECTED';
}

const AGORA_BG = '#080A10';
const AGORA_FOG = '#0C1018';
const STONE = '#1B1D25';
const STONE_DARK = '#121419';

// Shared module-level materials: this room is a handful of draw calls and every
// one of them is reused across mounts (same approach as the legacy Stargate).
const stoneMat = new THREE.MeshStandardMaterial({ color: STONE, roughness: 0.94, metalness: 0.04 });
const darkStoneMat = new THREE.MeshStandardMaterial({ color: STONE_DARK, roughness: 0.97, metalness: 0.02 });
const bronzeMat = new THREE.MeshStandardMaterial({ color: ENV.bronze, roughness: 0.34, metalness: 0.78 });
const dormantChevronMat = new THREE.MeshStandardMaterial({ color: ENV.bronze, roughness: 0.4, metalness: 0.7 });
const liveChevronMat = new THREE.MeshStandardMaterial({
  color: ENV.bronze,
  emissive: ENV.horizonBlue,
  emissiveIntensity: 1.4,
  roughness: 0.25,
  metalness: 0.6,
});
const steleMat = new THREE.MeshStandardMaterial({ color: '#15171E', roughness: 0.9, metalness: 0.06 });

const chevronGeo = new THREE.BoxGeometry(0.42, 0.62, 0.7);
const steleGeo = new THREE.BoxGeometry(0.6, 3.2, 0.3);

// Event horizon — the ripple/vortex shader from
// `world/areas/antechamber/Stargate.tsx`, reused verbatim so the two portals in
// the product read as the same technology. Only ever mounted when the status is
// genuinely 'connected'.
const HORIZON_VERT = `
varying vec2 vUv;
void main() {
  vUv = uv;
  gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
}`;

const HORIZON_FRAG = `
uniform float uTime;
uniform vec3 uColor;
varying vec2 vUv;

void main() {
  vec2 c = vUv - 0.5;
  float r = length(c);
  float angle = atan(c.y, c.x);

  float ripple1 = sin(r * 18.0 - uTime * 2.5) * 0.5 + 0.5;
  float ripple2 = sin(r * 12.0 + uTime * 1.8 + angle * 3.0) * 0.5 + 0.5;
  float spiral = sin(angle * 4.0 + r * 8.0 - uTime * 1.2) * 0.5 + 0.5;

  float pattern = ripple1 * 0.4 + ripple2 * 0.3 + spiral * 0.3;
  float edgeFade = smoothstep(0.5, 0.35, r);
  float coreBright = smoothstep(0.3, 0.0, r) * 0.3;

  float alpha = (pattern * 0.6 + 0.2 + coreBright) * edgeFade;
  vec3 col = uColor * (0.8 + pattern * 0.4) + vec3(coreBright * 0.5);

  gl_FragColor = vec4(col, alpha);
}`;

function EventHorizon() {
  const material = useRef<THREE.ShaderMaterial>(null);
  const uniforms = useMemo(
    () => ({ uTime: { value: 0 }, uColor: { value: new THREE.Color(ENV.horizonBlue) } }),
    []
  );
  // Reduced motion freezes the horizon into a still pattern rather than hiding
  // it: the room must still look connected when it is connected.
  useFrame(() => {
    if (material.current && !getReduceMotion()) {
      material.current.uniforms.uTime.value = performance.now() * 0.001;
    }
  });
  return (
    <mesh position={[0, MESH_GATE.center[1], 0]}>
      <circleGeometry args={[MESH_GATE.radius - MESH_GATE.tube - 0.1, 48]} />
      <shaderMaterial
        ref={material}
        vertexShader={HORIZON_VERT}
        fragmentShader={HORIZON_FRAG}
        uniforms={uniforms}
        transparent
        side={THREE.DoubleSide}
        depthWrite={false}
        blending={THREE.AdditiveBlending}
      />
    </mesh>
  );
}

function PortalRing({ connected }: { connected: boolean }) {
  const chevrons = useMemo<InstanceTransform[]>(
    () =>
      Array.from({ length: 9 }, (_, i) => {
        const a = (i / 9) * Math.PI * 2 + Math.PI / 2;
        return {
          position: [Math.cos(a) * MESH_GATE.radius, MESH_GATE.center[1] + Math.sin(a) * MESH_GATE.radius, 0] as [number, number, number],
          rotation: [0, 0, a - Math.PI / 2] as [number, number, number],
        };
      }),
    []
  );
  // The inlaid horizonBlue channel is engraved into the ring's inner face and is
  // faintly lit even when dormant (bible §1: light is engraved into the stone).
  const channelMat = useMemo(
    () =>
      new THREE.MeshStandardMaterial({
        color: ENV.horizonBlue,
        emissive: ENV.horizonBlue,
        emissiveIntensity: connected ? 1.8 : 0.42,
        roughness: 0.2,
        metalness: 0.4,
        side: THREE.DoubleSide,
      }),
    [connected]
  );
  useEffect(() => () => channelMat.dispose(), [channelMat]);
  return (
    <group position={[0, 0, 0]}>
      <mesh position={[0, MESH_GATE.center[1], 0]} material={bronzeMat} castShadow={false}>
        <torusGeometry args={[MESH_GATE.radius, MESH_GATE.tube, 12, 48]} />
      </mesh>
      {/* The inlaid channel is the ring's inner bore, not a hoop buried inside
          the tube — a torus at that radius sits entirely within the ring body
          and is never visible. An open cylinder at the bore radius is the face
          the walker actually looks at on the way through. */}
      <mesh position={[0, MESH_GATE.center[1], 0]} rotation-x={Math.PI / 2} material={channelMat}>
        <cylinderGeometry args={[MESH_GATE.radius - MESH_GATE.tube, MESH_GATE.radius - MESH_GATE.tube, 0.34, 48, 1, true]} />
      </mesh>
      <InstancedProp
        name="meshGate.chevron"
        geometry={chevronGeo}
        material={connected ? liveChevronMat : dormantChevronMat}
        transforms={chevrons}
      />
      {connected && <EventHorizon />}
    </group>
  );
}

function Chamber() {
  return (
    <group>
      {/* Floor: the court on the forum side plus the eight metres of walk-through
          and the exedra floor beyond the ring. */}
      <mesh position={[0, -0.1, 2]} material={stoneMat} receiveShadow={false}>
        <boxGeometry args={[22, 0.2, 50]} />
      </mesh>
      {/* Semicircular exedra, bowing away from the forum. */}
      <mesh position={[0, MESH_GATE.exedra.height / 2, MESH_GATE.exedra.offset]} material={darkStoneMat}>
        <cylinderGeometry
          args={[
            MESH_GATE.exedra.radius,
            MESH_GATE.exedra.radius,
            MESH_GATE.exedra.height,
            28,
            1,
            true,
            -Math.PI / 2,
            Math.PI,
          ]}
        />
      </mesh>
      {/* Low parapet at the outer lip. */}
      <mesh position={[0, 0.45, MESH_GATE.exedra.offset + MESH_GATE.exedra.radius + 0.6]} material={darkStoneMat}>
        <boxGeometry args={[16, 0.9, 0.5]} />
      </mesh>
      {/* Three-step dais under the ring; its top face is the authored y=0.72. */}
      {MESH_GATE.dais.radii.map((r, i) => (
        <mesh key={r} position={[0, (MESH_GATE.dais.top / 3) * (i + 0.5), 0]} material={stoneMat}>
          <cylinderGeometry args={[r, r, MESH_GATE.dais.top / 3, 24]} />
        </mesh>
      ))}
    </group>
  );
}

function Steles() {
  // Two dormant Chitin sigil steles flanking the plaque — unlit engraved
  // channels, never a status light.
  const transforms = useMemo<InstanceTransform[]>(
    () => [
      { position: [-2.4, 1.6, MESH_GATE.exedra.offset + 5.4] },
      { position: [2.4, 1.6, MESH_GATE.exedra.offset + 5.4] },
    ],
    []
  );
  return <InstancedProp name="meshGate.stele" geometry={steleGeo} material={steleMat} transforms={transforms} />;
}

function AgoraCamera() {
  const camera = useThree((s) => s.camera);
  const controls = useRef<React.ComponentRef<typeof OrbitControls>>(null);
  useEffect(() => {
    const eye = gateLocalToWorld([5.5, 4.6, -22]);
    const look = gateLocalToWorld([0, 4.6, 5]);
    camera.position.set(eye[0], eye[1], eye[2]);
    camera.lookAt(look[0], look[1], look[2]);
    controls.current?.target.set(look[0], look[1], look[2]);
    controls.current?.update();
  }, [camera]);
  return (
    <OrbitControls ref={controls} makeDefault minDistance={4} maxDistance={48} maxPolarAngle={Math.PI / 2.05} />
  );
}

/**
 * @param walking true while the Sovereign is on foot — the room then leaves the
 * camera alone so they can keep walking (and walk back in through the ring).
 */
export function MeshAgora({ walking = false }: { walking?: boolean }) {
  const status = useMeshStatus();
  const connected = status.state === 'connected';
  const plaque = meshPlaqueText(status);
  const [cx, , cz] = MESH_GATE.center;
  return (
    <>
      <color attach="background" args={[AGORA_BG]} />
      <fogExp2 attach="fog" args={[AGORA_FOG, 0.011]} />
      <hemisphereLight args={['#36435F', '#05070C', 0.5]} />
      <group position={[cx, 0, cz]} rotation-y={gateYaw()}>
        <Chamber />
        <PortalRing connected={connected} />
        <Steles />
        {/* The single cold downlight and its housing. */}
        <mesh position={[0, 4.9, MESH_GATE.exedra.offset + 4.2]} material={bronzeMat}>
          <cylinderGeometry args={[0.34, 0.26, 0.3, 12]} />
        </mesh>
        <pointLight
          position={[0, 4.6, MESH_GATE.exedra.offset + 4.2]}
          color="#9FB8D8"
          intensity={44}
          distance={22}
          decay={2}
        />
        {/* A second cold wash over the ring itself, so the portal reads as
            stone and bronze rather than as a silhouette. */}
        <pointLight position={[0, 10, -2]} color="#8FA6C6" intensity={70} distance={34} decay={2} />
        {/* …and a dim wash over the approach floor, so the room the walker is
            standing in is stone rather than a void. */}
        <pointLight position={[0, 6, -13]} color="#7E93B2" intensity={34} distance={28} decay={2} />
        <Html
          position={[0, 3.05, MESH_GATE.exedra.offset + 5.9]}
          center
          distanceFactor={30}
          zIndexRange={[8, 0]}
        >
          <div className="forum-mesh-plaque" data-testid="mesh-plaque" data-state={status.state}>
            {plaque}
          </div>
        </Html>
        {!walking && <AgoraCamera />}
      </group>
    </>
  );
}
