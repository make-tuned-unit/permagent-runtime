// W4 camera — TOUR MODE (bible §7).
// Press T (orbit mode) to start a slow drift along a precomputed spline:
// establishing shot → into the hall → past each zone threshold (Workbench E,
// Brain S, Automate W, Lab N — §3 layout) → the Antechamber NW → closing rise.
// ANY user input (pointer, wheel, key, touch) cancels instantly.
// reduceMotion (tokens.ts): static establishing shot instead of a drift.
// LAW: zero per-frame allocations — curves are precomputed at module scope
// and sampled into scratch vectors.

import { useEffect, useMemo, useRef } from 'react';
import { useFrame, useThree } from '@react-three/fiber';
import * as THREE from 'three';
import type { CameraMode } from '../types';
import { getReduceMotion } from '../../../styles/tokens';
import { isTourActive, setTourActive } from './tourState';

const TOUR_DURATION_S = 60;

// Establishing shot (matches the default orbit camera framing).
const ESTABLISHING_POS = new THREE.Vector3(20, 14, 20);
const ESTABLISHING_LOOK = new THREE.Vector3(0, 4, 0);

// Zone thresholds: Workbench x=+24 (E), Brain z=+24 (S), Automate x=-24 (W),
// Lab z=-24 (N), Antechamber (-19, -19) NW diagonal. Camera path stays inside
// the colonnade (r ≈ 8.5, eye height 2.6); the look path leads down each
// threshold sightline. The spline is the rotunda-level loop around the colonnade.
const POSITION_CURVE = new THREE.CatmullRomCurve3(
  [
    ESTABLISHING_POS.clone(),
    new THREE.Vector3(11, 6, 13), //  descend toward the hall
    new THREE.Vector3(7, 3, 7), //    enter at eye level
    new THREE.Vector3(8.5, 2.6, 0), //   E — Workbench threshold
    new THREE.Vector3(6, 2.6, 6),
    new THREE.Vector3(0, 2.6, 8.5), //   S — Brain Archive threshold
    new THREE.Vector3(-6, 2.6, 6),
    new THREE.Vector3(-8.5, 2.6, 0), //  W — Automate threshold
    new THREE.Vector3(-6, 2.6, -6),
    new THREE.Vector3(0, 2.6, -8.5), //  N — Lab threshold
    new THREE.Vector3(-6.5, 2.6, -6.5), // NW — Antechamber approach
    new THREE.Vector3(-9, 4, -9), //     push toward the gate
    new THREE.Vector3(-12, 8, -12), //   closing rise back to overview
  ],
  false,
  'catmullrom',
  0.5
);

const LOOK_CURVE = new THREE.CatmullRomCurve3(
  [
    ESTABLISHING_LOOK.clone(),
    new THREE.Vector3(0, 3.5, 0),
    new THREE.Vector3(12, 3, 0),
    new THREE.Vector3(24, 3, 0), //    Workbench
    new THREE.Vector3(12, 3, 12),
    new THREE.Vector3(0, 3, 24), //    Brain Archive
    new THREE.Vector3(-12, 3, 12),
    new THREE.Vector3(-24, 3, 0), //   Automate
    new THREE.Vector3(-12, 3, -12),
    new THREE.Vector3(0, 3, -24), //   Lab
    new THREE.Vector3(-19, 4, -19), // Antechamber
    new THREE.Vector3(-19, 3, -19),
    new THREE.Vector3(0, 4, 0),
  ],
  false,
  'catmullrom',
  0.5
);

// Scratch vectors — sampled into every frame, never reallocated.
const _pos = new THREE.Vector3();
const _look = new THREE.Vector3();

function easeInOutCubic(t: number): number {
  return t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2;
}

export function TourMode({ cameraMode }: { cameraMode: CameraMode }) {
  const camera = useThree((s) => s.camera);
  const reduceMotion = useMemo(() => getReduceMotion(), []);
  const progress = useRef(0);

  // Start: T key, orbit mode only, never while typing in an input.
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (isTourActive()) return; // cancel path handles keys during the tour
      if (e.key.toLowerCase() !== 't') return;
      if (cameraMode !== 'orbit') return;
      const target = e.target as HTMLElement | null;
      if (target && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable)) {
        return;
      }
      progress.current = 0;
      setTourActive(true);
      if (reduceMotion) {
        // Static establishing shot — no drift. Stays until any input cancels.
        camera.position.copy(ESTABLISHING_POS);
        camera.lookAt(ESTABLISHING_LOOK);
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [cameraMode, camera, reduceMotion]);

  // Cancel on ANY user input while active. Listeners are registered after the
  // starting keydown has finished dispatching, so the trigger never
  // self-cancels.
  useEffect(() => {
    const cancel = () => {
      if (isTourActive()) setTourActive(false);
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

  // Leaving orbit mode (agent selected → third-person) also ends the tour.
  useEffect(() => {
    if (cameraMode !== 'orbit' && isTourActive()) setTourActive(false);
  }, [cameraMode]);

  useFrame((_, delta) => {
    if (!isTourActive()) return;
    if (reduceMotion) return; // static establishing shot

    progress.current = Math.min(1, progress.current + delta / TOUR_DURATION_S);
    const u = easeInOutCubic(progress.current);
    POSITION_CURVE.getPointAt(u, _pos);
    LOOK_CURVE.getPointAt(u, _look);
    camera.position.copy(_pos);
    camera.lookAt(_look);

    if (progress.current >= 1) setTourActive(false);
  });

  return null;
}
