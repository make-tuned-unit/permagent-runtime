import { Suspense, useState, useCallback, useEffect, useMemo, useRef, type CSSProperties } from 'react';
import { Canvas, useThree, useFrame } from '@react-three/fiber';
import { AdaptiveEvents, PerformanceMonitor, usePerformanceMonitor } from '@react-three/drei';
import * as THREE from 'three';
import type { CameraMode, AgentState } from './types';
import { COLORS, STATIONS } from './constants';
import { navigateToTool, useCommandCenter, type ToolType } from '../../lib/store';
import { createPedestalNavController, worldNavAllowed, type PedestalNavController } from './pedestalNav';
import { WorldSceneContent } from './WorldScene';
// W3 v2 agent stack (mount swap, bible §5 — replaces legacy WorldCharacters/useAgentStates).
import { WorldAgents, ROSTER, getAgentPosition, getHenryPresence, nudgeAgent, setPath } from './agents';
import { enterAgora, exitAgora, useAgoraPhase, AGORA_CENTER, HALL_HOME } from './areas/forum/agoraArc';
import { WorldCamera } from './camera/WorldCamera';
import { WorldPostProcessing } from './WorldPostProcessing';
import { WorldHUD } from './WorldHUD';
import { LibrarianHUD } from './LibrarianHUD';
import { HenryHUD } from './HenryHUD';
import { ReaderHUD } from './ReaderHUD';
import { WatcherHUD } from './WatcherHUD';
import { StewardHUD } from './StewardHUD';
import { StrixHUD } from './StrixHUD';
import { FinancierHUD } from './FinancierHUD';
import { AgentPicker } from './AgentPicker';
import { PerfProbe, perfProbeEnabled, devDprOverride } from './shared/PerfProbe';
import { useWorldVisibility } from './atmosphere/useWorldVisibility';
import { installDevHarness } from './atmosphere/devHarness';
import { getReduceMotion, radius, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import { TourMode } from './camera/TourMode';

// DEV-ONLY: window.__worldDev harness for ambience evidence (no-op in prod).
installDevHarness();

// One-shot fact-finding, not a feature: does the WebView we actually ship in
// expose WebGPU? macOS 26 / Safari 26 enables it by default and Apple says
// WKWebView inherits that, but nobody has confirmed it inside our Tauri shell.
// R3F v8 has no WebGPU path, so this changes no behaviour — it just means the
// next person deciding about a renderer migration has a measurement instead of
// a blog post. (Research note THREEJS_WORLD_2026-08-24 §5 risk 1.)
if (typeof navigator !== 'undefined') {
  const hasWebGPU = 'gpu' in navigator && !!(navigator as { gpu?: unknown }).gpu;
  console.info(`[world] navigator.gpu present: ${hasWebGPU}`);
}

// Cardinal pedestal → product tab (World click-through / launchpad). Station ids
// are the World's own names (constants.ts); each maps to the ToolType its label
// names. Brain is the 'memory' tool (WorkspaceRenderer → BrainView). The Lab
// pedestal is intentionally absent: there is no 'lab' ToolType — no product tab
// exists for it — so it stays glide-only (see PHASE-0 report). forum-portal is
// special-cased to the Agora arc, never a tab.
const STATION_TOOL: Partial<Record<string, ToolType>> = {
  library: 'build',
  observatory: 'memory',
  automate: 'automate',
};
// The pedestal glide delay + pending-nav semantics live in pedestalNav.ts.

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
        fontSize: textSize.body,
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
    // Evidence read for Henry presence.
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

// The World is ambience, not a game. It has no reason to render at whatever
// rate the panel happens to refresh at — on a ProMotion display that is 120
// frames a second of a marble hall standing still, on a machine that is always
// on and often also running local inference on the same GPU.
//
// So the render loop is driven here instead of by r3f. `frameloop="never"`
// hands us the wheel: r3f stops its own requestAnimationFrame loop and only
// renders when `advance(t)` is called, and in that mode it takes the clock
// time from the timestamp we pass — in SECONDS, not milliseconds, which is the
// one thing that is easy to get wrong and produces a scene that either freezes
// or runs at a thousand times speed.
//
// The tolerance below matters more than it looks. Without it, a display whose
// refresh rate is already at or near the target aliases catastrophically: a
// frame arriving at 33.2ms against a 33.3ms budget gets skipped, the next one
// lands at 66.6ms, and a 30Hz panel renders at 15fps. Allowing a frame that is
// within a few milliseconds of due costs nothing and removes the cliff.
const TARGET_FPS = 30;
const FRAME_TOLERANCE_MS = 4;

function FrameCap({ active }: { active: boolean }) {
  const advance = useThree((s) => s.advance);
  useEffect(() => {
    if (!active) return;
    const minDelta = 1000 / TARGET_FPS - FRAME_TOLERANCE_MS;
    const t0 = performance.now();
    let last = -Infinity;
    let handle = 0;
    const tick = (now: number) => {
      handle = requestAnimationFrame(tick);
      if (now - last < minDelta) return;
      last = now;
      advance((now - t0) / 1000);
    };
    handle = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(handle);
  }, [active, advance]);
  return null;
}

// Resolution is the one lever that trades quality for frame time linearly on a
// fill-rate-bound scene, which makes it the right response to "something else
// took the GPU" — and on this machine something else regularly does, because
// local inference runs on it.
//
// One owner. drei ships an <AdaptiveDpr/> that also writes dpr, driven off
// r3f's separate regression system, and running both would give two components
// a different opinion about the same number every frame. This reads the
// PerformanceMonitor's factor and is the only thing that calls setDpr.
//
// Bounds are expressed against the monitor's own observed refresh rate rather
// than as absolute fps, because the target here is not 60: the frame cap above
// holds us at 30, and this machine's display refreshes at 30Hz anyway. An
// absolute lower bound of 40fps — drei's default — would read a perfectly
// healthy capped scene as permanently failing and grind the resolution down to
// nothing.
const MIN_DPR = 0.75;
const MAX_DPR = 1.5;

// Module scope so the identity is stable across renders — drei re-reads this
// on every sample.
const adaptiveBounds = (refreshrate: number): [number, number] => [
  refreshrate * 0.8,
  refreshrate * 0.95,
];

function AdaptiveResolution({ enabled }: { enabled: boolean }) {
  const setDpr = useThree((s) => s.setDpr);
  const applied = useRef(MAX_DPR);
  usePerformanceMonitor({
    onChange: ({ factor }) => {
      if (!enabled) return;
      const next = Math.round((MIN_DPR + factor * (MAX_DPR - MIN_DPR)) * 20) / 20;
      if (next === applied.current) return;
      applied.current = next;
      setDpr(next);
    },
  });
  return null;
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
  focusPoint,
  onFocusDone,
}: {
  cameraMode: CameraMode;
  selectedAgentId: string | null;
  onModeChange: (mode: CameraMode) => void;
  onHoverAgent: (id: string | null) => void;
  onSelectAgent: (id: string) => void;
  hoveredAgent: string | null;
  onHoverStation: (id: string | null) => void;
  onClickStation: (id: string) => void;
  focusPoint: [number, number, number] | null;
  onFocusDone: () => void;
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
        focusPoint={focusPoint}
        onFocusDone={onFocusDone}
      />
      <TourMode cameraMode={cameraMode} />
      <WorldPostProcessing />
      {import.meta.env.DEV && <AgentEvidenceHooks />}
    </>
  );
}

export function WorldView({ visible = true }: { visible?: boolean }) {
  // The world's chrome paints from its own palette; the theme is here only to
  // feed the button primitive's variant defaults.
  const { colors: themeColors } = useTheme();
  const [cameraMode, setCameraMode] = useState<CameraMode>('orbit');
  const [hoveredAgent, setHoveredAgent] = useState<string | null>(null);
  const [selectedAgent, setSelectedAgent] = useState<string | null>(null);
  const [hoveredStation, setHoveredStation] = useState<string | null>(null);
  const [showFps, setShowFps] = useState(false);
  const [activeHud, setActiveHud] = useState<
    'henry' | 'librarian' | 'reader' | 'watcher' | 'steward' | 'strix' | 'financier' | null
  >(null);
  const containerRef = useRef<HTMLDivElement>(null);
  // Perf (bible §8 item 2): pause the render loop whenever this view has no
  // layout box — i.e. its workspace tab is hidden (display:none) or the canvas
  // has not been sized yet. Prevents GPU burn behind other tabs and the
  // zero-size GL_INVALID_FRAMEBUFFER_OPERATION spam at startup.
  const canvasActive = useWorldVisibility(containerRef);
  // Read once per mount, like every other reduceMotion consumer in world/.
  const reduceMotion = useMemo(() => getReduceMotion(), []);
  // Someone who asked for less motion did not ask for the resolution to
  // breathe in and out underneath them, so the adaptive lever is held still
  // for them (bible §8: reduceMotion gets a static fallback, not a quieter
  // version of the same movement). A dev dpr sweep also pins it, or the sweep
  // would be measuring the adaptation rather than the scene.
  const adaptiveDprEnabled = !reduceMotion && !(import.meta.env.DEV && devDprOverride());

  const handleSelectAgent = useCallback((id: string) => {
    if (id === 'henry') {
      setActiveHud('henry');
    } else if (id === 'librarian') {
      setActiveHud('librarian');
    } else if (id === 'reader') {
      setActiveHud('reader');
    } else if (id === 'watcher') {
      setActiveHud('watcher');
    } else if (id === 'steward') {
      setActiveHud('steward');
    } else if (id === 'strix') {
      setActiveHud('strix');
    } else if (id === 'financier') {
      setActiveHud('financier');
    } else {
      setActiveHud(null);
    }
    setSelectedAgent(id);
    setCameraMode('third-person');
  }, []);

  const pendingWorldAgent = useCommandCenter(s => s.pendingWorldAgent);
  const clearPendingWorldAgent = useCommandCenter(s => s.clearPendingWorldAgent);
  useEffect(() => {
    if (!pendingWorldAgent) return;
    if (ROSTER.some(a => a.id === pendingWorldAgent)) {
      handleSelectAgent(pendingWorldAgent);
    }
    clearPendingWorldAgent();
  }, [pendingWorldAgent, clearPendingWorldAgent, handleSelectAgent]);

  const handleModeChange = useCallback((mode: CameraMode) => {
    setCameraMode(mode);
    if (mode === 'orbit') {
      setSelectedAgent(null);
      setActiveHud(null);
    }
  }, []);

  // #386: clicking a station glides the camera to it. The forum-portal
  // pedestal / the Stargate itself are special — clicking them plays the Agora
  // arc (#306): the sovereign agent descends into the portal, dissolves into
  // code, and the camera dives through the membrane into the collective mind.
  const [focusPoint, setFocusPoint] = useState<[number, number, number] | null>(null);
  const agoraPhase = useAgoraPhase();
  // Pending pedestal→tab navigation (C2): the controller owns the glide timer.
  // It cancels on a new station click, on unmount, and — because App keeps
  // every workspace MOUNTED behind display:none — whenever this view stops
  // being the visible workspace (workspace switch / overlay open, via
  // canvasActive below), with a store re-check at fire time so a manual
  // navigation during the 700ms glide can never yank the user. pedestalNav.ts
  // carries the semantics + tests.
  const pedestalNavRef = useRef<PedestalNavController | null>(null);
  if (pedestalNavRef.current === null) {
    pedestalNavRef.current = createPedestalNavController(navigateToTool, {
      canNavigate: () => worldNavAllowed(useCommandCenter.getState()),
    });
  }
  const handleClickStation = useCallback((id: string) => {
    setHoveredStation(id);
    pedestalNavRef.current?.cancel();
    if (id === 'forum-portal') {
      // Beat 1 (descent): draw the sovereign to the portal mouth. Beats 2-4
      // (dissolve → absorb → witness) run off the arc + the camera dive.
      const a = (5 * Math.PI) / 4;
      setPath('henry', [
        { x: Math.cos(a) * 13.6, y: 0, z: Math.sin(a) * 13.6, facing: a },
      ]);
      enterAgora();
      setFocusPoint([...AGORA_CENTER]);
      return;
    }
    const station = STATIONS.find((s) => s.id === id);
    if (station) setFocusPoint([...station.position] as [number, number, number]);
    // Launchpad: glide toward the pedestal, then land on its product tab. The Lab
    // pedestal has no tab (absent from STATION_TOOL) — it stays glide-only.
    const tool = STATION_TOOL[id];
    if (tool) pedestalNavRef.current?.schedule(tool);
  }, []);
  const handleFocusDone = useCallback(() => setFocusPoint(null), []);
  // C2: keep the controller's visibility current — going hidden (workspace
  // switch or overlay open) cancels any pending pedestal landing — and clear
  // it if the view ever unmounts mid-glide.
  useEffect(() => {
    pedestalNavRef.current?.setVisible(canvasActive);
  }, [canvasActive]);
  useEffect(() => () => pedestalNavRef.current?.dispose(), []);

  // Beat 5 (return): pull back through the portal — Henry rematerializes on the
  // Rotunda side (the dissolve played backward) and the camera comes home.
  const handleExitAgora = useCallback(() => {
    exitAgora();
    setFocusPoint([...HALL_HOME]);
  }, []);

  // ESC returns from the Agora (in orbit mode ESC is otherwise free; third-person
  // ESC is owned by WorldCamera). Gated on the World being the VISIBLE
  // workspace (P2): every workspace stays mounted behind display:none, so
  // without the gate an Escape aimed at an overlay would also silently exit
  // the Agora arc in the hidden World.
  useEffect(() => {
    if (agoraPhase === 'home' || !canvasActive) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') handleExitAgora();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [agoraPhase, handleExitAgora, canvasActive]);

  // Toggle FPS with ~ key — also gated on visibility (P2): the hidden World
  // must not eat keystrokes typed into other surfaces.
  useEffect(() => {
    if (!canvasActive) return;
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === '`') {
        setShowFps((prev) => !prev);
      }
    };
    window.addEventListener('keydown', handleKey);
    return () => window.removeEventListener('keydown', handleKey);
  }, [canvasActive]);

  // C7: station/prop hovers set a body-level pointer cursor that pointerOut
  // normally resets — but navigating away hides the canvas with no pointerOut,
  // leaving the destination tab showing a pointer over dead space. Reset it
  // whenever this view stops being visible (and on unmount). '' clears the
  // inline override; hovers re-set it live.
  useEffect(() => {
    const resetPointerCursor = () => {
      if (document.body.style.cursor === 'pointer') document.body.style.cursor = '';
    };
    if (!canvasActive) resetPointerCursor();
    return resetPointerCursor;
  }, [canvasActive]);

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
          shadows={{ type: THREE.PCFSoftShadowMap }}
          // Bible §8 item 1: the scene is fill-rate bound (~4fps at dpr 2).
          // `?dpr=N` in dev pins it for a fill-rate sweep; never set in the app.
          dpr={(import.meta.env.DEV && devDprOverride()) || [1, 1.5]}
          // Bible §8 item 2 still holds — nothing renders when the World tab
          // is hidden — but the gate now lives in <FrameCap/>, which owns the
          // loop and simply does not run while `canvasActive` is false.
          frameloop="never"
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
          {/* Everything that renders sits inside the monitor so its frame
              sampling sees the real scene. */}
          <PerformanceMonitor factor={1} bounds={adaptiveBounds}>
            <SceneContent
              cameraMode={cameraMode}
              selectedAgentId={selectedAgent}
              onModeChange={handleModeChange}
              onHoverAgent={setHoveredAgent}
              onSelectAgent={handleSelectAgent}
              hoveredAgent={hoveredAgent}
              onHoverStation={setHoveredStation}
              onClickStation={handleClickStation}
              focusPoint={focusPoint}
              onFocusDone={handleFocusDone}
            />
            <AdaptiveResolution enabled={adaptiveDprEnabled} />
          </PerformanceMonitor>
          {/* Stops raycasting the scene on every pointer move while the frame
              budget is already under pressure. Cheap, and invisible when it is
              not needed. */}
          <AdaptiveEvents />
          <FrameCap active={canvasActive} />
          {/* The shared perf probe (bible §6) lives in WorldScene — one per
              Canvas, or the two of them zero each other's gl.info reads. */}
          {/* Dev measurement harness, only with ?perf=1 (frame-time percentiles
              into document.title so WKWebView runs can be read from outside). */}
          {import.meta.env.DEV && perfProbeEnabled() && <PerfProbe />}
        </Canvas>
      </Suspense>
      <WorldHUD
        mode={cameraMode}
        showFps={showFps}
        hoveredStation={hoveredStation}
        stationTooltip={stationTooltip}
      />
      {/* Agora return affordance (#306 arc beat 5) — visible once you cross into
          the mesh; returns you home through the portal (also bound to ESC). */}
      {agoraPhase !== 'home' && (
        <Button
          colors={themeColors}
          variant="ghostOn"
          type="button"
          onClick={handleExitAgora}
          flashSuccess={false}
          style={{
            '--pa-btn-bg': 'rgba(10, 14, 26, 0.82)',
            '--pa-btn-fg': COLORS.neonCyan,
            '--pa-btn-border': `${COLORS.neonCyan}66`,
            '--pa-btn-bg-hover': 'rgba(16, 24, 44, 0.9)',
            '--pa-btn-border-hover': COLORS.neonCyan,
            '--pa-btn-bg-active': 'rgba(10, 14, 26, 0.82)',
            '--pa-btn-pad': '9px 20px',
            '--pa-btn-radius': `${radius.sm}px`,
            position: 'absolute',
            bottom: 28,
            left: '50%',
            // The centring transform has to stay inline, which does mean
            // `.pa-btn`'s press scale cannot apply to this one button.
            transform: 'translateX(-50%)',
            fontFamily: 'JetBrains Mono, monospace',
            fontSize: textSize.micro,
            letterSpacing: '0.18em',
            boxShadow: `0 0 18px ${COLORS.neonCyan}33`,
            backdropFilter: 'blur(4px)',
          } as CSSProperties}
        >
          ↩ RETURN TO THE ROTUNDA · ESC
        </Button>
      )}
      <HenryHUD
        visible={activeHud === 'henry'}
        onClose={() => setActiveHud(null)}
      />
      <LibrarianHUD
        visible={activeHud === 'librarian'}
        onClose={() => setActiveHud(null)}
      />
      <ReaderHUD
        visible={activeHud === 'reader'}
        onClose={() => setActiveHud(null)}
      />
      <WatcherHUD
        visible={activeHud === 'watcher'}
        onClose={() => setActiveHud(null)}
      />
      <StewardHUD
        visible={activeHud === 'steward'}
        onClose={() => setActiveHud(null)}
      />
      <StrixHUD
        visible={activeHud === 'strix'}
        onClose={() => setActiveHud(null)}
      />
      <FinancierHUD
        visible={activeHud === 'financier'}
        onClose={() => setActiveHud(null)}
      />
      <AgentPicker
        selectedAgentId={selectedAgent}
        onSelectAgent={handleSelectAgent}
      />
    </div>
  );
}
