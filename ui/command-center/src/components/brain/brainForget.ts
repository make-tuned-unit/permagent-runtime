/**
 * Schedules a memory's hard-delete after an undo window — the write side of
 * R5's "explicit forget with undo" (no confirm dialog; a toast with Undo,
 * ~8s, and the request itself fires only once that window lapses unactioned).
 *
 * Deliberately NOT React state: the toast it pairs with is a global portal
 * (`NotificationHost`) that outlives whatever view opened it, so the
 * countdown that decides whether the delete actually happens must too —
 * navigating away from the Brain mid-countdown must not silently cancel a
 * forget the user already committed to by clicking it.
 */

import { api } from '../../lib/api';

/** The undo window. Matches the lane brief's "~8s"; intentionally independent
 *  of `NotificationHost`'s shared `TOAST_MS` (6s) — that constant governs
 *  every toast in the app and is owned by the motion lane, so this schedules
 *  its own timer rather than widen a shared one for a single feature. The
 *  practical effect: the toast itself may fade a couple seconds before the
 *  delete actually fires, which is not observable (the row is hidden either
 *  way, and Undo stops working the moment the toast is gone regardless). */
export const FORGET_UNDO_MS = 8_000;

export type ForgetOutcome = 'deleted' | 'restored';

/**
 * Start the countdown for `key`. Returns a `cancel` function — call it (e.g.
 * from the toast's Undo action) to stop the delete before it fires.
 *
 * `onSettled('deleted')` fires once the request completes (whether or not a
 * matching memory actually existed — a no-op delete is still "gone" from the
 * user's point of view, they never see the key). `onSettled('restored')`
 * fires only when the request itself fails (network/server error) — the
 * caller should bring the row back and the caller is expected to surface a
 * failure toast, matching Button.tsx's own rejected-promise convention.
 */
export function scheduleForget(
  key: string,
  onSettled: (outcome: ForgetOutcome) => void,
): () => void {
  let cancelled = false;
  const timer = setTimeout(() => {
    if (cancelled) return;
    api.forgetBrainMemory(key)
      .then(() => { if (!cancelled) onSettled('deleted'); })
      .catch(() => { if (!cancelled) onSettled('restored'); });
  }, FORGET_UNDO_MS);

  return () => {
    cancelled = true;
    clearTimeout(timer);
  };
}
