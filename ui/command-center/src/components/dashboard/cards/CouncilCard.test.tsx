/**
 * @vitest-environment jsdom
 *
 * A daemon that can't be reached is not a Council with nothing to say. The
 * card has to name what failed and offer a way back, not print a raw
 * exception and stop.
 */

import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('../../../lib/api', () => ({
  api: { getCouncilLatest: vi.fn() },
}));
vi.mock('../decisions/useDecisions', () => ({ useDecisions: () => ({}) }));
vi.mock('../decisions/DecisionInbox', () => ({ DecisionInbox: () => null }));

import { CouncilCard } from './CouncilCard';
import { MIN_PENDING_MS } from '../../common/Button';
import { api } from '../../../lib/api';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const getLatest = vi.mocked(api.getCouncilLatest);

let container: HTMLDivElement;
let root: Root;

async function settle() {
  await act(async () => {
    for (let i = 0; i < 8; i += 1) await Promise.resolve();
  });
}

async function advance(ms: number) {
  await act(async () => {
    vi.advanceTimersByTime(ms);
    for (let i = 0; i < 8; i += 1) await Promise.resolve();
  });
}

beforeEach(() => {
  vi.useFakeTimers();
  getLatest.mockReset();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

it('names the failure in interface voice and offers a retry', async () => {
  getLatest.mockRejectedValue(new Error('connection refused'));
  await act(async () => { root.render(<CouncilCard />); });
  await settle();

  expect(container.textContent).toMatch(/Couldn't load/i);
  const retry = Array.from(container.querySelectorAll('button')).find((b) => /Retry/i.test(b.textContent ?? ''));
  expect(retry).toBeTruthy();
});

it('retry refetches and clears the error once the daemon answers', async () => {
  getLatest.mockRejectedValueOnce(new Error('connection refused'));
  await act(async () => { root.render(<CouncilCard />); });
  await settle();

  getLatest.mockResolvedValue({ report: null, session: null, positions: [], openActions: 0 } as never);
  const retry = Array.from(container.querySelectorAll('button')).find((b) => /Retry/i.test(b.textContent ?? ''))!;
  await act(async () => { retry.click(); });
  await advance(MIN_PENDING_MS + 50);

  expect(getLatest).toHaveBeenCalledTimes(2);
  expect(container.textContent).not.toMatch(/Couldn't load/i);
  expect(container.textContent).toMatch(/No weekly report yet/i);
});
