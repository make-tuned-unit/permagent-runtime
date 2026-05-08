import { useRef, useEffect, useCallback } from 'react';
import { useThree, useFrame } from '@react-three/fiber';
import { OrbitControls } from '@react-three/drei';
import * as THREE from 'three';
import type { CameraMode, AgentState } from './types';

interface WorldCameraProps {
  mode: CameraMode;
  selectedAgent: AgentState | null;
  onModeChange: (mode: CameraMode) => void;
}

const ORBIT_POSITION = new THREE.Vector3(20, 15, 20);
const ORBIT_TARGET = new THREE.Vector3(0, 2, 0);
const AUTO_ROTATE_DELAY = 5000;
const TRANSITION_DURATION = 1.5;

function easeInOutCubic(t: number): number {
  return t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2;
}

export function WorldCamera({ mode, selectedAgent, onModeChange }: WorldCameraProps) {
  const { camera, gl } = useThree();
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
  const fpMovement = useRef({ forward: false, backward: false, left: false, right: false });
  const fpRotation = useRef({ yaw: 0, pitch: 0 });
  const isPointerLocked = useRef(false);

  // Handle ESC and right-click to return to orbit
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && mode === 'first-person') {
        startTransitionToOrbit();
      }
      if (mode === 'first-person') {
        switch (e.key.toLowerCase()) {
          case 'w': case 'arrowup': fpMovement.current.forward = true; break;
          case 's': case 'arrowdown': fpMovement.current.backward = true; break;
          case 'a': case 'arrowleft': fpMovement.current.left = true; break;
          case 'd': case 'arrowright': fpMovement.current.right = true; break;
        }
      }
    };
    const handleKeyUp = (e: KeyboardEvent) => {
      switch (e.key.toLowerCase()) {
        case 'w': case 'arrowup': fpMovement.current.forward = false; break;
        case 's': case 'arrowdown': fpMovement.current.backward = false; break;
        case 'a': case 'arrowleft': fpMovement.current.left = false; break;
        case 'd': case 'arrowright': fpMovement.current.right = false; break;
      }
    };
    const handleContextMenu = (e: MouseEvent) => {
      if (mode === 'first-person') {
        e.preventDefault();
        startTransitionToOrbit();
      }
    };
    const handleMouseMove = (e: MouseEvent) => {
      if (mode === 'first-person' && isPointerLocked.current) {
        fpRotation.current.yaw -= e.movementX * 0.002;
        fpRotation.current.pitch -= e.movementY * 0.002;
        fpRotation.current.pitch = Math.max(-Math.PI / 3, Math.min(Math.PI / 3, fpRotation.current.pitch));
      }
    };
    const handlePointerLockChange = () => {
      isPointerLocked.current = document.pointerLockElement === gl.domElement;
      if (!isPointerLocked.current && mode === 'first-person' && !transitionRef.current?.active) {
        startTransitionToOrbit();
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    window.addEventListener('keyup', handleKeyUp);
    gl.domElement.addEventListener('contextmenu', handleContextMenu);
    document.addEventListener('mousemove', handleMouseMove);
    document.addEventListener('pointerlockchange', handlePointerLockChange);

    return () => {
      window.removeEventListener('keydown', handleKeyDown);
      window.removeEventListener('keyup', handleKeyUp);
      gl.domElement.removeEventListener('contextmenu', handleContextMenu);
      document.removeEventListener('mousemove', handleMouseMove);
      document.removeEventListener('pointerlockchange', handlePointerLockChange);
    };
  }, [mode, gl.domElement]);

  const startTransitionToOrbit = useCallback(() => {
    if (document.pointerLockElement) {
      document.exitPointerLock();
    }
    isPointerLocked.current = false;
    fpMovement.current = { forward: false, backward: false, left: false, right: false };

    transitionRef.current = {
      active: true,
      startTime: performance.now() / 1000,
      startPos: camera.position.clone(),
      endPos: ORBIT_POSITION.clone(),
      startTarget: new THREE.Vector3(0, 1.7, 0).add(
        new THREE.Vector3(0, 0, -1).applyEuler(camera.rotation)
      ).add(camera.position),
      endTarget: ORBIT_TARGET.clone(),
    };
    onModeChange('orbit');
  }, [camera, onModeChange]);

  // Start transition to first-person when agent is selected
  useEffect(() => {
    if (mode === 'first-person' && selectedAgent) {
      const agentPos = new THREE.Vector3(selectedAgent.position.x, 1.7, selectedAgent.position.z);
      const behindOffset = new THREE.Vector3(0, 0, 0.5);

      const eyePos = agentPos.clone().add(behindOffset);

      transitionRef.current = {
        active: true,
        startTime: performance.now() / 1000,
        startPos: camera.position.clone(),
        endPos: eyePos,
        startTarget: controlsRef.current
          ? new THREE.Vector3().setFromSpherical(
              new THREE.Spherical().setFromVector3(
                new THREE.Vector3(0, 0, -1).applyQuaternion(camera.quaternion)
              )
            ).add(camera.position)
          : ORBIT_TARGET.clone(),
        endTarget: new THREE.Vector3(
          selectedAgent.position.x,
          1.7,
          selectedAgent.position.z - 2
        ),
      };

      fpRotation.current.yaw = Math.atan2(
        -(selectedAgent.position.z - 2 - selectedAgent.position.z),
        0
      );
      fpRotation.current.pitch = 0;
    }
  }, [mode, selectedAgent, camera]);

  // Animation frame for transitions and first-person movement
  useFrame(({ scene }, delta) => {
    // Handle camera transition
    if (transitionRef.current?.active) {
      const t = transitionRef.current;
      const elapsed = performance.now() / 1000 - t.startTime;
      const progress = Math.min(elapsed / TRANSITION_DURATION, 1);
      const eased = easeInOutCubic(progress);

      camera.position.lerpVectors(t.startPos, t.endPos, eased);

      const currentTarget = new THREE.Vector3().lerpVectors(t.startTarget, t.endTarget, eased);
      camera.lookAt(currentTarget);

      // "Diving in" fog pulse during transition — tighten fog at midpoint, release at end
      if (scene.fog && scene.fog instanceof THREE.Fog) {
        const fogPulse = Math.sin(progress * Math.PI); // peaks at 0.5
        scene.fog.near = 30 - fogPulse * 20;
        scene.fog.far = 150 - fogPulse * 80;
      }

      if (progress >= 1) {
        transitionRef.current = { ...t, active: false };
        // Restore fog to defaults
        if (scene.fog && scene.fog instanceof THREE.Fog) {
          scene.fog.near = 30;
          scene.fog.far = 150;
        }
        if (mode === 'first-person') {
          gl.domElement.requestPointerLock();
        }
      }
      return;
    }

    // First-person controls
    if (mode === 'first-person' && selectedAgent) {
      const speed = 3 * delta;
      const euler = new THREE.Euler(fpRotation.current.pitch, fpRotation.current.yaw, 0, 'YXZ');
      const forward = new THREE.Vector3(0, 0, -1).applyEuler(euler);
      const right = new THREE.Vector3(1, 0, 0).applyEuler(euler);

      forward.y = 0;
      forward.normalize();
      right.y = 0;
      right.normalize();

      if (fpMovement.current.forward) camera.position.addScaledVector(forward, speed);
      if (fpMovement.current.backward) camera.position.addScaledVector(forward, -speed);
      if (fpMovement.current.left) camera.position.addScaledVector(right, -speed);
      if (fpMovement.current.right) camera.position.addScaledVector(right, speed);

      camera.position.y = 1.7;
      camera.rotation.set(fpRotation.current.pitch, fpRotation.current.yaw, 0, 'YXZ');
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

  if (mode === 'first-person') {
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
