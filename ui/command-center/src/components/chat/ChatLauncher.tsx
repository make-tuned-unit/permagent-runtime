import { useEffect, useLayoutEffect, useRef, useState, useCallback } from 'react';
import { FiMessageSquare } from 'react-icons/fi';
import { duration, ease, font, radius, space, textSize } from '../../styles/tokens';
import { api } from '../../lib/api';
import { useTheme } from '../../styles/useTheme';
import { useCommandCenter } from '../../lib/store';
import { useGlass } from '../common/Glass';

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

// Distance from the viewport's bottom/right edges. The Browser derives the
// launcher's reserved corner from this anchor + the published size (#553).
export const CHAT_LAUNCHER_MARGIN = 20;

export function ChatLauncher() {
  const { colors, reduceMotion } = useTheme();
  const glass = useGlass('glass');
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

  // The pointer ladder as FILL, not as a second material (D2/D10). The glass
  // token already supplies the surface; hover and press add the theme's own
  // neutral ink on top of it as an extra background layer, which is the one
  // way to lighten a translucent fill without stacking a second
  // `backdrop-filter` on it. `fillHover`/`fillActive` carry the theme's ink, so
  // the same token reads as a lift on the void and as a shade on the pearl.
  const lift = pressed ? colors.fillActive : hovered ? colors.fillHover : null;
  // `glass.background` is always set by `glassSurface()` — translucent normally,
  // the theme's flat surface under Reduce Transparency — but it is typed
  // optional, and a stray `undefined` in a background list would drop the fill
  // entirely rather than fail loudly.
  const base = glass.background ?? colors.surface;
  const background = lift ? `linear-gradient(0deg, ${lift}, ${lift}), ${base}` : base;

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
      display: 'flex', alignItems: 'center', gap: space.lg,
      padding: `${space.xl}px ${space.xxxl}px`,
      // Capsule, and it is the one control in the app that has earned one:
      // Tahoe rounds LARGE and extra-large prominent controls into capsules and
      // keeps dense ones as rounded rectangles (D5). This is the single
      // most prominent floating action on every screen.
      borderRadius: radius.pill,
      // Real glass. A fixed pill at z-index 9999 that hovers over every screen
      // in the app is the floating control layer by definition, and it is
      // small — so the default `glass`, not the sidebar-weight `glassHi`.
      // It was already asking for blur(16px) over an opaque fill.
      ...glass,
      background,
      border: `1px solid ${hovered ? colors.cyan : colors.borderHi}`,
      color: colors.cyan, cursor: 'pointer',
      fontFamily: font.body, fontSize: textSize.small, fontWeight: 600,
      // ONE ambient shadow, the glass token's own. It was stacked with
      // `colors.cardShadow`, which is a second ambient plus a second hairline
      // over the rim the material already draws — two shadows doing one job
      // (D13, and Apple.com ships exactly one drop shadow in its whole system).
      boxShadow: glass.boxShadow,
      // Physicality, not a colour swap (D10): lift on hover, settle on press.
      transform: pressed ? 'scale(0.97)' : hovered ? 'translateY(-2px)' : 'translateY(0)',
      // Springs, and named properties rather than `all` — `all` would also
      // animate the padding and border the store's published size is measured
      // from. `snappy` (bounce 0.15, 240ms) is the control-state-change spring;
      // `smooth` carries the border tint. Both under Apple's 500ms ceiling.
      //
      // The FILL is deliberately not in this list. A layered gradient cannot
      // interpolate, so naming it would be a transition that silently does
      // nothing — and an instant highlight under the pointer with the give
      // animated is what a Mac control actually does.
      transition: reduceMotion
        ? 'none'
        : `transform ${duration.snappy}ms ${ease.snappy}, border-color ${duration.fast}ms ${ease.smooth}`,
    }}>
      <FiMessageSquare size={16} />
      Chat with {agentName}
    </button>
  );
}
