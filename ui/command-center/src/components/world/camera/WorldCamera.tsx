import { useRef, useEffect, useCallback } from 'react';
import { useThree, useFrame } from '@react-three/fiber';
import { OrbitControls } from '@react-three/drei';
import * as THREE from 'three';
import type { CameraMode, AgentState } from '../types';
import { useTourActive } from './tourState';
import { zoneCenter, type ZoneDef } from '../areas/zones';

interface WorldCameraProps {
  mode: CameraMode;
  selectedAgent: AgentState | null;
  /** When mode === 'zone', the zone the camera travels to (null otherwise). */
  focusZone: ZoneDef | null;
  onModeChange: (mode: CameraMode) => void;
  onMoveAgent: (dx: number, dz: number) => void;
}

const ORBIT_POSITION = new THREE.Vector3(20, 15, 20);
const ORBIT_TARGET = new THREE.Vector3(0, 2, 0);
const AUTO_ROTATE_DELAY = 5000;
const TRANSITION_DURATION = 1.5;

// Third-person offset: behind + above the agent
const TP_OFFSET = new THREE.Vector3(0, 3, 6);
const TP_LOOK_OFFSET = new THREE.Vector3(0, 1.5, 0);

// Zone-travel framing: stand back from the zone's room center toward the hall,
// elevated, looking at the zone. ZONE_BACK pulls the eye toward the rotunda so
// the threshold reads in frame; ZONE_HEIGHT/ZONE_LOOK_Y set the descent angle.
const ZONE_BACK = 13;
const ZONE_HEIGHT = 9;
const ZONE_LOOK_Y = 2.5;

// Scratch vectors — bible §8: zero per-frame allocations in useFrame.
// Single camera instance, so module-level scratch is safe.
const _transitionTarget = new THREE.Vector3();
const _camDir = new THREE.Vector3();
const _camRight = new THREE.Vector3();
const _UP = new THREE.Vector3(0, 1, 0);
const _desiredPos = new THREE.Vector3();
const _desiredTarget = new THREE.Vector3();

function easeInOutCubic(t: number): number {
  return t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2;
}

export function WorldCamera({ mode, selectedAgent, focusZone, onModeChange, onMoveAgent }: WorldCameraProps) {
  const { camera, gl } = useThree();
  // While the tour owns the camera, OrbitControls must unmount (its damped
  // update loop would fight the spline every frame).
  const tourActive = useTourActive();
  const controlsRef = useRef<React.ComponentRef<typeof OrbitControls>>(null);
  const lastInteraction = useRef(Date.now());
  const transitionRef = useRef<{
    active: boolean;
    startTime: number;
    startPos: THREE.Vector3;
    endPos: THREE.Vector3;
    startTarget: THREE.Vector3;
    endTarget: THREE.Vector3;
  } | null>(null);

  // Smoothly interpolated camera position/target for third-person following
  const smoothCamPos = useRef(new THREE.Vector3());
  const smoothCamTarget = useRef(new THREE.Vector3());

  const startTransitionToOrbit = useCallback(() => {
    transitionRef.current = {
      active: true,
      startTime: performance.now() / 1000,
      startPos: camera.position.clone(),
      endPos: ORBIT_POSITION.clone(),
      startTarget: smoothCamTarget.current.clone(),
      endTarget: ORBIT_TARGET.clone(),
    };
    onModeChange('orbit');
  }, [camera, onModeChange]);

  // Track held movement keys for per-frame movement
  const moveKeys = useRef({ forward: false, backward: false, left: false, right: false });

  // ESC, right-click, and movement keys for third-person
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      // ESC returns to the rotunda from any non-orbit owner (follow or zone).
      if (e.key === 'Escape' && mode !== 'orbit') {
        startTransitionToOrbit();
      }
      if (mode === 'third-person') {
        switch (e.key.toLowerCase()) {
          case 'w': case 'arrowup': moveKeys.current.forward = true; break;
          case 's': case 'arrowdown': moveKeys.current.backward = true; break;
          case 'a': case 'arrowleft': moveKeys.current.left = true; break;
          case 'd': case 'arrowright': moveKeys.current.right = true; break;
        }
      }
    };
    const handleKeyUp = (e: KeyboardEvent) => {
      switch (e.key.toLowerCase()) {
        case 'w': case 'arrowup': moveKeys.current.forward = false; break;
        case 's': case 'arrowdown': moveKeys.current.backward = false; break;
        case 'a': case 'arrowleft': moveKeys.current.left = false; break;
        case 'd': case 'arrowright': moveKeys.current.right = false; break;
      }
    };
    const handleContextMenu = (e: MouseEvent) => {
      if (mode === 'third-person') {
        e.preventDefault();
        startTransitionToOrbit();
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    window.addEventListener('keyup', handleKeyUp);
    gl.domElement.addEventListener('contextmenu', handleContextMenu);

    return () => {
      window.removeEventListener('keydown', handleKeyDown);
      window.removeEventListener('keyup', handleKeyUp);
      gl.domElement.removeEventListener('contextmenu', handleContextMenu);
      moveKeys.current = { forward: false, backward: false, left: false, right: false };
    };
  }, [mode, gl.domElement, startTransitionToOrbit]);

  // Start transition to third-person when agent is selected
  useEffect(() => {
    if (mode === 'third-person' && selectedAgent) {
      const agentPos = new THREE.Vector3(selectedAgent.position.x, 0, selectedAgent.position.z);
      const targetPos = agentPos.clone().add(TP_OFFSET);
      const lookAt = agentPos.clone().add(TP_LOOK_OFFSET);

      transitionRef.current = {
        active: true,
        startTime: performance.now() / 1000,
        startPos: camera.position.clone(),
        endPos: targetPos,
        startTarget: ORBIT_TARGET.clone(),
        endTarget: lookAt,
      };

      smoothCamPos.current.copy(targetPos);
      smoothCamTarget.current.copy(lookAt);
    }
  }, [mode, selectedAgent?.id]); // eslint-disable-line react-hooks/exhaustive-deps

  // Travel to a zone when one is clicked. Reuses the SAME transition primitive
  // as agent click-to-zoom — one camera owner at a time (composes with Tour,
  // which gates on orbit, and the puppet-walk/cave-descent rulings: a discrete,
  // mutually-exclusive mode that holds a static framing once it arrives).
  useEffect(() => {
    if (mode === 'zone' && focusZone) {
      const [cx, , cz] = zoneCenter(focusZone);
      const center = new THREE.Vector3(cx, 0, cz);
      const outward = new THREE.Vector3(Math.cos(focusZone.angle), 0, Math.sin(focusZone.angle));
      const endPos = center.clone().addScaledVector(outward, -ZONE_BACK);
      endPos.y = ZONE_HEIGHT;
      const endTarget = center.clone();
      endTarget.y = ZONE_LOOK_Y;

      transitionRef.current = {
        active: true,
        startTime: performance.now() / 1000,
        startPos: camera.position.clone(),
        endPos,
        startTarget: ORBIT_TARGET.clone(),
        endTarget,
      };
      smoothCamPos.current.copy(endPos);
      smoothCamTarget.current.copy(endTarget);
    }
  }, [mode, focusZone?.id]); // eslint-disable-line react-hooks/exhaustive-deps

  useFrame(({ scene }) => {
    // Tour mode owns the camera entirely (see camera/TourMode.tsx).
    if (tourActive) return;

    // Handle camera transition
    if (transitionRef.current?.active) {
      const t = transitionRef.current;
      const elapsed = performance.now() / 1000 - t.startTime;
      const progress = Math.min(elapsed / TRANSITION_DURATION, 1);
      const eased = easeInOutCubic(progress);

      camera.position.lerpVectors(t.startPos, t.endPos, eased);

      _transitionTarget.lerpVectors(t.startTarget, t.endTarget, eased);
      camera.lookAt(_transitionTarget);

      // "Diving in" fog pulse during transition
      if (scene.fog) {
        const fogPulse = Math.sin(progress * Math.PI);
        if (scene.fog instanceof THREE.Fog) {
          scene.fog.near = 30 - fogPulse * 20;
          scene.fog.far = 150 - fogPulse * 80;
        } else if (scene.fog instanceof THREE.FogExp2) {
          scene.fog.density = 0.012 + fogPulse * 0.008;
        }
      }

      if (progress >= 1) {
        transitionRef.current = { ...t, active: false };
        if (scene.fog) {
          if (scene.fog instanceof THREE.Fog) {
            scene.fog.near = 30;
            scene.fog.far = 150;
          } else if (scene.fog instanceof THREE.FogExp2) {
            scene.fog.density = 0.012;
          }
        }
      }
      return;
    }

    // Third-person movement: arrow keys / WASD move the agent
    if (mode === 'third-person' && selectedAgent) {
      const m = moveKeys.current;
      if (m.forward || m.backward || m.left || m.right) {
        // Move relative to camera facing direction (projected to XZ)
        camera.getWorldDirection(_camDir);
        _camDir.y = 0;
        _camDir.normalize();
        _camRight.crossVectors(_camDir, _UP).normalize();

        const speed = 0.15;
        let dx = 0, dz = 0;
        if (m.forward) { dx += _camDir.x * speed; dz += _camDir.z * speed; }
        if (m.backward) { dx -= _camDir.x * speed; dz -= _camDir.z * speed; }
        if (m.left) { dx -= _camRight.x * speed; dz -= _camRight.z * speed; }
        if (m.right) { dx += _camRight.x * speed; dz += _camRight.z * speed; }
        onMoveAgent(dx, dz);
      }
    }

    // Third-person follow: smoothly track the agent
    if (mode === 'third-person' && selectedAgent) {
      _desiredPos
        .set(selectedAgent.position.x, selectedAgent.position.y, selectedAgent.position.z)
        .add(TP_OFFSET);
      _desiredTarget.set(
        selectedAgent.position.x,
        selectedAgent.position.y + TP_LOOK_OFFSET.y,
        selectedAgent.position.z
      );

      // Smooth lerp toward desired position
      smoothCamPos.current.lerp(_desiredPos, 0.05);
      smoothCamTarget.current.lerp(_desiredTarget, 0.08);

      camera.position.copy(smoothCamPos.current);
      camera.lookAt(smoothCamTarget.current);
    }

    // Auto-rotate orbit when idle
    if (mode === 'orbit' && controlsRef.current) {
      const ctrl = controlsRef.current as unknown as { autoRotate: boolean };
      const timeSinceInteraction = Date.now() - lastInteraction.current;
      ctrl.autoRotate = timeSinceInteraction > AUTO_ROTATE_DELAY;
    }
  });

  const handleOrbitInteraction = useCallback(() => {
    lastInteraction.current = Date.now();
  }, []);

  if (mode === 'third-person' || mode === 'zone' || tourActive) {
    return null;
  }

  return (
    <OrbitControls
      ref={controlsRef}
      args={[camera, gl.domElement]}
      minDistance={8}
      maxDistance={50}
      minPolarAngle={0.2}
      maxPolarAngle={Math.PI / 2 - 0.1}
      enableDamping
      dampingFactor={0.05}
      autoRotate
      autoRotateSpeed={0.3}
      onChange={handleOrbitInteraction}
    />
  );
}
