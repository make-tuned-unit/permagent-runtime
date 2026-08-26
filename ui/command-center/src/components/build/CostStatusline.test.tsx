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

vi.mock('../../lib/useLiveGoals', () => ({
  useLiveGoals: () => ({ goals: [], activeCount: 0, loaded: true, refresh: () => {} }),
}));

vi.mock('../../lib/api', () => ({
  api: {
    getSessionCost: vi.fn(async () => ({
      own: 0.42,
      childrenTotal: 0.17,
      perChild: [
        { sessionId: 'child-a', costUsd: 0.1 },
        { sessionId: 'child-b', costUsd: 0.07 },
      ],
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
});
