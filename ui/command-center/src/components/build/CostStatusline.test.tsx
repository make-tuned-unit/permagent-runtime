/**
 * @vitest-environment jsdom
 *
 * Statusline suffix: when a parent session has child subagent spend, the meter
 * shows `incl. N subagents $X` beside the running total.
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
  resetStore();
  vi.clearAllMocks();
});

describe('CostStatusline subagent suffix', () => {
  it('shows incl. N subagents $X when the parent cost rollup has children', async () => {
    await act(async () => {
      root!.render(<CostStatusline />);
    });
    // Let the getSessionCost promise resolve and re-render.
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(api.getSessionCost).toHaveBeenCalledWith('parent-1');
    expect(container!.textContent).toContain('incl. 2 subagents $0.17');
    expect(container!.textContent).toContain('$0.42');
  });
});
