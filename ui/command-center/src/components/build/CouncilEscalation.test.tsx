/** @vitest-environment jsdom */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';

const { getActiveHarnessRuns, conveneCouncil } = vi.hoisted(() => ({
  getActiveHarnessRuns: vi.fn(),
  conveneCouncil: vi.fn(),
}));

vi.mock('../../lib/api', () => ({
  api: { getActiveHarnessRuns, conveneCouncil },
}));

import { CouncilEscalation } from './CouncilEscalation';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let root: Root;
let container: HTMLDivElement;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  getActiveHarnessRuns.mockResolvedValue([]);
  conveneCouncil.mockResolvedValue({ accepted: true, message: 'started' });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.clearAllMocks();
});

describe('CouncilEscalation', () => {
  it('offers one explicit Council approval for a recommended live run', async () => {
    getActiveHarnessRuns.mockResolvedValue([{
      runId: 'run-1', sessionId: 'session-1', project: 'permagent',
      promptTitle: 'Design the voice DAG', promptDigest: 'digest',
      promptContext: 'Design the voice DAG with project memories.',
      councilRecommendation: {
        recommended: true,
        reason: 'Council review may help because this request spans architecture.',
        signals: ['architecture'],
      },
      status: 'running', tokens: 0, spendUsd: 0,
    }]);

    await act(async () => {
      root.render(<CouncilEscalation />);
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(container.querySelector('[data-testid="council-escalation"]')).not.toBeNull();

    const button = [...container.querySelectorAll('button')]
      .find(node => node.textContent === 'Convene Council')!;
    await act(async () => {
      button.dispatchEvent(new MouseEvent('click', { bubbles: true }));
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(conveneCouncil).toHaveBeenCalledWith('Design the voice DAG with project memories.', 'permagent');
    expect(container.textContent).toContain('Council is preparing the DAG');
  });

  it('stays quiet for work a single routed worker can handle', async () => {
    getActiveHarnessRuns.mockResolvedValue([{
      runId: 'run-2', sessionId: 'session-2', project: 'permagent',
      promptTitle: 'Rename button', promptDigest: 'digest', promptContext: 'Rename Save.',
      councilRecommendation: { recommended: false, reason: 'single worker', signals: [] },
      status: 'running', tokens: 0, spendUsd: 0,
    }]);
    await act(async () => {
      root.render(<CouncilEscalation />);
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(container.querySelector('[data-testid="council-escalation"]')).toBeNull();
  });
});
