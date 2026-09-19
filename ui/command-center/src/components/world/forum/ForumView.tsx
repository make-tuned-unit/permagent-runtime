import { applyForumSurfaceMaterials } from './ForumSurfaceMaterials';
import { Suspense, useEffect, useMemo, useState, useCallback, useRef, type CSSProperties, type RefObject } from 'react';
import { Canvas, useFrame } from '@react-three/fiber';
import { AgXToneMapping } from 'three';
import { useGLTF } from '@react-three/drei';
import { Mesh, PCFShadowMap } from 'three';
import { ROSTER } from '../agents';
import { ForumAgents } from './ForumAgents';
import { ForumCapabilities } from './ForumCapabilities';
import { VaultBoundary } from '../areas/hall/BlenderVault';
import { useActiveGoals } from '../agents/goalActivity';
import { PerfSampler } from '../shared/perf';
import { Button } from '../../common/Button';
import { useTheme } from '../../../styles/useTheme';
import { font, radius, type as typeRamp } from '../../../styles/tokens';
import { ENV, STATE } from '../shared/palette';
import { useOrchestratorName } from '../shared/useOrchestratorName';
import type { ToolType } from '../../../lib/store';
import { DISTRICTS } from './districts';
import { ForumNavigation } from './ForumNavigation';
import './forum.css';
import { askFromForum } from './forumQuery';
import { ForumConversation } from './ForumConversation';
import { useSystemAppearance } from './useSystemAppearance';
import { ForumLighting } from './ForumLighting';
import { FORUM_AGENT_PROFILES } from './agentProfiles';
import { ForumWalk } from './ForumWalk';
import { FORUM_ASSET_REVISION } from './assetRevision';
import { registerForumCollision } from './walkCollision';
import { MeshAgora } from './MeshAgora';
import { ForumPostProcessing } from './ForumPostProcessing';
import { useCommandCenter } from '../../../lib/store';
import { useIdentityStore } from '../../../stores/identityStore';
import { ForumPassport } from './ForumPassport';
import { ForumPlaceRail } from './ForumPlaceRail';
import { useDismissible } from './useDismissible';

/**
 * Depth range, and why these two numbers matter more than they look.
 *
 * The camera used to run from the default near of 0.1 out to a far of 10000: a
 * 100,000:1 ratio across a 24-bit depth buffer. Depth precision is spent
 * logarithmically near the camera, so at that ratio two nearly-coplanar
 * surfaces — a path laid on a terrace, an inlay in the plaza — land on the same
 * depth value and the renderer picks a winner per frame per pixel. That is the
 * flashing on the ground: not a material bug, not shadow acne, but z-fighting.
 *
 * The far plane only has to contain the sky: the dome is radius 4800 and the
 * celestial bodies sit at 1080, so 6000 keeps every one of them and throws away
 * 4000 units of empty space. The near plane is bounded by walking: the walker's
 * body radius is 0.28m, so anything closer than that to the eye is already
 * inside a wall, and 0.25 stays just under it.
 *
 * Together that is 24,000:1 instead of 100,000:1 — four times the precision,
 * for nothing.
 */
/**
 * Exposure for the AgX view transform.
 *
 * The scene had no tone mapping set at all, which meant r3f's default of ACES
 * Filmic. ACES imparts a strong film response that reshapes saturated colours,
 * and the lagoon's teal is exactly the kind of colour it pushes toward
 * cyan-white — the island's most recognisable feature, altered by a default
 * nobody chose.
 *
 * AgX is the same view transform Blender has used by default since 4.0, and
 * Blender is now the lighting authority for this world (Unreal cannot bake
 * lighting on an Apple GPU — no DirectX, no DXR, and its light-build farm is
 * Windows-only). Matching it means what the Cycles bake looks like in Blender
 * is what the browser shows, which is worth more than any single look choice.
 * Grade back toward a warmer or punchier look with a LUT, not by picking a more
 * opinionated tone mapper.
 */
const FORUM_EXPOSURE = 1.0;

const FORUM_CAMERA_NEAR = 0.25;
const FORUM_CAMERA_FAR = 6000;

/** Remembers whether the Agent Desk is put away. */
const FORUM_DESK_KEY = 'permagent.forum.desk';
/** Remembers whether the place rail is open. */
const FORUM_RAIL_KEY = 'permagent.forum.rail';
import { ForumEnvironment } from './ForumEnvironment';
import { ForumFlight } from './ForumFlight';
import { ForumShips } from './ForumShips';
import { advanceForumSurfaceClock, releaseForumSurfaceClock, setForumWaterInfo } from './ForumSurfaceMaterials';
import { boundsVector, loadWaterInfo } from './waterInfo';
import { useHudPresence } from './useHudPresence';
import { gateApproachPoint } from './meshPortal';
import { extendForumLoader } from './forumGltf';
import { FORUM_VISTA } from './vistaCamera';

const lessons = [
  ['A brief becomes a plan', 'An agent starts with your outcome, context and constraints. It chooses steps and tools. A useful brief also says what evidence would count as success.', 'Explain how you would turn my request into a plan. Identify missing context before taking action.'],
  ['Tools connect thought to action', 'A model proposes an action; a tool performs it. Tool inputs, results and errors are the evidence to inspect. A moving avatar alone does not prove work happened.', 'Describe the tools needed for this task, what each can change, and where you would ask for approval.'],
  ['Memory needs provenance', 'Stored context helps across sessions, but can be stale or wrong. Ask where a fact came from, when it was observed, and how to correct it.', 'Explain which memories would help with this request and how you would verify their accuracy.'],
  ['Completion needs evidence', 'A successful tool call is not always a successful job. Compare the result with your acceptance criteria and inspect the actual artifact or test result.', 'Propose acceptance criteria and the evidence you would return when this job is complete.'],
];
const profiles: Record<string, string> = {
  henry: 'Your sovereign orchestrator — the leader you name. Discuss an outcome, clarify the scope, and decide which specialist should help.',
  librarian: 'Organizes and curates the Brain. Ask how a memory was formed, what supports it, and what needs updating.',
  reader: 'Helps make source material understandable. Start with a document and a question you want answered.',
  steward: 'Reviews repository hygiene and proposes maintenance. Distinguish a review from a change to your files.',
  watcher: 'Surfaces proactive observations. Inspect the source and timing of a nudge before acting on it.',
  strix: 'Examines your projects for security issues. Findings should include evidence and a bounded next step.',
  financier: 'Reads financial figures and their sources. Ask for calculations and assumptions rather than treating a forecast as certainty.',
  forecaster: 'Studies project trends and possible futures. Ask which observations would change a forecast.',
  polybot: 'A separate automated process with configured operating rules. Ask about its actual board status; its avatar cannot prove that the process is running.',
  picker: 'Compares candidates and ranks a shortlist. Ask about criteria, exclusions and the evidence behind a ranking.',
  growth_measurement: 'Evaluates whether an intervention helped. Ask about the baseline, observation window and competing explanations.',
  council: 'Brings several model perspectives to a shared brief, then produces a chaired report. Compare disagreement as well as consensus.',
};
function ForumAsset() {
  // The shipped GLB is meshopt-compressed; drei installs no decoder we want
  // (its Draco path fetches from a CDN), so both built-ins are off and
  // `extendForumLoader` supplies three's own MeshoptDecoder.
  const { scene } = useGLTF(`${import.meta.env.BASE_URL}world/solar-forum.glb?v=${FORUM_ASSET_REVISION}`, false, false, extendForumLoader);
  const copy = useMemo(() => {
    const clone = scene.clone(true);
    clone.traverse(o => { if (o instanceof Mesh) { o.receiveShadow = true; o.castShadow = true; o.raycast = () => {}; } });
    return clone;
  }, [scene]);
  useEffect(() => applyForumSurfaceMaterials(copy), [copy]);
  useEffect(() => registerForumCollision(copy), [copy]);
  return <primitive object={copy} dispose={null} />;
}
function Fallback() {
  return <mesh rotation-x={-Math.PI / 2}><circleGeometry args={[21, 64]} /><meshStandardMaterial color={ENV.marble} /></mesh>;
}
// The app mounts every workspace at once and hides the inactive ones with
// `display: none` (App.tsx), so a World panel keeps its canvas mounted while
// another workspace is on screen. A zero-size box is the per-instance signal
// that this forum is not being shown: it covers workspace switches, the
// Settings/Skills overlays, a collapsed panel, and the pre-layout mount that
// would otherwise render at 0x0. Same approach as
// `world/atmosphere/useWorldVisibility`, with a guard so the component still
// works where ResizeObserver does not exist (jsdom, the standalone forum page
// in an old WebView) — there we assume on-screen rather than freeze the world.
function useOnScreen(ref: RefObject<HTMLElement | null>): boolean {
  const [onScreen, setOnScreen] = useState(typeof ResizeObserver === 'undefined');
  useEffect(() => {
    if (typeof ResizeObserver === 'undefined') return;
    const el = ref.current;
    if (!el) return;
    const observer = new ResizeObserver(entries => {
      const rect = entries[entries.length - 1].contentRect;
      setOnScreen(rect.width > 1 && rect.height > 1);
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, [ref]);
  return onScreen;
}
/** The corner control that puts a card away. Small, quiet, always in the same
 *  place, and labelled for a screen reader with the card it closes. */
function DismissCard({ label, onDismiss }: { label: string; onDismiss: () => void }) {
  const { colors } = useTheme();
  return (
    <Button
      colors={colors}
      variant="bare"
      className="forum-card-dismiss"
      aria-label={`Hide ${label}`}
      onClick={onDismiss}
    >
      <span aria-hidden="true">×</span>
    </Button>
  );
}

/** Drives the shared water clock so the lagoon moves. One number per frame. */
function ForumWater() {
  useFrame((_, delta) => advanceForumSurfaceClock(delta));
  useEffect(() => releaseForumSurfaceClock, []);
  // The baked depth and shoreline, handed to the shared water uniforms. It can
  // arrive after the materials have compiled; they hold the uniform objects by
  // reference, so it reaches them without a recompile, and a build without a
  // bake simply never turns it on.
  useEffect(() => {
    let cancelled = false;
    void loadWaterInfo(import.meta.env.BASE_URL).then((info) => {
      if (cancelled || !info) return;
      setForumWaterInfo(info.texture, boundsVector(info.manifest), info.manifest.depthRange, info.manifest.shoreRange);
    });
    return () => {
      cancelled = true;
      setForumWaterInfo(null, boundsVector({ bounds: { minX: 0, maxX: 1, minZ: 0, maxZ: 1 } } as never), 14, 12);
    };
  }, []);
  return null;
}

export function ForumView({ visible = true, onNavigate, onOpenGoal, onManageAgent }: { visible?: boolean; onNavigate?: (tool: ToolType) => void; onOpenGoal?: (projectId:string,id:string)=>void; onManageAgent?: (id:string)=>void }) {
  const { colors } = useTheme();
  const appearance = useSystemAppearance();
  // The agent wears the name its user gave it, everywhere. `useOrchestratorName`
  // reads that from the daemon; the store's `agentName` is the same name kept
  // live by `refreshIdentity`, and it is the one that survives when the identity
  // request itself fails — which is exactly when the desk was falling back to
  // the literal words "Your agent" and to the roster's placeholder "Agent".
  // 'Agent' is the store's own unset default, so it is treated as "not named
  // yet" rather than as a name.
  const storeAgentName = useCommandCenter((s) => s.agentName);
  const namedByUser = storeAgentName && storeAgentName !== 'Agent' ? storeAgentName : null;
  // Third source, and the one that actually answers when the daemon does not.
  // Both of the above read `/api/agent/identity`, which returns 401 against an
  // older daemon — which is exactly the state that left the desk saying "Your
  // agent". The identity store holds the name from the sovereign's own record
  // (the same one the passport shows) and persists it locally, so it survives a
  // daemon that cannot be reached.
  const passportName = useIdentityStore((s) => s.data?.name)?.trim() || null;
  const configuredName = useOrchestratorName() ?? namedByUser ?? passportName;
  const [pageVisible, setPageVisible] = useState(!document.hidden);
  useEffect(() => { const change = () => setPageVisible(!document.hidden); document.addEventListener('visibilitychange', change); return () => document.removeEventListener('visibilitychange', change); }, []);
  const worldRef = useRef<HTMLElement>(null);
  const [walking,setWalking] = useState(false);
  const onScreen = useOnScreen(worldRef);
  // `visible` is the host's own claim; `onScreen` is what the layout actually
  // gives this instance. Both must hold before the world renders or takes keys.
  const onPanel = visible && onScreen;
  // Leaving the panel ends walk mode, which unmounts ForumWalk — that removes
  // its window keydown/mousemove listeners and releases pointer lock, so a
  // hidden World cannot keep eating WASD from the workspace you switched to.
  useEffect(() => { if (!onPanel) setWalking(false); }, [onPanel]);
  const [focusAgent, setFocusAgent] = useState<{id:string;revision:number}|null>(null);
  const exitWalk = useCallback(()=>setWalking(false),[]);
  const inspectAgent = useCallback((id:string)=>{setSelected(id);setTab('meet');},[]);
  const [districtId, setDistrictId] = useState('commons');
  const district = DISTRICTS.find(d => d.id === districtId)!;
  // The Mesh Agora — the antechamber beyond the gate. It is a branch of this
  // one Canvas, not a tool or a tab, and everything it claims about the Mesh
  // comes from `shared/meshStatus.ts`.
  const [agoraOpen, setAgoraOpen] = useState(false);
  const [meshPrompt, setMeshPrompt] = useState(false);
  // Set when the walker is put down somewhere the selected district does not
  // imply — today only on the way back out of the Agora.
  const [walkFrom, setWalkFrom] = useState<[number,number,number]|null>(null);
  const districtStart = useMemo<[number,number,number]>(() => district.id==='commons' ? [0,0,14] : district.id==='gallery' ? [2,4.32,-20] : district.id==='mesh' ? gateApproachPoint() : [district.position[0]+2,district.position[1],district.position[2]+5], [district]);
  const walkStart = walkFrom ?? districtStart;
  const enterMesh = useCallback(()=>{ setMeshPrompt(false); setAgoraOpen(true); },[]);
  const leaveMesh = useCallback(()=>{
    setMeshPrompt(false); setAgoraOpen(false); setDistrictId('mesh');
    // A fresh array each time, so ForumWalk's placement effect re-runs and puts
    // the walker back down on the spur in front of the ring.
    setWalkFrom(gateApproachPoint());
  },[]);
  // Escape leaves the Agora. While walking, ForumWalk owns Escape and calls the
  // same handler; this listener covers the overview (non-walking) case.
  useEffect(()=>{
    if (!agoraOpen || walking) return;
    const key=(e:KeyboardEvent)=>{ if(e.code==='Escape'){ e.preventDefault(); leaveMesh(); } };
    window.addEventListener('keydown',key);
    return ()=>window.removeEventListener('keydown',key);
  },[agoraOpen,walking,leaveMesh]);
  // Leaving the panel closes the Agora with it, so returning to World never
  // lands in a room the user did not ask for.
  useEffect(()=>{ if(!onPanel) setAgoraOpen(false); },[onPanel]);
  const style = { '--forum-bg': colors.bg, '--forum-panel': colors.surface, '--forum-inset': colors.inputBg,
    '--forum-text': colors.text, '--forum-muted': colors.textMuted, '--forum-border': colors.border,
    '--forum-accent': colors.cyan, '--forum-accent-soft': colors.cyanSoft, '--forum-ink': colors.textOnCyan,
    '--forum-mono': font.mono, '--forum-display': font.display, '--forum-body': font.body,
    '--forum-radius': `${radius.sm}px`, '--forum-idle': STATE.idle, '--forum-bronze': ENV.bronze,
  } as CSSProperties;
  function selectDistrict(id: string) {
    // Selecting the gate a second time walks through it. This is the path for
    // anyone who never enters walk mode; the gate is not walk-only.
    if (id === 'mesh' && districtId === 'mesh' && !agoraOpen) { enterMesh(); return; }
    setWalking(false); const next = DISTRICTS.find(d=>d.id===id)!; setDistrictId(id); setSelected(next.agent); setTab('meet'); setSaved(false);
    setAgoraOpen(false); setWalkFrom(null);
  }

  const [selected, setSelected] = useState('henry');
  const [hovered, setHovered] = useState<string | null>(null);
  const [tab, setTab] = useState('meet');
  const [lesson, setLesson] = useState(0);
  const [brief, setBrief] = useState('');
  const [criteria, setCriteria] = useState('');
  const [kind, setKind] = useState('Query');
  const [saved, setSaved] = useState(false);
  const [queryError, setQueryError] = useState('');
  const { goals, loaded } = useActiveGoals();
  const profile = FORUM_AGENT_PROFILES[selected];
  const identity = ROSTER.find(a => a.id === selected)!;
  const agent = identity.isHenry && configuredName ? { ...identity, name: configuredName } : identity;
  function downloadBrief() {
    const content = `# ${kind} for ${agent.name}\n\nStatus: draft, not dispatched\nRequested specialist: ${agent.id}\n\n## Request\n${brief.trim()}\n\n## Acceptance criteria\n${criteria.trim() || 'Clarify with me before beginning.'}\n\n## Execution contract\nConfirm available tools and permissions. Return evidence, limitations, and any approval requests.\n`;
    const url = URL.createObjectURL(new Blob([content], { type: 'text/markdown' }));
    const a = document.createElement('a'); a.href = url; a.download = `forum-${agent.id}-brief.md`; a.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000); setSaved(true);
  }
  // The HUD rests when you are not using it and returns the moment you are.
  // The Agent Desk can be put away, so the world gets the whole window. The
  // choice is remembered: someone who explores with the desk closed should not
  // have to close it again on every visit. Walking opens the world up too, but
  // only as a default for a session that has not expressed a preference —
  // overriding a deliberate choice is worse than a slightly busy screen.
  // Flying is a third mode beside orbiting and walking. It is not a camera
  // trick: you board a craft, you fly it, and where you set it down is where
  // you continue on foot.
  const [flying, setFlying] = useState(false);
  const [railOpen, setRailOpen] = useState(() => {
    try { return localStorage.getItem(FORUM_RAIL_KEY) !== 'closed'; } catch { return true; }
  });
  const cards = useDismissible();
  // "Sovereign agent view": walking the world as the agent, or with the
  // sovereign selected at the desk.
  const sovereignInView = walking || flying || selected === 'henry';
  const [walkStartOverride, setWalkStartOverride] = useState<[number, number, number] | null>(null);
  const [deskOpen, setDeskOpen] = useState(() => {
    try {
      return localStorage.getItem(FORUM_DESK_KEY) !== 'closed';
    } catch {
      return true;
    }
  });
  const toggleDesk = useCallback(() => {
    setDeskOpen((open) => {
      const next = !open;
      try {
        localStorage.setItem(FORUM_DESK_KEY, next ? 'open' : 'closed');
      } catch {
        /* a private window still gets the toggle, just not the memory */
      }
      return next;
    });
  }, []);

  // The HUD rests when you are not driving; flying counts as driving.
  const hud = useHudPresence({ walking: walking || flying, enabled: !agoraOpen });
  return <main className="forum-shell" style={style} data-appearance={appearance} data-desk={deskOpen ? 'open' : 'closed'}>
    <section
      ref={worldRef}
      className="forum-world"
      aria-label="Interactive solarpunk forum"
      data-hud={hud.awake ? 'awake' : 'resting'}
      onPointerMove={hud.wake}
      onPointerDown={hud.wake}
      onWheel={hud.wake}
      onKeyDown={hud.wake}
      onTouchStart={hud.wake}
    >
      {/* Explicit shadow-map type. A bare `shadows` makes r3f set
          `PCFSoftShadowMap`, which three 0.184 deprecated: it warns and
          silently downgrades to `PCFShadowMap` on every shadow render — 39
          warnings per load in job 18. Asking for `PCFShadowMap` outright is
          the same shadows the scene has actually been getting, without the
          deprecation path. */}
      {/* `forum-canvas` makes the canvas host its own stacking context (see
          forum.css). The in-world place tags are drei <Html> overlays, which
          portal into this host with z-index up to 16777271 by default; with
          no context around them they out-ranked the HUD lanes and QA saw
          "Automate" drawn over the Places rail and "Build" over the job feed. */}
      <Canvas className="forum-canvas" frameloop={onPanel && pageVisible ? 'always' : 'never'} shadows={{ type: PCFShadowMap }} dpr={[1, 1.5]} camera={{ position: [...FORUM_VISTA.position] as [number,number,number], fov: FORUM_VISTA.fov, near: FORUM_CAMERA_NEAR, far: FORUM_CAMERA_FAR }} gl={{ antialias: true, toneMapping: AgXToneMapping, toneMappingExposure: FORUM_EXPOSURE }}>
        {agoraOpen ? <MeshAgora walking={walking} /> : <ForumLighting appearance={appearance} />}
        {/* The forum is hidden rather than unmounted while the Agora is open:
            an invisible group costs no draw calls, and keeping it mounted means
            the GLB is never fetched twice and its collision BVH — which the
            walker is still standing on out at the gate — is never rebuilt. */}
        <group visible={!agoraOpen}>
          <VaultBoundary fallback={<Fallback />}><Suspense fallback={<Fallback />}><ForumAsset /></Suspense></VaultBoundary>
          <ForumEnvironment appearance={appearance} />
          <ForumWater />
          <Suspense fallback={null}><ForumShips /></Suspense>
          <Suspense fallback={null}><ForumAgents hoveredAgent={hovered} onHoverAgent={setHovered} onSelectAgent={id => { setSelected(id); setTab('meet'); setSaved(false); }} /></Suspense>
        </group>
        {flying
          ? <Suspense fallback={null}><ForumFlight
              onExit={() => setFlying(false)}
              onDisembark={(at) => {
                // Step out onto the deck the skiff is standing on, a little
                // ahead of the hull so you are beside the craft, not inside it.
                setWalkStartOverride([at[0], at[1], at[2] + 4]);
                setFlying(false);
                setWalking(true);
              }}
            /></Suspense>
          : walking
          ? <ForumWalk start={walkStartOverride ?? walkStart} onExit={exitWalk} onInspect={inspectAgent} inMesh={agoraOpen} onEnterMesh={enterMesh} onExitMesh={leaveMesh} onMeshPrompt={setMeshPrompt} />
          : !agoraOpen && <ForumNavigation district={district} onDistrict={selectDistrict} focusAgent={focusAgent} />}
        <ForumPostProcessing />
        <PerfSampler />
      </Canvas>
      {/* The left edge is ONE column, not five boxes each pinned to the corner
          with its own hand-tuned offset. The place rail was added to that
          corner after the cards were, open by default and on top of them, and
          nothing about a set of absolute offsets can notice a newcomer: at
          1512x945 it hid the title card's "Living forum · Night" line and
          drew "Debating theatre" and "Harbor" over the place card and the job
          feed. In one flex column the rail takes whatever height is left
          between the title and the bottom cards, and scrolls inside that, so
          it cannot sit on any of them however tall the window is. */}
      <div className="forum-lane forum-lane-left">
      {!cards.isDismissed('brand') && <header className="forum-brand"><DismissCard label="the world title" onDismiss={()=>cards.dismiss('brand')} /><div><div className="eyebrow">WORLD / SOLAR FORUM</div><h1 style={typeRamp.display}>The Solar Forum</h1><p>Living forum · <span data-testid="forum-appearance">{appearance === 'night' ? 'Night' : 'Day'} / follows system appearance</span></p></div></header>}
      <ForumPlaceRail
        places={DISTRICTS.map(d => ({ id: d.id, name: d.name, subtitle: d.subtitle }))}
        activeId={districtId}
        open={railOpen}
        colors={colors}
        onSelect={selectDistrict}
        onToggle={(next) => {
          setRailOpen(next);
          try { localStorage.setItem(FORUM_RAIL_KEY, next ? 'open' : 'closed'); } catch { /* private window */ }
        }}
        onHold={hud.hold}
      />
      {!cards.isDismissed('place') && <div className="forum-location"><DismissCard label="the place card" onDismiss={()=>cards.dismiss('place')} /><span>{agoraOpen ? 'MESH AGORA' : district.name.toUpperCase()}</span><p>{agoraOpen ? 'The antechamber beyond the gate' : district.subtitle}</p></div>}
      {!cards.isDismissed('feed') && <div className="forum-live"><DismissCard label="the job feed" onDismiss={()=>cards.dismiss('feed')} />{loaded ? `${goals.length} active jobs reported` : 'Job feed unavailable / connecting'}<small>Architecture preview · existing World agent feeds</small></div>}
      {!cards.isDismissed('hints') && <div className="forum-controls"><DismissCard label="the control hints" onDismiss={()=>cards.dismiss('hints')} />{agoraOpen ? (walking ? 'Arrow keys walk · Walk back through the ring or press Esc to return to the forum' : 'Drag to orbit · Esc or Return to the Forum') : flying ? 'Arrow keys fly · Space climb · C descend · Shift boost · land on a terrace, then E to step out · Esc overview' : walking ? 'Arrow keys walk · Shift faster · Mouse / drag to look · E meet nearby agent · Esc overview' : 'Drag to orbit · Scroll to explore · Select a place or walk from this place'}</div>}
      </div>
      {meshPrompt && walking && !agoraOpen && <div className="forum-gate-prompt" role="status">E · Enter the MESH</div>}
      {cards.hiddenCount > 0 && (
        <div className="forum-restore" onPointerEnter={()=>hud.hold(true)} onPointerLeave={()=>hud.hold(false)}>
          <Button colors={colors} variant="ghost" flashSuccess={false} onClick={cards.restoreAll}>
            {`Show ${cards.hiddenCount} hidden card${cards.hiddenCount === 1 ? '' : 's'}`}
          </Button>
        </div>
      )}
      <div className="forum-desk-toggle">
        <Button
          colors={colors}
          variant="ghost"
          flashSuccess={false}
          aria-expanded={deskOpen}
          aria-controls="forum-agent-desk"
          onClick={toggleDesk}
        >
          {deskOpen ? 'Hide the desk' : 'Show the desk'}
        </Button>
      </div>
      {walking && <div className="forum-crosshair" aria-hidden="true">+</div>}
      {/* The bottom-right corner is one column too, for the same reason as the
          left edge: the passport (bottom:132px) was laid over the skiff button
          (bottom:118px), and the Agora's return button (bottom:126px) over the
          skiff as well, because each offset was chosen without the others.
          Stacked in one flow they sit above one another at any size. */}
      <div className="forum-lane forum-lane-right">
      {/* The passport belongs to the agent, so it appears where the agent does:
          while you are embodied in the world as your sovereign, or while the
          sovereign is the one selected at the desk. Everywhere else it is just
          a card sitting on the view. */}
      {sovereignInView && !cards.isDismissed('passport') && <div className="forum-passport-slot" onPointerEnter={()=>hud.hold(true)} onPointerLeave={()=>hud.hold(false)}><DismissCard label="the passport" onDismiss={()=>cards.dismiss('passport')} /><ForumPassport /></div>}
      {agoraOpen && <div className="forum-agora-return"><Button colors={colors} variant="primary" flashSuccess={false} onClick={leaveMesh}>Return to the Forum</Button></div>}
      <div className="forum-fly"><Button colors={colors} variant={flying?'ghostOn':'ghost'} flashSuccess={false} onClick={()=>{
        if (flying) { setFlying(false); return; }
        setWalking(false);
        setFlying(true);
        const canvas=worldRef.current?.querySelector('canvas');
        if(canvas){canvas.tabIndex=0;canvas.focus({preventScroll:true});}
      }}>{flying?'Leave the skiff':'Fly the skiff'}</Button></div>
      <div className="forum-play"><Button colors={colors} variant={walking?'ghost':'primary'} flashSuccess={false} onClick={()=>{
        if(walking){setWalking(false);return;}
        setWalking(true);
        const canvas=worldRef.current?.querySelector('canvas');
        if(canvas){canvas.tabIndex=0;canvas.focus({preventScroll:true});}
        // Native WebViews may not support pointer lock. Drag + arrow look stays
        // available, so exploration does not depend on that browser feature.
        try { const result=canvas?.requestPointerLock?.(); if(result && typeof result.catch==='function')void result.catch(()=>{}); } catch { /* drag look remains available */ }
      }}>{walking?'Return to overview':'Walk the world'}</Button></div>
      </div>
    </section>
    <aside className="forum-panel" id="forum-agent-desk" aria-label="Agent learning and task desk" aria-hidden={!deskOpen}>
      <div className="eyebrow">AGENT DESK</div><h2 style={typeRamp.title}>Meet your collaborators.</h2>
      <p className="intro">{district.description}</p>
      {onNavigate && district.tool && district.id !== 'commons' && <Button colors={colors} variant="ghost" onClick={()=>onNavigate(district.tool!)} flashSuccess={false}>Open {district.name} ↗</Button>}
      {district.id === 'mesh' && <div className="note"><b>The MESH gate</b><p>Mesh is Permagent's peer network. Walk out through the ring, or enter from here. The Agora reports the real connection state and nothing else — there is no live Mesh to connect to today.</p><Button colors={colors} variant={agoraOpen?'ghost':'primary'} flashSuccess={false} onClick={agoraOpen?leaveMesh:enterMesh}>{agoraOpen ? 'Return to the Forum' : 'Enter the MESH ↗'}</Button></div>}
      <div className="note"><b>{configuredName || 'Your agent'}</b><p>Your sovereign orchestrator · leads your team</p><Button colors={colors} flashSuccess={false} onClick={()=>{
        setWalking(false); setSelected('henry'); setTab('meet'); setSaved(false);
        setFocusAgent(previous=>({id:'henry',revision:(previous?.revision??0)+1}));
      }}>Find {configuredName || 'your orchestrator'}</Button></div>
      <label className="field-label" htmlFor="forum-agent">Choose an agent</label>
      <select id="forum-agent" value={selected} onChange={e => { setSelected(e.target.value); setSaved(false); }}>{ROSTER.map(a => <option key={a.id} value={a.id}>{a.isHenry ? `${configuredName || 'Your agent'} · Sovereign orchestrator` : a.name}</option>)}</select>
      <nav className="forum-tabs" aria-label="Agent desk">{['meet','learn','task','conversation'].map(t => <Button colors={colors} flashSuccess={false} key={t} aria-pressed={tab===t} onClick={() => setTab(t)}>{t==='meet' ? 'Meet' : t==='learn' ? 'Learn' : t==='task' ? 'Give a brief' : 'Conversation'}</Button>)}</nav>
      {tab==='meet' && <section><div className="forum-agent-portrait" style={{borderColor:agent.trimColor}}><img key={agent.id} src={`${import.meta.env.BASE_URL}world/forum-characters/${agent.id}.png`} alt={profile.signature} onError={e=>{e.currentTarget.style.visibility='hidden';}} /><div><span className="eyebrow">{profile.archetype}</span><h3>{agent.name}</h3></div></div><p>{profiles[selected] ?? `${agent.name} is part of the existing World roster. Its dedicated panel in the main World describes its capabilities and operating limits.`}</p><div className="forum-character"><b>Working style</b><p>{profile.manner}</p><p className="forum-voice">“{profile.voice}”</p><Button colors={colors} flashSuccess={false} onClick={()=>{setBrief(profile.question);setKind('Query');setTab('task');setSaved(false);}}>Try a question for this role ↗</Button></div><div className="note"><b>What does its presence mean?</b><p>{agent.wire==='daemon' ? 'This agent has a daemon event source. Its presence alone does not confirm a connection or a running job.' : agent.wire==='sim' ? 'Ambient presence. Idle movement is simulated; it is not evidence of a running job.' : 'Static presence. This character has no live activity emitter.'}</p></div><Button colors={colors} flashSuccess={false} variant="primary" className="forum-primary" onClick={() => setTab('learn')}>How agents work ↗</Button></section>}
      {tab==='learn' && <section><div className="lesson-count">FIELD GUIDE / 0{lesson+1}</div><h3>{lessons[lesson][0]}</h3><p>{lessons[lesson][1]}</p><div className="lesson-steps">{lessons.map((l,i) => <Button colors={colors} flashSuccess={false} key={l[0]} aria-label={l[0]} aria-pressed={lesson===i} onClick={() => setLesson(i)}>{i+1}</Button>)}</div><div className="note"><b>Try asking {agent.name}</b><p>{lessons[lesson][2]}</p></div><Button colors={colors} flashSuccess={false} variant="primary" className="forum-primary" onClick={() => { setBrief(lessons[lesson][2]); setKind('Query'); setSaved(false); setTab('task'); }}>Use this question ↗</Button></section>}
      {tab==='task' && <section><h3>A clear brief is a good beginning.</h3><p>Prepare a query or job for {agent.name}. Send a question, job or capability request to your orchestrator through the shared conversation. Execution and approvals use the existing runtime; exporting alone does not dispatch work.</p><label className="field-label" htmlFor="forum-kind">Type</label><select id="forum-kind" value={kind} onChange={e => { setKind(e.target.value); setSaved(false); }}><option>Query</option><option>Job</option><option>Capability</option></select><label className="field-label" htmlFor="forum-brief">What would you like to achieve?</label><textarea id="forum-brief" value={brief} placeholder="Describe the outcome and context…" onChange={e => { setBrief(e.target.value); setSaved(false); }} /><label className="field-label" htmlFor="forum-criteria">What would count as done?</label><textarea id="forum-criteria" className="short" value={criteria} placeholder="An artifact, a cited answer, a passing check…" onChange={e => { setCriteria(e.target.value); setSaved(false); }} /><Button colors={colors} flashSuccess={false} variant="primary" className="forum-primary" disabled={!brief.trim()} onClick={async()=>{
        setQueryError('');
        try { await askFromForum(brief, configuredName && agent.isHenry ? configuredName : agent.name, criteria, profile.voice, kind==='Job'?'job':kind==='Capability'?'capability':'query'); setTab('conversation'); }
        catch (error) { setQueryError(error instanceof Error ? error.message : 'Could not send your question.'); return false; }
      }}>{kind==='Query'?'Ask':kind==='Job'?'Ask to run job ·':'Develop capability with'} {configuredName || 'your orchestrator'}</Button>
      {queryError && <p role="alert">{queryError}</p>}
      <Button colors={colors} flashSuccess={false} variant="primary" className="forum-primary" disabled={!brief.trim()} onClick={downloadBrief}>Export brief ↓</Button><p role="status" className="save-state">{saved ? 'Brief exported. No job was dispatched.' : 'Export keeps this as a draft. Send requests work through your orchestrator.'}</p></section>}
      {tab==='conversation' && <ForumConversation />}
      <section className="forum-work"><div className="eyebrow">WORK IN FLIGHT</div>{!loaded ? <p>Waiting for the job feed. No activity is inferred.</p> : goals.length === 0 ? <p>No active jobs reported.</p> : goals.map(g=><div className="forum-job" key={g.id}><b>{g.title}</b><span>{g.state.replace(/_/g,' ')}</span>{onOpenGoal&&g.project_id&&<Button colors={colors} flashSuccess={false} onClick={()=>onOpenGoal(g.project_id!,g.id)}>Open job</Button>}</div>)}</section>
      <ForumCapabilities onManageAgent={onManageAgent} onSkills={onNavigate?()=>onNavigate('skills'):undefined} onDevelop={()=>{
        setSelected('henry');setKind('Capability');setTab('task');setSaved(false);
        setBrief('Using our conversation and existing goals, identify a missing capability that would help. Build and test it with the existing tools, preserve the established permissions, and save a reusable skill with evidence of what it can do.');
        setCriteria('Demonstrate the capability on a representative task; record tests, limitations and the registered skill.');
      }}/>
      <footer>WORLD / SOLAR FORUM <span>Architecture study</span></footer>
    </aside>
  </main>;
}
