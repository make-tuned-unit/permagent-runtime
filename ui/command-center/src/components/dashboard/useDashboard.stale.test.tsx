/** @vitest-environment jsdom
 *
 * Home polls every 15 seconds and keeps the last good payload when a poll
 * fails — which is right, but on its own it means the landing page shows
 * yesterday's session count in the same type as a live one, forever, with
 * nothing on screen saying the connection is gone. Frozen numbers that look
 * current are the failure this covers: the hook has to remember WHEN the
 * figures were true, and the header has to say so.
 *
 * This file covers the hook's half — remembering. The sentence the header
 * says is now the app's one staleness rendering, so it is covered where that
 * lives: `components/common/AsOf.test.tsx`.
 */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../lib/api', () => ({ apiFetch: vi.fn() }));

import { apiFetch } from '../../lib/api';
import { type DashboardData, useDashboard } from './useDashboard';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
const apiFetchMock = vi.mocked(apiFetch);

function dashboard(summary: string): DashboardData {
  return {
    agent: { name: 'Aria', state: 'idle', active_count: 0, summary },
    stats: { sessions_today: 3, sessions_total: 9, memory_count: 12, memory_delta_today: 1 },
    in_flight: [],
    recent: [],
  };
}

let container: HTMLDivElement;
let root: Root;
let state: ReturnType<typeof useDashboard> | undefined;

function Harness() {
  state = useDashboard();
  return <div>{state.data?.agent.summary ?? 'loading'}</div>;
}

beforeEach(() => {
  vi.useFakeTimers();
  apiFetchMock.mockReset();
  state = undefined;
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

describe('useDashboard freshness', () => {
  it('records when the figures on screen were last true', async () => {
    vi.setSystemTime(new Date('2026-08-31T12:00:00Z'));
    apiFetchMock.mockResolvedValue(dashboard('all quiet'));

    await act(async () => { root.render(<Harness />); });
    await act(async () => { await Promise.resolve(); });

    expect(state!.lastOkAt).toBe(Date.parse('2026-08-31T12:00:00Z'));
    expect(state!.failing).toBe(false);
  });

  it('keeps the last good figures but stops calling them current', async () => {
    vi.setSystemTime(new Date('2026-08-31T12:00:00Z'));
    apiFetchMock.mockResolvedValueOnce(dashboard('all quiet'));

    await act(async () => { root.render(<Harness />); });
    await act(async () => { await Promise.resolve(); });

    apiFetchMock.mockRejectedValue(new Error('daemon down'));
    vi.setSystemTime(new Date('2026-08-31T12:05:00Z'));
    await act(async () => {
      vi.advanceTimersByTime(15_000);
      for (let i = 0; i < 8; i += 1) await Promise.resolve();
    });

    // The payload stays — a failed poll is not an empty dashboard.
    expect(container.textContent).toBe('all quiet');
    expect(state!.failing).toBe(true);
    // ...but the timestamp does NOT move forward on a failure.
    expect(state!.lastOkAt).toBe(Date.parse('2026-08-31T12:00:00Z'));
  });
});
