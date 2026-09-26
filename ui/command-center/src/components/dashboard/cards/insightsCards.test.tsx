/** @vitest-environment jsdom */
/**
 * dagK home-templates lane — the five new synthesis cards backed by
 * `/api/dashboard/insights` (`useDashboardInsights.ts`). Each must cover
 * loading/empty/error/populated per UX_STANDARD R2; these pin the honest-data
 * rules from HOME_TEMPLATES_RESEARCH.md §3 specifically: today_focus never
 * forces a pick, overnight_digest says "Quiet night" rather than four blank
 * rows, fleet_kpis reads "No projects yet" rather than a fabricated 0%, and
 * stalled_pattern hides itself rather than force a pattern from one goal.
 */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { ReactElement } from 'react';

const apiFetch = vi.fn();
vi.mock('../../../lib/api', () => ({ apiFetch: (...a: unknown[]) => apiFetch(...a) }));
const openGoalDetail = vi.fn();
const setPendingProjectNavigation = vi.fn();
vi.mock('../../../lib/store', () => ({
  useCommandCenter: (sel: (s: Record<string, unknown>) => unknown) =>
    sel({ openGoalDetail, setPendingProjectNavigation }),
  navigateToTool: () => {},
}));
vi.mock('../decisions/useDecisions', () => ({ useDecisions: () => ({ data: null, loading: false, error: null }) }));
vi.mock('../decisions/DecisionInbox', () => ({ DecisionInbox: () => null }));

import { TodayFocusCard } from './TodayFocusCard';
import { OvernightDigestCard } from './OvernightDigestCard';
import { FleetKpisCard } from './FleetKpisCard';
import { StalledPatternCard } from './StalledPatternCard';
import { ProjectHealthGridCard } from './ProjectHealthGridCard';
import type { DashboardInsights } from './useDashboardInsights';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;
beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  apiFetch.mockReset();
  openGoalDetail.mockReset();
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function insights(over: Partial<DashboardInsights> = {}): DashboardInsights {
  return {
    todayFocus: { top: null, runnersUp: [] },
    overnightDigest: { finished: [], stalledNew: [], newDecisions: [], newMemories: 0, quiet: true },
    fleetKpis: { projectCount: 0, activeGoalCount: 0, stalledGoalCount: 0, pendingDecisionCount: 0, sessionsToday: 0 },
    stalledPatterns: [],
    projectHealthGrid: [],
    ...over,
  };
}

async function render(el: ReactElement) {
  await act(async () => { root.render(el); });
  await act(async () => { await Promise.resolve(); });
}

// ── TodayFocusCard ──────────────────────────────────────────────────────────

it('TodayFocusCard: never forces a pick when nothing qualifies', async () => {
  apiFetch.mockResolvedValue(insights());
  await render(<TodayFocusCard />);
  expect(container.textContent).toContain('Nothing urgent');
});

it('TodayFocusCard: shows the daemon-ranked pick, its reason, and the runners-up', async () => {
  apiFetch.mockResolvedValue(insights({
    todayFocus: {
      top: { kind: 'decision', refId: 'd1', projectId: null, title: 'Approve the People merge', reason: 'the oldest of 3 decisions waiting, from 2d ago' },
      runnersUp: [{ kind: 'decision', refId: 'd2', projectId: null, title: 'Confirm the bench plan', reason: '#2 oldest of 3 decisions waiting, from 1d ago' }],
    },
  }));
  await render(<TodayFocusCard />);
  expect(container.textContent).toContain('Approve the People merge');
  expect(container.textContent).toContain('2d ago');
  expect(container.textContent).toContain('Confirm the bench plan');
  expect(container.textContent).toContain('Next up');
});

it('TodayFocusCard: reports a fetch failure honestly, not as empty', async () => {
  apiFetch.mockRejectedValue(new Error('offline'));
  await render(<TodayFocusCard />);
  expect(container.textContent).toContain("Couldn't load today's focus");
});

// ── OvernightDigestCard ──────────────────────────────────────────────────────

it('OvernightDigestCard: says "Quiet night" instead of four blank rows', async () => {
  apiFetch.mockResolvedValue(insights());
  await render(<OvernightDigestCard />);
  expect(container.textContent).toContain('Quiet night');
});

it('OvernightDigestCard: shows only the buckets that actually happened, named', async () => {
  apiFetch.mockResolvedValue(insights({
    overnightDigest: {
      finished: [], stalledNew: [], newMemories: 0, quiet: false,
      newDecisions: [{ id: 'd1', title: 'Approve the People merge', projectId: null }, { id: 'd2', title: 'Confirm bench plan', projectId: null }],
    },
  }));
  await render(<OvernightDigestCard />);
  expect(container.textContent).toContain('New decisions');
  expect(container.textContent).toContain('Approve the People merge');
  expect(container.textContent).not.toContain('Newly stalled');
  expect(container.textContent).not.toContain('Finished');
});

// ── FleetKpisCard ────────────────────────────────────────────────────────────

it('FleetKpisCard: reads "No projects yet" rather than a fabricated 0%', async () => {
  apiFetch.mockResolvedValue(insights());
  await render(<FleetKpisCard />);
  expect(container.textContent).toContain('No projects yet');
});

it('FleetKpisCard: shows the real counts once there is a project', async () => {
  apiFetch.mockResolvedValue(insights({
    fleetKpis: { projectCount: 5, activeGoalCount: 8, stalledGoalCount: 2, pendingDecisionCount: 3, sessionsToday: 4 },
  }));
  await render(<FleetKpisCard />);
  expect(container.textContent).toContain('5');
  expect(container.textContent).toContain('8');
});

// ── StalledPatternCard ───────────────────────────────────────────────────────

it('StalledPatternCard: hides itself rather than force a pattern', async () => {
  apiFetch.mockResolvedValue(insights());
  await render(<StalledPatternCard />);
  expect(container.textContent).toContain('Not enough concurrent activity');
});

it('StalledPatternCard: renders the sentence, next step, and each goal by name+project', async () => {
  apiFetch.mockResolvedValue(insights({
    stalledPatterns: [{
      phrase: 'daemon restarts',
      sentence: '2 goals in 2 projects are stuck on the same thing: daemon restarts.',
      nextStep: 'Fixing this unblocks 2 goals.',
      goals: [
        { goalId: 'g1', projectId: 'p1', title: 'Ship the telemetry lane' },
        { goalId: 'g2', projectId: 'p2', title: 'Bench Prime synthesis' },
      ],
    }],
    projectHealthGrid: [
      { projectId: 'p1', projectName: 'Harness Bench', activeGoals: 1, heldGoals: 1, status: 'stalled', reason: null, lastTouched: null, sparkline: [] },
      { projectId: 'p2', projectName: 'Solar Forum', activeGoals: 1, heldGoals: 1, status: 'stalled', reason: null, lastTouched: null, sparkline: [] },
    ],
  }));
  await render(<StalledPatternCard />);
  expect(container.textContent).toContain('stuck on the same thing: daemon restarts');
  expect(container.textContent).toContain('Fixing this unblocks 2 goals');
  expect(container.textContent).toContain('Ship the telemetry lane');
  expect(container.textContent).toContain('Harness Bench');
  const buttons = Array.from(container.querySelectorAll('button')).filter(b => b.textContent?.includes('Bench Prime synthesis'));
  expect(buttons.length).toBe(1);
  (buttons[0] as HTMLButtonElement).click();
  expect(openGoalDetail).toHaveBeenCalledWith('p2', 'g2');
});

// ── ProjectHealthGridCard ────────────────────────────────────────────────────

it('ProjectHealthGridCard: reads "No projects yet" when the fleet is empty', async () => {
  apiFetch.mockResolvedValue(insights());
  await render(<ProjectHealthGridCard />);
  expect(container.textContent).toContain('No projects yet');
});

it('ProjectHealthGridCard: labels a project with no goals "Dormant" with a reason, no fabricated sparkline', async () => {
  apiFetch.mockResolvedValue(insights({
    projectHealthGrid: [{ projectId: 'p1', projectName: 'Solar Forum', activeGoals: 0, heldGoals: 0, status: 'dormant', reason: 'No active goals right now', lastTouched: null, sparkline: [] }],
  }));
  await render(<ProjectHealthGridCard />);
  expect(container.textContent).toContain('Solar Forum');
  expect(container.textContent).toContain('Dormant');
  expect(container.textContent).toContain('No active goals right now');
  expect(container.textContent).toContain('No activity history yet');
});

it('ProjectHealthGridCard: clicking a row navigates to that project', async () => {
  apiFetch.mockResolvedValue(insights({
    projectHealthGrid: [{ projectId: 'p1', projectName: 'Solar Forum', activeGoals: 2, heldGoals: 0, status: 'active', reason: null, lastTouched: null, sparkline: [] }],
  }));
  await render(<ProjectHealthGridCard />);
  const row = container.querySelector('button');
  row?.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  expect(setPendingProjectNavigation).toHaveBeenCalledWith('p1');
});
