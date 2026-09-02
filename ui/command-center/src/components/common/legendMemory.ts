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

/** True unless this canvas's key has been dismissed before. */
export function readLegendOpen(canvasId: string): boolean {
  try {
    return localStorage.getItem(canvasLegendStorageKey(canvasId)) !== 'dismissed';
  } catch {
    return true;
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
