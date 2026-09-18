// NotificationHost (#618) — the bell, the tray, and transient toasts.
// Mounted once at App root. Every item is a real daemon event (see
// lib/notifications.ts); clicking one deep-links to its surface.

import { useEffect, useState } from 'react';
import {
  useNotifications,
  markAllRead,
  setTrayOpen,
  useTrayOpen,
  type AppNotification,
} from '../../lib/notifications';
import { navigateToTool, useCommandCenter } from '../../lib/store';
import { font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { useGlass } from '../common/Glass';
import { ToastCard } from './Toast';

const TOAST_MS = 6000;
/** Never more than this many stacked at once — a wall of toasts is its own
 *  kind of noise. */
const MAX_TOASTS = 3;

export function NotificationHost() {
  const { colors } = useTheme();
  // The reference conversion for the glass tokens.
  //
  // A toast and the tray are the clearest case Apple's layer rule has: both
  // float above whatever screen happens to be underneath, belong to no view,
  // and are gone in six seconds. Nothing about them is content. They were
  // already reaching for the material — `blur(24px) saturate(140%)` over an
  // opaque `colors.surface`, so the blur never showed — and this makes it real
  // rather than adding it.
  //
  // `glass` and not `glassHi`: both are small surfaces (320/300px), and the
  // more opaque step is for sidebar-scale glass.
  const glass = useGlass('glass');
  const { items } = useNotifications();
  const open = useTrayOpen();
  const [toastIds, setToastIds] = useState<string[]>([]);
  const pushBrowserOverlay = useCommandCenter((s) => s.pushBrowserOverlay);
  const popBrowserOverlay = useCommandCenter((s) => s.popBrowserOverlay);

  // Newest item becomes a transient toast (skip when the tray is open).
  // Each toast owns its own dismiss timer now (`ToastCard`), so an arrival
  // never has to reach into a sibling's countdown — the old bug this
  // replaced was exactly that: a shared effect-cleanup that cancelled the
  // PREVIOUS toast's timer whenever a new one arrived, leaving it on screen
  // forever with no way to dismiss it.
  const newestId = items[0]?.id;
  useEffect(() => {
    if (!newestId || open) return;
    setToastIds((prev) => (prev.includes(newestId) ? prev : [newestId, ...prev].slice(0, MAX_TOASTS)));
  }, [newestId, open]);

  // The toast IS the browser-overlay-hiding case `pushBrowserOverlay` exists
  // for: the native browser webview composites above every DOM layer
  // regardless of z-index (the "corner-cede trap" — see MeetingRecorder's
  // picker and ProjectChip's popover for the same fix), so a toast that lands
  // while the in-app browser is full-bleed would otherwise render invisibly
  // underneath it. `lib/notifications.ts` flagged this exact gap when the
  // download toast shipped; this closes it for every toast, not just that
  // kind. The tray gets the same treatment — it can sit open indefinitely
  // over whatever workspace is showing, browser included.
  const hasToasts = toastIds.length > 0;
  useEffect(() => {
    if (!hasToasts) return;
    pushBrowserOverlay();
    return () => popBrowserOverlay();
  }, [hasToasts, pushBrowserOverlay, popBrowserOverlay]);
  useEffect(() => {
    if (!open) return;
    pushBrowserOverlay();
    return () => popBrowserOverlay();
  }, [open, pushBrowserOverlay, popBrowserOverlay]);

  // Called by a ToastCard once its own exit spring has finished playing.
  const dismissToast = (id: string) =>
    setToastIds((prev) => prev.filter((tid) => tid !== id));

  const activate = (n: AppNotification) => {
    // A custom action (e.g. "note saved" → the exact note) wins; a notification
    // carrying a link (e.g. a Watcher nudge's source article) opens it in the
    // in-app browser on the Build tab; otherwise fall back to the target tab.
    if (n.onActivate) {
      n.onActivate();
    } else if (n.url) {
      useCommandCenter.getState().openInBrowser(n.url);
    } else if (n.target) {
      navigateToTool(n.target);
    }
    setTrayOpen(false);
    markAllRead();
  };

  // Click-away dismissal: any press outside the tray (and outside the bell
  // row, whose own toggle handles those clicks) closes the tray and marks
  // read — same as closing via the bell. Capture phase so it wins even when
  // the clicked surface stops propagation.
  useEffect(() => {
    if (!open) return;
    const onPress = (e: MouseEvent) => {
      const el = e.target as Element | null;
      if (el?.closest('[data-notifications-ui]')) return;
      markAllRead();
      setTrayOpen(false);
    };
    document.addEventListener('mousedown', onPress, true);
    return () => document.removeEventListener('mousedown', onPress, true);
  }, [open]);

  const toasts = items.filter((i) => toastIds.includes(i.id));

  return (
    <>
      {/* Tray — anchored bottom-left, beside the sidebar's Notifications row
          (which sits just above Settings). */}
      {open && (
        <div data-notifications-ui style={{
          position: 'fixed', bottom: 96, left: 60, zIndex: 90, width: 320,
          maxHeight: '60vh', overflowY: 'auto',
          ...glass,
          // D4: the tray is the same floating-glass class as a toast, so it
          // takes the same outer step — not the old, uncoordinated `radius.lg`.
          border: `1px solid ${colors.borderHi}`, borderRadius: radius.glass,
          // The material's own shadow is a specular rim plus a close ambient;
          // the tray's existing float height is kept by composing the theme's
          // elevation on top of it rather than replacing it.
          boxShadow: `${glass.boxShadow}, ${colors.elevationFloating}`,
          padding: space.md,
        }}>
          {items.length === 0 ? (
            <div style={{ padding: 18, textAlign: 'center', fontSize: textSize.caption, color: colors.textDim }}>
              Nothing needs you — all quiet.
            </div>
          ) : items.map((n) => (
            /* Left as a raw <button> on purpose: this row IS the card — three
               stacked blocks (title, body, timestamp) laid out by the button
               itself. `.pa-btn`'s inline-flex + centring would turn them into
               one centred row.
               `radius.md`, NOT a `concentric()` derivation off the tray's own
               radius: a scrolling list of independent rows is content sitting
               INSIDE the glass shell (D1 — "list rows are content"), the same
               way a card is content inside a glass sidebar. Concentricity is
               for chrome nested directly against the glass edge (the toast's
               dismiss button below); it isn't owed to everything a glass
               container happens to contain. */
            <button key={n.id} onClick={() => activate(n)} style={{
              display: 'block', width: '100%', textAlign: 'left', cursor: 'pointer',
              padding: `${space.md}px ${space.lg}px`, borderRadius: radius.md, marginBottom: space.xxs,
              background: n.read ? 'transparent' : colors.cyanSoft,
              border: 'none', color: colors.text,
            }}>
              <div style={{ fontFamily: font.body, fontSize: textSize.caption, fontWeight: 600 }}>{n.title}</div>
              {n.body && (
                <div style={{ fontSize: textSize.micro, color: colors.textMuted, marginTop: space.xxs, lineHeight: 1.4 }}>{n.body}</div>
              )}
              <div style={{ fontFamily: font.mono, fontSize: 10, color: colors.textDim, marginTop: space.xxs }}>
                {new Date(n.ts).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
              </div>
            </button>
          ))}
        </div>
      )}

      {/* Toasts. Each card owns its own spring in/out, hover-pause and
          Escape-dismiss (`ToastCard`) — this is just the stack: newest on
          top, a fixed gap between cards, nothing else. */}
      <div style={{
        // Top-right (2026-07-27): the bottom-right corner belongs to the
        // Chat-with-Henry pill, which was overlapping toasts.
        position: 'fixed', top: 40, right: 14, zIndex: 95,
        display: 'flex', flexDirection: 'column', gap: space.md, pointerEvents: 'none',
      }}>
        {toasts.map((n) => (
          <ToastCard key={n.id} notification={n} ttlMs={TOAST_MS} onDismiss={dismissToast} onActivate={activate} />
        ))}
      </div>
    </>
  );
}
