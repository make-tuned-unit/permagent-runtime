// DescentMode — the travel affordance down the Carved Cave (C-nav).
// THE_CAVE_vision_bible.md §2 "depth is time": a VERTICAL rail down the throat that
// STOPS discretely at each derived stratum (never a continuous free-fall). Clones the
// camera/TourMode.tsx spline-ownership pattern and fulfils the downward leg TourMode
// documents as deferred (TourMode.tsx:27-33): THROAT.center → each stratum top → the
// derived CAVE_FLOOR.
//
// Owns the camera while active (descentState mirrors tourState's active-flag). Because
// WorldCamera returns null for non-orbit modes, OrbitControls unmounts in descent → the
// polar clamp is gone automatically; no clamp edit is needed.
//
// Movement: W/S (and the HUD ▲/▼) step to the adjacent stratum stop; ESC / right-click
// ascends to orbit (mirrors WorldCamera.startTransitionToOrbit). reduceMotion: instant
// cuts between stops (same fallback Tour/Turn use).
//
// LAW: zero per-frame allocations — scratch vectors at module scope; stops precomputed
// in useMemo from the derived strata.

import { useEffect, useMemo, useRef } from 'react';
import { useFrame, useThree } from '@react-three/fiber';
import * as THREE from 'three';
import type { CameraMode } from '../types';
import { getReduceMotion } from '../../../styles/tokens';
import { useStrata } from '../areas/strata/strataState';
import { THROAT, caveFloorFor, type StratumDef } from '../areas/strata/strata';
import {
  isDescentActive,
  setDescentActive,
  getDescentStop,
  setDescentStop,
  useDescentStop,
} from './descentState';

// How long a glide between two stops takes (seconds). reduceMotion cuts instantly.
const STEP_DURATION_S = 1.1;
// Radial standoff from the throat axis — the eye sits this far out so the chamber wall
// reads. Just outside the bore so the camera looks across the abyss into the rock.
const EYE_STANDOFF = THROAT.radiusTop + 5.5;

interface Stop {
  pos: THREE.Vector3;
  look: THREE.Vector3;
}

/** Build the discrete rail stops from the derived strata: verge → each chamber → floor. */
function buildStops(strata: StratumDef[]): Stop[] {
  const [cx, , cz] = THROAT.center;
  const stops: Stop[] = [];
  // Stop 0: the verge — hover above the throat lip looking down the shaft.
  const top = strata.length > 0 ? strata[0].top : -2;
  stops.push({
    pos: new THREE.Vector3(cx, top + 3.5, cz - EYE_STANDOFF),
    look: new THREE.Vector3(cx, top - 1, cz),
  });
  // One stop per chamber: eye at the chamber mid-Y, standing off the throat, looking
  // across the abyss into that chamber's wall.
  for (const s of strata) {
    const midY = (s.top + s.floor) / 2;
    stops.push({
      pos: new THREE.Vector3(cx, midY + 1.2, cz - EYE_STANDOFF),
      look: new THREE.Vector3(cx, midY, cz),
    });
  }
  // Final stop: the cave floor — bottom of the dig.
  const floor = caveFloorFor(strata);
  stops.push({
    pos: new THREE.Vector3(cx, floor + 2.5, cz - (EYE_STANDOFF - 1)),
    look: new THREE.Vector3(cx, floor + 0.5, cz),
  });
  return stops;
}

// Scratch — sampled every frame, never reallocated (single camera).
const _fromPos = new THREE.Vector3();
const _toPos = new THREE.Vector3();
const _fromLook = new THREE.Vector3();
const _toLook = new THREE.Vector3();
const _pos = new THREE.Vector3();
const _look = new THREE.Vector3();

function easeInOutCubic(t: number): number {
  return t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2;
}

export function DescentMode({
  cameraMode,
  onAscend,
}: {
  cameraMode: CameraMode;
  onAscend: () => void;
}) {
  const camera = useThree((s) => s.camera);
  const gl = useThree((s) => s.gl);
  const reduceMotion = useMemo(() => getReduceMotion(), []);
  const strata = useStrata();
  const stops = useMemo(() => buildStops(strata), [strata]);
  // Stop index lives in the store so W/S keys AND the HUD ▲/▼ buttons both just call
  // setDescentStop; this component reacts to the change and glides there.
  const targetStop = useDescentStop();

  // Glide animation state between two stops.
  const anim = useRef<{ active: boolean; t: number; from: number; to: number }>({
    active: false,
    t: 0,
    from: 0,
    to: 0,
  });
  // The stop the camera is currently resting at (start of any in-flight glide).
  const restingStop = useRef(0);

  // Snap the camera onto a stop instantly (entry + reduceMotion steps).
  const snapTo = (i: number) => {
    const s = stops[Math.max(0, Math.min(stops.length - 1, i))];
    if (!s) return;
    camera.position.copy(s.pos);
    camera.lookAt(s.look);
  };

  // Entering descent: own the camera, snap to the verge stop.
  useEffect(() => {
    if (cameraMode !== 'descent') return;
    if (!isDescentActive()) setDescentActive(true);
    restingStop.current = getDescentStop();
    anim.current.active = false;
    snapTo(getDescentStop());
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cameraMode, stops]);

  // Leaving descent: release the flag.
  useEffect(() => {
    if (cameraMode !== 'descent' && isDescentActive()) {
      setDescentActive(false);
      anim.current.active = false;
    }
  }, [cameraMode]);

  // React to a target-stop change (from keys or the HUD) — start the glide, or snap
  // instantly under reduceMotion.
  useEffect(() => {
    if (cameraMode !== 'descent') return;
    if (targetStop === restingStop.current && !anim.current.active) return;
    if (reduceMotion) {
      restingStop.current = targetStop;
      anim.current.active = false;
      snapTo(targetStop);
      return;
    }
    anim.current = { active: true, t: 0, from: restingStop.current, to: targetStop };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [targetStop, cameraMode, reduceMotion]);

  // W/S step stops; ESC / right-click ascend to orbit. Only while in descent.
  useEffect(() => {
    if (cameraMode !== 'descent') return;
    const lastStop = () => Math.max(0, stops.length - 1);
    const onKey = (e: KeyboardEvent) => {
      const tgt = e.target as HTMLElement | null;
      if (tgt && (tgt.tagName === 'INPUT' || tgt.tagName === 'TEXTAREA' || tgt.isContentEditable)) {
        return;
      }
      switch (e.key.toLowerCase()) {
        case 's':
        case 'arrowdown':
          setDescentStop(Math.min(lastStop(), getDescentStop() + 1)); // descend
          break;
        case 'w':
        case 'arrowup':
          setDescentStop(Math.max(0, getDescentStop() - 1)); // ascend a stop
          break;
        case 'escape':
          onAscend();
          break;
      }
    };
    const onContext = (e: MouseEvent) => {
      e.preventDefault();
      onAscend();
    };
    window.addEventListener('keydown', onKey);
    gl.domElement.addEventListener('contextmenu', onContext);
    return () => {
      window.removeEventListener('keydown', onKey);
      gl.domElement.removeEventListener('contextmenu', onContext);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cameraMode, stops, gl.domElement, onAscend]);

  useFrame((_, delta) => {
    if (cameraMode !== 'descent') return;
    const a = anim.current;
    if (!a.active) return;
    a.t = Math.min(1, a.t + delta / STEP_DURATION_S);
    const e = easeInOutCubic(a.t);
    const from = stops[a.from];
    const to = stops[a.to];
    if (!from || !to) {
      a.active = false;
      restingStop.current = a.to;
      return;
    }
    _fromPos.copy(from.pos);
    _toPos.copy(to.pos);
    _fromLook.copy(from.look);
    _toLook.copy(to.look);
    _pos.lerpVectors(_fromPos, _toPos, e);
    _look.lerpVectors(_fromLook, _toLook, e);
    camera.position.copy(_pos);
    camera.lookAt(_look);
    if (a.t >= 1) {
      a.active = false;
      restingStop.current = a.to;
    }
  });

  return null;
}
