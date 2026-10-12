/** @vitest-environment jsdom
 *
 * A daemon whose /api/dashboard never answers used to leave Home on the loading
 * logo forever: no error, no "Try again". The read now gives up after
 * DASHBOARD_TIMEOUT_MS and the hook reports the failure.
 */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../lib/api', () => ({ apiFetch: vi.fn() }));

import { apiFetch } from '../../lib/api';
import { DASHBOARD_TIMEOUT_MS, useDashboard } from './useDashboard';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
const apiFetchMock = vi.mocked(apiFetch);

let container: HTMLDivElement;
let root: Root;
let state: ReturnType<typeof useDashboard> | undefined;

function Harness() {
  state = useDashboard();
  return <div>{state.loading ? 'loading' : state.error ? 'error' : 'ok'}</div>;
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

describe('useDashboard timeout', () => {
  it('turns a read that never answers into an error instead of loading forever', async () => {
    // Rejects only when the caller's AbortSignal fires, like a real fetch.
    apiFetchMock.mockImplementation((_url, init) => new Promise((_resolve, reject) => {
      init?.signal?.addEventListener('abort', () => reject(new Error('aborted')));
    }));
    await act(async () => { root.render(<Harness />); });
    expect(state?.loading).toBe(true);
    await act(async () => { await vi.advanceTimersByTimeAsync(DASHBOARD_TIMEOUT_MS + 100); });
    expect(state?.loading).toBe(false);
    expect(state?.error).toBe(true);
  });

  it('does not stack a new poll on a read that is still outstanding', async () => {
    apiFetchMock.mockImplementation(() => new Promise(() => {}));
    await act(async () => { root.render(<Harness />); });
    await act(async () => { await vi.advanceTimersByTimeAsync(16_000); });
    expect(apiFetchMock).toHaveBeenCalledTimes(1);
  });
});
