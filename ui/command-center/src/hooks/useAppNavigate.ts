import { useEffect, useRef } from 'react';
import { getApiBaseUrl } from '../lib/api';
import { wireEventType } from '../lib/wireEvent';
import { useCommandCenter } from '../lib/store';
import type { ActivePanel } from '../lib/store';

const VALID_TOOL_TYPES = new Set<string>([
  'chat', 'skills', 'trace', 'world', 'terminal', 'browser', 'memory', 'dashboard', 'build', 'automate', 'projects',
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

  // Keep refs so the WebSocket callback always sees latest state
  const workspacesRef = useRef(workspaces);
  workspacesRef.current = workspaces;
  const switchWorkspaceRef = useRef(switchWorkspace);
  switchWorkspaceRef.current = switchWorkspace;
  const setActivePanelRef = useRef(setActivePanel);
  setActivePanelRef.current = setActivePanel;
  const setPendingProjectNavigationRef = useRef(setPendingProjectNavigation);
  setPendingProjectNavigationRef.current = setPendingProjectNavigation;

  // Shared navigation logic — used by both the daemon WS bus and the
  // cross-window Tauri event (the chat window's "Open Brain" button).
  const navigate = (payload: {
    tool_type?: string;
    panel_type?: string;
    state?: { project_id?: string };
    reason?: string;
  }) => {
    const { tool_type, panel_type, state, reason } = payload ?? {};
    if (!tool_type) return;

    if (panel_type === 'overlay') {
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
        const stop = await listen<{ tool_type?: string; reason?: string }>(
          'app_navigate',
          (ev) => navigateRef.current(ev.payload ?? {}),
        );
        if (disposed) stop(); else unlisten = stop;
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
