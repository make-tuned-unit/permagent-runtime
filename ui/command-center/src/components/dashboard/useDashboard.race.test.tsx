/** @vitest-environment jsdom */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../lib/api', () => ({ apiFetch: vi.fn() }));

import { apiFetch } from '../../lib/api';
import { type DashboardData, useDashboard } from './useDashboard';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
const apiFetchMock = vi.mocked(apiFetch);

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

function dashboard(summary: string): DashboardData {
  return {
    agent: { name: 'Aria', state: 'idle', active_count: 0, summary },
    stats: { sessions_today: 0, sessions_total: 0, memory_count: 0, memory_delta_today: 0 },
    in_flight: [],
    recent: [],
  };
}

let container: HTMLDivElement;
let root: Root;
let refresh: (() => Promise<void>) | undefined;

function Harness() {
  const state = useDashboard();
  refresh = state.refresh;
  return <div>{state.data?.agent.summary ?? 'loading'}</div>;
}

beforeEach(() => {
  vi.useFakeTimers();
  apiFetchMock.mockReset();
  refresh = undefined;
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

describe('useDashboard request ordering', () => {
  it('discards an older response that resolves after a manual refresh', async () => {
    const oldRequest = deferred<DashboardData>();
    const newRequest = deferred<DashboardData>();
    apiFetchMock
      .mockImplementationOnce(() => oldRequest.promise)
      .mockImplementationOnce(() => newRequest.promise);

    await act(async () => { root.render(<Harness />); });
    await act(async () => { void refresh?.(); });
    await act(async () => { newRequest.resolve(dashboard('current')); });
    expect(container.textContent).toBe('current');

    await act(async () => { oldRequest.resolve(dashboard('stale')); });
    expect(container.textContent).toBe('current');
  });
});
