import { useEffect, useMemo, useRef, useState } from 'react';
import { useFrame, useThree } from '@react-three/fiber';
import { useGLTF } from '@react-three/drei';
import * as THREE from 'three';
import { getReduceMotion } from '../../../styles/tokens';
import { FORUM_ASSET_REVISION } from './assetRevision';
import { extendForumLoader } from './forumGltf';
import { landingSampleAt } from './walkCollision';
import {
  flightStep,
  flightInputFromKeys,
  gearIsDown,
  thrusterGlow,
  type FlightState,
} from './flightMotion';

/** Where the skiff waits when you have not flown it yet: off the north quay. */
const SKIFF_PARKED: [number, number, number] = [0, 6, -86];

/**
 * The sovereign's skiff: board it, fly the world, land on the floating islands.
 *
 * The camera rides behind and above the hull rather than inside it. Landing is
 * the point of this vehicle — you are aiming at terraces that hang in the air —
 * and you cannot judge a touchdown from a cockpit you cannot see past.
 *
 * Flying and walking are the same journey: you fly somewhere, you set down, and
 * you step out where you landed. `onDisembark` hands the walker the exact spot
 * the skiff is standing on, so the world you explore on foot is the world you
 * chose from the air.
 */
export function ForumFlight({
  start,
  onExit,
  onDisembark,
}: {
  start?: [number, number, number];
  onExit: () => void;
  onDisembark: (at: [number, number, number]) => void;
}) {
  const { camera, gl } = useThree();
  const url = `${import.meta.env.BASE_URL}world/forum-skiff.glb?v=${FORUM_ASSET_REVISION}`;
  const { scene } = useGLTF(url, false, false, extendForumLoader);
  const skiff = useMemo(() => scene.clone(true), [scene]);
  const keys = useRef(new Set<string>());
  const [landed, setLanded] = useState(false);

  const state = useRef<FlightState>({
    x: (start ?? SKIFF_PARKED)[0],
    y: (start ?? SKIFF_PARKED)[1],
    z: (start ?? SKIFF_PARKED)[2],
    yaw: 0,
    vx: 0,
    vy: 0,
    vz: 0,
    landed: false,
  });

  useEffect(() => {
    const editable = (target: EventTarget | null) =>
      target instanceof HTMLElement &&
      (target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT', 'BUTTON'].includes(target.tagName));
    const shell = gl.domElement.closest('.forum-shell');
    const focusedElsewhere = () => {
      const active = document.activeElement;
      if (!active || active === document.body || active === document.documentElement) return false;
      return !!shell && !shell.contains(active);
    };
    // Same window-level binding and same guards as walking, so the two modes
    // never fight over a key or steer from behind another panel's back.
    const down = (e: KeyboardEvent) => {
      if (focusedElsewhere()) {
        keys.current.clear();
        return;
      }
      if (editable(e.target) || e.metaKey || e.ctrlKey || e.altKey) return;
      const bound = [
        'ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight',
        'KeyW', 'KeyS', 'KeyA', 'KeyD',
        'Space', 'KeyC', 'ShiftLeft', 'ShiftRight',
      ];
      if (bound.includes(e.code)) {
        e.preventDefault();
        keys.current.add(e.code);
      }
      if (e.code === 'Escape') {
        e.preventDefault();
        onExit();
      }
      if (e.code === 'KeyE' && !e.repeat && state.current.landed) {
        // Step out where the skiff is standing.
        onDisembark([state.current.x, state.current.y, state.current.z]);
      }
    };
    const up = (e: KeyboardEvent) => keys.current.delete(e.code);
    const clear = () => keys.current.clear();
    window.addEventListener('keydown', down);
    window.addEventListener('keyup', up);
    window.addEventListener('blur', clear);
    return () => {
      window.removeEventListener('keydown', down);
      window.removeEventListener('keyup', up);
      window.removeEventListener('blur', clear);
    };
  }, [gl, onExit, onDisembark]);

  useFrame((_, rawDt) => {
    // Reduce Motion quiets the incidental camera and hull response below, not
    // the controls the user needs to reach a destination and land.
    const reducedMotion = getReduceMotion();
    const input = flightInputFromKeys(keys.current);

    const next = flightStep(state.current, input, rawDt, (x, z) => landingSampleAt(x, z));
    state.current = next;
    if (next.landed !== landed) setLanded(next.landed);

    skiff.position.set(next.x, next.y, next.z);
    // Bank into the turn and pitch into the throttle: the cheapest honest cue
    // that the craft is doing what you asked, without an animation system.
    skiff.rotation.set(reducedMotion ? 0 : -input.thrust * 0.08, next.yaw, reducedMotion ? 0 : -input.turn * 0.22, 'YXZ');

    const ground = landingSampleAt(next.x, next.z);
    const gear = gearIsDown(next, ground);
    for (const name of ['LegFore', 'LegPortAft', 'LegStarAft']) {
      const leg = skiff.getObjectByName(name);
      if (leg) leg.visible = gear;
      const pad = skiff.getObjectByName(`${name}Pad`);
      if (pad) pad.visible = gear;
    }

    const glow = thrusterGlow(next, input);
    for (const name of ['ThrusterL', 'ThrusterR', 'ThrusterAft', 'Beacon']) {
      const part = skiff.getObjectByName(name) as THREE.Mesh | undefined;
      const material = part?.material as THREE.MeshStandardMaterial | undefined;
      if (material) {
        material.emissive = material.emissive ?? new THREE.Color();
        material.emissive.setRGB(0.25 * glow, 0.85 * glow, 0.95 * glow);
        material.emissiveIntensity = 0.4 + glow * 2.2;
      }
    }

    // Chase camera: behind the bow, above the deck, looking slightly down.
    const behind = new THREE.Vector3(Math.sin(next.yaw), 0, Math.cos(next.yaw)).multiplyScalar(16);
    const cameraTarget = new THREE.Vector3(next.x + behind.x, next.y + 6.5, next.z + behind.z);
    if (reducedMotion) camera.position.copy(cameraTarget);
    else camera.position.lerp(cameraTarget, Math.min(1, rawDt * 3.2));
    camera.lookAt(next.x, next.y + 1.2, next.z);
  });

  return <primitive object={skiff} />;
}

useGLTF.preload(`${import.meta.env.BASE_URL}world/forum-skiff.glb?v=${FORUM_ASSET_REVISION}`);
