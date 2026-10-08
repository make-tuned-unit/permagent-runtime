/** @vitest-environment jsdom */
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
vi.mock('../../lib/api', () => ({ api: {}, apiFetch: vi.fn(), getApiBaseUrl: () => 'http://localhost:1234', extractText: vi.fn(), extractThinking: vi.fn() }));
import { apiFetch } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';
import { MeetingCardSource } from './MeetingCardSource';
import { CardDetailModal } from './CardDetailModal';
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
const fetcher = vi.mocked(apiFetch);
const projectId = 'p/1';
const base = '/api/projects/p%2F1';
const triagePath = `${base}/recordings/r%2F1/triage`;
const metadata = { source: 'meeting_triage', source_note_id: 'n1', source_recording_id: 'r/1', source_triage_id: 't1' };
const receipt = { id: 't1', projectId, recordingId: 'r/1', noteId: 'n1', status: 'completed', sessionId: 's1', cardIds: ['c1'], attemptCount: 1, truncated: false, created: false };
const note = { id: 'n1', project_id: projectId, title: 'Practice meeting', body: 'Do not launch.', memory_key: null, created_at: '', updated_at: '' };
const session = { sessionId: 's1', projectId, recordingId: 'r/1', noteId: 'n1' };
const onNavigate = vi.fn();
let root: Root; let host: HTMLDivElement;
const focusNote = vi.fn(); const openDock = vi.fn();
const switchSession = vi.fn(async (id: string) => { useCommandCenter.setState({ chatSessionId: id }); });
function serve(url: string) {
  if (url === `${base}/notes`) return Promise.resolve([note]);
  if (url === triagePath) return Promise.resolve(receipt);
  if (url === `${triagePath}/session`) return Promise.resolve(session);
  return Promise.reject(new Error(`unexpected GET ${url}`));
}
beforeEach(() => {
  localStorage.clear(); fetcher.mockReset().mockImplementation(serve as typeof apiFetch);
  onNavigate.mockReset(); focusNote.mockReset(); openDock.mockReset(); switchSession.mockClear();
  useCommandCenter.setState({ focusProjectNote: focusNote, switchToSession: switchSession, openChatDock: openDock, chatSessionId: null });
  host = document.createElement('div'); document.body.append(host); root = createRoot(host);
});
afterEach(() => { act(() => root.unmount()); host.remove(); });
async function render(meta: Record<string, unknown> = metadata) {
  await act(async () => root.render(<MeetingCardSource projectId={projectId} metadata={meta} onNavigate={onNavigate} />));
}
function button(label: string) { return Array.from(document.querySelectorAll<HTMLButtonElement>('button')).find(b => b.textContent === label); }
async function click(label: string) { expect(button(label)).toBeDefined(); await act(async () => button(label)!.click()); }

it('fetches no sources until requested, then opens the verified note on its own project', async () => {
  await render(); expect(fetcher).not.toHaveBeenCalled();
  await click('View meeting source');
  expect(host.textContent).toContain('Practice meeting'); expect(host.textContent).toContain('Orchestrator triage: completed');
  await click('Open source note');
  expect(focusNote).toHaveBeenCalledWith(projectId, 'n1'); expect(onNavigate).toHaveBeenCalledTimes(1);
  expect(fetcher.mock.calls.map(c => c[0])).toEqual([`${base}/notes`, triagePath]);
  expect(fetcher.mock.calls.every(c => !c[1]?.method || c[1].method === 'GET')).toBe(true);
});
it('resolves the real triage session before switching and exposing the chat dock', async () => {
  await render(); await click('View meeting source'); await click('Open triage session');
  expect(fetcher).toHaveBeenCalledWith(`${triagePath}/session`);
  expect(switchSession).toHaveBeenCalledWith('s1'); expect(openDock).toHaveBeenCalledTimes(1); expect(onNavigate).toHaveBeenCalledTimes(1);
});
it('does not navigate to a mismatched triage or a note from another project', async () => {
  fetcher.mockImplementation((async (url: string) => url === `${base}/notes` ? [{ ...note, project_id: 'other' }] : { ...receipt, projectId: 'other' }) as typeof apiFetch);
  await render(); await click('View meeting source');
  expect(button('Open source note')).toBeUndefined(); expect(button('Open triage session')).toBeUndefined();
  expect(host.textContent).toContain('does not match this card’s meeting source');
  expect(focusNote).not.toHaveBeenCalled(); expect(switchSession).not.toHaveBeenCalled();
});
it('shows missing metadata without inventing a recording or triage session', async () => {
  await render({ source_note_id: 'n1' }); await click('View meeting source');
  expect(button('Open source note')).toBeDefined(); expect(button('Open triage session')).toBeUndefined();
  expect(host.textContent).toContain('incomplete triage reference');
  expect(fetcher.mock.calls.map(c => c[0])).toEqual([`${base}/notes`]);
});
it('keeps failures visible and retries only reads', async () => {
  fetcher.mockRejectedValue(new Error('offline'));
  await render(); await click('View meeting source');
  expect(host.textContent).toContain('Could not check the source note'); expect(button('Open source note')).toBeUndefined();
  fetcher.mockImplementation(serve as typeof apiFetch); await click('Refresh source');
  expect(button('Open source note')).toBeDefined();
  expect(fetcher.mock.calls.every(c => !c[1]?.method)).toBe(true);
});
it.each(['deleted', 'wrong source', 'disowned'])('does not claim or close on an unavailable session: %s', async failure => {
  fetcher.mockImplementation(((url: string) => {
    if (url !== `${triagePath}/session`) return serve(url);
    return failure === 'deleted' ? Promise.reject(new Error('404')) : Promise.resolve(failure === 'wrong source' ? { ...session, noteId: 'other' } : session);
  }) as typeof apiFetch);
  if (failure === 'disowned') switchSession.mockImplementationOnce(async () => { useCommandCenter.setState({ chatSessionId: null }); });
  await render(); await click('View meeting source'); await click('Open triage session');
  expect(host.querySelector('[role="alert"]')?.textContent).toContain('Could not open');
  expect(openDock).not.toHaveBeenCalled(); expect(onNavigate).not.toHaveBeenCalled();
  if (failure !== 'disowned') expect(switchSession).not.toHaveBeenCalled();
});
it('does not offer a session that the receipt has not created', async () => {
  fetcher.mockImplementation(((url: string) => url === triagePath ? Promise.resolve({ ...receipt, sessionId: undefined, status: 'pending' }) : serve(url)) as typeof apiFetch);
  await render(); await click('View meeting source');
  expect(button('Open triage session')).toBeUndefined(); expect(host.textContent).toContain('No triage session is available yet');
});
it('card details retain the source affordance after Record UI state is gone', async () => {
  const card = { id: 'c1', projectId, cardType: 'standard', title: 'Review launch', description: 'Do not launch yet.', columnId: 'col1', metadataJson: metadata, createdAt: '', updatedAt: '', assignedTo: null };
  fetcher.mockImplementation(((url: string) => url === `${base}/cards/c1` ? Promise.resolve(card) : url === `${base}/columns` ? Promise.resolve([]) : serve(url)) as typeof apiFetch);
  await act(async () => root.render(<CardDetailModal projectId={projectId} projectName="Practice" cardId="c1" onClose={onNavigate} />));
  expect(button('View meeting source')).toBeDefined();
  expect(fetcher.mock.calls.map(c => c[0])).toEqual([`${base}/cards/c1`, `${base}/columns`]);
  expect(document.body.textContent).toContain('source_recording_id'); // Original metadata stays inspectable.
  await click('View meeting source'); expect(button('Open source note')).toBeDefined();
});

it('closing card evidence while session resolution waits prevents a late navigation', async () => {
  let resolveSession!: (value: unknown) => void;
  fetcher.mockImplementation(((url: string) => url === `${triagePath}/session`
    ? new Promise<unknown>(resolve => { resolveSession = resolve; }) : serve(url)) as typeof apiFetch);
  await render(); await click('View meeting source');
  act(() => button('Open triage session')!.click());
  act(() => root.render(<div>Card closed</div>));
  await act(async () => resolveSession(session));
  expect(switchSession).not.toHaveBeenCalled(); expect(openDock).not.toHaveBeenCalled(); expect(onNavigate).not.toHaveBeenCalled();
});
