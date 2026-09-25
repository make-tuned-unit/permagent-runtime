// @vitest-environment jsdom
/**
 * QA 2026-09-24: the flight monitor, resumed after its recipe was fixed,
 * still showed "invalid config: Unknown extension: bravesearch" and an
 * ERROR chip until its next scheduled fire. These pin the rule that tells a
 * still-broken job from one that's only waiting, plus the acknowledgement
 * tracking added for the unfixed-failure alert (UX_BENCHMARK_RESEARCH §4).
 */
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import {
  acknowledgeFailure,
  failureCause,
  historicalFailureNote,
  isFailureAcknowledged,
  lastFailure,
  nextRunLabel,
  runChip,
  type ScheduleHealthFacts,
} from './scheduleHealth';

const base: ScheduleHealthFacts = {
  last_status: 'error',
  last_error: 'invalid config: Unknown extension: bravesearch',
  last_run: '2026-09-20T06:00:00Z',
  paused: false,
  currently_running: false,
};

describe('failureCause', () => {
  it('unwraps the auto-pause wrapper to the real error', () => {
    expect(failureCause(
      'auto-paused after 3 consecutive failures (resume from Automate once fixed). Last error: invalid config: Unknown extension: bravesearch',
    )).toBe('invalid config: Unknown extension: bravesearch');
  });

  it('leaves a plain error untouched', () => {
    expect(failureCause('timeout waiting for response')).toBe('timeout waiting for response');
  });
});

describe('lastFailure', () => {
  it('is "none" for a job with no error on record', () => {
    expect(lastFailure({ ...base, last_status: 'ok', last_error: null })).toEqual({ kind: 'none' });
  });

  it('is "none" while the failing run is still in flight', () => {
    expect(lastFailure({ ...base, currently_running: true })).toEqual({ kind: 'none' });
  });

  it('is "current" while still paused — nobody has acted on it yet', () => {
    const f = lastFailure({ ...base, paused: true });
    expect(f.kind).toBe('current');
  });

  it('is "current" when there is no resume or edit after the failure', () => {
    const f = lastFailure(base);
    expect(f.kind).toBe('current');
  });

  it('is "historical" once resumed after the failing run', () => {
    const f = lastFailure({ ...base, resumed_at: '2026-09-24T09:00:00Z' });
    expect(f).toMatchObject({ kind: 'historical', since: 'resumed', cause: 'invalid config: Unknown extension: bravesearch' });
  });

  it('is "historical" once the recipe was edited after the failing run', () => {
    const f = lastFailure({ ...base, recipe_modified_at: '2026-09-24T09:00:00Z' });
    expect(f).toMatchObject({ kind: 'historical', since: 'edited' });
  });

  it('is still "current" when the resume raced a still-finishing run', () => {
    // The resume landed BEFORE the run that failed even started — a stale
    // resume from an earlier episode, not an acknowledgement of this one.
    const f = lastFailure({ ...base, resumed_at: '2026-09-19T00:00:00Z' });
    expect(f.kind).toBe('current');
  });

  it('uses the latest failed_runs timestamp over last_run when both are present', () => {
    const f = lastFailure({
      ...base,
      last_run: '2026-09-20T06:00:00Z',
      failed_runs: [{ timestamp: '2026-09-22T06:00:00Z', error: 'x' }],
      resumed_at: '2026-09-23T00:00:00Z',
    });
    expect(f.kind).toBe('historical');
    if (f.kind === 'historical') expect(f.failedAt).toBe('2026-09-22T06:00:00Z');
  });
});

describe('nextRunLabel', () => {
  it('is null with no next_run', () => {
    expect(nextRunLabel(null)).toBeNull();
    expect(nextRunLabel(undefined)).toBeNull();
  });

  it('is null for an unparsable value', () => {
    expect(nextRunLabel('not-a-date')).toBeNull();
  });

  it('reads "Next <weekday time>"', () => {
    expect(nextRunLabel('2026-09-28T14:00:00Z')).toMatch(/^Next \w+ /);
  });
});

describe('historicalFailureNote', () => {
  const failure = lastFailure({ ...base, resumed_at: '2026-09-24T09:00:00Z' });

  it('names the day of the failure and says "resumed since"', () => {
    if (failure.kind !== 'historical') throw new Error('fixture');
    const note = historicalFailureNote(failure, { next_run: null }, 'Weekdays at 6:00 AM');
    expect(note).toContain('resumed since');
    expect(note).toContain('next run: Weekdays at 6:00 AM');
  });

  it('prefers the daemon-sent next_run over the schedule words', () => {
    if (failure.kind !== 'historical') throw new Error('fixture');
    const note = historicalFailureNote(failure, { next_run: '2026-09-29T13:00:00Z' }, 'Weekdays at 6:00 AM');
    expect(note).toMatch(/next run \w+ /);
    expect(note).not.toContain('Weekdays at 6:00 AM');
  });

  it('says "recipe edited since" for an edit-cleared failure', () => {
    const edited = lastFailure({ ...base, recipe_modified_at: '2026-09-24T09:00:00Z' });
    if (edited.kind !== 'historical') throw new Error('fixture');
    expect(historicalFailureNote(edited, { next_run: null })).toContain('recipe edited since');
  });
});

describe('runChip', () => {
  it('reads WAITING (neutral) for a historical failure, not ERROR', () => {
    const chip = runChip({ ...base, resumed_at: '2026-09-24T09:00:00Z' });
    expect(chip).toMatchObject({ label: 'WAITING', token: 'textDim' });
  });

  it('reads ERROR with the cause as its hint for a current failure', () => {
    const chip = runChip(base);
    expect(chip).toMatchObject({ label: 'ERROR', token: 'danger' });
    expect(chip?.hint).toContain('bravesearch');
  });

  it('reads OK for a healthy job', () => {
    expect(runChip({ ...base, last_status: 'ok', last_error: null })).toMatchObject({ label: 'OK', token: 'success' });
  });
});

describe('acknowledgement (persists across "reloads" via localStorage)', () => {
  beforeEach(() => localStorage.clear());
  afterEach(() => localStorage.clear());

  it('a non-current failure needs no acknowledgement', () => {
    expect(isFailureAcknowledged('job-1', { kind: 'none' })).toBe(true);
    expect(isFailureAcknowledged('job-1', { kind: 'historical', cause: 'x', failedAt: null, since: 'resumed', sinceAt: 'y' })).toBe(true);
  });

  it('a current failure starts unacknowledged, and stays acknowledged once marked', () => {
    const failure = lastFailure(base);
    expect(isFailureAcknowledged('job-1', failure)).toBe(false);
    acknowledgeFailure('job-1', failure);
    expect(isFailureAcknowledged('job-1', failure)).toBe(true);
  });

  it('a NEW failure (different failedAt) on the same job is unacknowledged again', () => {
    const first = lastFailure(base);
    acknowledgeFailure('job-1', first);
    expect(isFailureAcknowledged('job-1', first)).toBe(true);

    const second = lastFailure({ ...base, last_run: '2026-09-25T06:00:00Z' });
    expect(isFailureAcknowledged('job-1', second)).toBe(false);
  });

  it('acknowledging one job never marks another job\'s identical failure acknowledged', () => {
    const failure = lastFailure(base);
    acknowledgeFailure('job-1', failure);
    expect(isFailureAcknowledged('job-2', failure)).toBe(false);
  });
});
