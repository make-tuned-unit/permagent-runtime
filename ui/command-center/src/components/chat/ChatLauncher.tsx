import { useEffect, useLayoutEffect, useRef, useState, useCallback } from 'react';
import { font, ease } from '../../styles/tokens';
import { api } from '../../lib/api';
import { useTheme } from '../../styles/useTheme';
import { useCommandCenter } from '../../lib/store';

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

// Distance from the viewport's bottom/right edges. The Browser derives the
// launcher's reserved corner from this anchor + the published size (#553).
export const CHAT_LAUNCHER_MARGIN = 20;

export function ChatLauncher() {
  const { colors } = useTheme();
  const [agentName, setAgentName] = useState('Agent');
  const [chatWindowOpen, setChatWindowOpen] = useState(false);
  const [hovered, setHovered] = useState(false);
  const [pressed, setPressed] = useState(false);
  const setChatLauncherSize = useCommandCenter(s => s.setChatLauncherSize);
  const buttonRef = useRef<HTMLButtonElement>(null);

  // Publish the pill's measured size so the Browser can subtract its corner
  // from the native webview bounds (#553). ResizeObserver fires only on real
  // layout changes (e.g. the agent name loading) — no polling.
  useLayoutEffect(() => {
    const el = buttonRef.current;
    if (chatWindowOpen || !el) {
      setChatLauncherSize(null);
      return;
    }
    const publish = () => {
      const r = el.getBoundingClientRect();
      setChatLauncherSize({ width: r.width, height: r.height });
    };
    publish();
    const observer = new ResizeObserver(publish);
    observer.observe(el);
    return () => {
      observer.disconnect();
      setChatLauncherSize(null);
    };
  }, [chatWindowOpen, setChatLauncherSize]);

  useEffect(() => {
    api.getIdentity().then(id => setAgentName(id.first_name)).catch(() => {});
  }, []);

  // Check if chat window is already open on mount (handles main window reload)
  useEffect(() => {
    if (!isTauri) return;
    (async () => {
      try {
        const { WebviewWindow } = await import('@tauri-apps/api/webviewWindow');
        const existing = await WebviewWindow.getByLabel('chat');
        if (existing) setChatWindowOpen(true);
      } catch { /* ignore */ }
    })();
  }, []);

  // React to the chat window closing (e.g. user hits the traffic-light) via its
  // close event instead of polling once a second — no timer, fires immediately.
  useEffect(() => {
    if (!isTauri || !chatWindowOpen) return;
    let unlisten: (() => void) | undefined;
    let disposed = false;
    (async () => {
      try {
        const { WebviewWindow } = await import('@tauri-apps/api/webviewWindow');
        const existing = await WebviewWindow.getByLabel('chat');
        if (!existing) {
          setChatWindowOpen(false);
          return;
        }
        const un = await existing.onCloseRequested(() => setChatWindowOpen(false));
        if (disposed) un(); else unlisten = un;
      } catch {
        setChatWindowOpen(false);
      }
    })();
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [chatWindowOpen]);

  // While the chat window is open, re-assert its stacking just above the main
  // window whenever the main window gains focus. The chat window is independent
  // (not a child of main — so it can fullscreen/tile and doesn't move with main),
  // so the native `raise_chat_above_main` command re-orders it above main without
  // stealing focus, keeping it above main's browser child-webview (#461/#477).
  useEffect(() => {
    if (!isTauri || !chatWindowOpen) return;
    let unlisten: (() => void) | undefined;
    (async () => {
      try {
        const { getCurrentWindow } = await import('@tauri-apps/api/window');
        const { invoke } = await import('@tauri-apps/api/core');
        unlisten = await getCurrentWindow().onFocusChanged(({ payload: focused }) => {
          if (focused) invoke('raise_chat_above_main').catch(() => {});
        });
      } catch { /* non-Tauri or unavailable — graceful */ }
    })();
    return () => { unlisten?.(); };
  }, [chatWindowOpen]);

  const openChatDock = useCommandCenter(s => s.openChatDock);
  const openChatWindow = useCallback(async () => {
    // Dock-first (2026-07-11): the pill opens the right-side sidebar. If the
    // user has already DETACHED chat to its own window, focus that instead.
    if (!isTauri) {
      openChatDock();
      return;
    }
    try {
      const { WebviewWindow } = await import('@tauri-apps/api/webviewWindow');

      const existing = await WebviewWindow.getByLabel('chat');
      if (existing) {
        await existing.show();
        // The chat window is independent; the main-window focus listener
        // re-asserts its stacking above main (see the onFocusChanged effect and
        // main.rs). Opening it is an explicit user action, so focus it now.
        await existing.setFocus();
        setChatWindowOpen(true);
        return;
      }

      // No detached window → open the dock (the new default surface).
      openChatDock();
    } catch (e) {
      console.error('Failed to open chat:', e);
      openChatDock();
    }
  }, [openChatDock]);

  if (chatWindowOpen) return null;

  return (
    <button
      ref={buttonRef}
      onClick={openChatWindow}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => { setHovered(false); setPressed(false); }}
      onMouseDown={() => setPressed(true)}
      onMouseUp={() => setPressed(false)}
      style={{
      position: 'fixed', bottom: CHAT_LAUNCHER_MARGIN, right: CHAT_LAUNCHER_MARGIN, zIndex: 9999,
      display: 'flex', alignItems: 'center', gap: 10,
      padding: '12px 20px', borderRadius: 999,
      background: colors.surface, backdropFilter: 'blur(16px)',
      // Theme-safe elevation — a cool soft shadow on silver, deep on dark
      // (the hardcoded black glow was invisible on the light themes).
      border: `1px solid ${hovered ? colors.cyan : colors.borderHi}`,
      color: colors.cyan, cursor: 'pointer',
      fontFamily: font.body, fontSize: 13, fontWeight: 600,
      boxShadow: colors.cardShadow,
      // Tactile feedback: lift on hover, settle on press.
      transform: pressed ? 'scale(0.97)' : hovered ? 'translateY(-2px)' : 'translateY(0)',
      transition: `all 200ms ${ease.out}`,
    }}>
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.8} strokeLinecap="round" strokeLinejoin="round">
        <path d="M21 15a2 2 0 01-2 2H7l-4 4V5a2 2 0 012-2h14a2 2 0 012 2v10z" />
      </svg>
      Chat with {agentName}
    </button>
  );
}
