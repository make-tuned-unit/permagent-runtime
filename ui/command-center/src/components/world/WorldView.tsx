import { Suspense, useState, useCallback, useEffect, useRef } from 'react';
import { Canvas } from '@react-three/fiber';
import * as THREE from 'three';
import type { CameraMode } from './types';
import { COLORS, STATIONS } from './constants';
import { useAgentStates } from './agents/useAgentStates';
import { WorldSceneContent } from './WorldScene';
import { WorldCharacters } from './agents/WorldCharacters';
import { WorldCamera } from './camera/WorldCamera';
import { WorldPostProcessing } from './WorldPostProcessing';
import { WorldHUD } from './WorldHUD';
import { LibrarianHUD } from './LibrarianHUD';
import { HenryHUD } from './HenryHUD';
import { AgentPicker } from './AgentPicker';
import { PerfSampler } from './shared/perf';
import { useWorldVisibility } from './atmosphere/useWorldVisibility';
import { installDevHarness } from './atmosphere/devHarness';
import { TourMode } from './camera/TourMode';

// DEV-ONLY: window.__worldDev harness for ambience evidence (no-op in prod).
installDevHarness();

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
  const { agents, moveAgent } = useAgentStates();
  const selectedAgent = agents.find((a) => a.id === selectedAgentId) ?? null;

  const handleMoveAgent = useCallback((dx: number, dz: number) => {
    if (selectedAgentId) {
      moveAgent(selectedAgentId, dx, dz);
    }
  }, [selectedAgentId, moveAgent]);

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
        onMoveAgent={handleMoveAgent}
      />
      <TourMode cameraMode={cameraMode} />
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
  const [activeHud, setActiveHud] = useState<'henry' | 'librarian' | null>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  // Perf (bible §8 item 2): pause the render loop whenever this view has no
  // layout box — i.e. its workspace tab is hidden (display:none) or the canvas
  // has not been sized yet. Prevents GPU burn behind other tabs and the
  // zero-size GL_INVALID_FRAMEBUFFER_OPERATION spam at startup.
  const canvasActive = useWorldVisibility(containerRef);

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
          // Bible §8 item 3: explicit PCF — `shadows="soft"` is deprecated in
          // three 0.184 and silently fell back to PCF anyway. Map size stays
          // 2048 (set on the key light in atmosphere/Lighting).
          shadows={{ type: THREE.PCFShadowMap }}
          // Bible §8 item 1: the scene is fill-rate bound (~4fps at dpr 2).
          dpr={[1, 1.5]}
          // Bible §8 item 2: stop rendering when the World tab is hidden.
          frameloop={canvasActive ? 'always' : 'never'}
          camera={{
            position: [20, 15, 20],
            fov: 50,
            near: 0.1,
            far: 500,
          }}
          gl={{
            // Bible §8 item 1: MSAA off — the post chain owns AA duties and
            // antialias:true multiplies the fill-rate cost at full dpr.
            antialias: false,
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
          {/* Shared perf probe (bible §6): publishes window.__worldPerf 1/s. */}
          <PerfSampler />
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
