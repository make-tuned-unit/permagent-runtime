/**
 * Whether a canvas is still showing its key — remembered per canvas.
 *
 * Teach once, then be quiet. A key that opens on every visit is a nag, and one
 * that never opens by itself teaches nobody: the World's only interaction hint
 * appeared *after* the camera had already switched, which is worse than none
 * because it implies a key exists somewhere. So the default is open, the
 * dismissal is permanent, and the answer is kept per canvas — learning to
 * orbit the hall tells you nothing about what a dimmed face in the People
 * graph means.
 *
 * Storage may be unavailable (a private window, blocked site data). A
 * preference that cannot be saved is a preference that does not persist, never
 * a failure the user has to see — and the safe direction is *open*, because
 * the cost of teaching someone twice is smaller than never teaching them.
 */

export const CANVAS_LEGEND_KEY_PREFIX = 'permagent-canvas-legend:';

export function canvasLegendStorageKey(canvasId: string): string {
  return `${CANVAS_LEGEND_KEY_PREFIX}${canvasId}`;
}

/**
 * Whether this canvas's key is open. With nothing remembered it is
 * `firstVisit` — open by default (teach once), but a canvas whose key would
 * cover much of the scene (the People graph at ~660x560: ~45% of it) starts
 * collapsed behind its "Key" button instead. An explicit choice always wins.
 */
export function readLegendOpen(canvasId: string, firstVisit = true): boolean {
  try {
    const stored = localStorage.getItem(canvasLegendStorageKey(canvasId));
    if (stored === 'dismissed') return false;
    if (stored === 'open') return true;
    return firstVisit;
  } catch {
    return firstVisit;
  }
}

/** Call ONLY from the control the user pressed. */
export function rememberLegendOpen(canvasId: string, open: boolean): void {
  try {
    localStorage.setItem(canvasLegendStorageKey(canvasId), open ? 'open' : 'dismissed');
  } catch {
    // Not persisting a preference is not an error worth a user's attention.
  }
}
