// W4 camera — THE TURN, the day-one framing (THE CAVE bible §2).
//
// "A new user's world opens facing the cave wall … The light casting the shadows
//  comes from the Mouth, behind the camera, not yet visible … the view earns its
//  turn — and the user discovers the Mouth was behind them all along." (§2)
//
// This parks the camera at the day-one pose (TURN_CAMERA from
// atmosphere/ShadowsOnTheWall): low, facing the dark wall (-z), the Mouth and its
// daylight blade behind the lens. It is the establishing frame for a brand-new,
// near-empty Brain. A single tween then "turns" the camera 180° back to the
// overview so the user discovers the Mouth — the §2 reveal.
//
// Bindings:
//   • Shift+T (orbit mode, not while typing) → snap to the day-one wall frame.
//   • Any user input → release back to orbit (same cancel contract as TourMode).
//   • reduceMotion → the turn is an instant cut, not a sweep (bible §8).
//   • DEV: window.__worldTurn.face() / .turn() for evidence capture.
//
// LAW: bible §8 — zero per-frame allocations (module-scope scratch vectors).

import { useEffect, useMemo, useRef } from 'react';
import { useFrame, useThree } from '@react-three/fiber';
import * as THREE from 'three';
import type { CameraMode } from '../types';
import { getReduceMotion } from '../../../styles/tokens';
import { isTourActive, setTourActive } from './tourState';
import { TURN_CAMERA } from '../atmosphere/ShadowsOnTheWall';

const WALL_POS = new THREE.Vector3(...TURN_CAMERA.position);
const WALL_LOOK = new THREE.Vector3(...TURN_CAMERA.lookAt);
// The "turn": pull back and up to an overview that now includes the Mouth (which
// was behind the camera). Matches the default orbit establishing framing.
const OVERVIEW_POS = new THREE.Vector3(20, 14, 20);
const OVERVIEW_LOOK = new THREE.Vector3(0, 4, 0);

const TURN_DURATION_S = 4;

const _pos = new THREE.Vector3();
const _look = new THREE.Vector3();

function easeInOutCubic(t: number): number {
  return t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2;
}

// Reuse the tour's activity flag so WorldCamera unmounts OrbitControls while the
// turn owns the camera (no fighting the damped orbit loop).
declare global {
  interface Window {
    __worldTurn?: { face: () => void; turn: () => void };
  }
}

export function TurnFraming({ cameraMode }: { cameraMode: CameraMode }) {
  const camera = useThree((s) => s.camera);
  const reduceMotion = useMemo(() => getReduceMotion(), []);
  // mode: 'idle' = inactive, 'facing' = parked at the wall, 'turning' = sweeping out.
  const mode = useRef<'idle' | 'facing' | 'turning'>('idle');
  const progress = useRef(0);

  const face = () => {
    mode.current = 'facing';
    setTourActive(true);
    camera.position.copy(WALL_POS);
    camera.lookAt(WALL_LOOK);
  };

  const turn = () => {
    if (reduceMotion) {
      camera.position.copy(OVERVIEW_POS);
      camera.lookAt(OVERVIEW_LOOK);
      mode.current = 'idle';
      setTourActive(false);
      return;
    }
    progress.current = 0;
    mode.current = 'turning';
  };

  // Shift+T starts the day-one frame; T (handled by TourMode) is the full tour.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (isTourActive() && mode.current === 'idle') return; // TourMode owns it
      if (e.key.toLowerCase() !== 't' || !e.shiftKey) return;
      if (cameraMode !== 'orbit') return;
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return;
      if (mode.current === 'idle') face();
      else if (mode.current === 'facing') turn();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cameraMode, camera, reduceMotion]);

  // Any non-(Shift+T) input cancels and releases to orbit.
  useEffect(() => {
    const cancel = (e: Event) => {
      if (mode.current === 'idle') return;
      if (e instanceof KeyboardEvent && e.key.toLowerCase() === 't' && e.shiftKey) return;
      mode.current = 'idle';
      setTourActive(false);
    };
    window.addEventListener('pointerdown', cancel, true);
    window.addEventListener('wheel', cancel, true);
    window.addEventListener('keydown', cancel, true);
    window.addEventListener('touchstart', cancel, true);
    return () => {
      window.removeEventListener('pointerdown', cancel, true);
      window.removeEventListener('wheel', cancel, true);
      window.removeEventListener('keydown', cancel, true);
      window.removeEventListener('touchstart', cancel, true);
    };
  }, []);

  useEffect(() => {
    if (cameraMode !== 'orbit' && mode.current !== 'idle') {
      mode.current = 'idle';
      setTourActive(false);
    }
  }, [cameraMode]);

  useEffect(() => {
    if (!import.meta.env.DEV) return;
    window.__worldTurn = { face, turn };
    return () => {
      delete window.__worldTurn;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useFrame((_, delta) => {
    if (mode.current === 'turning') {
      progress.current = Math.min(1, progress.current + delta / TURN_DURATION_S);
      const u = easeInOutCubic(progress.current);
      _pos.lerpVectors(WALL_POS, OVERVIEW_POS, u);
      _look.lerpVectors(WALL_LOOK, OVERVIEW_LOOK, u);
      camera.position.copy(_pos);
      camera.lookAt(_look);
      if (progress.current >= 1) {
        mode.current = 'idle';
        setTourActive(false);
      }
    } else if (mode.current === 'facing') {
      // Hold the wall frame steady.
      camera.position.copy(WALL_POS);
      camera.lookAt(WALL_LOOK);
    }
  });

  return null;
}
