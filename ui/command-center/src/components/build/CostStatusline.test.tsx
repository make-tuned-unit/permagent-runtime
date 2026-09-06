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
    expect(eventText).toContain('$0.75');
    expect(eventText).toContain('session unknown $0.75');
    expect(eventText).toContain('session remaining $0.25');
    expect(eventText).toContain('session budget unknown');

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
});
