import { Suspense, useState, useCallback, useEffect, useRef } from 'react';
import { Canvas, useThree, useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import type { CameraMode, AgentState } from './types';
import { COLORS, STATIONS } from './constants';
import { WorldSceneContent } from './WorldScene';
// W3 v2 agent stack (mount swap, bible §5 — replaces legacy WorldCharacters/useAgentStates).
import { WorldAgents, ROSTER, getAgentPosition, getHenryPresence, nudgeAgent } from './agents';
import { getBankSnapshot } from './shared/tendingBank';
import { getConstructionProgress } from './agents/construction';
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
import { TurnFraming } from './camera/TurnFraming';
import { DescentMode } from './camera/DescentMode';
import { setDescentStop, getDescentStop, useDescentActive } from './camera/descentState';
import { getStrata } from './areas/strata/strataState';
import { StratumPlaque } from './hud/StratumPlaque';
import { requestCaveLoad } from './areas/strata/CaveSystem';

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

// EVIDENCE-ONLY hooks (no-op in production): expose the CDP harness surface the W3
// rig drives — countMeshes + per-agent live position from the motion store. The camera
// pin hook (window.__worldDebug.setCam) is owned by WorldScene's WorldDevHooks; this adds
// the agents reads the harness's drive.mjs expects (window.__worldAgents.getAgentPosition).
function AgentEvidenceHooks() {
  const scene = useThree((s) => s.scene);
  useEffect(() => {
    if (!import.meta.env.DEV) return;
    const w = window as unknown as Record<string, unknown>;
    w.__worldAgents = { getAgentPosition };
    // Evidence reads for the tending bank ledger + construction progress + Henry presence.
    w.__worldBank = () => ({ ...getBankSnapshot(), construction: getConstructionProgress() });
    w.__worldHenry = () => ({ ...getHenryPresence() });
    const dbg = (w.__worldDebug ?? {}) as Record<string, unknown>;
    dbg.countMeshes = () => {
      let mesh = 0;
      let skinned = 0;
      scene.traverse((o) => {
        const obj = o as THREE.Object3D & { isMesh?: boolean; isSkinnedMesh?: boolean };
        if (obj.isSkinnedMesh) skinned++;
        else if (obj.isMesh) mesh++;
      });
      return { mesh, skinned };
    };
    w.__worldDebug = dbg;
    return () => {
      delete w.__worldAgents;
      delete w.__worldBank;
      delete w.__worldHenry;
    };
  }, [scene]);
  return null;
}

// Bridges the autonomous V2 motion store to the (W4-owned) camera's third-person follow.
// The V2 agents move themselves; this proxies the selected agent's LIVE position into the
// AgentState shape the camera reads, refreshed each frame. When zoomed in (third-person),
// arrow keys / WASD drive the selected agent via nudgeAgent — manual control overrides the
// agent's autonomous walk while the user holds the keys (see motion.ts nudgeAgent).
function useSelectedAgentProxy(selectedAgentId: string | null): AgentState | null {
  const ref = useRef<AgentState | null>(null);
  useFrame(() => {
    if (!selectedAgentId) {
      ref.current = null;
      return;
    }
    const pos = getAgentPosition(selectedAgentId);
    const id = ROSTER.find((r) => r.id === selectedAgentId);
    if (!pos || !id) {
      ref.current = null;
      return;
    }
    if (!ref.current || ref.current.id !== selectedAgentId) {
      ref.current = {
        id: id.id,
        name: id.name,
        role: id.role,
        position: { x: pos.x, y: pos.y, z: pos.z },
        activity: 'idle',
        currentStation: null,
        togaTrimColor: id.trimColor,
        isHenry: id.isHenry,
      };
    } else {
      ref.current.position.x = pos.x;
      ref.current.position.y = pos.y;
      ref.current.position.z = pos.z;
    }
  });
  return ref.current;
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
  const selectedAgent = useSelectedAgentProxy(selectedAgentId);
  const handleMoveAgent = useCallback(
    (dx: number, dz: number) => {
      if (selectedAgentId) nudgeAgent(selectedAgentId, dx, dz);
    },
    [selectedAgentId],
  );

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
      <TourMode cameraMode={cameraMode} />
      {/* Carved Cave descent — vertical rail down the throat, stops per stratum. */}
      <DescentMode cameraMode={cameraMode} onAscend={() => onModeChange('orbit')} />
      {/* THE TURN — day-one wall framing + the 180° reveal (Shift+T). */}
      <TurnFraming cameraMode={cameraMode} />
      <WorldPostProcessing />
      {import.meta.env.DEV && <AgentEvidenceHooks />}
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

  // The throat-collar click (entry point B, inside the Canvas) sets descentActive
  // directly — sync that to cameraMode so the camera actually descends. Orbit-side
  // ascent clears the flag via handleAscend → handleModeChange.
  const descentActive = useDescentActive();
  useEffect(() => {
    if (descentActive) {
      setCameraMode((m) => (m === 'descent' ? m : 'descent'));
    }
  }, [descentActive]);

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

  // ── Carved Cave descent controls (C-nav) ──
  // Entry point A: the ↓ Descend HUD control. Also force the cave chunk to load
  // (folds in nav-bug #1: don't rely on the proximity-only trigger).
  const handleDescend = useCallback(() => {
    requestCaveLoad();
    setCameraMode('descent');
  }, []);
  const handleAscend = useCallback(() => {
    setCameraMode('orbit');
  }, []);
  // HUD ▲/▼ step the stratum stop; DescentMode reacts to the store change and glides.
  // Rail stops = verge (0) + one per chamber (1..N) + the floor (N+1) → max index N+1.
  const handleDescentStep = useCallback((dir: 1 | -1) => {
    const maxIndex = getStrata().length + 1;
    const next = Math.max(0, Math.min(maxIndex, getDescentStop() + dir));
    setDescentStop(next);
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
        onDescend={handleDescend}
        onDescentStep={handleDescentStep}
        onAscend={handleAscend}
      />
      {/* Descent legibility plaque — leads with real range + counts (knob 1). */}
      <StratumPlaque cameraMode={cameraMode} />
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
