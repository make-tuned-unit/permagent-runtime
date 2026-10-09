/** @vitest-environment jsdom
 *
 * QA 2026-09-24: after Resume, the card stayed PAUSED for ~15s although
 * POST /schedule/{id}/unpause returned 204 — the list was only refetched
 * after the button's success tick (via `guardedAction`) or by the next
 * event/poll. Pause/Resume now refetch the moment the daemon says yes.
 * Also pins that the card reads the ONE cron helper: "0 0 6 * * 1-5" used to
 * render as "Day 6 of every month at 12 AM".
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

const job = (paused: boolean, requiresApproval: boolean) => ({
  id: 'git-steward', source: 'x', cron: '0 0 6 * * 1-5', last_run: null, currently_running: false,
  paused, requires_approval: requiresApproval, current_session_id: null, process_start_time: null, worker_persona: null,
  display_name: 'Git steward', description: null, starter_id: null, starter_version: null,
  user_customized: null, version: null, embedded_version: null,
});

let paused = true;
let requiresApproval = false;
let container: HTMLDivElement;
let root: Root;
beforeEach(() => {
  paused = true;
  requiresApproval = false;
  useCommandCenter.setState({ loadSkills: async () => {}, loadProposals: async () => {}, skills: [], proposals: [] } as never);
  vi.mocked(apiFetch).mockReset().mockImplementation((async (url: string, init?: RequestInit) => {
    if (url === '/schedule/list') return { jobs: [job(paused, requiresApproval)] };
    if (url === '/schedule/git-steward/unpause' && init?.method === 'POST') { paused = false; requiresApproval = false; return undefined; }
    if (url.startsWith('/schedule/sessions_recent')) return { sessions: {} };
    if (url === '/config/extensions') return { extensions: [], warnings: [] };
    if (url === '/api/runs') return { runs: [] };
    return {};
  }) as never);
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});
afterEach(() => { act(() => root.unmount()); container.remove(); });

async function settle() {
  for (let i = 0; i < 6; i++) await act(async () => { await Promise.resolve(); });
}

it('shows the card schedule in words and flips PAUSED right after a successful Resume', async () => {
  await act(async () => { root.render(<AutomateView />); });
  await settle();
  expect(container.textContent).toContain('Weekdays at 6:00 AM');
  expect(container.textContent).not.toContain('Day 6 of every month');
  expect(container.textContent).toContain('PAUSED');
  expect([...container.querySelectorAll('button')].some(b => b.textContent === 'Run Now')).toBe(true);

  const listCallsBefore = vi.mocked(apiFetch).mock.calls.filter(c => c[0] === '/schedule/list').length;
  const resume = [...container.querySelectorAll('button')].find(b => b.textContent === 'Resume')!;
  expect(resume).toBeTruthy();
  vi.useFakeTimers(); // freeze the post-tick refresh: only the immediate one may run
  try {
    await act(async () => { resume.click(); });
    await settle();
  } finally {
    vi.useRealTimers();
  }
  const listCallsAfter = vi.mocked(apiFetch).mock.calls.filter(c => c[0] === '/schedule/list').length;
  expect(listCallsAfter).toBeGreaterThan(listCallsBefore);
  expect(container.textContent).not.toContain('PAUSED');
});

it('holds an approval-pending automation for review and enables it through unpause', async () => {
  requiresApproval = true;
  await act(async () => { root.render(<AutomateView />); });
  await settle();
  expect(container.textContent).toContain('REVIEW NEEDED');
  expect(container.querySelector('[data-testid="automation-approval-note"]')?.textContent).toContain('only after you enable it');
  expect([...container.querySelectorAll('button')].some(b => b.textContent === 'Run Now')).toBe(false);
  expect([...container.querySelectorAll('button')].some(b => b.textContent === 'Resume')).toBe(false);

  const review = [...container.querySelectorAll('button')].find(b => b.textContent === 'Review to enable')!;
  await act(async () => { review.click(); });
  expect(container.querySelector('[data-testid="automation-approval-detail"]')?.textContent).toContain('will not run');
  expect(container.textContent).toContain('Needs review');
  expect([...container.querySelectorAll('button')].some(b => b.textContent === 'Run Now')).toBe(false);

  const enable = [...container.querySelectorAll('button')].find(b => b.textContent === 'Enable schedule')!;
  await act(async () => { enable.click(); });
  await settle();
  expect(apiFetch).toHaveBeenCalledWith('/schedule/git-steward/unpause', expect.objectContaining({ method: 'POST' }));
  expect(container.querySelector('[data-testid="automation-approval-note"]')).toBeNull();
  expect([...container.querySelectorAll('button')].some(b => b.textContent === 'Run Now')).toBe(true);
});
