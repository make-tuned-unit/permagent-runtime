import { useEffect, useRef } from 'react';
import { getApiBaseUrl } from '../lib/api';
import { wireEventType } from '../lib/wireEvent';
import { useCommandCenter, navigateToTool } from '../lib/store';
import type { ActivePanel } from '../lib/store';
import { createChatWindow } from '../lib/chatWindow';
import { useTheme } from '../styles/useTheme';

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

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

const VALID_TOOL_TYPES = new Set<string>([
  'chat', 'skills', 'trace', 'world', 'terminal', 'browser', 'memory', 'dashboard', 'build', 'automate', 'projects', 'grow',
]);

/**
 * Subscribes to the daemon's global event bus (/events WebSocket).
 * When an AppNavigate event arrives, navigates the UI to the specified tab.
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
  const navigate = (payload: {
    tool_type?: string;
    panel_type?: string;
    section?: string;
    state?: { project_id?: string };
    reason?: string;
  }) => {
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
  const launch = (payload: {
    root_path?: string;
    label?: string;
    command?: string | null;
    reason?: string;
  }) => {
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
      ws = new WebSocket(`${base}/events`);

      ws.onmessage = (ev) => {
        try {
          const event = JSON.parse(ev.data);
          const eventType = wireEventType(event);
          if (eventType === 'project_launch') {
            launchRef.current(event.payload ?? {});
            return;
          }
          if (eventType === 'app_action') {
            dispatchAppAction(event.payload ?? {}, themeRef.current);
            return;
          }
          if (eventType === 'app_open_item') {
            dispatchOpenItem(event.payload ?? {});
            return;
          }
          if (eventType !== 'app_navigate') return;
          navigateRef.current(event.payload ?? {});
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
  useEffect(() => {
    if (!('__TAURI_INTERNALS__' in window)) return;
    let unlisten: (() => void) | undefined;
    let disposed = false;
    (async () => {
      try {
        const { listen } = await import('@tauri-apps/api/event');
        const stop = await listen<{
          tool_type?: string;
          panel_type?: string;
          section?: string;
          state?: { project_id?: string };
          reason?: string;
        }>(
          'app_navigate',
          (ev) => navigateRef.current(ev.payload ?? {}),
        );
        const stopLaunch = await listen<{ root_path?: string; label?: string; command?: string | null; reason?: string }>(
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
