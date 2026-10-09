/** @vitest-environment jsdom */
import { beforeEach, expect, it, vi } from 'vitest';
import { applyFinalDraft, isStaleDraftSync, type DraftFinal } from './chatDraftHandoff';
import { peekChatSetupIntentRecord, stashChatSetupIntent } from './wizardIntent';

beforeEach(() => localStorage.clear());

it('commits final text, attachments, and scoped setup clear before marking return ready', () => {
  stashChatSetupIntent('Help me set up Tailscale');
  const setupId = peekChatSetupIntentRecord()!.id;
  const actions: string[] = [];
  const setDraft = vi.fn((draft: { text: string; files: File[] }) => {
    expect(draft.text).toBe('final text');
    expect(draft.files[0].name).toBe('note.txt');
    actions.push('draft');
  });
  const markReturnPending = vi.fn(() => {
    expect(peekChatSetupIntentRecord()).toBeNull();
    actions.push('ready');
  });
  const payload: DraftFinal = {
    id: 'return-1', source: 'chat-1', revision: 4, text: 'final text',
    files: [{ name: 'note.txt', mime_type: 'text/plain', data_b64: 'ZmlsZQ==' }],
    setup: null, lastSeenSetupId: setupId,
  };
  applyFinalDraft(payload, setDraft, markReturnPending);
  expect(actions).toEqual(['draft', 'ready']);
  expect(setDraft).toHaveBeenCalledOnce();
  expect(markReturnPending).toHaveBeenCalledOnce();
});

it('rejects pre-final late sync but accepts edits after a failed close', () => {
  expect(isStaleDraftSync({ source: 'chat-1', revision: 3, text: 'stale' }, 4)).toBe(true);
  expect(isStaleDraftSync({ source: 'chat-1', revision: 4, files: [] }, 4)).toBe(true);
  expect(isStaleDraftSync({ source: 'chat-1', revision: 5, text: 'new edit' }, 4)).toBe(false);
});
