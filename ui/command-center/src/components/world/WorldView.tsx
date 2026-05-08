import { Suspense, useState, useCallback, useEffect } from 'react';
import { Canvas } from '@react-three/fiber';
import * as THREE from 'three';
import type { CameraMode } from './types';
import { COLORS, STATIONS } from './constants';
import { useAgentStates } from './useAgentStates';
import { WorldSceneContent } from './WorldScene';
import { WorldCharacters } from './WorldCharacters';
import { WorldCamera } from './WorldCamera';
import { WorldPostProcessing } from './WorldPostProcessing';
import { WorldHUD } from './WorldHUD';

function LoadingShimmer() {
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        width: '100%',
        height: '100%',
        background: COLORS.deepVoid,
        fontFamily: 'monospace',
        color: COLORS.neonCyan,
        fontSize: 14,
      }}
    >
      <div style={{ textAlign: 'center' }}>
        <div
          style={{
            width: 40,
            height: 40,
            border: `2px solid ${COLORS.neonCyan}30`,
            borderTop: `2px solid ${COLORS.neonCyan}`,
            borderRadius: '50%',
            animation: 'spin 1s linear infinite',
            margin: '0 auto 12px',
          }}
        />
        Initializing Lab...
        <style>{`@keyframes spin { to { transform: rotate(360deg); } }`}</style>
      </div>
    </div>
  );
}

function SceneContent({
  cameraMode,
  selectedAgentId,
  onModeChange,
  onHoverAgent,
  onSelectAgent,
  hoveredAgent,
  onHoverStation,
  onClickStation,
}: {
  cameraMode: CameraMode;
  selectedAgentId: string | null;
  onModeChange: (mode: CameraMode) => void;
  onHoverAgent: (id: string | null) => void;
  onSelectAgent: (id: string) => void;
  hoveredAgent: string | null;
  onHoverStation: (id: string | null) => void;
  onClickStation: (id: string) => void;
}) {
  const { agents } = useAgentStates();
  const selectedAgent = agents.find((a) => a.id === selectedAgentId) ?? null;

  return (
    <>
      <WorldSceneContent onHoverStation={onHoverStation} onClickStation={onClickStation} />
      <WorldCharacters
        agents={agents}
        hoveredAgent={hoveredAgent}
        onHoverAgent={onHoverAgent}
        onSelectAgent={onSelectAgent}
      />
      <WorldCamera
        mode={cameraMode}
        selectedAgent={selectedAgent}
        onModeChange={onModeChange}
      />
      <WorldPostProcessing />
    </>
  );
}

export function WorldView({ visible = true }: { visible?: boolean }) {
  const [cameraMode, setCameraMode] = useState<CameraMode>('orbit');
  const [hoveredAgent, setHoveredAgent] = useState<string | null>(null);
  const [selectedAgent, setSelectedAgent] = useState<string | null>(null);
  const [hoveredStation, setHoveredStation] = useState<string | null>(null);
  const [showFps, setShowFps] = useState(false);

  const handleSelectAgent = useCallback((id: string) => {
    setSelectedAgent(id);
    setCameraMode('first-person');
  }, []);

  const handleModeChange = useCallback((mode: CameraMode) => {
    setCameraMode(mode);
    if (mode === 'orbit') {
      setSelectedAgent(null);
    }
  }, []);

  // TODO: Wire station clicks to real actions in future prompt
  const handleClickStation = useCallback((id: string) => {
    setHoveredStation(id);
  }, []);

  // Toggle FPS with ~ key
  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === '`') {
        setShowFps((prev) => !prev);
      }
    };
    window.addEventListener('keydown', handleKey);
    return () => window.removeEventListener('keydown', handleKey);
  }, []);

  // Find hovered station tooltip
  const stationTooltip = hoveredStation
    ? STATIONS.find((s) => s.id === hoveredStation)?.tooltip ?? null
    : null;

  if (!visible) {
    return (
      <div style={{ width: '100%', height: '100%', background: COLORS.deepVoid }} />
    );
  }

  return (
    <div style={{ width: '100%', height: '100%', position: 'relative', background: COLORS.deepVoid }}>
      <Suspense fallback={<LoadingShimmer />}>
        <Canvas
          shadows
          camera={{
            position: [20, 15, 20],
            fov: 50,
            near: 0.1,
            far: 500,
          }}
          gl={{
            antialias: true,
            toneMapping: THREE.ACESFilmicToneMapping,
            toneMappingExposure: 1.1,
            outputColorSpace: THREE.SRGBColorSpace,
          }}
          style={{ width: '100%', height: '100%' }}
          onPointerMissed={() => {
            if (cameraMode === 'orbit') {
              setHoveredStation(null);
            }
          }}
        >
          <SceneContent
            cameraMode={cameraMode}
            selectedAgentId={selectedAgent}
            onModeChange={handleModeChange}
            onHoverAgent={setHoveredAgent}
            onSelectAgent={handleSelectAgent}
            hoveredAgent={hoveredAgent}
            onHoverStation={setHoveredStation}
            onClickStation={handleClickStation}
          />
        </Canvas>
      </Suspense>
      <WorldHUD
        mode={cameraMode}
        showFps={showFps}
        hoveredStation={hoveredStation}
        stationTooltip={stationTooltip}
      />
    </div>
  );
}
