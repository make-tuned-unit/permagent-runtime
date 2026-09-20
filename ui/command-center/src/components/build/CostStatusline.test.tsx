/**
 * @vitest-environment jsdom
 *
 * CostStatusline render tests:
 * - the UI half of the Build-tab cost-meter fix (coding-harness spend via
 *   `session_spend_changed` through `applyLivenessFrame`)
 * - the subagent suffix: when a parent session has child spend, the meter
 *   shows `incl. N subagents $X` beside the running total
 *
 * useLiveGoals is mocked: it polls /api/goals/active and opens its own event
 * subscription, neither of which this test needs or wants touching the network.
 */
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';
import restartProjection from '../../../../../scripts/testdata/budget_projection_v1.json';

const { getActiveHarnessRuns } = vi.hoisted(() => ({ getActiveHarnessRuns: vi.fn() }));

vi.mock('../../lib/useLiveGoals', () => ({
  useLiveGoals: () => ({ goals: [], activeCount: 0, loaded: true, refresh: () => {} }),
}));

vi.mock('../../lib/api', () => ({
  api: {
    getActiveHarnessRuns,
    // The statusline's idle model control reads the harness default.
    readConfig: vi.fn(async () => null),
    setModelRoute: vi.fn(async () => ({})),
    getSessionCost: vi.fn(async () => ({
      own: 0.42,
      childrenTotal: 0.17,
      perChild: [
        { sessionId: 'child-a', costUsd: 0.1 },
        { sessionId: 'child-b', costUsd: 0.07 },
      ],
    })),
    getPacks: vi.fn(async () => ({
      prompt: false,
      configured: [],
      recommendation: { recommendations: [], considered: [] },
    })),
  },
}));

import { CostStatusline } from './CostStatusline';
import { useCommandCenter } from '../../lib/store';
import { api } from '../../lib/api';
import { applyLivenessFrame, _resetLivenessSync } from '../../lib/livenessSync';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement | null = null;
let root: Root | null = null;

function resetStore() {
  useCommandCenter.setState({
    liveTokens: {
      inputTokens: 0,
      outputTokens: 0,
      totalTokens: 0,
      accumulatedInputTokens: 1000,
      accumulatedOutputTokens: 200,
      accumulatedTotalTokens: 1200,
      costUsd: 0.01,
      accumulatedCostUsd: 0.42,
      cacheSavingsUsd: 0,
      contextPercent: null,
      model: '',
    },
    codingSpend: null,
    codingSpendLastKnown: null,
    codingHarnessHydration: 'initial',
    codingHarnessRunId: null,
    codingHarnessRevision: 0,
    chatSessionId: 'parent-1',
  });
}

beforeEach(() => {
  _resetLivenessSync();
  resetStore();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  container?.remove();
  container = null;
  root = null;
  _resetLivenessSync();
  resetStore();
  vi.clearAllMocks();
});

describe('CostStatusline', () => {
  it('shows $0.00 with an empty store, then updates to the coding-harness session total off a live wire frame', async () => {
    useCommandCenter.setState({ liveTokens: null, codingSpend: null, chatSessionId: null });
    await act(async () => {
      root!.render(<CostStatusline />);
    });
    expect(container!.textContent).toContain('$0.00');

    await act(async () => {
      applyLivenessFrame(
        {
          id: 'evt-1',
          type: 'session_spend_changed',
          timestamp: new Date().toISOString(),
          payload: {
            session_id: 'harness-1',
            turn_usd: 0.0032,
            session_usd: 0.0332,
            today_usd: 0.5332,
            total_tokens: 12800,
            provider: 'zai',
            model: 'glm-5.3',
            working_dir: '/tmp/proj',
            estimated: false,
            final_turn: false,
          },
        },
        Date.parse('2020-01-01T00:00:00Z'),
      );
    });

    const costSpan = container!.querySelector('span[style*="font-weight"]');
    expect(costSpan?.textContent).toBe('$0.03');
  });

  it('renders hydration unavailable instead of substituting browser chat cost', async () => {
    useCommandCenter.setState({
      liveTokens: {
        inputTokens: 1, outputTokens: 1, totalTokens: 2,
        accumulatedInputTokens: 100, accumulatedOutputTokens: 100,
        accumulatedTotalTokens: 200, costUsd: 99, accumulatedCostUsd: 99,
        cacheSavingsUsd: 0, contextPercent: null, model: 'chat-model',
      },
      codingSpend: null,
      codingHarnessHydration: 'unavailable',
    });
    await act(async () => {
      root!.render(<CostStatusline />);
    });
    expect(container!.textContent).toContain('Budget unavailable');
    expect(container!.textContent).not.toContain('$99.00');
  });

  it('shows incl. N subagents $X when the parent cost rollup has children', async () => {
    await act(async () => {
      root!.render(<CostStatusline />);
    });
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(api.getSessionCost).toHaveBeenCalledWith('parent-1');
    expect(container!.textContent).toContain('incl. 2 subagents $0.17');
    expect(container!.textContent).toContain('$0.42');
  });

  it('does not attribute the previous session child rollup to a new session while loading', async () => {
    await act(async () => { root!.render(<CostStatusline />); });
    expect(container!.textContent).toContain('incl. 2 subagents $0.17');
    let finish!: (value: Awaited<ReturnType<typeof api.getSessionCost>>) => void;
    vi.mocked(api.getSessionCost).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
    await act(async () => { useCommandCenter.setState({ chatSessionId: 'parent-2' }); });
    expect(container!.textContent).not.toContain('incl. 2 subagents $0.17');
    await act(async () => { finish({ own: 1, childrenTotal: 0.5, perChild: [{ sessionId: 'new-child', costUsd: 0.5 }] }); });
    expect(container!.textContent).toContain('incl. 1 subagent $0.50');
  });

  it('renders the Rust restart-ledger golden identically through events and reload hydration', async () => {
    // Rust's real reopened-ledger test asserts this same fixture in full.
    // Only nondeterministic identities/time are replaced on both sides.
    const budget = {
      ...restartProjection,
      rootSessionId: 'restarted-harness',
      taskId: 'restarted-task',
      provenance: { ...restartProjection.provenance, asOf: '2026-09-05T12:00:00Z' },
    };
    vi.mocked(api.getSessionCost).mockResolvedValue({ own: 0, childrenTotal: 0, perChild: [] });
    await act(async () => {
      root!.render(<CostStatusline />);
      applyLivenessFrame({
        id: 'restart-golden-event', type: 'session_spend_changed',
        timestamp: '2026-09-05T12:00:00Z',
        payload: {
          session_id: budget.rootSessionId, budget,
          session_usd: 0, turn_usd: 0, today_usd: 0, total_tokens: 0,
          provider: null, model: null, working_dir: '/tmp/fixture',
          estimated: false, final_turn: false,
        },
      }, Date.parse('2026-09-05T11:59:00Z'));
    });
    expect(useCommandCenter.getState().codingSpend?.budget).toEqual(budget);
    const eventText = container!.textContent;
    // These figures come from the shared Rust golden
    // (scripts/testdata/budget_projection_v1.json), and they changed on
    // 2026-09-09 in ce4f111b, "Reconcile stale unknown provider-spend
    // reservations instead of refusing forever": that lane moved the fixture
    // out of the unknown-spend state it used to model (unknown $0.75,
    // remaining $0.25) into a settled one, and these assertions were not
    // updated with it. Reading them off the fixture keeps the two sides of the
    // golden honest the next time it moves.
    // Wording changed deliberately on 2026-09-18: the bar used to print the
    // ledger inline ("session settled $0.00 · session used $0.10 · session
    // remaining $0.90 · session held $0.10 · session billing paid_api"), and
    // The user, reading a line like it, said "I am unsure what all of this is".
    // It now leads with spend and says the rest in plain words; this fixture is
    // complete and fully priced, so its remainder is stated without a hedge.
    const costSpan = container!.querySelector('span[style*="font-weight"]');
    expect(costSpan?.textContent).toBe('$0.10');
    expect(eventText).toContain('$0.10 pending');
    expect(eventText).toContain('$0.90 left');
    expect(eventText).not.toContain('unconfirmed');
    expect(eventText).not.toContain('session settled');
    expect(eventText).not.toContain('session billing');
    getActiveHarnessRuns.mockResolvedValueOnce([{
      runId: 'restarted-run', sessionId: budget.rootSessionId,
      project: '/tmp/fixture', status: 'waiting_gate',
      updatedAt: budget.provenance.asOf, tokens: 0,
      provider: null, model: null, budget,
    }]);
    await act(async () => {
      useCommandCenter.setState({
        codingSpend: null, codingSpendLastKnown: null,
        codingHarnessRunId: null, codingHarnessHydration: 'initial',
      });
      await useCommandCenter.getState().hydrateCodingHarness();
    });
    expect(container!.textContent).toBe(eventText);
    expect(useCommandCenter.getState().codingHarnessRunId).toBe('restarted-run');
    expect(useCommandCenter.getState().codingSpend?.budget).toEqual(budget);
  });

  it('renders the reported bar in plain words, qualifies the remainder, and keeps the detail in the tooltip', async () => {
    // The user's screenshot of the Build cost bar: "$2.09 · session settled $0.53 ·
    // session used $2.09 · session remaining $47.91 · session unknown $1.56 ·
    // session budget unknown · task settled ..." — "I am unsure what all of
    // this is". Same figures through the real liveness path and component.
    const scope = {
      ...restartProjection.session,
      cap: { softUsd: 40, gateUsd: 45, hardUsd: 50, source: 'current_budget_config' },
      settledUsd: 0.53, heldUsd: 0, unknownUsd: 1.56,
      effectiveUsedUsd: 2.09, remainingUsd: 47.91,
      band: 'unknown', completeness: 'partial',
    };
    const budget = {
      ...restartProjection,
      rootSessionId: 'reported-harness', taskId: 'reported-task',
      session: scope, task: scope,
      provenance: { ...restartProjection.provenance, asOf: '2026-09-18T12:00:00Z' },
    };
    vi.mocked(api.getSessionCost).mockResolvedValue({ own: 0, childrenTotal: 0, perChild: [] });
    await act(async () => {
      root!.render(<CostStatusline />);
      applyLivenessFrame({
        id: 'reported-event', type: 'session_spend_changed',
        timestamp: '2026-09-18T12:00:00Z',
        payload: {
          session_id: budget.rootSessionId, budget,
          session_usd: 2.09, turn_usd: 0, today_usd: 2.09, total_tokens: 0,
          provider: null, model: null, working_dir: '/tmp/fixture',
          estimated: false, final_turn: false,
        },
      }, Date.parse('2026-09-18T11:59:00Z'));
    });
    const status = container!.querySelector('[role="status"]')!;
    const text = status.textContent!;
    expect(text).toContain('$2.09');
    // "$1.56 unconfirmed": the daemon's unknown_usd is budget held for calls
    // whose charge could not be confirmed, not spend lacking a price.
    expect(text).toContain('$1.56 unconfirmed');
    expect(text).not.toContain('unpriced');
    expect(text).toContain('~$47.91 left (unconfirmed)');
    // The self-contradiction the user saw: a flat remainder beside "unknown".
    expect(text).not.toMatch(/(^|[^~])\$47\.91 left(?! \(unconfirmed\))/);
    expect(text).not.toContain('budget unknown');
    expect(text).not.toContain('session');
    expect(status.getAttribute('aria-label')).toContain('$1.56 of that is held for calls whose charge could not be confirmed');
    expect(status.getAttribute('aria-label')).toContain('not confirmed');
    // The model picker group is still its own right-hand sibling, untouched.
    expect(container!.querySelector('[data-testid="statusline-model-default"]')).not.toBeNull();
    expect(status.contains(container!.querySelector('[data-testid="statusline-model-default"]'))).toBe(false);

    // Focusing the figures opens the tooltip, which keeps every ledger figure.
    const trigger = status.parentElement as HTMLElement;
    await act(async () => { trigger.focus(); });
    const detail = document.querySelector('[data-testid="cost-budget-detail"]');
    expect(detail?.textContent).toContain('settled $0.53');
    expect(detail?.textContent).not.toContain('priced and settled');
    expect(detail?.textContent).toContain('unconfirmed (held for calls whose charge could not be confirmed) $1.56');
    expect(detail?.textContent).toContain('budget left about $47.91 (unconfirmed)');
  });
});

describe('CostStatusline after an app launch (reload hydration, no live frame yet)', () => {
  it('shows the plain-words bar for the reported figures, not a bare "Budget unknown" and not the ledger dump', async () => {
    // QA could not confirm the new wording in the real app: the Build tab was
    // never opened. What the user sees first after launching Permagent is not
    // a live `session_spend_changed` frame but `hydrateCodingHarness()` reading
    // /api/harness/runs/active — and for these figures that path marks the run
    // `budgetStatus: 'unknown'` (band is 'unknown' whenever the daemon holds
    // unconfirmed spend, budget_projection.rs `scope_from_aggregate`). This
    // pins that the launch path lands on the same condensed bar as the live
    // one. It proves the store → formatCostMeter → DOM wiring for that state;
    // it does not prove the packaged app is on this build.
    const scope = {
      ...restartProjection.session,
      cap: { softUsd: 40, gateUsd: 45, hardUsd: 50, source: 'current_budget_config' },
      settledUsd: 0.53, heldUsd: 0, unknownUsd: 1.56,
      effectiveUsedUsd: 2.09, remainingUsd: 47.91,
      band: 'unknown', completeness: 'partial',
    };
    const budget = {
      ...restartProjection,
      rootSessionId: 'launched-harness', taskId: 'launched-task',
      session: scope, task: scope,
      provenance: { ...restartProjection.provenance, asOf: '2026-09-18T12:00:00Z', completeness: 'partial' },
    };
    vi.mocked(api.getSessionCost).mockResolvedValue({ own: 0, childrenTotal: 0, perChild: [] });
    getActiveHarnessRuns.mockResolvedValueOnce([{
      runId: 'launched-run', sessionId: budget.rootSessionId,
      project: '/tmp/fixture', status: 'running',
      updatedAt: budget.provenance.asOf, tokens: 0,
      provider: null, model: null, budget,
    }]);
    await act(async () => {
      root!.render(<CostStatusline />);
      await useCommandCenter.getState().hydrateCodingHarness();
    });
    // The launch path's own verdict, so a later change to it is visible here.
    expect(useCommandCenter.getState().codingSpend?.budgetStatus).toBe('unknown');
    const status = container!.querySelector('[role="status"]')!;
    const text = status.textContent!;
    const costSpan = container!.querySelector('span[style*="font-weight"]');
    expect(costSpan?.textContent).toBe('$2.09');
    expect(text).toContain('$1.56 unconfirmed');
    expect(text).toContain('~$47.91 left (unconfirmed)');
    expect(text).not.toMatch(/budget unknown/i);
    expect(text).not.toContain('session');
    expect(text).not.toContain('task settled');
  });
});
