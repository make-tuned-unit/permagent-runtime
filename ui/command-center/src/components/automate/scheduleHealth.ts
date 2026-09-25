/**
 * Is a schedule's last failure still the truth, or history?
 *
 * The daemon keeps `last_status: 'error'` and `last_error` until the job runs
 * again. Resuming an auto-paused job, or fixing its recipe, does not run it —
 * so a job that was repaired and resumed kept a red ERROR chip and a
 * "Last error: …" box until its next fire, reading as broken when it was only
 * waiting (QA 2026-09-24: the flight monitor, fixed and resumed, still said
 * "invalid config: Unknown extension: bravesearch").
 *
 * The facts that decide it, all from `/schedule/list`:
 *  - `last_run`    — stamped when the failing run STARTED;
 *  - `failed_runs` — each failure's own timestamp (when it ended);
 *  - `resumed_at`  — set by every Resume (`unpause_schedule`);
 *  - `recipe_modified_at`, `next_run` — newer daemons only (routes/schedule.rs);
 *    without them the page falls back to resume-only detection and to the
 *    schedule's own words for "next run".
 */

export type ScheduleRunStatus = 'ok' | 'error' | 'skipped' | 'missed';

export interface ScheduleHealthFacts {
  last_status?: ScheduleRunStatus | null;
  last_error?: string | null;
  last_run?: string | null;
  paused?: boolean;
  currently_running?: boolean;
  resumed_at?: string | null;
  failed_runs?: Array<{ timestamp: string; error?: string }> | null;
  recipe_modified_at?: string | null;
  next_run?: string | null;
}

export type LastFailure =
  | { kind: 'none' }
  /** The job is failing (or paused because it failed) right now. */
  | { kind: 'current'; cause: string; failedAt: string | null }
  /** The job failed, then was resumed or its recipe changed; it has not run since. */
  | { kind: 'historical'; cause: string; failedAt: string | null; since: 'resumed' | 'edited'; sinceAt: string };

const time = (iso: string | null | undefined): number | null => {
  if (!iso) return null;
  const t = new Date(iso).getTime();
  return Number.isNaN(t) ? null : t;
};

/**
 * The underlying cause inside an error string. An old daemon wrote its
 * auto-pause notice around the real error ("auto-paused after 3 consecutive
 * failures (resume from Automate once fixed). Last error: invalid config: …"),
 * which the page then prefixed with "Last error: " again. The pause is shown
 * by the PAUSED state; the sentence worth reading is the cause.
 */
export function failureCause(error: string): string {
  const trimmed = error.trim();
  const nested = /^auto-paused\b[\s\S]*?Last error:\s*([\s\S]+)$/i.exec(trimmed);
  return (nested ? nested[1] : trimmed).trim();
}

/** When the last failure happened: its own record, else the run that produced it. */
function failedAtOf(job: ScheduleHealthFacts): string | null {
  const runs = job.failed_runs ?? [];
  let latest: string | null = null;
  for (const r of runs) {
    const t = time(r.timestamp);
    if (t !== null && (latest === null || t > (time(latest) ?? 0))) latest = r.timestamp;
  }
  return latest ?? job.last_run ?? null;
}

export function lastFailure(job: ScheduleHealthFacts): LastFailure {
  const error = job.last_error?.trim();
  if (job.last_status !== 'error' || !error || job.currently_running) return { kind: 'none' };
  const cause = failureCause(error);
  const failedAt = failedAtOf(job);
  // Still paused: nobody has acted on it yet, so it is the current state.
  if (job.paused) return { kind: 'current', cause, failedAt };

  // Compare against the START of the failing run as well as its end, so a
  // resume/edit that raced a still-finishing run is never called a fix.
  const failed = Math.max(time(failedAt) ?? -Infinity, time(job.last_run) ?? -Infinity);
  if (failed === -Infinity) return { kind: 'current', cause, failedAt };
  const resumed = time(job.resumed_at);
  const edited = time(job.recipe_modified_at);
  if (resumed !== null && resumed > failed) {
    return { kind: 'historical', cause, failedAt, since: 'resumed', sinceAt: job.resumed_at! };
  }
  if (edited !== null && edited > failed) {
    return { kind: 'historical', cause, failedAt, since: 'edited', sinceAt: job.recipe_modified_at! };
  }
  return { kind: 'current', cause, failedAt };
}

function shortDate(iso: string): string {
  return new Date(iso).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
}

/** "Mon 10:00 AM" — the one "next run" phrasing, shared by the historical-
 *  failure note and the row's own meta line so they never say it differently. */
export function weekdayTime(iso: string): string {
  const d = new Date(iso);
  const day = d.toLocaleDateString(undefined, { weekday: 'short' });
  const clock = d.toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' });
  return `${day} ${clock}`;
}

/** "Next Mon 10:00 AM", from the daemon's `next_run` (routes/schedule.rs).
 *  `null` while paused/running or on an old daemon that omits the field —
 *  callers fall back to the schedule's own words, same as `historicalFailureNote`. */
export function nextRunLabel(nextRun: string | null | undefined): string | null {
  if (!nextRun || time(nextRun) === null) return null;
  return `Next ${weekdayTime(nextRun)}`;
}

// ── Unacknowledged-failure tracking ─────────────────────────────────────
//
// UX_BENCHMARK_RESEARCH.md §4: an unacknowledged failure is a distinct,
// PERSISTENT row state — not just a red dot that clears itself. It stays
// until a person has actually looked at the failing run, across reloads, so
// it lives in localStorage keyed to the failure that produced it (its own
// `failedAt`) rather than to the job — a NEW failure after an old one was
// acknowledged is unacknowledged again on its own key.

const ACK_PREFIX = 'automate:ack:';

function ackKey(jobId: string, failedAt: string | null): string {
  return `${ACK_PREFIX}${jobId}:${failedAt ?? 'unknown'}`;
}

/** A failure that isn't `current` needs no acknowledgement — it already reads
 *  as history, not an open alert. */
export function isFailureAcknowledged(jobId: string, failure: LastFailure): boolean {
  if (failure.kind !== 'current') return true;
  try {
    return localStorage.getItem(ackKey(jobId, failure.failedAt)) === '1';
  } catch {
    // No storage (private mode, disabled cookies): never claim acknowledged,
    // so the alert stays visible rather than silently disappearing.
    return false;
  }
}

/** Record that a person has seen the failing run — called when they open it. */
export function acknowledgeFailure(jobId: string, failure: LastFailure): void {
  if (failure.kind !== 'current') return;
  try {
    localStorage.setItem(ackKey(jobId, failure.failedAt), '1');
  } catch {
    // Best-effort only.
  }
}

/**
 * One line for a failure the job has moved past:
 * "Last run failed on Sep 20 — resumed since; next run Mon 10:00 AM".
 *
 * "resumed since", not "fixed since": a resume is the user's word that it is
 * fixed, not proof. `scheduleWords` is the fallback when the daemon does not
 * send `next_run` ("next run: Weekdays at 6:00 AM").
 */
export function historicalFailureNote(
  failure: Extract<LastFailure, { kind: 'historical' }>,
  job: Pick<ScheduleHealthFacts, 'next_run'>,
  scheduleWords?: string,
): string {
  const when = failure.failedAt ? ` on ${shortDate(failure.failedAt)}` : '';
  const since = failure.since === 'resumed' ? 'resumed since' : 'recipe edited since';
  const nextRun = nextRunLabel(job.next_run);
  const next = nextRun
    ? nextRun.replace(/^Next /, 'next run ')
    : scheduleWords ? `next run: ${scheduleWords}` : 'waiting for its next run';
  return `Last run failed${when} — ${since}; ${next}`;
}

/**
 * What to say about a job's last/current run — one function so the card's
 * meta line and the detail panel's "Last Run" field can never disagree with
 * each other or with its RUNNING/SCHEDULED/PAUSED chip.
 *
 * QA 2026-09-25: a card reading "RUNNING NOW" also read "Never run yet · 12
 * runs" underneath it — `last_run` (stamped when a run completes, on some
 * daemons only when it STARTS) can lag or predate the in-progress run, and
 * `run_count` disagreeing with "never" made it worse. While a job is
 * running, the run in progress is what's true, not whatever its last
 * finished run says.
 */
export interface RunStateFacts {
  currently_running?: boolean;
  last_run?: string | null;
  process_start_time?: string | null;
}

export type RunState =
  | { kind: 'running'; sinceIso: string | null }
  | { kind: 'ran'; sinceIso: string }
  | { kind: 'never' };

export function runState(job: RunStateFacts): RunState {
  if (job.currently_running) return { kind: 'running', sinceIso: job.process_start_time ?? job.last_run ?? null };
  if (job.last_run) return { kind: 'ran', sinceIso: job.last_run };
  return { kind: 'never' };
}

/** The status chip's word for a job's last result, honest about history. */
export type RunChip = { label: string; token: 'success' | 'danger' | 'warning' | 'textDim'; hint?: string };

export function runChip(job: ScheduleHealthFacts): RunChip | null {
  const failure = lastFailure(job);
  if (failure.kind === 'historical') {
    return { label: 'WAITING', token: 'textDim', hint: 'Waiting for the next run — the last failure was before it was ' + (failure.since === 'resumed' ? 'resumed.' : 'edited.') };
  }
  switch (job.last_status) {
    case 'ok': return { label: 'OK', token: 'success' };
    case 'error': return { label: 'ERROR', token: 'danger', hint: failure.kind === 'current' ? failure.cause : undefined };
    case 'missed': return { label: 'MISSED', token: 'warning' };
    case 'skipped': return { label: 'SKIPPED', token: 'textDim' };
    default: return null;
  }
}
