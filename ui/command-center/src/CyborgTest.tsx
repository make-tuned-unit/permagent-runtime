import { Suspense, useState, type CSSProperties } from 'react';
import { Canvas } from '@react-three/fiber';
import { OrbitControls } from '@react-three/drei';
import * as THREE from 'three';
import { CyborgCharacterModel } from './components/world/agents/CyborgCharacter';
import { AGENT_TRIM } from './components/world/shared/palette';
import { Button } from './components/common/Button';
import { radius } from './styles/tokens';
import { useTheme } from './styles/useTheme';

// Trim colors come from the world palette SOT (Henry's old neon-cyan preset
// was stale — his trim is warm white-gold since issue #87 resolved).
const PRESETS = [
  { label: 'Librarian', trimColor: AGENT_TRIM.librarian, weathering: 0.4, crown: false },
  { label: 'Orchestrator', trimColor: AGENT_TRIM.henry, weathering: 0, crown: true },
  { label: 'Aria', trimColor: AGENT_TRIM.aria, weathering: 0, crown: false },
  { label: 'Felix', trimColor: AGENT_TRIM.felix, weathering: 0, crown: false },
  { label: 'Nova', trimColor: AGENT_TRIM.nova, weathering: 0, crown: false },
];

// URL param ?cam=back for rear view screenshot
const camParam = new URLSearchParams(window.location.search).get('cam');
const CAM_FRONT: [number, number, number] = [3, 2.5, 4];
const CAM_BACK: [number, number, number] = [-3, 2.5, -4];
const initialCam = camParam === 'back' ? CAM_BACK : CAM_FRONT;

export default function CyborgTest() {
  // This harness paints against a fixed studio backdrop, not the app theme, so
  // every visible colour below stays hard-coded; `colors` is here only because
  // the button primitive's contract asks for it.
  const { colors } = useTheme();
  const [preset, setPreset] = useState(0);
  const current = PRESETS[preset];

  return (
    <div style={{ width: '100vw', height: '100vh', background: '#18181B', position: 'relative' }}>
      <Canvas
        shadows
        camera={{ position: initialCam, fov: 40, near: 0.1, far: 100 }}
        gl={{
          antialias: true,
          toneMapping: THREE.ACESFilmicToneMapping,
          toneMappingExposure: 1.3,
          outputColorSpace: THREE.SRGBColorSpace,
        }}
      >
        <Suspense fallback={null}>
          {/* Lighting — matches A1 lighting direction */}
          <ambientLight intensity={0.08} color="#B8C4D8" />
          <directionalLight
            position={[4, 8, 3]}
            intensity={1.6}
            color="#FFF0D4"
            castShadow
            shadow-mapSize-width={1024}
            shadow-mapSize-height={1024}
          />
          <directionalLight position={[-3, 6, -2]} intensity={0.25} color="#8EC8E8" />

          {/* Ground plane for shadow reference */}
          <mesh rotation-x={-Math.PI / 2} position-y={-0.01} receiveShadow>
            <planeGeometry args={[8, 8]} />
            <meshStandardMaterial color="#1A1D25" roughness={0.8} />
          </mesh>

          {/* The character */}
          <CyborgCharacterModel
            trimColor={current.trimColor}
            weathering={current.weathering}
            showCrown={current.crown}
          />

          <OrbitControls
            target={[0, 1.2, 0]}
            minDistance={2}
            maxDistance={12}
            enableDamping
          />
        </Suspense>
      </Canvas>

      {/* Preset switcher */}
      <div style={{
        position: 'absolute', top: 16, left: 16,
        display: 'flex', gap: 8, flexWrap: 'wrap',
      }}>
        {PRESETS.map((p, i) => (
          <Button
            key={p.label}
            colors={colors}
            onClick={() => setPreset(i)}
            style={{
              '--pa-btn-bg': i === preset ? `${p.trimColor}22` : '#222',
              '--pa-btn-fg': i === preset ? p.trimColor : '#888',
              '--pa-btn-border': i === preset ? p.trimColor : '#444',
              '--pa-btn-bg-hover': `${p.trimColor}22`,
              '--pa-btn-fg-hover': p.trimColor,
              '--pa-btn-border-hover': p.trimColor,
              '--pa-btn-bg-active': `${p.trimColor}33`,
              '--pa-btn-pad': '6px 14px',
              '--pa-btn-radius': `${radius.sm}px`,
              fontFamily: 'monospace',
              fontSize: 13,
            } as CSSProperties}
          >
            {p.label}
          </Button>
        ))}
      </div>

      {/* Info */}
      <div style={{
        position: 'absolute', bottom: 16, left: 16,
        color: '#666', fontFamily: 'monospace', fontSize: 12,
      }}>
        B1 Cyborg Character Test — orbit with mouse, switch presets above
      </div>
    </div>
  );
}
