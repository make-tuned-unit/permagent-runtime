import { useEffect, useLayoutEffect, useRef, useState, useCallback } from 'react';
import { ease, font, radius, textSize } from '../../styles/tokens';
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
  // Store-tracked (not local): createChatWindow sets it on EVERY open path
  // (dock-detach, drop handler, navigate) — local state only learned about
  // opens made through this button, leaving the pill visible over a chat
  // window detached from the dock.
  const chatWindowOpen = useCommandCenter(s => s.chatWindowOpen);
  const setChatWindowOpen = useCommandCenter(s => s.setChatWindowOpen);
  const chatDockOpen = useCommandCenter(s => s.chatDockOpen);
  const [hovered, setHovered] = useState(false);
  const [pressed, setPressed] = useState(false);
  const setChatLauncherSize = useCommandCenter(s => s.setChatLauncherSize);
  const buttonRef = useRef<HTMLButtonElement>(null);

  // Publish the pill's measured size so the Browser can subtract its corner
  // from the native webview bounds (#553). ResizeObserver fires only on real
  // layout changes (e.g. the agent name loading) — no polling.
  useLayoutEffect(() => {
    const el = buttonRef.current;
    if (chatWindowOpen || chatDockOpen || !el) {
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
  }, [chatWindowOpen, chatDockOpen, setChatLauncherSize]);

  // #629 multi-client liveness: identityRev bumps when `identity_changed`
  // arrives on /events, so a persona rename on another device relabels the
  // launcher pill without a reload.
  const identityRev = useCommandCenter(s => s.identityRev);
  useEffect(() => {
    api.getIdentity().then(id => setAgentName(id.first_name)).catch(() => {});
  }, [identityRev]);

  // Track the chat window's existence directly, not just via the flag-setting
  // code paths: check on mount (main-window reload), and re-check whenever ANY
  // window is created (`tauri://window-created` fires app-wide) — so the pill
  // hides even for creation paths the store flag misses.
  useEffect(() => {
    if (!isTauri) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    const check = async () => {
      try {
        const { WebviewWindow } = await import('@tauri-apps/api/webviewWindow');
        const existing = await WebviewWindow.getByLabel('chat');
        if (!disposed && existing) setChatWindowOpen(true);
      } catch { /* ignore */ }
    };
    void check();
    (async () => {
      try {
        const { listen } = await import('@tauri-apps/api/event');
        const un = await listen('tauri://window-created', () => void check());
        if (disposed) un(); else unlisten = un;
      } catch { /* ignore */ }
    })();
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [setChatWindowOpen]);

  // React to the chat window going away — by OBSERVING it, never by
  // intercepting its close.
  //
  // This used to call `existing.onCloseRequested(...)`. That is a trap: Tauri
  // core calls `api.prevent_close()` whenever ANY js listener is registered for
  // `tauri://close-requested` on that window (manager/window.rs), so merely
  // watching the close suppressed the native one — the window then survived
  // unless some JS path called destroy(), and when that didn't land the X
  // stopped working entirely. `tauri://destroyed` fires AFTER the window is
  // already gone and carries no such veto, so it is safe to listen for.
  //
  // The poll is the backstop: it also covers a crash or force-quit, and it
  // guarantees the sidebar comes back even if the destroyed event is missed
  // while the webview is being torn down.
  useEffect(() => {
    if (!isTauri || !chatWindowOpen) return;
    let unlisten: (() => void) | undefined;
    let disposed = false;
    let poll: ReturnType<typeof setInterval> | undefined;

    const markGone = () => {
      if (disposed) return;
      disposed = true; // idempotent: event and poll race, only one may win
      setChatWindowOpen(false);
    };

    (async () => {
      try {
        const { WebviewWindow } = await import('@tauri-apps/api/webviewWindow');
        const { TauriEvent } = await import('@tauri-apps/api/event');
        // Window creation is fire-and-forget: right after createChatWindow()
        // the label may not exist YET. Marking gone on the first null flipped
        // chatWindowOpen false mid-creation, which VoiceHost answered by
        // re-opening the dock — dock AND window open, the bug-2 precondition.
        // Only a window we have actually observed can be declared gone; the
        // window-created listener (below) re-arms this effect for late birth.
        const existing = await WebviewWindow.getByLabel('chat');
        if (existing) {
          const un = await existing.listen(TauriEvent.WINDOW_DESTROYED, markGone);
          if (disposed) un(); else unlisten = un;
        }
        let seen = existing !== null;
        let misses = 0;
        poll = setInterval(() => {
          void WebviewWindow.getByLabel('chat')
            .then(w => {
              if (w) { seen = true; misses = 0; return; }
              // Unseen window: allow ~3s of creation grace before giving up.
              misses += 1;
              if (seen || misses >= 6) markGone();
            })
            .catch(() => {});
        }, 500);
      } catch {
        markGone();
      }
    })();

    return () => {
      disposed = true;
      if (poll) clearInterval(poll);
      unlisten?.();
    };
  }, [chatWindowOpen, setChatWindowOpen]);

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
  }, [openChatDock, setChatWindowOpen]);

  // Hide whenever chat is already showing — in the detached window OR in the
  // dock. Dock-first made the dock the default surface, but the pill still only
  // checked the window, so it stayed on screen over an open chat.
  if (chatWindowOpen || chatDockOpen) return null;

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
      padding: '12px 20px', borderRadius: radius.pill,
      background: colors.surface, backdropFilter: 'blur(16px)',
      // Theme-safe elevation — a cool soft shadow on silver, deep on dark
      // (the hardcoded black glow was invisible on the light themes).
      border: `1px solid ${hovered ? colors.cyan : colors.borderHi}`,
      color: colors.cyan, cursor: 'pointer',
      fontFamily: font.body, fontSize: textSize.small, fontWeight: 600,
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
