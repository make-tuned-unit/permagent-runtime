/**
 * @vitest-environment jsdom
 *
 * QA 2026-09-24, Automate → "Agents at work":
 *  - schedule rows showed the raw croner expression ("0 0 6 * * 1-5");
 *  - last-run ages read "82h 3m ago" / "174h 3m ago";
 *  - two schedules auto-paused after 3 failures showed that only as a muted
 *    dot colour behind a hover tooltip, with no error and no way to resume.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const { apiFetch } = vi.hoisted(() => ({ apiFetch: vi.fn() }));
vi.mock('../../lib/api', () => ({ apiFetch }));

import { RunRoster } from './RunRoster';
import { reconcileRosterRuns, rosterDetail, rosterProblem, type RosterRunItem, type RosterScheduleFacts } from './runRosterModel';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const hoursAgo = (h: number) => new Date(Date.now() - h * 3_600_000 - 3 * 60_000).toISOString();

const RUNS = [
  { kind: 'schedule', id: 'morning-brief', name: 'Morning brief', status: 'idle', detail: '0 0 6 * * 1-5', last_activity: hoursAgo(82), started_at: null, interruptible: false },
  { kind: 'schedule', id: 'git-steward', name: 'Git steward', status: 'paused', detail: '0 0 10 * * 1,4', last_activity: hoursAgo(174), started_at: null, interruptible: false },
];
const SCHEDULES = [
  { id: 'morning-brief', cron: '0 0 6 * * 1-5', paused: false },
  { id: 'git-steward', cron: '0 0 10 * * 1,4', paused: true, last_error: 'git push: authentication failed', consecutive_failures: 3 },
];

let container: HTMLDivElement;
let root: Root;
beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  apiFetch.mockReset().mockImplementation(async (url: string) => {
    if (url === '/api/runs') return { runs: RUNS };
    return {};
  });
});
afterEach(() => { act(() => root.unmount()); container.remove(); });

async function mount() {
  await act(async () => { root.render(<RunRoster schedules={SCHEDULES} />); });
  await act(async () => { await Promise.resolve(); await Promise.resolve(); });
}

describe('runRosterModel (pure)', () => {
  it('puts a schedule in words and keeps the expression as the secondary', () => {
    expect(rosterDetail('schedule', '0 0 6 * * 1-5')).toEqual({ words: 'Weekdays at 6:00 AM', raw: '0 0 6 * * 1-5' });
    expect(rosterDetail('schedule', '0 30 19 * * 0').words).toBe('Sundays at 7:30 PM');
    expect(rosterDetail('schedule', 'every 3600s')).toEqual({ words: 'Every 1 hour', raw: null });
  });
  it('an unrecognised expression is shown once, raw, not twice', () => {
    expect(rosterDetail('schedule', '0 0 9 L * *')).toEqual({ words: '0 0 9 L * *', raw: null });
  });
  it('workers and sessions keep their detail as sent', () => {
    expect(rosterDetail('worker', 'describing memories')).toEqual({ words: 'describing memories', raw: null });
  });
  it('says why a schedule paused', () => {
    expect(rosterProblem('paused', SCHEDULES[1])).toBe('Paused after 3 failed runs: git push: authentication failed');
    expect(rosterProblem('paused', { id: 'x' })).toBeNull();
    expect(rosterProblem('error', { id: 'x', last_error: 'boom' })).toBe('Last error: boom');
    expect(rosterProblem('idle', { id: 'x', last_error: 'old' })).toBeNull();
  });
});

describe('reconcileRosterRuns — one source of truth for "is it running" (R5)', () => {
  const idleRun: RosterRunItem = {
    kind: 'schedule', id: 'downloads-sweep', name: 'Downloads sweep', status: 'idle',
    detail: null, last_activity: null, started_at: null, interruptible: false,
  };

  it('upgrades a stale-idle roster row when the schedule list says it is running', () => {
    const facts: RosterScheduleFacts[] = [{ id: 'downloads-sweep', currently_running: true, process_start_time: '2026-09-25T00:00:00Z' }];
    const [row] = reconcileRosterRuns([idleRun], facts);
    expect(row).toMatchObject({ status: 'working', started_at: '2026-09-25T00:00:00Z' });
  });

  it('synthesizes a working row for a schedule missing from /api/runs entirely', () => {
    const facts: RosterScheduleFacts[] = [{ id: 'downloads-sweep', currently_running: true, display_name: 'Downloads sweep', process_start_time: '2026-09-25T00:00:00Z' }];
    const rows = reconcileRosterRuns([], facts);
    expect(rows).toEqual([{
      kind: 'schedule', id: 'downloads-sweep', name: 'Downloads sweep', status: 'working',
      detail: null, last_activity: null, started_at: '2026-09-25T00:00:00Z', interruptible: true,
    }]);
  });

  it('downgrades a stale-working roster row once the schedule list says it stopped', () => {
    const workingRun: RosterRunItem = { ...idleRun, status: 'working' };
    const facts: RosterScheduleFacts[] = [{ id: 'downloads-sweep', currently_running: false }];
    const [row] = reconcileRosterRuns([workingRun], facts);
    expect(row.status).toBe('idle');
  });

  it('leaves a row alone when the schedule facts do not say either way (old fixture, no field)', () => {
    const workingRun: RosterRunItem = { ...idleRun, status: 'working' };
    const [row] = reconcileRosterRuns([workingRun], [{ id: 'downloads-sweep' }]);
    expect(row.status).toBe('working');
  });

  it('never touches a non-schedule row', () => {
    const worker: RosterRunItem = { kind: 'worker', id: 'w1', name: 'Librarian', status: 'idle', detail: null, last_activity: null, started_at: null, interruptible: false };
    const [row] = reconcileRosterRuns([worker], [{ id: 'w1', currently_running: true }]);
    expect(row.status).toBe('idle');
  });
});

describe('RunRoster rows', () => {
  it('shows schedules in plain words with the expression as a secondary', async () => {
    await mount();
    const text = container.textContent ?? '';
    expect(text).toContain('Weekdays at 6:00 AM');
    expect(text).toContain('Mon & Thu at 10:00 AM');
    const raw = Array.from(container.querySelectorAll('[data-testid="roster-cron"]')).map(e => e.textContent);
    expect(raw).toEqual(expect.arrayContaining(['0 0 6 * * 1-5', '0 0 10 * * 1,4']));
  });

  it('reads last-run ages in days, not "82h 3m ago"', async () => {
    await mount();
    const text = container.textContent ?? '';
    expect(text).toContain('3d ago');
    expect(text).toContain('7d ago');
    expect(text).not.toMatch(/\d{2,}h \d+m ago/);
  });

  it('never says "all resting" while a schedule the daemon says is running is on screen (R5)', async () => {
    apiFetch.mockReset().mockImplementation(async (url: string) => {
      // /api/runs hasn't caught up yet — an empty aggregate, same shape as
      // the QA repro (a brand-new run, the aggregate poll a beat behind).
      if (url === '/api/runs') return { runs: [] };
      return {};
    });
    await act(async () => {
      root.render(<RunRoster schedules={[{
        id: 'downloads-sweep', display_name: 'Downloads sweep', currently_running: true,
        process_start_time: new Date().toISOString(),
      }]} />);
    });
    await act(async () => { await Promise.resolve(); await Promise.resolve(); });
    const text = container.textContent ?? '';
    expect(text).not.toContain('all resting');
    expect(text).toContain('1 active');
    expect(text).toContain('Downloads sweep');
  });

  it('labels a paused row, says why, and resumes it through /schedule/{id}/unpause', async () => {
    await mount();
    expect(container.querySelectorAll('[data-testid="roster-paused"]')).toHaveLength(1);
    expect(container.querySelector('[data-testid="roster-problem"]')!.textContent)
      .toBe('Paused after 3 failed runs: git push: authentication failed');
    const resume = Array.from(container.querySelectorAll('button')).find(b => b.textContent === 'Resume')!;
    expect(resume).toBeTruthy();
    await act(async () => { resume.click(); });
    expect(apiFetch).toHaveBeenCalledWith('/schedule/git-steward/unpause', { method: 'POST' });
  });
});
