/**
 * The Automate Hall's honesty pin.
 *
 * The room used to be a blockout: four stone masses and an amber tick on a
 * hardcoded 4-second clock, identical whether the scheduler had twelve live
 * automations, one failing one, or none at all. `schedule_changed` has fired
 * from nine call sites the whole time and no world module listened (agent-QA
 * D19 — the cheapest defect in the World-liveness table, because the wire was
 * already there).
 *
 * These pin the merge, which is where the claims are made: a lit stele is a
 * real registered job, a pulsing tick is a run that is genuinely in flight
 * right now, and an unknown scheduler produces no steles rather than
 * reassuring ones.
 */

import { describe, expect, it } from 'vitest';
import {
  mergeHallJobs,
  HALL_CAPACITY,
  type JobHealthRow,
  type ScheduleRow,
} from './scheduleActivity';

const health = (over: Partial<JobHealthRow> = {}): JobHealthRow => ({
  id: 'job-1',
  label: 'job-1',
  cadence: '0 0 6 * * 1-5',
  outcome: 'ok',
  last_run: '2026-08-31T09:00:00Z',
  failure_streak: 0,
  ...over,
});

const sched = (over: Partial<ScheduleRow> = {}): ScheduleRow => ({
  id: 'job-1',
  cron: '0 0 6 * * 1-5',
  display_name: null,
  last_run: '2026-08-31T09:00:00Z',
  currently_running: false,
  paused: false,
  ...over,
});

describe('Automate Hall schedule binding', () => {
  it('claims nothing when neither source answered', () => {
    expect(mergeHallJobs(null, null)).toEqual([]);
  });

  it('renders a stele per real registered job, health outcome and all', () => {
    const jobs = mergeHallJobs(
      [health({ id: 'a', label: 'The Guard', outcome: 'failed', failure_streak: 3 })],
      [sched({ id: 'a' })],
    );
    expect(jobs).toHaveLength(1);
    expect(jobs[0]).toMatchObject({ id: 'a', label: 'The Guard', outcome: 'failed', running: false });
  });

  it('prefers job-health outcome over the schedule row, which cannot tell "never ran" from "fine"', () => {
    const jobs = mergeHallJobs([health({ id: 'a', outcome: 'never' })], [sched({ id: 'a' })]);
    expect(jobs[0].outcome).toBe('never');
  });

  it('takes the in-flight bit from the schedule list — job-health has no live column', () => {
    const jobs = mergeHallJobs([health({ id: 'a' })], [sched({ id: 'a', currently_running: true })]);
    expect(jobs[0].running).toBe(true);
  });

  it('reads a paused schedule as off, never as healthy', () => {
    const jobs = mergeHallJobs([health({ id: 'a', outcome: 'ok' })], [sched({ id: 'a', paused: true })]);
    expect(jobs[0].outcome).toBe('off');
    expect(jobs[0].running).toBe(false);
  });

  it('still binds when only the schedule list answered', () => {
    const jobs = mergeHallJobs(null, [sched({ id: 'a', display_name: 'Workspace snapshot' })]);
    expect(jobs).toEqual([
      expect.objectContaining({ id: 'a', label: 'Workspace snapshot', outcome: 'ok', running: false }),
    ]);
  });

  it('still binds when only job-health answered — no live column, so nothing claims to be running', () => {
    const jobs = mergeHallJobs([health({ id: 'a' })], null);
    expect(jobs[0].running).toBe(false);
  });

  it('puts the running job first, then the unhealthy ones, so the room reads at a glance', () => {
    const jobs = mergeHallJobs(
      [
        health({ id: 'ok', label: 'ok' }),
        health({ id: 'bad', label: 'bad', outcome: 'missed' }),
        health({ id: 'live', label: 'live' }),
      ],
      [sched({ id: 'live', currently_running: true })],
    );
    expect(jobs.map(j => j.id)).toEqual(['live', 'bad', 'ok']);
  });

  it('caps at the number of steles the gallery actually has', () => {
    const many = Array.from({ length: HALL_CAPACITY + 4 }, (_, i) =>
      health({ id: `j${i}`, label: `j${i}` }));
    expect(mergeHallJobs(many, null)).toHaveLength(HALL_CAPACITY);
  });

  it('drops the audit-chain self-check: the Hall is the scheduler, not the doctor', () => {
    const jobs = mergeHallJobs(
      [health({ id: 'decision_audit_chain', label: 'Decision audit chain' }), health({ id: 'a' })],
      null,
    );
    expect(jobs.map(j => j.id)).toEqual(['a']);
  });
});
