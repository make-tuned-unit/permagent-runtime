/** @vitest-environment jsdom */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { useCommandCenter } from '../../../lib/store';
import { HankConversation } from './HankConversation';
vi.mock('./habitatMotion', () => ({ useHabitatTravel: () => null }));
let host: HTMLDivElement, root: Root;
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
const controls = { setHandsFree: vi.fn(), deactivate: vi.fn(), interrupt: vi.fn() };
beforeEach(() => {
  Object.values(controls).forEach(fn => fn.mockReset());
  useCommandCenter.setState({ voiceEngine: { state: 'idle', handsFree: false, error: null, ...controls } as unknown as NonNullable<ReturnType<typeof useCommandCenter.getState>['voiceEngine']>, chatMessages: [] });
  host = document.createElement('div'); document.body.append(host); root = createRoot(host);
});
afterEach(() => { act(() => root.unmount()); host.remove(); useCommandCenter.setState({ voiceEngine: null }); });
const button = (label: string) => [...host.querySelectorAll('button')].find(b => b.textContent?.includes(label))!;
it('waits for an explicit voice start and releases voice when leaving the table', async () => {
  const leave = vi.fn();
  act(() => root.render(<HankConversation name="Hank" onLeave={leave} />));
  expect(controls.setHandsFree).not.toHaveBeenCalled();
  await act(async () => button('Start voice chat').click());
  expect(controls.setHandsFree).toHaveBeenCalledWith(true);
  act(() => button('Leave table').click());
  expect(controls.setHandsFree).toHaveBeenLastCalledWith(false);
  expect(controls.deactivate).toHaveBeenCalledOnce();
  expect(leave).toHaveBeenCalledOnce();
});
it('interrupts existing playback and shows actual errors without creating a second engine', () => {
  act(() => useCommandCenter.setState(s => ({ voiceEngine: { ...s.voiceEngine!, state: 'playing', handsFree: true } })));
  act(() => root.render(<HankConversation name="Hank" onLeave={() => {}} />));
  expect(host.textContent).toContain('Speaking');
  act(() => button('Let me speak').click()); expect(controls.interrupt).toHaveBeenCalledOnce();
  act(() => useCommandCenter.setState(s => ({ voiceEngine: { ...s.voiceEngine!, state: 'error', error: 'Voice disconnected' } })));
  expect(host.querySelector('[role="alert"]')?.textContent).toBe('Voice disconnected');
});
it('keeps the browser preview honest when the app voice host is unavailable', () => {
  useCommandCenter.setState({ voiceEngine: null });
  act(() => root.render(<HankConversation name="Hank" onLeave={() => {}} />));
  expect(button('Start voice chat').disabled).toBe(true);
  expect(host.textContent).toContain('Voice is available in the Permagent app.');
});
