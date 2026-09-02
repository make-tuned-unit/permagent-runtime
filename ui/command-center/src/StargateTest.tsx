import { Suspense } from 'react';
import { Canvas } from '@react-three/fiber';
import { OrbitControls } from '@react-three/drei';
import * as THREE from 'three';
import { StargatePortal } from './components/world/areas/antechamber/Stargate';
import { textSize } from './styles/tokens';

export default function StargateTest() {
  return (
    <div style={{ width: '100vw', height: '100vh', background: '#0A0E1A' }}>
      <Canvas
        camera={{ position: [8, 5, 10], fov: 45, near: 0.1, far: 100 }}
        gl={{
          antialias: true,
          toneMapping: THREE.ACESFilmicToneMapping,
          toneMappingExposure: 1.3,
          outputColorSpace: THREE.SRGBColorSpace,
        }}
      >
        <Suspense fallback={null}>
          <ambientLight intensity={0.08} color="#B8C4D8" />
          <directionalLight position={[6, 8, 4]} intensity={1.2} color="#FFF0D4" />
          <directionalLight position={[-4, 6, -3]} intensity={0.2} color="#8EC8E8" />

          {/* Ground plane */}
          <mesh rotation-x={-Math.PI / 2} position-y={-0.01}>
            <planeGeometry args={[20, 20]} />
            <meshStandardMaterial color="#1A1D25" roughness={0.8} />
          </mesh>

          <StargatePortal />

          <OrbitControls target={[0, 3.5, 0]} minDistance={4} maxDistance={20} enableDamping />
          <fogExp2 attach="fog" args={['#0A0E1A', 0.015]} />
        </Suspense>
      </Canvas>

      <div style={{
        position: 'absolute', bottom: 16, left: 16,
        color: '#666', fontFamily: 'monospace', fontSize: textSize.caption,
      }}>
        Stargate Portal Test — orbit with mouse
      </div>
    </div>
  );
}
