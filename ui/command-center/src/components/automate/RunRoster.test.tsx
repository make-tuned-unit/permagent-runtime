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
import { rosterDetail, rosterProblem } from './runRosterModel';

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
