// The Agora — the collective mind of the mesh, BEYOND the Stargate (#306 prep).
//
// ════════════════════════════════════════════════════════════════════════════
// CONCEPT: the private Brain graph turned INSIDE-OUT. Permagent already renders
// your PRIVATE mind as a constellation (brain/BrainScene.ts). The Agora is its
// OUTWARD TWIN — shared cognition in the same light-language, opposite scale.
// No floor, no walls: a weightless dark data-volume. Every sovereign agent
// present is a STAR (an instanced point-glyph, GlyphField.tsx), drifting into
// shared-interest clusters (agoraSim.ts), communion legible as light pulses
// flowing the edges between them. Body (warm marble Rotunda) ⇄ code (this void);
// the Stargate is the membrane between the two states.
//
// THE INTERACTION ARC (agoraArc.ts — one reversible system):
//   1. DESCENT   — Henry (the sovereign) walks to the portal (agents/behavior.ts).
//   2. DISSOLVE  — at the threshold his embodied rig unravels into a stream of
//                  code (his chiti hue) that flows through the portal (~1.35s).
//   3. ABSORB    — the stream coalesces into his glyph; the swarm receives it
//                  (a ripple of acknowledgment) and he drifts into the mesh.
//   4. WITNESS   — the user holds and watches the living constellation breathe.
//   5. RETURN    — the mirror image, played backward: code streams home through
//                  the portal and REMATERIALIZES his full rig on the Rotunda side.
//   dissolve (0=body … 1=code) is the single scalar every beat reads, so forward
//   and reverse are guaranteed symmetric. The rig fade/reform lives in
//   AgentCharacterV2 (isHenry), also reading `dissolve` — one system, both sides.
//
// HONEST-WHEN-EMPTY (§4): population reads only from the frozen meshStatus. Mesh
// offline today ⇒ a dark waiting void, one distant horizon, Henry's lone glyph at
// the threshold, "the Agora awaits the mesh". Never fabricates peers.
// BUDGET (§8): swarm = ONE points draw call (0→thousands); dormant ⇒ ~8 draw
// calls, zero per-frame cost. All useFrame loops are zero-allocation.
// ════════════════════════════════════════════════════════════════════════════

import { useMemo, useRef } from 'react';
import { useFrame, useThree } from '@react-three/fiber';
import { Html } from '@react-three/drei';
import * as THREE from 'three';
import { ENV, AGENT_TRIM } from '../../shared/palette';
import { getReduceMotion, radius } from '../../../../styles/tokens';
import { getAgentPosition } from '../../agents/motion';
import { GlyphField } from './GlyphField';
import { useAgoraPopulation } from './agoraPeers';
import { getArc, getDissolve, tickArc } from './agoraArc';

const ANTE_ANGLE = (Math.PI * 5) / 4; // local +x → world antechamber diagonal
const CX = 28; // swarm center distance along the diagonal (past the platform)
const SWARM_Y = 3; // the swarm floats
const THRESHOLD_LOCAL = -10; // self/Henry glyphs sit toward the gate side (inner frame)

// ─── Arc ticker: advances the state machine once per frame ───────────────────
function ArcTicker() {
  useFrame((_, dt) => tickArc(Math.min(dt, 0.1)));
  return null;
}

// ─── The distant horizon — "the mesh beyond" (the empty-state read) ──────────
function Horizon({ reduceMotion }: { reduceMotion: boolean }) {
  const ringRef = useRef<THREE.Mesh>(null);
  useFrame(() => {
    if (reduceMotion) return;
    const m = ringRef.current;
    if (!m) return;
    (m.material as THREE.MeshBasicMaterial).opacity = 0.1 + 0.03 * Math.sin(performance.now() * 0.0004);
  });
  return (
    <group>
      {/* A far luminous ring low in the void — the one distant horizon. */}
      <mesh ref={ringRef} rotation-x={-Math.PI / 2} position-y={-6}>
        <ringGeometry args={[26, 30, 96]} />
        <meshBasicMaterial color={ENV.horizonBlue} transparent opacity={0.1} side={THREE.DoubleSide} depthWrite={false} blending={THREE.AdditiveBlending} toneMapped={false} />
      </mesh>
    </group>
  );
}

// ─── Self + sovereign glyphs at the threshold (LOD tier a: full-formed) ──────
// These are YOU (isSelf) + your own sovereign agent (Henry) — the local presence,
// NOT mesh peers, so they are honestly present even offline. They are the two
// brightest, richest glyphs (BrainScene self-node language), distinguished from
// the instanced swarm. Henry's ignites as he is absorbed (dissolve → 1).
const selfMat = new THREE.MeshLambertMaterial({
  color: '#B8E8FF', emissive: ENV.neonCyan, emissiveIntensity: 1.2,
});
const henryMat = new THREE.MeshLambertMaterial({
  color: '#FFF4D8', emissive: AGENT_TRIM.henry, emissiveIntensity: 1.0,
});

function ThresholdGlyphs({ reduceMotion }: { reduceMotion: boolean }) {
  const selfRef = useRef<THREE.Mesh>(null);
  const henryRef = useRef<THREE.Mesh>(null);
  const rippleRef = useRef<THREE.Mesh>(null);

  useFrame((_, dt) => {
    const t = performance.now() * 0.001;
    const diss = getDissolve();
    if (selfRef.current && !reduceMotion) {
      selfRef.current.rotation.y += dt * 0.4;
      selfRef.current.rotation.x += dt * 0.15;
      (selfRef.current.material as THREE.MeshLambertMaterial).emissiveIntensity = 1.1 + 0.15 * Math.sin(t * 2);
    }
    if (henryRef.current) {
      if (!reduceMotion) henryRef.current.rotation.y += dt * 0.5;
      // Henry ignites as he is absorbed: dim at home, full in the mesh.
      (henryRef.current.material as THREE.MeshLambertMaterial).emissiveIntensity = 0.5 + 1.4 * diss;
      const s = 0.7 + 0.5 * diss;
      henryRef.current.scale.setScalar(s);
    }
    // Absorb ripple — a ring flashes as the swarm receives the new glyph
    // (peaks as dissolve nears 1; mirror-plays on the return). Zero alloc.
    if (rippleRef.current) {
      const k = Math.max(0, (diss - 0.65) / 0.35); // 0..1 over the last of the dissolve
      const scl = 0.5 + k * 5;
      rippleRef.current.scale.setScalar(scl);
      (rippleRef.current.material as THREE.MeshBasicMaterial).opacity = 0.5 * k * (1 - k) * 4;
    }
  });

  return (
    <group position={[THRESHOLD_LOCAL, 0, 0]}>
      <mesh ref={selfRef} position={[-1.3, 0, 0]} material={selfMat}>
        <icosahedronGeometry args={[0.7, 1]} />
      </mesh>
      <mesh ref={henryRef} position={[1.3, 0, 0]} material={henryMat}>
        <icosahedronGeometry args={[0.62, 1]} />
      </mesh>
      {/* Absorb ripple around Henry's glyph. */}
      <mesh ref={rippleRef} position={[1.3, 0, 0]} rotation-x={-Math.PI / 2}>
        <ringGeometry args={[0.5, 0.62, 40]} />
        <meshBasicMaterial color={AGENT_TRIM.henry} transparent opacity={0} depthWrite={false} blending={THREE.AdditiveBlending} toneMapped={false} />
      </mesh>
      {/* The glyphs and the ripple are emissive and additive — they carry
          the Agora's blue on their own. The pointLight that used to sit here
          only lit the floor under them. */}
    </group>
  );
}

// ─── Honest inscription (reads the frozen population) ────────────────────────
function AgoraInscription() {
  const pop = useAgoraPopulation();
  const accent = pop.online ? ENV.neonCyan : '#5A6478';
  return (
    <Html position={[0, 7, 0]} center distanceFactor={26} style={{ pointerEvents: 'none' }}>
      <div
        style={{
          padding: '6px 16px', borderRadius: radius.xs, background: 'rgba(10, 14, 26, 0.8)',
          border: `1px solid ${accent}55`, boxShadow: pop.online ? `0 0 20px ${accent}44` : 'none',
          color: '#E8E4DD', fontFamily: 'JetBrains Mono, monospace', fontSize: 10,
          letterSpacing: '0.2em', textAlign: 'center', whiteSpace: 'nowrap',
        }}
      >
        <div style={{ color: accent, fontWeight: 700 }}>THE AGORA</div>
        <div style={{ marginTop: 3, opacity: 0.75 }}>
          {pop.online
            ? `${pop.total} SOVEREIGN MIND${pop.total === 1 ? '' : 'S'} IN THE MESH${pop.overflow > 0 ? ` · +${pop.overflow} BEYOND THE HORIZON` : ''}`
            : 'AWAITS THE MESH'}
        </div>
      </div>
    </Html>
  );
}

/**
 * The Agora space. Mount once from WorldSceneContent. Places itself in world
 * space beyond the Stargate (local +x → the antechamber diagonal).
 */
export function Agora() {
  const reduceMotion = useMemo(() => getReduceMotion(), []);
  const pop = useAgoraPopulation();
  const pixelRatio = useThree((s) => s.gl.getPixelRatio());

  return (
    <>
      <ArcTicker />
      <group rotation-y={-ANTE_ANGLE}>
        <group position={[CX, SWARM_Y, 0]}>
          <Horizon reduceMotion={reduceMotion} />
          <ThresholdGlyphs reduceMotion={reduceMotion} />
          <GlyphField pop={pop} pixelRatio={pixelRatio} />
          <AgoraInscription />
        </group>
      </group>
    </>
  );
}

// ─── The portal dissolve/reconstitute stream (world space) ───────────────────
// A comet of code that flows Henry(rig) → portal mouth → his Agora glyph as he
// dissolves, and back on the return — the SAME buffer read forward/backward from
// `dissolve`, so body→code and code→body are literally one animation. Rendered in
// world space (it crosses the rotation boundary between Rotunda and Agora).
const STREAM_N = 44;
const _hp = new THREE.Vector3();
const _mid = new THREE.Vector3();
const _end = new THREE.Vector3();
const _p = new THREE.Vector3();

// Portal mouth + Henry's Agora glyph, in WORLD space (diagonal at r=14.6 / 17.3).
const PORTAL_W = new THREE.Vector3(Math.cos(ANTE_ANGLE) * 14.6, 3, Math.sin(ANTE_ANGLE) * 14.6);
const GLYPH_W = new THREE.Vector3(Math.cos(ANTE_ANGLE) * (CX + THRESHOLD_LOCAL), SWARM_Y, Math.sin(ANTE_ANGLE) * (CX + THRESHOLD_LOCAL));
const streamColor = new THREE.Color(AGENT_TRIM.henry);

function bezier(a: THREE.Vector3, b: THREE.Vector3, c: THREE.Vector3, s: number, out: THREE.Vector3): void {
  const u = 1 - s;
  out.set(
    u * u * a.x + 2 * u * s * b.x + s * s * c.x,
    u * u * a.y + 2 * u * s * b.y + s * s * c.y,
    u * u * a.z + 2 * u * s * b.z + s * s * c.z,
  );
}

export function PortalStream() {
  const ref = useRef<THREE.Points>(null);
  const pixelRatio = useThree((s) => s.gl.getPixelRatio());
  const geo = useMemo(() => {
    const g = new THREE.BufferGeometry();
    const p = new Float32Array(STREAM_N * 3);
    const c = new Float32Array(STREAM_N * 3);
    const s = new Float32Array(STREAM_N);
    for (let i = 0; i < STREAM_N; i++) {
      c[i * 3] = streamColor.r; c[i * 3 + 1] = streamColor.g; c[i * 3 + 2] = streamColor.b;
      s[i] = 0.5 + (1 - i / STREAM_N) * 0.5;
    }
    g.setAttribute('position', new THREE.BufferAttribute(p, 3));
    g.setAttribute('color', new THREE.BufferAttribute(c, 3));
    g.setAttribute('size', new THREE.BufferAttribute(s, 1));
    return g;
  }, []);
  const mat = useMemo(
    () => new THREE.ShaderMaterial({
      vertexShader: `attribute float size; varying vec3 vCol; uniform float uPx; uniform float uOpacity;
        void main(){ vCol = color * uOpacity; vec4 mv = modelViewMatrix*vec4(position,1.0);
        gl_PointSize = size*uPx*(300.0/-mv.z); gl_Position = projectionMatrix*mv; }`,
      fragmentShader: `varying vec3 vCol; void main(){ vec2 c=gl_PointCoord-0.5; float d=length(c);
        float a=smoothstep(0.5,0.0,d); float core=smoothstep(0.22,0.0,d);
        gl_FragColor=vec4(vCol+core*vec3(0.6), a); }`,
      uniforms: { uPx: { value: pixelRatio }, uOpacity: { value: 0 } },
      vertexColors: true, transparent: true, blending: THREE.AdditiveBlending, depthWrite: false,
    }),
    [pixelRatio],
  );

  useFrame(() => {
    const arc = getArc();
    if (arc.phase === 'home') {
      geo.setDrawRange(0, 0);
      return;
    }
    const diss = getDissolve();
    // Stream opacity peaks mid-transition, fades at both ends (home + full witness).
    mat.uniforms.uOpacity.value = Math.sin(Math.min(1, diss) * Math.PI);
    const hp = getAgentPosition('henry');
    _hp.set(hp?.x ?? PORTAL_W.x, (hp?.y ?? 0) + 1.4, hp?.z ?? PORTAL_W.z);
    // Control point at the portal mouth; the code arcs through the membrane.
    _mid.copy(PORTAL_W);
    _end.copy(GLYPH_W);
    const posAttr = geo.getAttribute('position') as THREE.BufferAttribute;
    const arr = posAttr.array as Float32Array;
    for (let i = 0; i < STREAM_N; i++) {
      // Comet: head at `diss` along the path, tail trailing behind it.
      let s = diss - (i / STREAM_N) * 0.55;
      if (s < 0) s = 0;
      bezier(_hp, _mid, _end, s, _p);
      arr[i * 3] = _p.x; arr[i * 3 + 1] = _p.y; arr[i * 3 + 2] = _p.z;
    }
    posAttr.needsUpdate = true;
    geo.setDrawRange(0, STREAM_N);
  });

  return <points ref={ref} geometry={geo} material={mat} />;
}
