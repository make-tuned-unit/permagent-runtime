/** @vitest-environment jsdom */
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { beforeEach, afterEach, expect, it, vi } from 'vitest';
const { apiFetch } = vi.hoisted(() => ({ apiFetch: vi.fn() }));
vi.mock('../../../lib/api', () => ({ apiFetch }));
import { LibrarianAwaiting } from './LibrarianAwaiting';
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root; let host: HTMLDivElement;
const onCount = vi.fn();
const row = { entity_id: 'ada', name: 'Ada', entity_type: 'person', why_thin: 'Role unknown', mention_count: 3 };
beforeEach(() => { apiFetch.mockReset(); onCount.mockReset(); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); });
async function render() { await act(async () => root.render(<LibrarianAwaiting onCount={onCount} />)); }
it('distinguishes an empty queue from a failed load', async () => {
  apiFetch.mockResolvedValue([]); await render(); expect(host.textContent).toContain('Nothing waiting on you');
});
it('offers retry on load failure', async () => {
  apiFetch.mockRejectedValue(new Error('offline')); await render();
  expect(host.textContent).toContain('Could not load'); expect(host.textContent).not.toContain('Nothing waiting');
});
it('saves the answer before removing the request and updating the count', async () => {
  apiFetch.mockResolvedValueOnce([row]).mockResolvedValueOnce({ saved: true }); await render();
  const input = host.querySelector('textarea')!;
  act(() => { Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(input, 'An advisor'); input.dispatchEvent(new Event('input', { bubbles: true })); });
  await act(async () => host.querySelector('button')!.click());
  expect(apiFetch).toHaveBeenLastCalledWith('/api/librarian/awaiting/ada/answer', { method: 'POST', body: JSON.stringify({ text: 'An advisor' }) });
  expect(host.textContent).toContain('Nothing waiting on you'); expect(onCount).toHaveBeenLastCalledWith(0);
});
it('keeps the answer and request after a failed save', async () => {
  apiFetch.mockResolvedValueOnce([row]).mockRejectedValueOnce(new Error('offline')); await render();
  const input = host.querySelector('textarea')!;
  act(() => { Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(input, 'An advisor'); input.dispatchEvent(new Event('input', { bubbles: true })); });
  await act(async () => host.querySelector('button')!.click());
  expect(input.value).toBe('An advisor'); expect(host.textContent).toContain('Could not save'); expect(onCount).toHaveBeenLastCalledWith(1);
});
