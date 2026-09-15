/** @vitest-environment jsdom */
import { expect, it, vi } from 'vitest';
import { act } from 'react-dom/test-utils';
import { createRoot } from 'react-dom/client';
const source = vi.hoisted(() => ({ getIdentity: vi.fn(), revision: 0 }));
vi.mock('../../../lib/api', () => ({ api: { getIdentity: source.getIdentity } }));
vi.mock('../../../lib/store', () => ({ useCommandCenter: () => source.revision }));
import { useOrchestratorName } from './useOrchestratorName';
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
it('recovers the configured name after daemon startup and refreshes a renamed identity', async () => {
  vi.useFakeTimers();
  const host = document.createElement('div');
  const root = createRoot(host);
  function Name() { return <span>{useOrchestratorName() ?? 'Connecting'}</span>; }
  source.getIdentity.mockRejectedValueOnce(new Error('daemon starting')).mockResolvedValueOnce({ first_name: 'Henry' });
  await act(async () => root.render(<Name />));
  expect(host.textContent).toBe('Connecting');
  await act(async () => { await vi.advanceTimersByTimeAsync(5000); });
  expect(host.textContent).toBe('Henry');
  source.revision++;
  source.getIdentity.mockResolvedValueOnce({ first_name: 'Aster' });
  await act(async () => root.render(<Name />));
  expect(host.textContent).toBe('Aster');
  act(() => root.unmount());
  expect(vi.getTimerCount()).toBe(0);
  vi.useRealTimers();
});
