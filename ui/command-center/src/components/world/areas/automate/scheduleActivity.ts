/**
 * Automate Hall ← the real scheduler (agent-QA D19).
 *
 * The honesty boundary for the Hall, exactly as `agents/goalActivity.ts` is for
 * the Workshop bay: every lit stele in the room is a job the runtime has really
 * promised to do, and the only thing that pulses is a run that is genuinely in
 * flight right now.
 *
 * Why this file exists at all: `schedule_changed` has been emitted from nine
 * call sites (`scheduler.rs` included, `run_started` among them) since the
 * 2026-08-25 polling-storm fix, and no world module ever subscribed. The flat
 * Automate tab was correctly live-wired; the room built to *show* the scheduler
 * was four stone masses on a hardcoded 4-second amber clock that looked
 * identical whether twelve automations were healthy, one was failing, or none
 * existed. Nothing here needed a new event — only a listener.
 *
 * TWO SOURCES, because neither alone is the truth:
 *
 *   /api/job-health  — the richer one, and the row set. It knows the thing a
 *                      schedule row cannot say: `outcome` (ok / failed /
 *                      missed / never / off) and the failure streak. "Ran
 *                      daily and has not worked in three weeks" is a sentence
 *                      only this endpoint can form.
 *   /schedule/list   — the live column. `currently_running` and `paused` exist
 *                      nowhere else, and a pulse is a claim about *now*.
 *
 * Either may answer alone. Neither answering means no steles — an unknown
 * scheduler renders as an empty gallery, never as a reassuring one.
 *
 * Pattern mirrors goalActivity/worldSignals: a module store updated only on
 * network events (never per frame), a plain getter for `useFrame` consumers,
 * and a `useSyncExternalStore` hook for React.
 */

import { useSyncExternalStore } from 'react';
import { apiFetch } from '../../../../lib/api';
import { subscribeWorldEvents } from '../../shared/worldEvents';

/** What the last run of a registered job came to. Mirrors `job_health::Outcome`. */
export type HallOutcome = 'ok' | 'failed' | 'missed' | 'never' | 'off';

/** One row of `/api/job-health`'s truth table (the fields the Hall reads). */
export interface JobHealthRow {
  id: string;
  label: string;
  cadence: string;
  outcome: HallOutcome;
  last_run?: string | null;
  failure_streak?: number;
}

/** One row of `/schedule/list` (the fields the Hall reads). */
export interface ScheduleRow {
  id: string;
  cron?: string | null;
  display_name?: string | null;
  last_run?: string | null;
  currently_running?: boolean;
  paused?: boolean;
}

/** One stele. */
export interface HallJob {
  id: string;
  label: string;
  cadence: string;
  outcome: HallOutcome;
  /** In flight RIGHT NOW. The only thing in the room allowed to pulse. */
  running: boolean;
  lastRun: string | null;
  failureStreak: number;
}

export interface ScheduleActivityState {
  jobs: HallJob[];
  /** False until a source has answered once. No answer ⇒ no claims. */
  loaded: boolean;
  /** When the reading was last confirmed, for anything that wants to say so. */
  asOf: number | null;
}

/** The gallery has six stele plinths — the room shows at most six jobs. */
export const HALL_CAPACITY = 6;

/**
 * `job_health` also reports its own audit-chain self-check, which is the
 * doctor's business rather than the scheduler's. The Hall is the room for
 * things the user scheduled; a verifier reading its own ledger is not one.
 * (Mirrors `job_health::AUDIT_CHAIN_CHECK_ID`.)
 */
const NOT_A_SCHEDULE = new Set(['decision_audit_chain']);

const UNHEALTHY: ReadonlySet<HallOutcome> = new Set<HallOutcome>(['failed', 'missed', 'never']);

/**
 * The merge — every claim the room makes is decided here, which is why it is
 * pure and pinned by tests rather than buried in a fetch.
 */
export function mergeHallJobs(
  health: JobHealthRow[] | null,
  schedules: ScheduleRow[] | null,
): HallJob[] {
  const live = new Map<string, ScheduleRow>();
  for (const row of schedules ?? []) live.set(row.id, row);

  const merged: HallJob[] = [];
  const seen = new Set<string>();

  const push = (job: HallJob) => {
    if (NOT_A_SCHEDULE.has(job.id) || seen.has(job.id)) return;
    seen.add(job.id);
    merged.push(job);
  };

  for (const row of health ?? []) {
    const l = live.get(row.id);
    // A paused schedule is OFF, whatever its last run came to — "ok" on a
    // switched-off job is the room claiming a cadence that is not running.
    const paused = l?.paused === true;
    push({
      id: row.id,
      label: row.label || row.id,
      cadence: row.cadence || l?.cron || '',
      outcome: paused ? 'off' : row.outcome,
      running: !paused && l?.currently_running === true,
      lastRun: row.last_run ?? l?.last_run ?? null,
      failureStreak: row.failure_streak ?? 0,
    });
  }

  // Schedules job-health did not cover (or job-health not answering at all).
  // Without its outcome column all this row can honestly say is "registered":
  // `ok` here means "no failure reported", not "verified healthy".
  for (const row of schedules ?? []) {
    push({
      id: row.id,
      label: row.display_name || row.id,
      cadence: row.cron ?? '',
      outcome: row.paused === true ? 'off' : 'ok',
      running: row.paused !== true && row.currently_running === true,
      lastRun: row.last_run ?? null,
      failureStreak: 0,
    });
  }

  // Reading order, because only six fit: what is happening now, then what is
  // wrong, then everything else by how recently it ran.
  merged.sort((a, b) => {
    if (a.running !== b.running) return a.running ? -1 : 1;
    const au = UNHEALTHY.has(a.outcome);
    const bu = UNHEALTHY.has(b.outcome);
    if (au !== bu) return au ? -1 : 1;
    return (Date.parse(b.lastRun ?? '') || 0) - (Date.parse(a.lastRun ?? '') || 0);
  });

  return merged.slice(0, HALL_CAPACITY);
}

let state: ScheduleActivityState = { jobs: [], loaded: false, asOf: null };
const subscribers = new Set<() => void>();

function publish(next: ScheduleActivityState): void {
  state = next;
  subscribers.forEach((fn) => fn());
}

interface JobHealthDigest {
  jobs?: JobHealthRow[];
  /** Present instead of `jobs` when the runtime database is unavailable. */
  error?: string;
}

async function refetch(): Promise<void> {
  const [healthRes, listRes] = await Promise.allSettled([
    apiFetch<JobHealthDigest>('/api/job-health'),
    apiFetch<{ jobs?: ScheduleRow[] }>('/schedule/list'),
  ]);

  const health =
    healthRes.status === 'fulfilled' && !healthRes.value.error
      ? healthRes.value.jobs ?? []
      : null;
  const schedules =
    listRes.status === 'fulfilled' ? listRes.value.jobs ?? [] : null;

  // Both silent: keep the last thing that was true rather than blanking the
  // room, and do not move `asOf` — a stale reading must not read as fresh.
  if (health === null && schedules === null) return;

  publish({ jobs: mergeHallJobs(health, schedules), loaded: true, asOf: Date.now() });
}

let started = false;

/** Idempotent; the Automate zone calls it on mount. */
export function ensureScheduleActivity(): void {
  if (started) return;
  started = true;
  void refetch();
  // The live wire, on the world's ONE socket (shared/worldEvents): every
  // schedule create/update/delete/pause/unpause/run_now, and `run_started` /
  // run completion from the scheduler itself. Refetch is idempotent, so a
  // replayed frame is a harmless trigger and the API stays the source of truth.
  subscribeWorldEvents((evt) => {
    if (evt.type === 'schedule_changed') void refetch();
  });
  // Backstop for whatever the socket misses (a dropped connection), on the
  // same 60s cadence as the Workshop bay's.
  setInterval(() => void refetch(), 60_000);
}

/** Plain snapshot for useFrame consumers (zero-alloc law: no copies). */
export function getHallJobs(): HallJob[] {
  return state.jobs;
}

export function useScheduleActivity(): ScheduleActivityState {
  ensureScheduleActivity();
  return useSyncExternalStore(
    (fn) => {
      subscribers.add(fn);
      return () => subscribers.delete(fn);
    },
    () => state,
  );
}

/** Test seam — resets the module store between cases. */
export function __resetScheduleActivity(): void {
  started = false;
  state = { jobs: [], loaded: false, asOf: null };
  subscribers.clear();
}
