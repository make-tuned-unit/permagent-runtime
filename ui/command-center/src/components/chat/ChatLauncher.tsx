import { useEffect, useLayoutEffect, useRef, useState, useCallback } from 'react';
import { font, ease } from '../../styles/tokens';
import { api } from '../../lib/api';
import { useTheme } from '../../styles/useTheme';
import { useCommandCenter } from '../../lib/store';
import { createChatWindow } from '../../lib/chatWindow';

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

// Distance from the viewport's bottom/right edges. The Browser derives the
// launcher's reserved corner from this anchor + the published size (#553).
export const CHAT_LAUNCHER_MARGIN = 20;

export function ChatLauncher() {
  const { colors, theme } = useTheme();
  const [agentName, setAgentName] = useState('Agent');
  const [chatWindowOpen, setChatWindowOpen] = useState(false);
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

  // Poll whether the chat window exists (handles user closing it via traffic light)
  useEffect(() => {
    if (!isTauri || !chatWindowOpen) return;
    const interval = setInterval(async () => {
      try {
        const { WebviewWindow } = await import('@tauri-apps/api/webviewWindow');
        const existing = await WebviewWindow.getByLabel('chat');
        if (!existing) setChatWindowOpen(false);
      } catch { setChatWindowOpen(false); }
    }, 1000);
    return () => clearInterval(interval);
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

  const openChatWindow = useCallback(async () => {
    if (!isTauri) return;
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

      const chatWindow = await createChatWindow(theme);

      chatWindow.once('tauri://created', async () => {
        setChatWindowOpen(true);
        // Ensure chat window comes to front above the main window
        await chatWindow.setFocus();
      });
      chatWindow.once('tauri://error', (e) => {
        console.error('Chat window error:', e);
        setChatWindowOpen(false);
      });
    } catch (e) {
      console.error('Failed to open chat window:', e);
    }
  }, [theme]);

  if (chatWindowOpen) return null;

  return (
    <button ref={buttonRef} onClick={openChatWindow} style={{
      position: 'fixed', bottom: CHAT_LAUNCHER_MARGIN, right: CHAT_LAUNCHER_MARGIN, zIndex: 9999,
      display: 'flex', alignItems: 'center', gap: 10,
      padding: '12px 20px', borderRadius: 999,
      background: colors.surface, backdropFilter: 'blur(16px)',
      border: `1px solid ${colors.borderHi}`,
      color: colors.cyan, cursor: 'pointer',
      fontFamily: font.body, fontSize: 13, fontWeight: 600,
      boxShadow: '0 8px 32px rgba(0,0,0,0.5)',
      transition: `all 200ms ${ease.out}`,
    }}>
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.8} strokeLinecap="round" strokeLinejoin="round">
        <path d="M21 15a2 2 0 01-2 2H7l-4 4V5a2 2 0 012-2h14a2 2 0 012 2v10z" />
      </svg>
      Chat with {agentName}
    </button>
  );
}
