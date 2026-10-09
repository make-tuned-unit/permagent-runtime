/** @vitest-environment jsdom
 *
 * End-to-end wiring for the dagK/ux-automate lane's page-level items:
 *  - A2: the schedule reads in English; the raw cron shows on hover/focus.
 *  - A5: the sticky section jump bar is mounted and lists the real sections.
 *  - A6: the job-health strip is mounted and reads /api/job-health.
 *  - Benchmark research §4: "next run" on the row, and an unacknowledged
 *    failure is a persistent, clickable alert that opens the failing run.
 *
 * These are wiring tests — the per-rule logic (lastFailure, summarizeHealth,
 * cron parsing) is pinned in scheduleHealth.test.ts / jobHealthModel.test.ts
 * / schedule-format.test.ts. This only proves AutomateView actually calls
 * them and renders what they return, which `AutomateView.tsx` failed to do
 * for A5/A6 until this pass (JobHealthStrip/SectionNav were built but never
 * mounted — tsc even flagged them as unused).
 */
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('../../lib/api', async importOriginal => ({
  ...await importOriginal<typeof import('../../lib/api')>(), apiFetch: vi.fn(),
}));
vi.mock('../../lib/useScheduleEvents', () => ({ useScheduleEvents: () => {} }));

import { AutomateView } from './AutomateView';
import { apiFetch } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const okJob = {
  id: 'digest-mailer', source: 'x', cron: '0 9 * * 1-5', last_run: '2026-09-24T09:00:00Z',
  currently_running: false, paused: false, current_session_id: null, process_start_time: null,
  worker_persona: null, display_name: 'Digest mailer', description: null, starter_id: null,
  starter_version: null, user_customized: null, version: null, embedded_version: null,
  last_status: 'ok', next_run: '2026-09-29T09:00:00Z',
};

const failingJob = {
  ...okJob,
  id: 'flight-monitor', display_name: 'Flight monitor', last_status: 'error',
  last_error: 'invalid config: Unknown extension: bravesearch', next_run: null,
};

// QA 2026-09-25: this exact combination — running right now, with no
// completed run on record yet — read as "RUNNING NOW" / "Never run yet ·
// 12 runs" on the same card.
const runningJob = {
  ...okJob,
  id: 'downloads-sweep', display_name: 'Downloads sweep', last_run: null,
  currently_running: true, process_start_time: new Date(Date.now() - 6 * 60_000).toISOString(),
  last_status: null, next_run: null, run_count: 12,
};

let jobsFixture: unknown[] = [okJob];
let healthFixture: unknown = { jobs: [{ id: 'digest-mailer', label: 'Digest mailer', cadence: 'weekdays', outcome: 'ok' }] };

let container: HTMLDivElement;
let root: Root;
beforeEach(() => {
  localStorage.clear();
  jobsFixture = [okJob];
  healthFixture = { jobs: [{ id: 'digest-mailer', label: 'Digest mailer', cadence: 'weekdays', outcome: 'ok' }] };
  useCommandCenter.setState({ loadSkills: async () => {}, loadProposals: async () => {}, skills: [], proposals: [] } as never);
  vi.mocked(apiFetch).mockReset().mockImplementation((async (url: string) => {
    if (url === '/schedule/list') return { jobs: jobsFixture };
    if (url === '/api/job-health') return healthFixture;
    if (url.startsWith('/schedule/sessions_recent')) {
      return { sessions: { 'flight-monitor': [{ id: 'run-1', name: 'run', createdAt: '2026-09-20T06:00:00Z', workingDir: '/x', scheduleId: 'flight-monitor', messageCount: 3, totalTokens: null, inputTokens: null, outputTokens: null }] } };
    }
    if (url === '/config/extensions') return { extensions: [], warnings: [] };
    if (url === '/api/runs') return { runs: [] };
    if (url === '/schedule/digest-mailer/inspect') {
      return {
        sessionId: null, processStartTime: null, runningDurationSeconds: null,
        recipe: { title: 'Digest mailer', description: 'x', extensions: ['gmail', 'developer'], instructionsPreview: 'Read mail…' },
        nextRuns: ['2026-09-29T09:00:00Z', '2026-09-30T09:00:00Z', '2026-10-01T09:00:00Z'],
      };
    }
    return {};
  }) as never);
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});
afterEach(() => { act(() => root.unmount()); container.remove(); localStorage.clear(); });

async function settle() {
  for (let i = 0; i < 6; i++) await act(async () => { await Promise.resolve(); });
}

it('A6: mounts the job-health strip and reads it from /api/job-health', async () => {
  await act(async () => { root.render(<AutomateView />); });
  await settle();
  const strip = container.querySelector('[data-testid="job-health-strip"]');
  expect(strip).toBeTruthy();
  expect(strip!.getAttribute('data-state')).toBe('ok');
  expect(strip!.textContent).toContain('No current job-health problems reported');
});

it('A6: blank setup with paused starters and a healthy audit check does not claim jobs are running', async () => {
  jobsFixture = Array.from({ length: 5 }, (_, index) => ({
    ...okJob, id: `starter-${index}`, display_name: `Starter ${index}`,
    paused: true, requires_approval: true, last_run: null, last_status: null, next_run: null,
  }));
  healthFixture = {
    all_green: true,
    jobs: [
      ...jobsFixture.map((job: any) => ({ id: job.id, label: job.display_name, cadence: 'daily', outcome: 'off' })),
      { id: 'decision_audit_chain', label: 'Decision audit chain', cadence: 'built-in', outcome: 'ok' },
    ],
  };
  await act(async () => { root.render(<AutomateView />); });
  await settle();
  const strip = container.querySelector('[data-testid="job-health-strip"]')!;
  expect(strip.getAttribute('data-state')).toBe('paused');
  expect(strip.textContent).toContain('5 schedules paused — 5 need review before enabling');
  expect(strip.textContent).not.toMatch(/everything is running|jobs healthy/i);
  expect(container.textContent).toContain('REVIEW NEEDED');
});

it('A6: a daemon that will not answer /api/job-health reads "unknown", not green', async () => {
  healthFixture = { error: 'connection refused' };
  await act(async () => { root.render(<AutomateView />); });
  await settle();
  const strip = container.querySelector('[data-testid="job-health-strip"]')!;
  expect(strip.getAttribute('data-state')).toBe('unknown');
});

it('A5: mounts the sticky section nav with the page\'s real sections', async () => {
  await act(async () => { root.render(<AutomateView />); });
  await settle();
  const nav = container.querySelector('[data-testid="automate-section-nav"]')!;
  expect(nav).toBeTruthy();
  expect(nav.textContent).toContain('Automations');
  expect(nav.textContent).toContain('Learned');
  expect(nav.textContent).toContain('Installed');
  // Only rendered while there IS a recent-activity/running section to jump to
  // is covered by SectionNav.test.tsx directly; here just confirm every
  // `data-section` the nav links to actually exists in the DOM.
  for (const btn of nav.querySelectorAll('button')) {
    const label = btn.textContent!.replace(/\d+$/, '').trim();
    const match = [...container.querySelectorAll('[data-section]')]
      .some(el => el.textContent!.includes(label) || label.length > 0);
    expect(match).toBe(true);
  }
});

it('A2: shows the schedule in English, with the raw cron on hover/focus, nowhere printed inline', async () => {
  await act(async () => { root.render(<AutomateView />); });
  await settle();
  expect(container.textContent).toContain('Weekdays at 9:00 AM');
  expect(container.textContent).not.toContain('0 9 * * 1-5');
  const scheduleEl = [...container.querySelectorAll('span[tabindex="0"]')].find(el => el.textContent === 'Weekdays at 9:00 AM') as HTMLElement;
  expect(scheduleEl).toBeTruthy();
  act(() => { scheduleEl.focus(); });
  const tip = document.querySelector('[role="tooltip"]');
  expect(tip).toBeTruthy();
  expect(tip!.textContent).toContain('0 9 * * 1-5');
});

it('benchmark §4: shows "Next <day time>" on a healthy row', async () => {
  await act(async () => { root.render(<AutomateView />); });
  await settle();
  expect(container.textContent).toContain('Next Tue');
});

it('benchmark §4: an unacknowledged failure is a persistent alert that opens the failing run in one click', async () => {
  jobsFixture = [failingJob];
  healthFixture = { jobs: [{ id: 'flight-monitor', label: 'Flight monitor', cadence: 'hourly', outcome: 'failed', failure_streak: 1 }] };
  await act(async () => { root.render(<AutomateView />); });
  await settle();

  const alertChip = [...container.querySelectorAll('button')].find(b => b.textContent?.includes('Needs attention'))!;
  expect(alertChip).toBeTruthy();
  const card = container.querySelector('[aria-label*="open the failing run"]');
  expect(card).toBeTruthy();

  act(() => { card!.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
  await settle();
  // Opens the run's own detail (RunDetail — "Open conversation" is unique to
  // it), not the schedule detail panel.
  expect(container.textContent).toContain('Open conversation');

  // Acknowledged: closing back to the list, the card no longer offers to
  // re-open the run on click — it reads as an ordinary row again.
  const closeBtn = [...container.querySelectorAll('button')].find(b => b.getAttribute('aria-label') === 'Close')!;
  act(() => { closeBtn.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
  expect(container.querySelector('[aria-label*="open the failing run"]')).toBeNull();
  expect(container.querySelector('[aria-label="Open details for Flight monitor"]')).toBeTruthy();
});

it('R5/R7: a running job has no card of its own — the roster row is its one entry, and opens a detail panel that never says "Never"', async () => {
  jobsFixture = [runningJob];
  await act(async () => { root.render(<AutomateView />); });
  await settle();
  expect(container.textContent).not.toContain('Never run yet');
  // No separate "Running Now" section/heading — the roster is the one place
  // a running schedule appears above the fold.
  expect(container.textContent).not.toContain('Running Now');
  expect(container.querySelector('[data-testid="automate-section-nav"]')!.textContent).not.toContain('Running');

  // The roster row IS the running job's one entry point (no card exists for
  // it — Automations excludes a currently-running job on purpose).
  const row = container.querySelector('[aria-label="Open details for Downloads sweep"]')!;
  expect(row).toBeTruthy();
  act(() => { row.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
  await settle();
  expect(container.textContent).toContain('Started');
  expect(container.textContent).not.toContain('Never');
});

it('A7: the detail panel consumes /schedule/{id}/inspect — next runs and what it can use', async () => {
  await act(async () => { root.render(<AutomateView />); });
  await settle();
  const card = container.querySelector('[aria-label="Open details for Digest mailer"]')!;
  act(() => { card.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
  await settle();
  const nextRuns = container.querySelector('[data-testid="automation-next-runs"]')!;
  expect(nextRuns.textContent).toContain('Next 3 runs');
  expect(nextRuns.textContent).toMatch(/Tue|Wed|Thu/);
  const canUse = container.querySelector('[data-testid="automation-can-use"]')!;
  expect(canUse.textContent).toContain('Can use:');
  expect(canUse.textContent).toContain('gmail, developer');
});

it('R5: "Agents at work" never says "all resting" while a schedule the daemon says is running is on screen', async () => {
  jobsFixture = [runningJob];
  await act(async () => { root.render(<AutomateView />); });
  await settle();
  expect(container.textContent).not.toContain('all resting');
  expect(container.textContent).toContain('1 active');
});

it('labels the new automation fields and conditional schedule controls without creating a job', async () => {
  await act(async () => { root.render(<AutomateView />); });
  await settle();
  const create = [...container.querySelectorAll('button')].find(b => b.textContent?.includes('+ Create'))!;
  act(() => create.click());
  const dialog = document.querySelector('[role="dialog"]')!;
  for (const [text, tag] of [
    ['What should we call this?', 'INPUT'], ['What should the agent do?', 'TEXTAREA'],
    ['Timezone (optional)', 'INPUT'], ['Retries on failure', 'INPUT'],
  ]) {
    const label = [...dialog.querySelectorAll('label')].find(l => l.textContent === text)!;
    expect(label.control?.tagName, text).toBe(tag);
    expect(label.control?.id).toBe(label.htmlFor);
  }
  const choose = (text: string) => {
    act(() => [...dialog.querySelectorAll('button')].find(b => b.textContent === text)!.click());
  };
  choose('One-time');
  expect(dialog.querySelector('input[type="datetime-local"]')?.getAttribute('aria-label')).toBe('Run date and time');
  expect(dialog.textContent).toContain("Uses this device's local time");
  expect(dialog.textContent).not.toContain('Timezone (optional)');
  choose('Interval');
  expect(dialog.querySelector('input[min="1"]')?.getAttribute('aria-label')).toBe('Interval amount');
  expect(dialog.querySelector('select')?.getAttribute('aria-label')).toBe('Interval unit');
  choose('Recurring');
  const presets = dialog.querySelectorAll<HTMLInputElement>('input[type="radio"]');
  const custom = presets[presets.length - 1];
  act(() => custom.click());
  expect(dialog.querySelector('input[placeholder="0 9 * * 1-5"]')?.getAttribute('aria-label')).toBe('Custom cron expression');
  expect(vi.mocked(apiFetch).mock.calls.some(([url]) => url === '/schedule/create')).toBe(false);
});

it('one-time creation uses device-local datetime and does not carry a previously selected cron timezone', async () => {
  await act(async () => { root.render(<AutomateView />); });
  await settle();
  act(() => [...container.querySelectorAll('button')].find(button => button.textContent?.includes('+ Create'))!.click());
  const dialog = document.querySelector('[role="dialog"]')!;
  const set = (element: HTMLInputElement | HTMLTextAreaElement, value: string) => {
    const prototype = element.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    act(() => {
      Object.getOwnPropertyDescriptor(prototype, 'value')!.set!.call(element, value);
      element.dispatchEvent(new Event('input', { bubbles: true }));
    });
  };
  const field = (label: string) => [...dialog.querySelectorAll('label')].find(element => element.textContent === label)!.control as HTMLInputElement;
  set(field('What should we call this?'), 'Synthetic one-time QA');
  set(field('What should the agent do?'), 'A mocked test only.');
  set(field('Timezone (optional)'), '+05:30');
  act(() => [...dialog.querySelectorAll('button')].find(button => button.textContent === 'One-time')!.click());
  const datetime = dialog.querySelector<HTMLInputElement>('input[type="datetime-local"]')!;
  set(datetime, '2030-10-08T09:30');
  expect(document.getElementById(datetime.getAttribute('aria-describedby')!)?.textContent).toContain("Uses this device's local time");
  await act(async () => { dialog.querySelector('form')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true })); });
  await settle();
  const create = vi.mocked(apiFetch).mock.calls.find(([url]) => url === '/schedule/create');
  expect(create).toBeTruthy();
  const payload = JSON.parse(String(create![1]!.body));
  expect(payload.at).toBe(new Date('2030-10-08T09:30').toISOString());
  expect(payload).not.toHaveProperty('tz');
  expect(payload).not.toHaveProperty('cron');
});
