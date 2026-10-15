/** @vitest-environment jsdom */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { useCommandCenter } from '../../../lib/store';
import { OrchestratorConversation } from './OrchestratorConversation';
vi.mock('./habitatMotion', () => ({ useHabitatTravel: () => null }));
const transcriptMocks = vi.hoisted(() => ({ resolve: vi.fn(), getSession: vi.fn() }));
vi.mock('../../../lib/voiceHomeSession', () => ({ resolveCurrentChatSession: transcriptMocks.resolve }));
vi.mock('../../../lib/api', async (orig) => {
  const real = await orig<typeof import('../../../lib/api')>();
  return { ...real, api: { ...real.api, getSession: transcriptMocks.getSession } };
});
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
  act(() => root.render(<OrchestratorConversation name="Ada" onLeave={leave} />));
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
  act(() => root.render(<OrchestratorConversation name="Ada" onLeave={() => {}} />));
  expect(host.textContent).toContain('Speaking');
  act(() => button('Let me speak').click()); expect(controls.interrupt).toHaveBeenCalledOnce();
  act(() => useCommandCenter.setState(s => ({ voiceEngine: { ...s.voiceEngine!, state: 'error', error: 'Voice disconnected' } })));
  expect(host.querySelector('[role="alert"]')?.textContent).toBe('Voice disconnected');
});
it('keeps the browser preview honest when the app voice host is unavailable', () => {
  useCommandCenter.setState({ voiceEngine: null });
  act(() => root.render(<OrchestratorConversation name="Ada" onLeave={() => {}} />));
  expect(button('Start voice chat').disabled).toBe(true);
  expect(host.textContent).toContain('Voice is available in the Permagent app.');
});
it('the World transcript shows the whole reply, not only the sentences that were spoken', () => {
  const spoken = 'Your launch is on track.';
  const full = `${spoken} The invoice run finished at nine. Two customers still owe you money. I drafted both reminders and left them for review.`;
  act(() => useCommandCenter.setState({ chatMessages: [
    { id: 'u1', role: 'user', content: 'How is the launch?' },
    { id: 'a1', role: 'assistant', content: full },
  ] as never }));
  act(() => root.render(<OrchestratorConversation name="Ada" onLeave={() => {}} />));
  act(() => button('Read conversation').click());
  const shown = host.querySelector('.orchestrator-transcript')?.textContent ?? '';
  expect(shown).toContain(full);
  expect(full.length).toBeGreaterThan(spoken.length);
});

it('reads the current chat, not the dock, so a long reply\'s text-only remainder is there', async () => {
  // Live 2026-10-15: "the rest is on screen", but Read conversation showed the
  // dock's chat. The current chat holds the full reply.
  const story = 'The lighthouse stood on black rock. ' + 'He kept the light for years. '.repeat(12) + 'And the storm finally broke.';
  transcriptMocks.resolve.mockResolvedValue('current-chat');
  transcriptMocks.getSession.mockResolvedValue({ id: 'current-chat', conversation: [
    { id: 'u1', role: 'user', created: 1, content: [{ type: 'text', text: 'Tell me a long story.' }], metadata: { userVisible: true, agentVisible: true } },
    { id: 'a1', role: 'assistant', created: 2, content: [{ type: 'text', text: '[pause] ' + story }], metadata: { userVisible: true, agentVisible: true } },
  ] });
  useCommandCenter.setState({ chatMessages: [{ id: 'dock', role: 'user', content: 'a different chat' }] as never });
  act(() => root.render(<OrchestratorConversation name="Ada" onLeave={() => {}} />));
  await act(async () => { button('Read conversation').click(); await Promise.resolve(); await Promise.resolve(); });
  expect(transcriptMocks.getSession).toHaveBeenCalledWith('current-chat');
  const text = host.querySelector('.orchestrator-transcript')?.textContent ?? '';
  expect(text).toContain('And the storm finally broke.');
  expect(text).not.toContain('[pause]');
  expect(text).not.toContain('a different chat');
});
