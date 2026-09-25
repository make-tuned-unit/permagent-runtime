// @vitest-environment jsdom
/**
 * The Automate health strip's rollup — see jobHealthModel.ts. Covers R2's
 * "unknown, not green" rule (a failed /api/job-health fetch must never read
 * as "everything is running") and that a resumed-past failure drops out of
 * the strip the same way it drops out of the card (both read `lastFailure`).
 */
import { describe, expect, it } from 'vitest';
import { summarizeHealth, WEDGED_AFTER_MS, type HealthScheduleFacts, type JobHealthDigest } from './jobHealthModel';

const now = new Date('2026-09-25T12:00:00Z').getTime();

describe('summarizeHealth', () => {
  it('is "unknown" — not "ok" — when the digest fetch failed', () => {
    const digest: JobHealthDigest = { error: 'the daemon did not answer' };
    expect(summarizeHealth(digest, [], now)).toMatchObject({ state: 'unknown' });
  });

  it('is "unknown" before the first fetch resolves (null digest)', () => {
    expect(summarizeHealth(null, [], now)).toMatchObject({ state: 'unknown' });
  });

  it('is "ok" with a count when every job is healthy', () => {
    const digest: JobHealthDigest = { jobs: [{ id: 'a', label: 'A', cadence: 'daily', outcome: 'ok' }] };
    expect(summarizeHealth(digest, [], now)).toMatchObject({ state: 'ok', total: 1 });
  });

  it('excludes disabled (off) jobs from the healthy count', () => {
    const digest: JobHealthDigest = {
      jobs: [
        { id: 'a', label: 'A', cadence: 'daily', outcome: 'ok' },
        { id: 'b', label: 'B', cadence: 'daily', outcome: 'off' },
      ],
    };
    expect(summarizeHealth(digest, [], now)).toMatchObject({ state: 'ok', total: 1 });
  });

  it('flags a failing job with its streak', () => {
    const digest: JobHealthDigest = {
      jobs: [{ id: 'a', label: 'Flight monitor', cadence: 'hourly', outcome: 'failed', failure_streak: 3 }],
    };
    const summary = summarizeHealth(digest, [], now);
    expect(summary.state).toBe('problems');
    if (summary.state === 'problems') {
      expect(summary.problems).toMatchObject([{ kind: 'failing', label: 'Flight monitor', reason: 'failed 3 times in a row' }]);
    }
  });

  it('does not flag a failure the schedule has already resumed past', () => {
    // QA 2026-09-24: fixed + resumed, still said "1 needs attention" until
    // this cross-check landed.
    const digest: JobHealthDigest = { jobs: [{ id: 'a', label: 'Flight monitor', cadence: 'hourly', outcome: 'failed', failure_streak: 3 }] };
    const schedules: HealthScheduleFacts[] = [{
      id: 'a', last_status: 'error', last_error: 'boom', last_run: '2026-09-20T00:00:00Z',
      resumed_at: '2026-09-24T00:00:00Z',
    }];
    expect(summarizeHealth(digest, schedules, now)).toMatchObject({ state: 'ok', total: 1 });
  });

  it('flags a missed and a never-run job by their own words', () => {
    const digest: JobHealthDigest = {
      jobs: [
        { id: 'a', label: 'A', cadence: 'daily', outcome: 'missed' },
        { id: 'b', label: 'B', cadence: 'daily', outcome: 'never' },
      ],
    };
    const summary = summarizeHealth(digest, [], now);
    expect(summary.state).toBe('problems');
    if (summary.state === 'problems') {
      expect(summary.problems.map(p => p.kind).sort()).toEqual(['missed', 'never']);
    }
  });

  it('flags a run stuck past WEDGED_AFTER_MS from the live schedule list, not the digest', () => {
    const schedules: HealthScheduleFacts[] = [{
      id: 'a', currently_running: true,
      process_start_time: new Date(now - WEDGED_AFTER_MS - 60_000).toISOString(),
      display_name: 'Nightly sweep',
    }];
    const summary = summarizeHealth({ jobs: [] }, schedules, now);
    expect(summary.state).toBe('problems');
    if (summary.state === 'problems') expect(summary.problems[0]).toMatchObject({ kind: 'wedged', label: 'Nightly sweep' });
  });

  it('does not call a normal-length run wedged', () => {
    const schedules: HealthScheduleFacts[] = [{
      id: 'a', currently_running: true, process_start_time: new Date(now - 5 * 60_000).toISOString(),
    }];
    expect(summarizeHealth({ jobs: [] }, schedules, now)).toMatchObject({ state: 'ok' });
  });

  it('surfaces undelivered dead letters as their own problem', () => {
    const digest: JobHealthDigest = { jobs: [], dead_letters: { dead: 2, pending: 1 } };
    const summary = summarizeHealth(digest, [], now);
    expect(summary.state).toBe('problems');
    if (summary.state === 'problems') expect(summary.problems[0]).toMatchObject({ kind: 'dead_letters', label: '2 undelivered effects' });
  });

  it('sorts wedged before failing before missed/never/dead-letters', () => {
    const digest: JobHealthDigest = {
      jobs: [
        { id: 'never', label: 'N', cadence: 'x', outcome: 'never' },
        { id: 'missed', label: 'M', cadence: 'x', outcome: 'missed' },
        { id: 'failed', label: 'F', cadence: 'x', outcome: 'failed' },
      ],
    };
    const schedules: HealthScheduleFacts[] = [{
      id: 'wedged', currently_running: true, process_start_time: new Date(now - WEDGED_AFTER_MS - 1000).toISOString(), display_name: 'W',
    }];
    const summary = summarizeHealth(digest, schedules, now);
    expect(summary.state).toBe('problems');
    if (summary.state === 'problems') expect(summary.problems.map(p => p.kind)).toEqual(['wedged', 'failing', 'missed', 'never']);
  });

  it('prefers the live schedule\'s display_name over the digest\'s label', () => {
    const digest: JobHealthDigest = { jobs: [{ id: 'a', label: 'raw-id', cadence: 'x', outcome: 'failed' }] };
    const schedules: HealthScheduleFacts[] = [{ id: 'a', display_name: 'Flight monitor' }];
    const summary = summarizeHealth(digest, schedules, now);
    if (summary.state === 'problems') expect(summary.problems[0].label).toBe('Flight monitor');
  });
});
