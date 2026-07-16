// Agent character — WORLD_VIEW_BIBLE.md §4 body language, §8 perf law.
// State is expressed ONLY through the state channels (visor, joint glow rings,
// cape circuit lines, feet aura) + posture; identity trim never changes color.
// All continuous motion runs in useFrame against refs — React state changes only
// on discrete events (hover, label visibility crossing 18u).

import { useEffect, useMemo, useRef, useState } from 'react';
import { useFrame } from '@react-three/fiber';
import { Html } from '@react-three/drei';
import * as THREE from 'three';
import { ENV, type AgentHudState } from '../shared/palette';
import type { AgentIdentity } from './roster';
import { getMotion } from './motion';
import { createAgentRig } from './rig';
import {
  BONE_NAMES,
  POSES,
  STATE_VISUALS,
  TRANSITION_S,
  ERROR_FLICKER_HZ,
  ERROR_FLICKER_S,
  resolvePose,
  resolveVisual,
  type PoseKey,
} from './poses';
import { publishHenryPosition } from './henryPresence';
import { advanceMining, resolveTablet } from './librarianMining';
import { getDissolve } from '../areas/forum/agoraArc';

const LABEL_ON_DIST = 18;
const LABEL_OFF_DIST = 20; // hysteresis so the label doesn't strobe at the boundary

// Shared hover-ring resources (created once, app lifetime).
let hoverRingGeo: THREE.RingGeometry | null = null;
let hoverRingMat: THREE.MeshBasicMaterial | null = null;
function getHoverRing() {
  if (!hoverRingGeo) {
    hoverRingGeo = new THREE.RingGeometry(0.8, 1.0, 32);
    hoverRingMat = new THREE.MeshBasicMaterial({
      color: ENV.neonCyan,
      transparent: true,
      opacity: 0.6,
    });
  }
  return { geo: hoverRingGeo, mat: hoverRingMat as THREE.MeshBasicMaterial };
}

function easeInOut(t: number): number {
  return t < 0.5 ? 2 * t * t : 1 - (-2 * t + 2) ** 2 / 2;
}

interface TransitionState {
  pose: PoseKey;
  poseT: number;
  fromRootY: number;
  toRootY: number;
  /** Color register key — a HUD state OR 'tending' (the third register, bible §4). */
  colorKey: string;
  colorT: number;
  errorStart: number;
  initialized: boolean;
}

interface AgentCharacterProps {
  identity: AgentIdentity;
  hudState: AgentHudState;
  hovered: boolean;
  onPointerOver: () => void;
  onPointerOut: () => void;
  onClick: () => void;
}

export function AgentCharacterV2({
  identity,
  hudState,
  hovered,
  onPointerOver,
  onPointerOut,
  onClick,
}: AgentCharacterProps) {
  const groupRef = useRef<THREE.Group>(null);
  const [labelOn, setLabelOn] = useState(false);

  const rig = useMemo(
    () =>
      createAgentRig({
        trimColor: identity.trimColor,
        weathering: identity.weathering,
        crown: identity.isHenry,
        variant: identity.isHenry ? 'henry' : identity.id === 'librarian' ? 'librarian' : null,
      }),
    [identity],
  );
  useEffect(() => () => rig.dispose(), [rig]);

  // Pre-allocated per-bone quaternion buffers (zero per-frame allocations).
  const quats = useMemo(
    () => ({
      pose: BONE_NAMES.map(() => new THREE.Quaternion()),
      from: BONE_NAMES.map(() => new THREE.Quaternion()),
      to: BONE_NAMES.map(() => new THREE.Quaternion()),
      euler: new THREE.Euler(),
      colorFrom: new THREE.Color(STATE_VISUALS.idle.color),
      colorTo: new THREE.Color(STATE_VISUALS.idle.color),
      visorFrom: STATE_VISUALS.idle.visorIntensity,
      stateFrom: STATE_VISUALS.idle.stateIntensity,
    }),
    [],
  );

  const trans = useRef<TransitionState>({
    pose: 'idle',
    poseT: 1,
    fromRootY: 0,
    toRootY: 0,
    colorKey: 'idle',
    colorT: 1,
    errorStart: -1,
    initialized: false,
  });

  const hudRef = useRef(hudState);
  hudRef.current = hudState;
  const hoveredRef = useRef(hovered);
  hoveredRef.current = hovered;

  useFrame((r3f, rawDt) => {
    const g = groupRef.current;
    const m = getMotion(identity.id);
    if (!g || !m) return;
    const dt = Math.min(rawDt, 0.1);
    const t = r3f.clock.elapsedTime;
    const hud = hudRef.current;
    const tr = trans.current;
    const bones = rig.bones;

    // ── Position + heading from the motion store ──
    g.position.set(m.x, m.y, m.z);
    g.rotation.y = m.heading;

    // ── Resolve target pose ──
    // The pure state→body mapping (poses.resolvePose, unit-tested): tending is a
    // THIRD register driven by engagement, never shown over real working/error;
    // the visual register lets tending override the HUD color with its warm-gray
    // (bible §4/§8). A change in tending-ness must also retrigger the color tween.
    const pose = resolvePose(hud, m.engaged);
    const visual = resolveVisual(hud, pose);
    const colorKey = pose === 'tending' ? 'tending' : hud;

    // ── Start transitions on discrete change (0.8s tween — no snapping) ──
    if (pose !== tr.pose || !tr.initialized) {
      const target = POSES[pose];
      for (let i = 0; i < BONE_NAMES.length; i++) {
        quats.from[i].copy(quats.pose[i]);
        const rot = target.rot[BONE_NAMES[i]];
        if (rot) quats.to[i].setFromEuler(quats.euler.set(rot[0], rot[1], rot[2]));
        else quats.to[i].identity();
      }
      tr.fromRootY = tr.fromRootY + (tr.toRootY - tr.fromRootY) * easeInOut(Math.min(1, tr.poseT));
      tr.toRootY = target.rootY;
      tr.pose = pose;
      tr.poseT = tr.initialized ? 0 : 1;
    }
    if (colorKey !== tr.colorKey || !tr.initialized) {
      // Capture the currently displayed values as the new "from".
      quats.colorFrom.copy(rig.stateMat.emissive);
      quats.visorFrom = rig.visorMat.emissiveIntensity;
      quats.stateFrom = rig.stateMat.emissiveIntensity;
      quats.colorTo.set(visual.color);
      if (colorKey === 'error') tr.errorStart = t;
      tr.colorKey = colorKey;
      tr.colorT = tr.initialized ? 0 : 1;
    }
    tr.initialized = true;

    // ── Pose blend ──
    tr.poseT = Math.min(1, tr.poseT + dt / TRANSITION_S);
    const pe = easeInOut(tr.poseT);
    for (let i = 0; i < BONE_NAMES.length; i++) {
      quats.pose[i].slerpQuaternions(quats.from[i], quats.to[i], pe);
      bones[BONE_NAMES[i]].quaternion.copy(quats.pose[i]);
    }
    bones.root.position.y = tr.fromRootY + (tr.toRootY - tr.fromRootY) * pe;

    // ── Color blend (state channels only — identity trim untouched) ──
    tr.colorT = Math.min(1, tr.colorT + dt / TRANSITION_S);
    const ce = easeInOut(tr.colorT);
    const visTarget = visual;
    rig.stateMat.emissive.lerpColors(quats.colorFrom, quats.colorTo, ce);
    rig.stateMat.color.copy(rig.stateMat.emissive);
    rig.visorMat.emissive.copy(rig.stateMat.emissive);
    rig.visorMat.color.copy(rig.stateMat.emissive);
    rig.stateMat.emissiveIntensity =
      quats.stateFrom + (visTarget.stateIntensity - quats.stateFrom) * ce;
    rig.visorMat.emissiveIntensity =
      quats.visorFrom + (visTarget.visorIntensity - quats.visorFrom) * ce;

    // Error visor: 2Hz flicker for 3s, then steady dim (bible §4).
    if (colorKey === 'error' && tr.errorStart >= 0) {
      const since = t - tr.errorStart;
      if (since < ERROR_FLICKER_S) {
        rig.visorMat.emissiveIntensity =
          Math.sin(since * Math.PI * 2 * ERROR_FLICKER_HZ) > 0 ? 1.8 : 0.12;
      }
    }

    // ── Ambient motion (layered on top of the blended pose) ──
    if (m.walking) {
      // Walk bob + limb swing.
      bones.root.position.y += 0.05 * Math.abs(Math.sin(t * 8));
      const swing = Math.sin(t * 8) * 0.45;
      bones.thighL.rotateX(swing);
      bones.thighR.rotateX(-swing);
      bones.armL.rotateX(-swing * 0.55);
      bones.armR.rotateX(swing * 0.55);
      g.rotation.z = 0;
    } else if (pose === 'tending') {
      // Unhurried haul/set sway (bible §4): a slow stoop-and-place cadence on the arms,
      // gentler and slower than the walk swing — never busy, never amber.
      g.rotation.z = Math.sin(t * 1.2) * 0.02;
      const haul = Math.sin(t * 1.1) * 0.18;
      bones.armL.rotateX(haul);
      bones.armR.rotateX(haul);
      bones.spine.rotateX(Math.max(0, Math.sin(t * 1.1)) * 0.06);
    } else {
      // Slow ambient sway (idle weight shift).
      g.rotation.z = Math.sin(t * 2) * 0.015;
      if (pose === 'seatedWork' || pose === 'standWork') {
        // Small periodic head nods while engaged.
        bones.head.rotateX(0.05 * Math.sin(t * 2.6) * Math.max(0, Math.sin(t * 0.45)));
      }
    }

    // ── Per-agent specials ──
    if (identity.isHenry) {
      // BODY ⇄ CODE (Agora arc, areas/forum/agoraArc.ts): Henry's embodied rig
      // dissolves into code as he crosses the portal and REMATERIALIZES on the
      // return — one system, both sides, driven by the single `dissolve` scalar
      // (0 = body … 1 = code). Scaling rig.root implodes/reforms the whole avatar
      // symmetrically; the code stream + his Agora glyph carry the crossing. The
      // literal per-vertex skinned-mesh shatter is deferred polish (see PR).
      const emb = 1 - getDissolve(); // 1 fully embodied … 0 fully code
      rig.root.scale.setScalar(Math.max(0.0001, emb));
      rig.root.visible = emb > 0.02;
      // Henry presides — publish his live position so W4 can gather light where he
      // stands (bible §4: "light gathers slightly where he stands"). He never lifts.
      publishHenryPosition(m.x, m.y, m.z, hud);
      // His own floor pool: gathers (brighter/larger) when he stops to inspect, fades
      // while walking. Tints toward his state color (the crown-gem crossover, bible §4).
      const pl = rig.presenceLight;
      if (pl) {
        const settle = m.walking ? 0.35 : 1;
        const pulse = 1 + 0.08 * Math.sin(t * 1.4);
        pl.scale.setScalar((0.7 + 0.3 * settle) * pulse);
        (pl.material as THREE.MeshBasicMaterial).opacity = (m.walking ? 0.09 : 0.18) * pulse;
      }
    } else if (identity.id === 'librarian') {
      // The Librarian's mining tablet — driven by REAL describe events (librarianMining).
      advanceMining(performance.now());
      const tab = resolveTablet(performance.now());
      const tablet = rig.tablet;
      if (tablet) {
        tablet.visible = tab.visible;
        if (tab.visible) {
          // Tablet rides from the shelf (reach 0) into the hands (reach 1).
          tablet.position.set(0, 1.05, 0.18 + tab.reach * 0.32);
          tablet.material.emissiveIntensity = tab.glow * 2.0;
        }
      }
    }

    // Feet aura: breathing when available; steady otherwise; slight lift on hover.
    const breathe = hud === 'available' ? 1 + 0.18 * Math.sin(t * 1.8) : 1;
    bones.aura.scale.setScalar(hoveredRef.current ? breathe * 1.15 : breathe);

    // ── Always-on small label within 18u of camera (discrete state change) ──
    const cam = r3f.camera.position;
    const dx = cam.x - m.x;
    const dy = cam.y - m.y;
    const dz = cam.z - m.z;
    const d2 = dx * dx + dy * dy + dz * dz;
    if (!labelOn && d2 < LABEL_ON_DIST * LABEL_ON_DIST) setLabelOn(true);
    else if (labelOn && d2 > LABEL_OFF_DIST * LABEL_OFF_DIST) setLabelOn(false);
  });

  const hover = getHoverRing();

  return (
    <group
      ref={groupRef}
      onPointerOver={(e) => {
        e.stopPropagation();
        onPointerOver();
      }}
      onPointerOut={(e) => {
        e.stopPropagation();
        onPointerOut();
      }}
      onClick={(e) => {
        e.stopPropagation();
        onClick();
      }}
    >
      <primitive object={rig.root} />

      {hovered && (
        <mesh rotation-x={-Math.PI / 2} position-y={0.02} geometry={hover.geo} material={hover.mat} />
      )}

      {/* Hover tooltip (existing drei Html pattern, kept) */}
      {hovered && (
        <Html position={[0, 3, 0]} center distanceFactor={15} style={{ pointerEvents: 'none' }}>
          <div
            style={{
              background: `${ENV.deepVoid}D9`,
              color: ENV.neonCyan,
              padding: '4px 12px',
              borderRadius: '6px',
              fontSize: '13px',
              fontFamily: 'monospace',
              border: `1px solid ${ENV.neonCyan}40`,
              whiteSpace: 'nowrap',
              backdropFilter: 'blur(4px)',
            }}
          >
            {identity.name}
            {identity.isHenry && ' (Orchestrator)'}
            {identity.id === 'librarian' && ' (The Brain)'}
          </div>
        </Html>
      )}

      {/* Always-on small label, camera within 18u only (bible §4) */}
      {labelOn && !hovered && (
        <Html position={[0, 2.7, 0]} center distanceFactor={12} style={{ pointerEvents: 'none' }}>
          <div
            style={{
              color: `${ENV.marble}B3`,
              fontSize: '10px',
              fontFamily: 'monospace',
              letterSpacing: '0.08em',
              textShadow: `0 0 6px ${ENV.deepVoid}`,
              whiteSpace: 'nowrap',
            }}
          >
            {identity.name.toUpperCase()}
          </div>
        </Html>
      )}
    </group>
  );
}
