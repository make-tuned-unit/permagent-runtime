// Agent character — WORLD_VIEW_BIBLE.md §4 body language, §8 perf law.
// State is expressed ONLY through the state channels (visor, joint glow rings,
// cape circuit lines, feet aura) + posture; identity trim never changes color.
// All continuous motion runs in useFrame against refs — React state changes only
// on discrete events (hover, label visibility crossing 18u).

import { useEffect, useMemo, useRef, useState } from 'react';
import { useFrame } from '@react-three/fiber';
import { Html } from '@react-three/drei';
import * as THREE from 'three';
import { getReduceMotion, textSize } from '../../../styles/tokens';
import { ENV, type AgentHudState } from '../shared/palette';
import { getAgentRuntimeStates } from '../shared/agentStatus';
import type { AgentIdentity } from './roster';
import { getMotion } from './motion';
import { createAgentRig } from './rig';
import { loadBlenderArmor, type BlenderArmor } from './blenderArmor';
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
import { getIdPhase } from './idPhase';
import {
  swayZ,
  tendingHaul,
  tendingSpineLean,
  headNod,
  breathingScale,
  blinkEnvelope,
  resolveLookYaw,
  resolveLookPitch,
} from './idleLife';
import { publishHenryPosition } from './henryPresence';
import { advanceMining, resolveTablet } from './librarianMining';
import { getDissolve } from '../areas/forum/agoraArc';

/** Head bone's rest-pose height above root (rig.ts BONE_BIND.head) — used only
 * as an approximation for the look-at target math below, not for rendering. */
const HEAD_HEIGHT = 1.78;
/** Look-at damping rate — matches the codebase's existing MathUtils.damp
 * lambdas (atmosphere/ColonnadeLanterns use 0.8-4); a head turn reads as more
 * alert than a light level fade, so it sits at the fast end of that range. */
const LOOK_DAMP = 6;

const LABEL_ON_DIST = 18;
const LABEL_OFF_DIST = 20; // hysteresis so the label doesn't strobe at the boundary

declare global {
  interface Window {
    __worldAgentScreens?: Record<string, { client: [number, number]; ndcZ: number }>;
  }
}

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
  /** Whichever agent id is currently hovered scene-wide (or null) — drives
   *  look-at (bible §4): an agent notices attention paid to it or to a
   *  neighbor. Not the same as `hovered`, which is only this agent's own flag. */
  hoveredAgentId: string | null;
  onPointerOver: () => void;
  onPointerOut: () => void;
  onClick: () => void;
}

export function AgentCharacterV2({
  identity,
  hudState,
  hovered,
  hoveredAgentId,
  onPointerOver,
  onPointerOut,
  onClick,
}: AgentCharacterProps) {
  const groupRef = useRef<THREE.Group>(null);
  const screenScratch = useRef(new THREE.Vector3());
  const [labelOn, setLabelOn] = useState(false);
  // Stable per-agent phase (idPhase.ts) — the fix for seven agents breathing/
  // swaying/blinking in lockstep off the one shared r3f clock. Derived from the
  // id, which never changes for a mounted character, so this is safe to memoize.
  const phase = useMemo(() => getIdPhase(identity.id), [identity.id]);
  // Read once per mount, same pattern as every other reduceMotion consumer in
  // world/ (PetitionBasin, Horologium, TourMode, ...) — a live toggle mid-session
  // is a reload away, which matches how the setting is surfaced elsewhere.
  const reduceMotion = useMemo(() => getReduceMotion(), []);
  // Damped look-at state (yaw/pitch), persisted across frames outside React.
  const look = useRef({ yaw: 0, pitch: 0 });

  const [armor, setArmor] = useState<BlenderArmor | null>(null);
  useEffect(() => {
    let mounted = true;
    setArmor(null);
    loadBlenderArmor(identity.id).then(
      value => { if (mounted) setArmor(value); },
      () => { console.warn('[world] Authored character unavailable; retaining live procedural rig'); },
    );
    return () => { mounted = false; };
  }, [identity.id]);

  const rig = useMemo(
    () =>
      createAgentRig({
        trimColor: identity.trimColor,
        weathering: identity.weathering,
        crown: identity.isHenry,
        armor,
        // Every roster id maps to a signature-gear variant (rig.gearChunks);
        // 'librarian'/'henry' additionally keep their tablet/presence extras.
        variant: (['henry', 'librarian', 'reader', 'watcher', 'steward', 'strix', 'financier'].includes(identity.id)
          ? identity.id
          : null) as import('./rig').RigVariant,
      }),
    [identity, armor],
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
  const hoveredAgentIdRef = useRef(hoveredAgentId);
  hoveredAgentIdRef.current = hoveredAgentId;

  useFrame((r3f, rawDt) => {
    const g = groupRef.current;
    const m = getMotion(identity.id);
    if (!g || !m) return;
    const dt = Math.min(rawDt, 0.1);
    const t = r3f.clock.elapsedTime;
    const hud = hudRef.current;
    const tr = trans.current;
    const bones = rig.bones;
    // Camera position — read once per frame, reused by both the look-at block
    // below and the label-distance check further down.
    const cam = r3f.camera.position;

    // ── Position + heading from the motion store ──
    g.position.set(m.x, m.y, m.z);
    g.rotation.y = m.heading;
    if (import.meta.env.DEV && typeof window !== 'undefined') {
      const projected = screenScratch.current.set(m.x, m.y + 1.2, m.z).project(r3f.camera);
      const rect = r3f.gl.domElement.getBoundingClientRect();
      const screens = (window.__worldAgentScreens ??= {});
      screens[identity.id] = {
        client: [
          rect.left + (projected.x + 1) * rect.width / 2,
          rect.top + (1 - projected.y) * rect.height / 2,
        ],
        ndcZ: projected.z,
      };
    }

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
    const errorFlickerActive =
      colorKey === 'error' && tr.errorStart >= 0 && t - tr.errorStart < ERROR_FLICKER_S;
    if (errorFlickerActive) {
      const since = t - tr.errorStart;
      rig.visorMat.emissiveIntensity =
        Math.sin(since * Math.PI * 2 * ERROR_FLICKER_HZ) > 0 ? 1.8 : 0.12;
    }

    // ── Blink (bible §4/§8) ── A short dip in the visor's own emissiveIntensity,
    // MULTIPLYING whatever the state channel above just set — never an absolute
    // value, never a hue change, so it can never fight the state colour law.
    // Skipped during the deliberate 2Hz error flicker just above: that's already
    // a faster, un-ignorable pulse, and stacking a second one on top would read
    // as a glitch rather than two systems cooperating.
    if (!errorFlickerActive) {
      rig.visorMat.emissiveIntensity *= blinkEnvelope(t, phase, reduceMotion);
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
      // gentler and slower than the walk swing — never busy, never amber. Phase-shifted
      // (idPhase.ts) so seven tending agents don't haul in lockstep; zeroed under
      // reduced motion (idleLife.ts) rather than just held at its current value, so
      // toggling the setting mid-tend snaps cleanly to the neutral pose.
      g.rotation.z = swayZ(t, phase, reduceMotion, 1.2, 0.02);
      const haul = tendingHaul(t, phase, reduceMotion);
      bones.armL.rotateX(haul);
      bones.armR.rotateX(haul);
      bones.spine.rotateX(tendingSpineLean(t, phase, reduceMotion));
    } else {
      // Slow ambient sway (idle weight shift) — same phase-shift/reduced-motion
      // treatment as the tending branch above.
      g.rotation.z = swayZ(t, phase, reduceMotion, 2, 0.015);
      if (pose === 'seatedWork' || pose === 'standWork') {
        // Small periodic head nods while engaged.
        bones.head.rotateX(headNod(t, phase, reduceMotion));
      }
    }

    // ── Breathing (bible §4: "subtle ... you notice it only when it stops") ──
    // A small vertical scale on the spine bone (the torso proxy in this rig —
    // there's no separate chest bone). Own phase AND own rate per agent
    // (idleLife.ts), so it reads as seven people, not one loop offset in time.
    bones.spine.scale.y = breathingScale(t, phase, reduceMotion);

    // ── Look-at (bible §4 + honesty law) ──
    // Priority: if THIS agent is the one being hovered, it looks back at the
    // camera (eye contact); else if a DIFFERENT agent is hovered, this agent
    // notices and glances at them; else it glances at whichever peer is
    // genuinely working — sourced from agentStatus's clamped state, so a sim
    // agent (never 'working') can never be invented as something to look at;
    // else neutral (no look-at offset — the pose's own head orientation stands).
    // Damped, not snapped (THREE.MathUtils.damp, same style used across
    // atmosphere/), and yaw/pitch are hard-clamped in idleLife.ts so a head
    // never spins past what a person's actually can (bible §4: "~60° before
    // their body follows").
    const headWorldY = m.y + bones.root.position.y + HEAD_HEIGHT;
    let lookDX = 0;
    let lookDY = 0;
    let lookDZ = 0;
    let hasLookTarget = false;
    const hoveredId = hoveredAgentIdRef.current;
    if (hoveredId === identity.id) {
      lookDX = cam.x - m.x;
      lookDY = cam.y - headWorldY;
      lookDZ = cam.z - m.z;
      hasLookTarget = true;
    } else if (hoveredId) {
      const hoveredM = getMotion(hoveredId);
      if (hoveredM) {
        lookDX = hoveredM.x - m.x;
        lookDY = hoveredM.y + HEAD_HEIGHT - headWorldY;
        lookDZ = hoveredM.z - m.z;
        hasLookTarget = true;
      }
    } else {
      const runtimeStates = getAgentRuntimeStates();
      for (let i = 0; i < runtimeStates.length; i++) {
        if (runtimeStates[i].id !== identity.id && runtimeStates[i].hudState === 'working') {
          const workerM = getMotion(runtimeStates[i].id);
          if (workerM) {
            lookDX = workerM.x - m.x;
            lookDY = workerM.y + HEAD_HEIGHT - headWorldY;
            lookDZ = workerM.z - m.z;
            hasLookTarget = true;
          }
          break;
        }
      }
    }
    let desiredYaw = 0;
    let desiredPitch = 0;
    if (!reduceMotion && hasLookTarget) {
      desiredYaw = resolveLookYaw(lookDX, lookDZ, g.rotation.y);
      desiredPitch = resolveLookPitch(lookDY, Math.hypot(lookDX, lookDZ));
    }
    look.current.yaw = reduceMotion
      ? 0
      : THREE.MathUtils.damp(look.current.yaw, desiredYaw, LOOK_DAMP, dt);
    look.current.pitch = reduceMotion
      ? 0
      : THREE.MathUtils.damp(look.current.pitch, desiredPitch, LOOK_DAMP, dt);
    if (!reduceMotion) {
      bones.head.rotateY(look.current.yaw);
      bones.head.rotateX(look.current.pitch);
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
        const pulse = reduceMotion ? 1 : 1 + 0.08 * Math.sin(t * 1.4 + phase);
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

    // ── Work halo (agents have no desks — the work orbits them) ──
    // Fades in while processing standing, follows the live state color
    // (amber working / red error via stateMat), orbits slowly. Hidden the
    // moment the agent walks or the state moves on.
    const halo = rig.workHalo;
    if (halo) {
      const want = hud === 'working' && !m.walking ? 0.55 : 0;
      // Two-sided clamp (defense in depth, WORLD_VIEW bugfix): guards against
      // a negative dt driving opacity the wrong way, same reasoning as the
      // damp() call sites above.
      const next = halo.mat.opacity + (want - halo.mat.opacity) * THREE.MathUtils.clamp(dt * 5, 0, 1);
      halo.mat.opacity = next;
      halo.group.visible = next > 0.02;
      if (halo.group.visible) {
        halo.mat.color.copy(rig.stateMat.emissive);
        halo.group.rotation.y += dt * 0.9;
        halo.group.position.y = 0.04 * Math.sin(t * 1.6);
      }
    }

    // Feet aura: breathing when available; steady otherwise; slight lift on hover.
    // Phase-shifted and reduced-motion-neutral like the rest of this file's idle life.
    const breathe =
      hud === 'available' && !reduceMotion ? 1 + 0.18 * Math.sin(t * 1.8 + phase) : 1;
    bones.aura.scale.setScalar(hoveredRef.current ? breathe * 1.15 : breathe);

    // ── Always-on small label within 18u of camera (discrete state change) ──
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
              fontSize: `${textSize.small}px`,
              fontFamily: 'monospace',
              border: `1px solid ${ENV.neonCyan}40`,
              whiteSpace: 'nowrap',
              backdropFilter: 'blur(4px)',
            }}
          >
            {/* Henry is the AGENT'S NAME (user-chosen persona), not a role —
                no "(Orchestrator)" title (ruling 2026-07-28). */}
            {identity.name}
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
