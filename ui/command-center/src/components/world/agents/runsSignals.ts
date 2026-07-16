// Runs roster — the daemon's REAL "agents at work" snapshot for the world.
//
// GET /api/runs already aggregates everything running or recently active:
// background workers (Librarian live phase + current task), scheduled jobs
// (with #694 run status: working / idle / error / missed / paused, cron
// detail, started_at) and active agent sessions. The Automate tab consumes it;
// the world never did. This store polls it slowly and the Horologium renders
// the schedule rows as physical tick-lights: one tick per REAL job, lit amber
// only while that job is genuinely running, red on a real error/miss, gray
// when paused. No jobs ⇒ a bare wheel (honest).
//
// Pattern mirrors goalActivity.ts: module store updated only on network
// responses (never per frame), plain getter for useFrame, hook for React.
//
// PURE CORE (unit-tested): horologiumTicks — schedule rows in, deterministic
// wheel model out (stable ordering, arc placement, overflow, running label).

import { useSyncExternalStore } from 'react';
import { apiFetch } from '../../../lib/api';

export interface RunRow {
  kind: 'worker' | 'schedule' | 'session';
  id: string;
  name: string;
  status: 'working' | 'idle' | 'error' | 'missed' | 'paused';
  detail?: string | null;
  started_at?: string | null;
  last_activity?: string | null;
}

interface RunsState {
  schedules: RunRow[];
  workers: RunRow[];
  /** Count of active agent sessions (the daemon's 2-minute window). */
  sessions: number;
  loaded: boolean;
}

let state: RunsState = { schedules: [], workers: [], sessions: 0, loaded: false };
const listeners = new Set<() => void>();

function publish(next: RunsState): void {
  state = next;
  listeners.forEach((l) => l());
}

const POLL_MS = 12_000;
let started = false;

async function refetch(): Promise<void> {
  try {
    const data = await apiFetch<{ runs: RunRow[] }>('/api/runs');
    const rows = data.runs ?? [];
    publish({
      schedules: rows.filter((r) => r.kind === 'schedule'),
      workers: rows.filter((r) => r.kind === 'worker'),
      sessions: rows.filter((r) => r.kind === 'session').length,
      loaded: true,
    });
  } catch {
    // Daemon unreachable: keep the last true snapshot — never fabricate.
  }
}

export function ensureRuns(): void {
  if (started || typeof window === 'undefined') return;
  started = true;
  void refetch();
  setInterval(() => void refetch(), POLL_MS);
}

export function getRuns(): RunsState {
  return state;
}

export function useRuns(): RunsState {
  ensureRuns();
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => state,
  );
}

// ── Pure wheel model ─────────────────────────────────────────────────────────

export interface HorologiumTick {
  id: string;
  name: string;
  status: RunRow['status'];
  /** Placement angle on the wheel face (radians; 0 = top, clockwise). */
  angle: number;
}

export interface HorologiumModel {
  ticks: HorologiumTick[];
  /** Jobs beyond the wheel's capacity — shown as "+N", never fabricated ticks. */
  overflow: number;
  /** Display name of the job running RIGHT NOW (first working row), if any. */
  runningName: string | null;
  anyRunning: boolean;
}

export const HOROLOGIUM_CAP = 12;
/** Ticks spread over this arc of the wheel face, symmetric about the top. */
const ARC_SPAN = (Math.PI * 2 * 300) / 360; // 300° — a clean clockface gap at the bottom

/**
 * Schedules register under their source path (e.g. ".../morning-brief.yaml").
 * Pure display cleanup (unit-tested): basename, extension off, dashes to
 * spaces. Never invents a name — empty in, empty out.
 */
export function prettifyScheduleName(source: string): string {
  const base = source.split('/').pop() ?? source;
  const stem = base.replace(/\.(yaml|yml|json|md)$/i, '');
  return stem.replace(/[-_]+/g, ' ').trim();
}

/**
 * Pure wheel-model derivation (unit-tested).
 * Ordering is stable (by id) so ticks never shuffle between polls; the first
 * `cap` jobs get physical ticks, the rest are an honest "+N" overflow.
 */
export function horologiumTicks(schedules: RunRow[], cap: number = HOROLOGIUM_CAP): HorologiumModel {
  const sorted = [...schedules].sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  const visible = sorted.slice(0, cap);
  const n = visible.length;
  const ticks: HorologiumTick[] = visible.map((row, i) => ({
    id: row.id,
    name: prettifyScheduleName(row.name),
    status: row.status,
    // Single tick sits at the top; multiple spread symmetrically over the arc.
    angle: n <= 1 ? 0 : -ARC_SPAN / 2 + (i / (n - 1)) * ARC_SPAN,
  }));
  const firstRunning = sorted.find((r) => r.status === 'working');
  return {
    ticks,
    overflow: Math.max(0, sorted.length - cap),
    runningName: firstRunning ? prettifyScheduleName(firstRunning.name) : null,
    anyRunning: Boolean(firstRunning),
  };
}
