// NotificationHost (#618) — the bell, the tray, and transient toasts.
// Mounted once at App root. Every item is a real daemon event (see
// lib/notifications.ts); clicking one deep-links to its surface.

import { useEffect, useState } from 'react';
import {
  useNotifications,
  markAllRead,
  type AppNotification,
} from '../../lib/notifications';
import { navigateToTool } from '../../lib/store';
import { font, radius, ease } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

const TOAST_MS = 6000;

export function NotificationHost() {
  const { colors } = useTheme();
  const { items, unread } = useNotifications();
  const [open, setOpen] = useState(false);
  const [toastIds, setToastIds] = useState<string[]>([]);

  // Newest item becomes a transient toast (skip when the tray is open).
  const newestId = items[0]?.id;
  useEffect(() => {
    if (!newestId || open) return;
    setToastIds((prev) => (prev.includes(newestId) ? prev : [newestId, ...prev].slice(0, 3)));
    const t = setTimeout(
      () => setToastIds((prev) => prev.filter((id) => id !== newestId)),
      TOAST_MS,
    );
    return () => clearTimeout(t);
  }, [newestId, open]);

  const activate = (n: AppNotification) => {
    if (n.target) navigateToTool(n.target);
    setOpen(false);
    markAllRead();
  };

  const toasts = items.filter((i) => toastIds.includes(i.id));

  return (
    <>
      {/* Bell */}
      <button
        onClick={() => {
          // Mark read on CLOSE so the unread highlight is visible while open.
          if (open) markAllRead();
          setOpen((o) => !o);
        }}
        title="Notifications"
        style={{
          position: 'fixed', top: 34, right: 14, zIndex: 90,
          width: 30, height: 30, borderRadius: radius.pill,
          background: unread > 0 ? colors.cyanSoft : 'transparent',
          border: `1px solid ${unread > 0 ? colors.borderHi : colors.border}`,
          color: unread > 0 ? colors.cyan : colors.textMuted,
          cursor: 'pointer', display: 'flex', alignItems: 'center', justifyContent: 'center',
        }}
      >
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round">
          <path d="M18 8a6 6 0 10-12 0c0 7-3 9-3 9h18s-3-2-3-9M13.7 21a2 2 0 01-3.4 0" />
        </svg>
        {unread > 0 && (
          <span style={{
            position: 'absolute', top: -4, right: -4, minWidth: 15, height: 15,
            borderRadius: radius.pill, background: colors.cyan, color: colors.textOnCyan,
            fontFamily: font.mono, fontSize: 9, fontWeight: 700,
            display: 'flex', alignItems: 'center', justifyContent: 'center', padding: '0 3px',
          }}>{unread > 9 ? '9+' : unread}</span>
        )}
      </button>

      {/* Tray */}
      {open && (
        <div style={{
          position: 'fixed', top: 70, right: 14, zIndex: 90, width: 320,
          maxHeight: '60vh', overflowY: 'auto',
          background: colors.surface, backdropFilter: 'blur(24px) saturate(140%)',
          border: `1px solid ${colors.borderHi}`, borderRadius: radius.lg,
          boxShadow: colors.elevationFloating, padding: 8,
        }}>
          {items.length === 0 ? (
            <div style={{ padding: 18, textAlign: 'center', fontSize: 12, color: colors.textDim }}>
              Nothing needs you — all quiet.
            </div>
          ) : items.map((n) => (
            <button key={n.id} onClick={() => activate(n)} style={{
              display: 'block', width: '100%', textAlign: 'left', cursor: 'pointer',
              padding: '9px 10px', borderRadius: radius.md, marginBottom: 2,
              background: n.read ? 'transparent' : colors.cyanSoft,
              border: 'none', color: colors.text,
            }}>
              <div style={{ fontFamily: font.body, fontSize: 12, fontWeight: 600 }}>{n.title}</div>
              {n.body && (
                <div style={{ fontSize: 11, color: colors.textMuted, marginTop: 2, lineHeight: 1.4 }}>{n.body}</div>
              )}
              <div style={{ fontFamily: font.mono, fontSize: 9, color: colors.textDim, marginTop: 3 }}>
                {new Date(n.ts).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
              </div>
            </button>
          ))}
        </div>
      )}

      {/* Toasts */}
      <div style={{
        position: 'fixed', bottom: 18, right: 18, zIndex: 95,
        display: 'flex', flexDirection: 'column', gap: 8, pointerEvents: 'none',
      }}>
        {toasts.map((n) => (
          <button key={n.id} onClick={() => activate(n)} style={{
            pointerEvents: 'auto', textAlign: 'left', cursor: 'pointer', width: 300,
            background: colors.surface, backdropFilter: 'blur(24px) saturate(140%)',
            border: `1px solid ${colors.borderHi}`, borderRadius: radius.md,
            boxShadow: colors.elevationFloating, padding: '10px 12px',
            color: colors.text, animation: `pa-toast-in 220ms ${ease.out}`,
          }}>
            <div style={{ fontFamily: font.body, fontSize: 12, fontWeight: 600, color: colors.cyan }}>{n.title}</div>
            {n.body && <div style={{ fontSize: 11, color: colors.textMuted, marginTop: 2 }}>{n.body}</div>}
          </button>
        ))}
      </div>
      <style>{`@keyframes pa-toast-in { from { opacity: 0; transform: translateY(8px); } to { opacity: 1; transform: translateY(0); } }`}</style>
    </>
  );
}
