import { useEffect, useRef } from 'react';
import { getApiBaseUrl } from '../lib/api';
import { wireEventType } from '../lib/wireEvent';
import { appendTraceRecord, claimTraceEventId, globalFrameToRecord } from '../lib/traceEvents';
import { useCommandCenter, navigateToTool } from '../lib/store';
import type { ActivePanel } from '../lib/store';
import { createChatWindow } from '../lib/chatWindow';
import { useTheme } from '../styles/useTheme';
// The world view's replay classifier — the one in-repo source of truth for
// "did this /events frame arrive live, or in the daemon's replay buffer?".
import { isReplayed } from '../components/world/shared/worldEvents';

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

/** Payload of an `app_navigate` frame (daemon WS bus and cross-window Tauri event). */
type NavigatePayload = {
  tool_type?: string;
  panel_type?: string;
  section?: string;
  state?: { project_id?: string };
  reason?: string;
};

/** Payload of a `project_launch` frame (daemon WS bus and cross-window Tauri event). */
type LaunchPayload = {
  root_path?: string;
  label?: string;
  command?: string | null;
  reason?: string;
};

/**
 * Agent-driven app_action: act WITHIN a surface (not just navigate to it).
 * Mirrors the daemon's app_conductor ACTION_CATALOG — surface/action pairs are
 * validated daemon-side before emit, so this only maps known pairs to the store
 * actions that already back the human buttons. Reads live state imperatively via
 * getState() so the WebSocket callback never goes stale.
 */
function dispatchAppAction(
  payload: { surface?: string; action?: string; reason?: string },
  theme: ReturnType<typeof useTheme>['theme'],
) {
  const { surface, action } = payload ?? {};
  if (!surface || !action) return;
  const s = useCommandCenter.getState();

  if (surface === 'chat') {
    if (action === 'open') s.openChatDock();
    else if (action === 'close') s.closeChatDock();
    else if (action === 'detach') {
      s.closeChatDock();
      if (isTauri) createChatWindow(theme).catch(() => { /* keep dock closed; user can reopen */ });
    }
  } else if (surface === 'build') {
    // Acting on a Build pane only makes sense with the Build tab visible.
    navigateToTool('build');
    const st = useCommandCenter.getState();
    if (action === 'hide_browser' && !st.buildBrowserHidden) st.toggleBuildBrowser();
    else if (action === 'show_browser' && st.buildBrowserHidden) st.toggleBuildBrowser();
    else if (action === 'hide_terminal' && !st.buildTerminalHidden) st.toggleBuildTerminal();
    else if (action === 'show_terminal' && st.buildTerminalHidden) st.toggleBuildTerminal();
  }

  if (payload?.reason) showNavigationCue(payload.reason);
}

/**
 * Agent-driven open_item: the last mile past a tab — open a SPECIFIC item by id.
 * Mirrors the daemon's app_conductor ITEM_CATALOG 1:1 (kind is validated
 * daemon-side before emit), so this only maps known kinds to the store seams
 * that already back the human buttons (goal → openGoalDetail, grow →
 * growProject). Both seams self-navigate to their surface, so no extra nav here.
 * Reads live state imperatively via getState() so the WS callback never goes stale.
 *
 * Exported for wiring tests. Keep the handled kinds in lockstep with the Rust
 * ITEM_CATALOG (app_conductor.rs) — a kind on one side but not the other is a
 * silently-dropped event.
 */
export function dispatchOpenItem(payload: {
  kind?: string;
  project_id?: string;
  card_id?: string;
  reason?: string;
}) {
  const { kind, project_id, card_id } = payload ?? {};
  if (!kind || !project_id) return;
  const s = useCommandCenter.getState();

  if (kind === 'goal') {
    // Goal-detail modal needs both ids; a goal without its card_id is unopenable
    // (the daemon rejects this too, but guard the direct/cross-window path).
    if (!card_id) return;
    s.openGoalDetail(project_id, card_id);
  } else if (kind === 'grow') {
    s.growProject(project_id);
  } else {
    return; // unknown kind — drop rather than guess
  }

  if (payload?.reason) showNavigationCue(payload.reason);
}

// Tool types that a workspace layout can host — the ONLY set the tool-host
// branch below searches. Overlay-only surfaces (inbox, sessions, trace) are
// deliberately absent: no seeded workspace hosts them, so their catalog entry
// carries panel_type:'overlay' and they route through the overlay branch
// (setActivePanel) instead. Listing 'trace' here made navigate_app("Trace")
// fall into the tool-host search, which no-ops because nothing hosts it.
const VALID_TOOL_TYPES = new Set<string>([
  'chat', 'skills', 'world', 'terminal', 'browser', 'memory', 'dashboard', 'build', 'automate', 'projects', 'grow',
]);

/**
 * Record a global `/events` frame into the trace buffer (`store.events`) with
 * its real wire type — the ExecutionTrace's global-bus feed. Before this seam
 * the trace's only writer was the chat SSE, so it showed the user's own chat
 * frames while the Trace catalog entry promised the whole running system. The
 * daemon replays its buffer on every (re)connect, so records are deduped by
 * envelope id (first connect lands recent history once; reconnect bursts are
 * dropped). Exported for wiring tests.
 */
export function recordGlobalTraceFrame(frame: unknown): void {
  const rec = globalFrameToRecord(frame);
  if (!rec || !claimTraceEventId(rec.id)) return;
  useCommandCenter.setState(s => ({ events: appendTraceRecord(s.events, rec) }));
}

/**
 * The wire types the /events handler ACTS on (navigate / act-in-surface /
 * open-item / launch-terminal) — every other frame is trace-only. One set, so
 * the replay gate below cannot drift out of sync with the routing.
 */
const ACTION_FRAME_TYPES = new Set([
  'app_navigate', 'app_action', 'app_open_item', 'project_launch',
]);

/**
 * Replay honesty for ACTION frames. The daemon's /events socket replays its
 * whole event buffer (≤1000 frames) on EVERY (re)connect with no server-side
 * replay marker (routes/events.rs streams `buffered_events()` before the live
 * subscription; frames are serialized identically either way). Without
 * discrimination, a sleep/wake or daemon-restart reconnect re-delivers an
 * hours-old `app_navigate` and the UI yanks the user.
 *
 * Discriminator — a per-CONNECTION epoch over the daemon's own timestamps,
 * through the same classifier the world view uses ({@link isReplayed}): a
 * frame stamped before THIS connection opened is history, not a live
 * instruction. That classifies the entire replay burst as stale (buffered
 * frames always predate the connection that delivers them) AND frames emitted
 * while we were disconnected — a navigation emitted mid-sleep is stale on
 * wake however unseen it is. This is the one deliberate departure from
 * worldEvents' once-per-app epoch: the world may still animate
 * never-witnessed work after a reconnect; an action must only fire for a
 * frame emitted while this client was actually connected.
 *
 * Clock-skew caveat (the same one worldEvents already accepts): server stamp
 * vs client clock. Both failure directions are bounded by the skew (NTP:
 * sub-second) and benign — a live action inside the first skew-window of a
 * connection is recorded but not acted on; a skew-old action acts, which is
 * indistinguishable from live. Untimestamped frames count as live (the daemon
 * stamps every event; absence means a test/dev frame — worldEvents' rule).
 *
 * Exported for wiring tests.
 */
export function shouldActOnFrame(event: unknown, connectionEpoch: number): boolean {
  if (!ACTION_FRAME_TYPES.has(wireEventType(event))) return false;
  const ts =
    event && typeof event === 'object'
      ? (event as { timestamp?: unknown }).timestamp
      : undefined;
  return !isReplayed(typeof ts === 'string' ? ts : undefined, connectionEpoch);
}

/**
 * Route one parsed /events frame: ALWAYS record it into the trace first
 * (deduped by envelope id, and a dispatch throw must not lose the record),
 * then dispatch to a UI action only when {@link shouldActOnFrame} says it is
 * live for this connection — replayed frames are recorded, never acted on.
 * Exported for wiring tests.
 */
export function routeGlobalFrame(
  event: unknown,
  connectionEpoch: number,
  handlers: {
    navigate: (payload: NavigatePayload) => void;
    launch: (payload: LaunchPayload) => void;
    theme: ReturnType<typeof useTheme>['theme'];
  },
): void {
  recordGlobalTraceFrame(event);
  if (!shouldActOnFrame(event, connectionEpoch)) return;
  const payload =
    (event && typeof event === 'object'
      ? (event as { payload?: unknown }).payload
      : undefined) ?? {};
  switch (wireEventType(event)) {
    case 'project_launch':
      handlers.launch(payload as LaunchPayload);
      break;
    case 'app_action':
      dispatchAppAction(payload as Parameters<typeof dispatchAppAction>[0], handlers.theme);
      break;
    case 'app_open_item':
      dispatchOpenItem(payload as Parameters<typeof dispatchOpenItem>[0]);
      break;
    case 'app_navigate':
      handlers.navigate(payload as NavigatePayload);
      break;
  }
}

/**
 * Subscribes to the daemon's global event bus (/events WebSocket).
 * When an AppNavigate event arrives LIVE, navigates the UI to the specified
 * tab; every frame (nav or not, live or replayed) is also mirrored into the
 * trace events buffer with its real type via {@link recordGlobalTraceFrame},
 * so the Execution trace genuinely shows the running system, not just chat
 * frames. Frames from the daemon's replay-on-(re)connect buffer are recorded
 * but never acted on — see {@link shouldActOnFrame}.
 */
export function useAppNavigate() {
  const switchWorkspace = useCommandCenter(s => s.switchWorkspace);
  const setActivePanel = useCommandCenter(s => s.setActivePanel);
  const workspaces = useCommandCenter(s => s.workspaces);
  const setPendingProjectNavigation = useCommandCenter(s => s.setPendingProjectNavigation);
  const setPendingSettingsSection = useCommandCenter(s => s.setPendingSettingsSection);
  const setPendingTerminalLaunch = useCommandCenter(s => s.setPendingTerminalLaunch);
  const { theme } = useTheme();

  // Keep refs so the WebSocket callback always sees latest state
  const themeRef = useRef(theme);
  themeRef.current = theme;
  const workspacesRef = useRef(workspaces);
  workspacesRef.current = workspaces;
  const switchWorkspaceRef = useRef(switchWorkspace);
  switchWorkspaceRef.current = switchWorkspace;
  const setActivePanelRef = useRef(setActivePanel);
  setActivePanelRef.current = setActivePanel;
  const setPendingProjectNavigationRef = useRef(setPendingProjectNavigation);
  setPendingProjectNavigationRef.current = setPendingProjectNavigation;
  const setPendingSettingsSectionRef = useRef(setPendingSettingsSection);
  setPendingSettingsSectionRef.current = setPendingSettingsSection;
  const setPendingTerminalLaunchRef = useRef(setPendingTerminalLaunch);
  setPendingTerminalLaunchRef.current = setPendingTerminalLaunch;

  // Shared navigation logic — used by both the daemon WS bus and the
  // cross-window Tauri event (the chat window's "Open Brain" button).
  const navigate = (payload: NavigatePayload) => {
    const { tool_type, panel_type, section, state, reason } = payload ?? {};
    if (!tool_type) return;

    if (panel_type === 'overlay') {
      // Deep-link into a sub-section (e.g. Settings → Devices). The target
      // overlay reads pendingSettingsSection on mount. Without this the daemon-
      // forwarded `section` was dropped and every deep-link fell to the default.
      if (section) setPendingSettingsSectionRef.current(section);
      setActivePanelRef.current(tool_type as ActivePanel);
    } else if (VALID_TOOL_TYPES.has(tool_type)) {
      // Find workspace containing this tool type
      const ws = workspacesRef.current.find(w => hasToolType(w.layoutJson, tool_type));
      if (ws) {
        // Close any overlay first
        setActivePanelRef.current('chat');
        switchWorkspaceRef.current(ws.id);

        // If navigating to projects with a specific project_id, queue drill-in
        if (tool_type === 'projects' && state?.project_id) {
          setPendingProjectNavigationRef.current(state.project_id);
        }
      }
    }

    // Show brief navigation cue
    if (reason) {
      showNavigationCue(reason);
    }
  };
  const navigateRef = useRef(navigate);
  navigateRef.current = navigate;

  // Agent project_launch → open a project-aware terminal in the Build tab.
  // Mirrors the human launch path: switch to the Build workspace, then queue a
  // pending launch that BuildView consumes via its TerminalManager ref
  // (createProjectTab → terminal.rs PTY). The agent never spawns the PTY itself.
  const launch = (payload: LaunchPayload) => {
    const { root_path, label, command, reason } = payload ?? {};
    if (!root_path) return;
    if (!navigateToTool('build')) return;
    setPendingTerminalLaunchRef.current({
      rootPath: root_path,
      label: label || root_path,
      command: command ?? undefined,
    });
    if (reason) showNavigationCue(reason);
  };
  const launchRef = useRef(launch);
  launchRef.current = launch;

  useEffect(() => {
    let ws: WebSocket | null = null;
    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
    let disposed = false;

    function connect() {
      if (disposed) return;
      const base = getApiBaseUrl().replace(/^http/, 'ws');
      // Per-connection replay epoch: captured fresh on every (re)connect so
      // the daemon's replay burst — and anything emitted while we were
      // disconnected — is recorded but never acted on (see shouldActOnFrame).
      const connectionEpoch = Date.now();
      ws = new WebSocket(`${base}/events`);

      ws.onmessage = (ev) => {
        try {
          routeGlobalFrame(JSON.parse(ev.data), connectionEpoch, {
            navigate: (p) => navigateRef.current(p),
            launch: (p) => launchRef.current(p),
            theme: themeRef.current,
          });
        } catch {
          // Ignore malformed events
        }
      };

      ws.onclose = () => {
        if (!disposed) {
          reconnectTimer = setTimeout(connect, 3000);
        }
      };

      ws.onerror = () => {
        ws?.close();
      };
    }

    connect();

    return () => {
      disposed = true;
      if (reconnectTimer) clearTimeout(reconnectTimer);
      ws?.close();
    };
  }, []);

  // Cross-window navigation: the chat webview (a separate window with its own
  // store) can't switch workspaces directly, so its "Open Brain" button emits a
  // global Tauri event that the main window honors via the shared nav logic.
  // No replay gate here — Tauri events are direct user actions delivered once,
  // live by construction; only the daemon WS bus replays a buffer.
  useEffect(() => {
    if (!('__TAURI_INTERNALS__' in window)) return;
    let unlisten: (() => void) | undefined;
    let disposed = false;
    (async () => {
      try {
        const { listen } = await import('@tauri-apps/api/event');
        const stop = await listen<NavigatePayload>(
          'app_navigate',
          (ev) => navigateRef.current(ev.payload ?? {}),
        );
        const stopLaunch = await listen<LaunchPayload>(
          'project_launch',
          (ev) => launchRef.current(ev.payload ?? {}),
        );
        const stopAction = await listen<{ surface?: string; action?: string; reason?: string }>(
          'app_action',
          (ev) => dispatchAppAction(ev.payload ?? {}, themeRef.current),
        );
        const stopOpenItem = await listen<{ kind?: string; project_id?: string; card_id?: string; reason?: string }>(
          'app_open_item',
          (ev) => dispatchOpenItem(ev.payload ?? {}),
        );
        const stopBoth = () => { stop(); stopLaunch(); stopAction(); stopOpenItem(); };
        if (disposed) stopBoth(); else unlisten = stopBoth;
      } catch { /* not in Tauri / event API unavailable */ }
    })();
    return () => { disposed = true; unlisten?.(); };
  }, []);
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function hasToolType(node: any, toolType: string): boolean {
  if (node?.type === 'panel') return node.tool === toolType;
  if (node?.type === 'split' && Array.isArray(node.children)) {
    return node.children.some((c: any) => hasToolType(c, toolType));
  }
  return false;
}

/** Briefly flash a navigation indicator. Uses a simple DOM toast. */
function showNavigationCue(reason: string) {
  const el = document.createElement('div');
  el.textContent = reason;
  Object.assign(el.style, {
    position: 'fixed',
    bottom: '24px',
    left: '50%',
    transform: 'translateX(-50%)',
    background: 'rgba(0, 217, 255, 0.15)',
    color: '#00D9FF',
    border: '1px solid #0891B2',
    borderRadius: '8px',
    padding: '8px 16px',
    fontSize: '13px',
    fontFamily: 'monospace',
    zIndex: '99999',
    transition: 'opacity 0.3s',
    pointerEvents: 'none',
  });
  document.body.appendChild(el);
  setTimeout(() => {
    el.style.opacity = '0';
    setTimeout(() => el.remove(), 300);
  }, 3000);
}
