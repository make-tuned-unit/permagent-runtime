/**
 * Whether the Brain graph's WebGL render loop should keep scheduling frames.
 *
 * QA 2026-09-25 at 1440px: switching to List left the graph's `requestAnimationFrame`
 * loop running against a `display:none` container — zero-size canvas — which
 * spammed the console with "GL_INVALID_FRAMEBUFFER_OPERATION: Framebuffer is
 * incomplete: Attachment has zero size" hundreds of times a minute, purely
 * wasted GPU work for a scene nobody can see. The same waste happens for any
 * 0×0 canvas (a mid-layout resize) and for a backgrounded tab (`document.hidden`).
 *
 * Pure so the pause/resume decision is testable without a renderer: BrainScene
 * calls this once per candidate frame and, on `false`, cancels rather than
 * reschedules — nothing polls while paused. Resuming is edge-triggered by
 * whatever caused the state to change (`resize()`, `setActive()`, or the
 * `visibilitychange` listener), each of which asks to schedule a frame again.
 */
export interface RenderGateInputs {
  /** False while the graph tab itself is not the shown mode (e.g. List). */
  active: boolean;
  /** The container's current `clientWidth`/`clientHeight`. */
  width: number;
  height: number;
  /** `document.hidden` — a backgrounded tab or minimized window. */
  documentHidden: boolean;
}

export function shouldRender(i: RenderGateInputs): boolean {
  return i.active && i.width > 0 && i.height > 0 && !i.documentHidden;
}
