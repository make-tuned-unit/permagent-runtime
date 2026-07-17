/**
 * Build-tab progress rail (2026-07 wiring audit).
 *
 * The 5-segment rail in the Build header used to be hardcoded to step 3
 * whenever any task ran — it showed identical "60%-ish" progress for a task
 * that just started and one about to finish. It now maps the daemon's real
 * per-task progress estimate (dashboard in_flight[].progress, 0..0.95) onto
 * the five segments.
 */

/**
 * Which of the 5 rail segments is the current/leading one, given a task's
 * progress in [0, 1] (or null when nothing is in flight).
 *   null → 0 (rail dark)
 *   >0   → 1..5, so a running task always lights at least the first segment.
 */
export function progressRailStep(progress: number | null | undefined): number {
  if (progress == null || progress <= 0) return 0;
  return Math.min(5, Math.max(1, Math.ceil(progress * 5)));
}
