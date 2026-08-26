/**
 * @vitest-environment jsdom
 *
 * CostStatusline render test — the UI half of the Build-tab cost-meter fix.
 * Mounts the REAL component against the REAL zustand store and drives a
 * `session_spend_changed` frame through the REAL `applyLivenessFrame`, proving
 * the meter updates by EVENT the instant a frame lands — no refetch, no timer,
 * no poll. This is the "updates every turn" claim from the bug report (the old
 * behavior was $0.00 forever because the browser chat SSE stream — the only
 * thing feeding the meter — stayed idle while the user coded in the CLI
 * harness's own PTY session).
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

import { CostStatusline } from './CostStatusline';
import { useCommandCenter } from '../../lib/store';
import { applyLivenessFrame, _resetLivenessSync } from '../../lib/livenessSync';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement | null = null;
let root: Root | null = null;

function resetStore() {
  useCommandCenter.setState({ liveTokens: null, codingSpend: null });
}

beforeEach(() => {
  _resetLivenessSync();
  resetStore();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => { root?.unmount(); });
  container?.remove();
  container = null;
  root = null;
  _resetLivenessSync();
  resetStore();
});

describe('CostStatusline', () => {
  it('shows $0.00 with an empty store, then updates to the coding-harness session total off a live wire frame', async () => {
    await act(async () => {
      root!.render(<CostStatusline />);
    });
    expect(container!.textContent).toContain('$0.00');

    await act(async () => {
      // A frame exactly as the daemon serializes it (snake_case payload keys),
      // pushed through the production entry point — no mock of the store
      // setter, no direct setState, no timer to advance.
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
        // Epoch far in the past, so a "now"-stamped frame always reads live.
        Date.parse('2020-01-01T00:00:00Z'),
      );
    });

    // The bold cost figure specifically, not just "somewhere in the text" —
    // "+$0.0032 this turn" also appears and its digits happen to start with
    // "$0.00", so asserting on the cost span's own content is the honest check.
    const costSpan = container!.querySelector('span[style*="font-weight"]');
    expect(costSpan?.textContent).toBe('$0.03');
  });
});
