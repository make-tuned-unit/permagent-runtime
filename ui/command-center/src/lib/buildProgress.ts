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

/** The part of a harness run Build needs; a structural subset of HarnessRunView. */
export interface BuildRunLike {
  sessionId: string;
  promptTitle: string;
  status: string;
  elapsedMs: number;
}

/** A turn is only in flight while the run is executing or verifying. */
const TURN_IN_FLIGHT = new Set(['running', 'verifying']);

export interface BuildActiveTask { id: string; title: string; progress: number }

/**
 * The run Build presents in its header, or null. This comes from the
 * coding-harness run list, never from the dashboard's `in_flight` alone: that
 * list is "any session touched in the last two minutes", so opening the chat
 * dock (which creates a session) used to make Build claim a coding agent was
 * "thinking" at "Step 0 of 5". The dashboard entry only lends its progress
 * estimate to a run that is genuinely in flight.
 */
export function selectBuildTask(
  runs: readonly BuildRunLike[],
  inFlight: ReadonlyArray<{ id: string; title: string; progress: number }>,
): BuildActiveTask | null {
  const run = runs.find(r => TURN_IN_FLIGHT.has(r.status));
  if (!run) return null;
  const match = inFlight.find(f => f.id === run.sessionId);
  return {
    id: run.sessionId,
    title: run.promptTitle || match?.title || `Session ${run.sessionId}`,
    progress: match?.progress ?? Math.min(0.95, Math.max(0, run.elapsedMs / 60000 / 5)),
  };
}
