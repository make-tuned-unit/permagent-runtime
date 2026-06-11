import { Suspense, useState, useCallback, useEffect, useRef } from 'react';
import { Canvas, useThree, useFrame } from '@react-three/fiber';
import * as THREE from 'three';
// EVIDENCE-ONLY (uncommitted): perf probe + camera pin hook for CDP capture.
import { PerfSampler } from './shared/perf';
// INTEGRATION PREVIEW (becomes permanent after W1's skeleton lands + rebase):
import { WorldAgents, getAgentPosition } from './agents';

function EvidenceCamHook() {
  const camera = useThree((s) => s.camera);
  const scene = useThree((s) => s.scene);
  const pin = useRef<{ p: [number, number, number]; l: [number, number, number] } | null>(null);
  useEffect(() => {
    (window as unknown as Record<string, unknown>).__worldDebug = {
      setCam: (p: [number, number, number], l: [number, number, number]) => {
        pin.current = { p, l };
      },
      clearCam: () => {
        pin.current = null;
      },
      countMeshes: () => {
        let mesh = 0;
        let skinned = 0;
        scene.traverse((o) => {
          const obj = o as THREE.Object3D & { isMesh?: boolean; isSkinnedMesh?: boolean };
          if (obj.isSkinnedMesh) skinned++;
          else if (obj.isMesh) mesh++;
        });
        return { mesh, skinned };
      },
    };
    (window as unknown as Record<string, unknown>).__worldAgents = { getAgentPosition };
  }, [scene]);
  useFrame(() => {
    if (!pin.current) return;
    camera.position.set(...pin.current.p);
    camera.lookAt(...pin.current.l);
  }, 998);
  return null;
}

// EVIDENCE-ONLY: with the post chain, gl.info resets per pass — accumulate the
// whole frame instead and reset AFTER PerfSampler (priority 1000) has sampled.
function EvidenceInfoReset() {
  const gl = useThree((s) => s.gl);
  useEffect(() => {
    gl.info.autoReset = false;
    return () => {
      gl.info.autoReset = true;
    };
  }, [gl]);
  useFrame(() => {
    gl.info.reset();
  }, 1001);
  return null;
}
import type { CameraMode } from './types';
import { COLORS, STATIONS } from './constants';
import { WorldSceneContent } from './WorldScene';
import { WorldCamera } from './WorldCamera';
import { WorldPostProcessing } from './WorldPostProcessing';
import { WorldHUD } from './WorldHUD';
import { LibrarianHUD } from './LibrarianHUD';
import { HenryHUD } from './HenryHUD';
import { AgentPicker } from './AgentPicker';

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
  // EVIDENCE PATCH: old useAgentStates sim removed; third-person camera follow is
  // disabled until W4 reads getAgentPosition() (see PR notes). selectedAgentId is
  // kept for the HUD flow.
  void selectedAgentId;
  const selectedAgent = null;
  const handleMoveAgent = useCallback((_dx: number, _dz: number) => {}, []);

  return (
    <>
      <WorldSceneContent onHoverStation={onHoverStation} onClickStation={onClickStation} />
      <WorldAgents
        hoveredAgent={hoveredAgent}
        onHoverAgent={onHoverAgent}
        onSelectAgent={onSelectAgent}
      />
      <WorldCamera
        mode={cameraMode}
        selectedAgent={selectedAgent}
        onModeChange={onModeChange}
        onMoveAgent={handleMoveAgent}
      />
      <WorldPostProcessing />
      <PerfSampler />
      <EvidenceCamHook />
      <EvidenceInfoReset />
    </>
  );
}

export function WorldView({ visible = true }: { visible?: boolean }) {
  const [cameraMode, setCameraMode] = useState<CameraMode>('orbit');
  const [hoveredAgent, setHoveredAgent] = useState<string | null>(null);
  const [selectedAgent, setSelectedAgent] = useState<string | null>(null);
  const [hoveredStation, setHoveredStation] = useState<string | null>(null);
  const [showFps, setShowFps] = useState(false);
  const [activeHud, setActiveHud] = useState<'henry' | 'librarian' | null>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  const handleSelectAgent = useCallback((id: string) => {
    if (id === 'henry') {
      setActiveHud('henry');
    } else if (id === 'librarian') {
      setActiveHud('librarian');
    } else {
      setActiveHud(null);
    }
    setSelectedAgent(id);
    setCameraMode('third-person');
  }, []);

  const handleModeChange = useCallback((mode: CameraMode) => {
    setCameraMode(mode);
    if (mode === 'orbit') {
      setSelectedAgent(null);
      setActiveHud(null);
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

  // Suppress WebKit's HTML5 drag indicator on this view.
  // The native Tauri bridge (onDragDropEvent) operates at the window level
  // independently of HTML5 drag events — preventDefault here only suppresses
  // WebKit's content-level "drop a file" overlay, not the native file bridge.
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const prevent = (e: Event) => e.preventDefault();
    el.addEventListener('dragover', prevent);
    el.addEventListener('dragenter', prevent);
    return () => {
      el.removeEventListener('dragover', prevent);
      el.removeEventListener('dragenter', prevent);
    };
  }, []);

  if (!visible) {
    return (
      <div style={{ width: '100%', height: '100%', background: COLORS.deepVoid }} />
    );
  }

  return (
    <div ref={containerRef} style={{ width: '100%', height: '100%', position: 'relative', background: COLORS.deepVoid }}>
      <Suspense fallback={<LoadingShimmer />}>
        <Canvas
          shadows="soft"
          camera={{
            position: [20, 15, 20],
            fov: 50,
            near: 0.1,
            far: 500,
          }}
          gl={{
            antialias: true,
            toneMapping: THREE.ACESFilmicToneMapping,
            toneMappingExposure: 1.3,
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
      <HenryHUD
        visible={activeHud === 'henry'}
        onClose={() => setActiveHud(null)}
      />
      <LibrarianHUD
        visible={activeHud === 'librarian'}
        onClose={() => setActiveHud(null)}
      />
      <AgentPicker
        selectedAgentId={selectedAgent}
        onSelectAgent={handleSelectAgent}
      />
    </div>
  );
}
