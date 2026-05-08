import { useRef, useMemo } from 'react';
import { useFrame } from '@react-three/fiber';
import { Html } from '@react-three/drei';
import * as THREE from 'three';
import type { AgentState } from './types';
import { COLORS } from './constants';

interface CharacterProps {
  agent: AgentState;
  isHovered: boolean;
  isSelected: boolean;
  onPointerOver: () => void;
  onPointerOut: () => void;
  onClick: () => void;
}

// Crown for Henry
function Crown() {
  const ref = useRef<THREE.Group>(null);

  useFrame(() => {
    if (ref.current) {
      ref.current.position.y = 0.1 * Math.sin(performance.now() * 0.003);
    }
  });

  return (
    <group ref={ref} position-y={2.6}>
      {/* Crown base ring */}
      <mesh>
        <torusGeometry args={[0.25, 0.04, 8, 5]} />
        <meshStandardMaterial color="#FFD700" roughness={0.2} metalness={0.8} emissive="#FFD700" emissiveIntensity={0.3} />
      </mesh>
      {/* Crown points */}
      {Array.from({ length: 5 }, (_, i) => {
        const angle = (i / 5) * Math.PI * 2;
        return (
          <mesh key={i} position={[Math.cos(angle) * 0.25, 0.15, Math.sin(angle) * 0.25]}>
            <coneGeometry args={[0.06, 0.2, 4]} />
            <meshStandardMaterial color="#FFD700" roughness={0.2} metalness={0.8} emissive="#FFD700" emissiveIntensity={0.3} />
          </mesh>
        );
      })}
      {/* Gem accents (2 pink gems) */}
      <mesh position={[0.25, 0.05, 0]}>
        <sphereGeometry args={[0.04, 8, 8]} />
        <meshStandardMaterial color="#FF69B4" emissive="#FF69B4" emissiveIntensity={0.5} />
      </mesh>
      <mesh position={[-0.25, 0.05, 0]}>
        <sphereGeometry args={[0.04, 8, 8]} />
        <meshStandardMaterial color="#FF69B4" emissive="#FF69B4" emissiveIntensity={0.5} />
      </mesh>
      {/* Ambient glow */}
      <pointLight color={COLORS.neonAmber} intensity={0.5} distance={3} />
    </group>
  );
}

// Stylized humanoid from primitives: capsule body, sphere head, cylinder limbs
// Marble-skin with cyan circuit lines, toga with colored trim
function CharacterModel({ agent }: { agent: AgentState }) {
  const groupRef = useRef<THREE.Group>(null);
  // Idle sway animation
  useFrame(() => {
    if (groupRef.current) {
      const sway = Math.sin(performance.now() * 0.002) * 0.02;
      groupRef.current.rotation.z = sway;
    }
  });

  const marbleMat = useMemo(
    () =>
      new THREE.MeshStandardMaterial({
        color: COLORS.primaryMarble,
        roughness: 0.4,
        metalness: 0.05,
      }),
    []
  );

  const circuitMat = useMemo(
    () =>
      new THREE.MeshBasicMaterial({
        color: COLORS.neonCyan,
        transparent: true,
        opacity: 0.6,
      }),
    []
  );

  const togaMat = useMemo(
    () =>
      new THREE.MeshStandardMaterial({
        color: '#F5F0E8',
        roughness: 0.7,
        metalness: 0,
      }),
    []
  );

  const trimMat = useMemo(
    () =>
      new THREE.MeshBasicMaterial({
        color: agent.togaTrimColor,
        transparent: true,
        opacity: 0.8,
      }),
    [agent.togaTrimColor]
  );

  return (
    <group ref={groupRef}>
      {/* Head */}
      <mesh position-y={2.1} material={marbleMat} castShadow>
        <sphereGeometry args={[0.3, 16, 16]} />
      </mesh>
      {/* Head circuit line */}
      <mesh position={[0.15, 2.15, 0.2]}>
        <cylinderGeometry args={[0.02, 0.02, 0.2, 4]} />
        <primitive object={circuitMat} attach="material" />
      </mesh>

      {/* Neck */}
      <mesh position-y={1.75} material={marbleMat}>
        <cylinderGeometry args={[0.1, 0.12, 0.15, 8]} />
      </mesh>

      {/* Torso (toga-wrapped) */}
      <mesh position-y={1.2} material={togaMat} castShadow>
        <cylinderGeometry args={[0.3, 0.35, 1, 8]} />
      </mesh>
      {/* Toga trim at neckline */}
      <mesh position-y={1.7}>
        <torusGeometry args={[0.3, 0.03, 8, 16]} />
        <primitive object={trimMat} attach="material" />
      </mesh>
      {/* Toga trim at hem */}
      <mesh position-y={0.7}>
        <torusGeometry args={[0.35, 0.03, 8, 16]} />
        <primitive object={trimMat} attach="material" />
      </mesh>

      {/* Circuit lines on torso */}
      <mesh position={[0.2, 1.2, 0.25]}>
        <cylinderGeometry args={[0.015, 0.015, 0.8, 4]} />
        <primitive object={circuitMat} attach="material" />
      </mesh>
      <mesh position={[-0.15, 1.3, 0.28]}>
        <cylinderGeometry args={[0.015, 0.015, 0.5, 4]} />
        <primitive object={circuitMat} attach="material" />
      </mesh>

      {/* Arms */}
      <mesh position={[0.45, 1.3, 0]} rotation-z={0.15} material={marbleMat} castShadow>
        <cylinderGeometry args={[0.08, 0.07, 0.7, 8]} />
      </mesh>
      <mesh position={[-0.45, 1.3, 0]} rotation-z={-0.15} material={marbleMat} castShadow>
        <cylinderGeometry args={[0.08, 0.07, 0.7, 8]} />
      </mesh>
      {/* Arm circuit veins */}
      <mesh position={[0.48, 1.3, 0.06]} rotation-z={0.15}>
        <cylinderGeometry args={[0.015, 0.015, 0.6, 4]} />
        <primitive object={circuitMat} attach="material" />
      </mesh>
      <mesh position={[-0.48, 1.3, 0.06]} rotation-z={-0.15}>
        <cylinderGeometry args={[0.015, 0.015, 0.6, 4]} />
        <primitive object={circuitMat} attach="material" />
      </mesh>

      {/* Legs */}
      <mesh position={[0.15, 0.35, 0]} material={marbleMat} castShadow>
        <cylinderGeometry args={[0.1, 0.08, 0.7, 8]} />
      </mesh>
      <mesh position={[-0.15, 0.35, 0]} material={marbleMat} castShadow>
        <cylinderGeometry args={[0.1, 0.08, 0.7, 8]} />
      </mesh>

      {/* Crown for Henry */}
      {agent.isHenry && <Crown />}
    </group>
  );
}

export function AgentCharacter({ agent, isHovered, onPointerOver, onPointerOut, onClick }: Omit<CharacterProps, 'isSelected'>) {
  const rotationRef = useRef<THREE.Group>(null);
  const bobRef = useRef<THREE.Group>(null);
  const prevPos = useRef({ x: agent.position.x, z: agent.position.z });

  // Smoothly rotate to face movement direction + walking bob
  useFrame(() => {
    if (!rotationRef.current) return;

    const dx = agent.position.x - prevPos.current.x;
    const dz = agent.position.z - prevPos.current.z;
    prevPos.current = { x: agent.position.x, z: agent.position.z };

    if (Math.abs(dx) > 0.001 || Math.abs(dz) > 0.001) {
      const targetAngle = Math.atan2(dx, dz);
      const current = rotationRef.current.rotation.y;
      let diff = targetAngle - current;
      while (diff > Math.PI) diff -= Math.PI * 2;
      while (diff < -Math.PI) diff += Math.PI * 2;
      rotationRef.current.rotation.y += diff * 0.1;
    }

    // Walking bob on inner group
    if (bobRef.current) {
      if (agent.activity === 'walking') {
        bobRef.current.position.y = 0.05 * Math.abs(Math.sin(performance.now() * 0.008));
      } else {
        bobRef.current.position.y = 0;
      }
    }
  });

  return (
    <group
      position={[agent.position.x, agent.position.y, agent.position.z]}
      onPointerOver={(e) => { e.stopPropagation(); onPointerOver(); }}
      onPointerOut={(e) => { e.stopPropagation(); onPointerOut(); }}
      onClick={(e) => { e.stopPropagation(); onClick(); }}
    >
      <group ref={rotationRef}>
        <group ref={bobRef}>
      <CharacterModel agent={agent} />

      {/* Hover outline effect — cyan glow ring at feet */}
      {isHovered && (
        <mesh rotation-x={-Math.PI / 2} position-y={0.02}>
          <ringGeometry args={[0.8, 1.0, 32]} />
          <meshBasicMaterial color={COLORS.neonCyan} transparent opacity={0.6} />
        </mesh>
      )}

      {/* Name tooltip on hover */}
      {isHovered && (
        <Html position={[0, 3, 0]} center distanceFactor={15} style={{ pointerEvents: 'none' }}>
          <div
            style={{
              background: 'rgba(10, 14, 26, 0.85)',
              color: COLORS.neonCyan,
              padding: '4px 12px',
              borderRadius: '6px',
              fontSize: '13px',
              fontFamily: 'monospace',
              border: `1px solid ${COLORS.neonCyan}40`,
              whiteSpace: 'nowrap',
              backdropFilter: 'blur(4px)',
            }}
          >
            {agent.name}
            {agent.isHenry && ' (Orchestrator)'}
            {agent.id === 'librarian' && ' (The Brain)'}
          </div>
        </Html>
      )}
        </group>
      </group>
    </group>
  );
}

interface WorldCharactersProps {
  agents: AgentState[];
  hoveredAgent: string | null;
  onHoverAgent: (id: string | null) => void;
  onSelectAgent: (id: string) => void;
}

export function WorldCharacters({ agents, hoveredAgent, onHoverAgent, onSelectAgent }: WorldCharactersProps) {
  return (
    <group>
      {agents.map((agent) => (
        <AgentCharacter
          key={agent.id}
          agent={agent}
          isHovered={hoveredAgent === agent.id}
          onPointerOver={() => onHoverAgent(agent.id)}
          onPointerOut={() => onHoverAgent(null)}
          onClick={() => onSelectAgent(agent.id)}
        />
      ))}
    </group>
  );
}
