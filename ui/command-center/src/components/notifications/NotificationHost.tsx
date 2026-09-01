// NotificationHost (#618) — the bell, the tray, and transient toasts.
// Mounted once at App root. Every item is a real daemon event (see
// lib/notifications.ts); clicking one deep-links to its surface.

import { useEffect, useState, type CSSProperties } from 'react';
import {
  useNotifications,
  markAllRead,
  setTrayOpen,
  useTrayOpen,
  type AppNotification,
} from '../../lib/notifications';
import { navigateToTool, useCommandCenter } from '../../lib/store';
import { font, radius, duration, ease, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import { useGlass } from '../common/Glass';

const TOAST_MS = 6000;

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

  // Newest item becomes a transient toast (skip when the tray is open).
  // One dismissal timer PER toast, never cancelled by a newer arrival: the
  // old effect-cleanup cleared the previous toast's timer whenever newestId
  // changed, so any toast followed within 6s by another had no timer left
  // and sat on screen forever with no way to dismiss it.
  const newestId = items[0]?.id;
  useEffect(() => {
    if (!newestId || open) return;
    setToastIds((prev) => (prev.includes(newestId) ? prev : [newestId, ...prev].slice(0, 3)));
    setTimeout(
      () => setToastIds((prev) => prev.filter((id) => id !== newestId)),
      TOAST_MS,
    );
  }, [newestId, open]);

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
          border: `1px solid ${colors.borderHi}`, borderRadius: radius.lg,
          // The material's own shadow is a specular rim plus a close ambient;
          // the tray's existing float height is kept by composing the theme's
          // elevation on top of it rather than replacing it.
          boxShadow: `${glass.boxShadow}, ${colors.elevationFloating}`,
          padding: 8,
        }}>
          {items.length === 0 ? (
            <div style={{ padding: 18, textAlign: 'center', fontSize: textSize.caption, color: colors.textDim }}>
              Nothing needs you — all quiet.
            </div>
          ) : items.map((n) => (
            /* Left as a raw <button> on purpose: this row IS the card — three
               stacked blocks (title, body, timestamp) laid out by the button
               itself. `.pa-btn`'s inline-flex + centring would turn them into
               one centred row. */
            <button key={n.id} onClick={() => activate(n)} style={{
              display: 'block', width: '100%', textAlign: 'left', cursor: 'pointer',
              padding: '9px 10px', borderRadius: radius.md, marginBottom: 2,
              background: n.read ? 'transparent' : colors.cyanSoft,
              border: 'none', color: colors.text,
            }}>
              <div style={{ fontFamily: font.body, fontSize: textSize.caption, fontWeight: 600 }}>{n.title}</div>
              {n.body && (
                <div style={{ fontSize: textSize.micro, color: colors.textMuted, marginTop: 2, lineHeight: 1.4 }}>{n.body}</div>
              )}
              <div style={{ fontFamily: font.mono, fontSize: 10, color: colors.textDim, marginTop: 3 }}>
                {new Date(n.ts).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
              </div>
            </button>
          ))}
        </div>
      )}

      {/* Toasts */}
      <div style={{
        // Top-right (2026-07-27): the bottom-right corner belongs to the
        // Chat-with-Henry pill, which was overlapping toasts.
        position: 'fixed', top: 40, right: 14, zIndex: 95,
        display: 'flex', flexDirection: 'column', gap: 8, pointerEvents: 'none',
      }}>
        {toasts.map((n) => (
          <div key={n.id} style={{ position: 'relative', pointerEvents: 'auto', width: 300 }}>
            {/* Same reasoning as the tray row: the toast IS the button, a
                stacked title + body block, and centring it would be wrong. */}
            <button
              onClick={() => { dismissToast(n.id); activate(n); }}
              style={{
                textAlign: 'left', cursor: 'pointer', width: '100%',
                ...glass,
                border: `1px solid ${colors.borderHi}`, borderRadius: radius.md,
                boxShadow: `${glass.boxShadow}, ${colors.elevationFloating}`,
                padding: '10px 28px 10px 12px',
                // A toast arriving is a spring, not a ramp: `snappy` settles in
                // 240ms with a 0.6% overshoot, which reads as the thing landing.
                color: colors.text, animation: `pa-toast-in ${duration.snappy}ms ${ease.snappy}`,
              }}
            >
              <div style={{ fontFamily: font.body, fontSize: textSize.caption, fontWeight: 600, color: colors.cyan }}>{n.title}</div>
              {n.body && <div style={{ fontSize: textSize.micro, color: colors.textMuted, marginTop: 2 }}>{n.body}</div>}
            </button>
            <Button
              colors={colors}
              variant="bare"
              onClick={(e) => { e.stopPropagation(); dismissToast(n.id); }}
              aria-label="Dismiss notification"
              title="Dismiss"
              style={{
                '--pa-btn-fg': colors.textDim,
                '--pa-btn-fg-hover': colors.text,
                '--pa-btn-pad': '0',
                position: 'absolute', top: 6, right: 6, width: 18, height: 18,
                fontSize: textSize.small, lineHeight: 1,
              } as CSSProperties}
            >
              ×
            </Button>
          </div>
        ))}
      </div>
      <style>{`@keyframes pa-toast-in { from { opacity: 0; transform: translateY(8px); } to { opacity: 1; transform: translateY(0); } }`}</style>
    </>
  );
}
