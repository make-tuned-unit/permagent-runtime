/**
 * The Automate view's health strip: one line that distinguishes paused jobs,
 * reported problems, and an available health digest without inferring a run.
 *
 * Sources:
 *  - `GET /api/job-health` (routes/job_health.rs → permagent::job_health) —
 *    every job the runtime has PROMISED to run, built-in loops included, with
 *    its outcome and failure streak. Built for exactly this question and, until
 *    now, read only by the unmounted world AutomateZone.
 *  - `/schedule/list` rows the view already holds — for what the digest cannot
 *    say: a run in flight for hours (wedged), and a failure the user has since
 *    resumed past (history, not a fault — see `scheduleHealth.lastFailure`).
 *
 * The world's `scheduleActivity.ts` also fetches `/api/job-health`, but it is a
 * module singleton wired to the world socket and merged into Hall steles, so
 * it is not reused here; the row type below mirrors `job_health::ExpectedJob`.
 */
import { lastFailure, type ScheduleHealthFacts } from './scheduleHealth';

export type JobOutcome = 'ok' | 'failed' | 'missed' | 'never' | 'off';

export interface JobHealthRow {
  id: string;
  label: string;
  cadence: string;
  outcome: JobOutcome;
  last_run?: string | null;
  last_success?: string | null;
  failure_streak?: number;
  was_catch_up?: boolean;
  detail?: string | null;
}

export interface JobHealthDigest {
  generated_at?: string;
  all_green?: boolean;
  jobs?: JobHealthRow[];
  dead_letters?: { dead: number; pending: number; oldest_dead_at?: string | null };
  /** Present instead of `jobs` when the runtime database is unavailable. */
  error?: string;
}

export interface HealthScheduleFacts extends ScheduleHealthFacts {
  id: string;
  display_name?: string | null;
  process_start_time?: string | null;
  requires_approval?: boolean;
}

/** A run in flight longer than this is called wedged. Scheduled recipes are
 *  minutes-long; two hours is well past any honest run. */
export const WEDGED_AFTER_MS = 2 * 60 * 60 * 1000;

export type ProblemKind = 'failing' | 'missed' | 'never' | 'wedged' | 'dead_letters';

export interface HealthProblem {
  id: string;
  label: string;
  kind: ProblemKind;
  /** Short reason, e.g. "failed 3 times in a row" or "running for 5h". */
  reason: string;
}

export type HealthSummary =
  | { state: 'unknown'; message: string }
  | { state: 'paused'; message: string }
  | { state: 'ok'; message: string; total: number }
  | { state: 'problems'; message: string; problems: HealthProblem[] };

const KIND_ORDER: Record<ProblemKind, number> = { wedged: 0, failing: 1, missed: 2, never: 3, dead_letters: 4 };

function hours(ms: number): string {
  const h = Math.floor(ms / 3_600_000);
  return h >= 24 ? `${Math.floor(h / 24)}d` : `${h}h`;
}

export function summarizeHealth(
  digest: JobHealthDigest | null,
  schedules: HealthScheduleFacts[],
  now: number = Date.now(),
): HealthSummary {
  const byId = new Map(schedules.map(s => [s.id, s] as const));
  const problems: HealthProblem[] = [];
  const seen = new Set<string>();

  // Wedged runs come from the live list; the digest has no "in flight" column.
  for (const s of schedules) {
    const started = s.currently_running && s.process_start_time ? new Date(s.process_start_time).getTime() : NaN;
    if (!Number.isNaN(started) && now - started > WEDGED_AFTER_MS) {
      problems.push({ id: s.id, label: s.display_name || s.id, kind: 'wedged', reason: `running for ${hours(now - started)}` });
      seen.add(s.id);
    }
  }

  const digestOk = !!digest && !digest.error && Array.isArray(digest.jobs);
  if (digestOk) {
    for (const row of digest!.jobs!) {
      if (seen.has(row.id)) continue;
      const schedule = byId.get(row.id);
      const label = schedule?.display_name || row.label || row.id;
      if (row.outcome === 'failed') {
        // Resumed past, or the recipe changed since: waiting, not failing.
        if (schedule && lastFailure(schedule).kind === 'historical') continue;
        const streak = row.failure_streak ?? 0;
        problems.push({ id: row.id, label, kind: 'failing', reason: streak > 1 ? `failed ${streak} times in a row` : 'last run failed' });
      } else if (row.outcome === 'missed') {
        problems.push({ id: row.id, label, kind: 'missed', reason: 'missed its last window' });
      } else if (row.outcome === 'never') {
        problems.push({ id: row.id, label, kind: 'never', reason: 'has never completed' });
      }
    }
    const dead = digest!.dead_letters?.dead ?? 0;
    if (dead > 0) {
      problems.push({ id: 'dead-letters', label: `${dead} undelivered effect${dead === 1 ? '' : 's'}`, kind: 'dead_letters', reason: 'gave up after retries' });
    }
  }

  if (problems.length > 0) {
    problems.sort((a, b) => KIND_ORDER[a.kind] - KIND_ORDER[b.kind]);
    const n = problems.length;
    return { state: 'problems', problems, message: `${n} need${n === 1 ? 's' : ''} attention` };
  }
  if (!digestOk) {
    // No digest: we cannot claim green. Say what we do not know.
    return { state: 'unknown', message: digest?.error ? `Job health unknown — ${digest.error}` : 'Job health unknown — the daemon did not answer' };
  }
  const paused = schedules.filter(s => s.paused && !s.currently_running);
  const review = paused.filter(s => s.requires_approval).length;
  if (paused.length > 0 && paused.length === schedules.length) {
    const count = paused.length;
    return {
      state: 'paused',
      message: `${count} schedule${count === 1 ? '' : 's'} paused${review ? ` — ${review} need${review === 1 ? 's' : ''} review before enabling` : ''}. No schedule is running.`,
    };
  }
  const total = digest!.jobs!.filter(j => j.outcome !== 'off').length;
  return {
    state: 'ok', total,
    message: `No current job-health problems reported${paused.length ? `; ${paused.length} schedule${paused.length === 1 ? '' : 's'} paused${review ? `, ${review} need${review === 1 ? 's' : ''} review` : ''}` : ''}.`,
  };
}
