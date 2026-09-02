/**
 * ToastCard — one transient toast, as its own floating glass control.
 *
 * Split out of `NotificationHost` so the spring-in/out and the auto-dismiss
 * timer can live where they belong: entirely local to the card, and testable
 * without dragging in the tray, the daemon event stream, or the other two
 * toasts stacked beside it.
 *
 * Follows the same shape as `common/Tooltip.tsx`'s `TooltipBubble` — the other
 * portalled floating-glass primitive in the app: mount in an "unentered" pose,
 * flip to entered on the next frame so the transition actually plays, and
 * respect Reduce Motion by skipping straight to the settled state. A toast
 * additionally owns a dismiss timer, because unlike a tooltip it disappears on
 * its own.
 *
 * D9: spring, under 500ms (`duration.snappy` in, `duration.snappy` — or
 * `duration.fast` under Reduce Motion — out). D4: concentric radius, the
 * outer glass step and nothing invented. Reduce Motion collapses the
 * transform to nothing and animates opacity alone — a fade, not a slide.
 */

import { useCallback, useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type KeyboardEvent, type FocusEvent } from 'react';
import { concentric, duration, ease, font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { useGlass } from '../common/Glass';
import { Button } from '../common/Button';
import type { AppNotification } from '../../lib/notifications';

/** Outer radius is the floating-glass step (D4) — the same one Tooltip uses
 *  for its bubble. The dismiss button nests inside it: its own radius is
 *  derived, not chosen, from that outer step and its inset from the edge. */
const OUTER_R = radius.glass;
const DISMISS_INSET = space.sm; // 6 — top/right offset of the × button
const DISMISS_R = concentric(OUTER_R, DISMISS_INSET);
const DISMISS_SIZE = 18;
/** Right-edge clearance the title/body text needs to clear the × button. */
const DISMISS_CLEARANCE = DISMISS_INSET * 2 + DISMISS_SIZE;

export interface ToastCardProps {
  notification: AppNotification;
  /** How long the toast stays up before it starts leaving, in ms. */
  ttlMs: number;
  /** Called once the exit transition has finished — remove it from the list. */
  onDismiss: (id: string) => void;
  /** Called when the toast body itself is activated (click or Enter/Space). */
  onActivate: (n: AppNotification) => void;
}

export function ToastCard({ notification, ttlMs, onDismiss, onActivate }: ToastCardProps) {
  const { colors, reduceMotion } = useTheme();
  const glass = useGlass('glass');

  const [entered, setEntered] = useState(reduceMotion);
  const [leaving, setLeaving] = useState(false);

  // Timer bookkeeping lives in refs, not state — pausing/resuming on hover
  // must never itself trigger a render (that would restart the spring-in).
  const remaining = useRef(ttlMs);
  const startedAt = useRef(0);
  const timer = useRef<ReturnType<typeof setTimeout>>();
  const leavingRef = useRef(false);

  const beginLeave = useCallback(() => {
    if (leavingRef.current) return; // already on the way out
    leavingRef.current = true;
    clearTimeout(timer.current);
    setLeaving(true);
    // Reduce Motion still gets an exit — just the fast, fade-only one — so
    // the toast is never yanked off screen with no transition at all.
    const exitMs = reduceMotion ? duration.fast : duration.snappy;
    setTimeout(() => onDismiss(notification.id), exitMs);
  }, [notification.id, onDismiss, reduceMotion]);

  const arm = useCallback(() => {
    startedAt.current = Date.now();
    timer.current = setTimeout(beginLeave, Math.max(0, remaining.current));
  }, [beginLeave]);

  const pause = useCallback(() => {
    if (leavingRef.current) return;
    clearTimeout(timer.current);
    remaining.current = Math.max(0, remaining.current - (Date.now() - startedAt.current));
  }, []);

  const resume = useCallback(() => {
    if (leavingRef.current) return;
    arm();
  }, [arm]);

  // Spring-in: mount closed, flip open on the next frame. Reduce Motion opens
  // immediately (no frame to flip on, nothing to animate toward).
  useLayoutEffect(() => {
    if (reduceMotion) { setEntered(true); return; }
    const raf = requestAnimationFrame(() => setEntered(true));
    return () => cancelAnimationFrame(raf);
  }, [reduceMotion]);

  // The dismiss timer. Armed once on mount; torn down on unmount so a toast
  // that gets yanked out of the list some other way never fires late.
  useEffect(() => {
    arm();
    return () => clearTimeout(timer.current);
    // `arm` closes over `beginLeave`, which closes over stable-enough deps;
    // re-arming on every render would reset the countdown on unrelated state
    // changes (e.g. a theme flip), which is not what "hover pauses" means.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === 'Escape') {
      e.stopPropagation();
      beginLeave();
    }
  };

  // Focus-within acts like hover-within for the pause: a keyboard user
  // tabbed onto the toast (or its dismiss button) must not have it vanish
  // out from under them mid-navigation. `relatedTarget` tells blur whether
  // focus is still inside this card (moving from the body to the × button)
  // or has actually left it.
  const onBlur = (e: FocusEvent<HTMLDivElement>) => {
    if (e.currentTarget.contains(e.relatedTarget as Node | null)) return;
    resume();
  };

  const transform = reduceMotion
    ? undefined
    : entered && !leaving
      ? 'translateY(0) scale(1)'
      : leaving
        ? 'translateY(-4px) scale(0.98)'
        : 'translateY(8px) scale(0.98)';

  const opacity = entered && !leaving ? 1 : 0;

  const exitDuration = reduceMotion ? duration.fast : duration.snappy;
  const exitEase = reduceMotion ? ease.smooth : ease.snappy;
  const activeDuration = leaving ? exitDuration : duration.snappy;
  const activeEase = leaving ? exitEase : ease.snappy;

  const transition = reduceMotion
    ? `opacity ${activeDuration}ms ${activeEase}`
    : [`opacity ${activeDuration}ms ${activeEase}`, `transform ${activeDuration}ms ${activeEase}`].join(', ');

  return (
    <div
      role="status"
      aria-live="polite"
      onMouseEnter={pause}
      onMouseLeave={resume}
      onFocus={pause}
      onBlur={onBlur}
      onKeyDown={onKeyDown}
      style={{
        position: 'relative',
        pointerEvents: leaving ? 'none' : 'auto',
        width: 300,
        opacity,
        transform,
        transition,
      }}
    >
      {/* Same reasoning as the tray row: the toast IS the button, a stacked
          title + body block, and centring it would be wrong. */}
      <button
        onClick={() => { onActivate(notification); beginLeave(); }}
        style={{
          textAlign: 'left', cursor: 'pointer', width: '100%',
          ...glass,
          border: `1px solid ${colors.borderHi}`, borderRadius: OUTER_R,
          boxShadow: `${glass.boxShadow}, ${colors.elevationFloating}`,
          padding: `${space.lg}px ${DISMISS_CLEARANCE}px ${space.lg}px ${space.xl}px`,
          color: colors.text,
        } as CSSProperties}
      >
        <div style={{ fontFamily: font.body, fontSize: textSize.caption, fontWeight: 600, color: colors.cyan }}>
          {notification.title}
        </div>
        {notification.body && (
          <div style={{ fontSize: textSize.micro, color: colors.textMuted, marginTop: 2 }}>
            {notification.body}
          </div>
        )}
      </button>
      <Button
        colors={colors}
        variant="bare"
        onClick={(e) => { e.stopPropagation(); beginLeave(); }}
        aria-label="Dismiss notification"
        title="Dismiss"
        style={{
          '--pa-btn-fg': colors.textDim,
          '--pa-btn-fg-hover': colors.text,
          '--pa-btn-pad': '0',
          '--pa-btn-radius': `${DISMISS_R}px`,
          position: 'absolute', top: DISMISS_INSET, right: DISMISS_INSET, width: DISMISS_SIZE, height: DISMISS_SIZE,
          fontSize: textSize.small, lineHeight: 1,
        } as CSSProperties}
      >
        ×
      </Button>
    </div>
  );
}
