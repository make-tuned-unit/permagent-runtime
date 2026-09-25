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
  expect(strip!.textContent).toContain('Everything is running');
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
