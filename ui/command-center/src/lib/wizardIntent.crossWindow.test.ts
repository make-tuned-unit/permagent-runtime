/** @vitest-environment jsdom */
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

const eventMocks = vi.hoisted(() => ({ emitTo: vi.fn(async (_target: string, _event: string, _payload: unknown) => {}) }));
vi.mock('@tauri-apps/api/event', () => ({ emitTo: eventMocks.emitTo }));

import {
  CHAT_SETUP_INTENT_EVENT, adoptChatSetupIntentRecord, discardChatSetupIntent,
  peekChatSetupIntent, peekChatSetupIntentRecord, receiveChatSetupIntentChange,
  stashChatSetupIntent, takeChatSetupIntent, type ChatSetupIntentChange,
} from './wizardIntent';

function usePartition(partition: Map<string, string>) {
  vi.stubGlobal('localStorage', {
    getItem: (key: string) => partition.get(key) ?? null,
    setItem: (key: string, value: string) => { partition.set(key, value); },
    removeItem: (key: string) => { partition.delete(key); },
  });
}

beforeEach(() => {
  eventMocks.emitTo.mockClear();
  Object.defineProperty(window, '__TAURI_INTERNALS__', { configurable: true, value: {} });
  window.history.replaceState({}, '', '/');
});
afterEach(() => {
  vi.unstubAllGlobals();
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  window.history.replaceState({}, '', '/');
});

it('delivers a new request to detached Chat with separate storage, then clears only its ID in main', async () => {
  const main = new Map<string, string>();
  const chat = new Map<string, string>();
  usePartition(main);
  expect(stashChatSetupIntent('Help me set up Tailscale')).toBe(true);
  const original = peekChatSetupIntentRecord()!;
  await vi.waitFor(() => expect(eventMocks.emitTo).toHaveBeenCalledWith('chat', CHAT_SETUP_INTENT_EVENT, {
    kind: 'upsert', record: original,
  }));

  usePartition(chat);
  window.history.replaceState({}, '', '/?view=chat');
  expect(peekChatSetupIntent()).toBeNull();
  receiveChatSetupIntentChange(eventMocks.emitTo.mock.calls[0][2] as ChatSetupIntentChange);
  expect(peekChatSetupIntentRecord()?.id).toBe(original.id);
  expect(discardChatSetupIntent(original.id)).toBe(true);
  expect(peekChatSetupIntent()).toBeNull();
  await vi.waitFor(() => expect(eventMocks.emitTo).toHaveBeenCalledWith('main', CHAT_SETUP_INTENT_EVENT, {
    kind: 'clear', id: original.id,
  }));

  usePartition(main);
  expect(peekChatSetupIntentRecord()?.id).toBe(original.id);
  const clear = eventMocks.emitTo.mock.calls.find(call => call[0] === 'main')![2] as ChatSetupIntentChange;
  receiveChatSetupIntentChange(clear);
  expect(peekChatSetupIntent()).toBeNull();
});

it('a late clear or old handoff cannot erase or replace a newer request', async () => {
  const main = new Map<string, string>();
  const chat = new Map<string, string>();
  usePartition(main);
  stashChatSetupIntent('First request');
  const old = peekChatSetupIntentRecord()!;
  usePartition(chat);
  window.history.replaceState({}, '', '/?view=chat');
  adoptChatSetupIntentRecord(old);
  expect(takeChatSetupIntent(old.id)).toBe('First request');
  const oldClear: ChatSetupIntentChange = { kind: 'clear', id: old.id };

  usePartition(main);
  window.history.replaceState({}, '', '/');
  stashChatSetupIntent('Newer request');
  const newer = peekChatSetupIntentRecord()!;
  expect(newer.issuedAt).toBeGreaterThan(old.issuedAt);
  receiveChatSetupIntentChange(oldClear);
  adoptChatSetupIntentRecord(old);
  expect(peekChatSetupIntentRecord()).toEqual(newer);
  expect(takeChatSetupIntent(old.id)).toBeNull();
  expect(peekChatSetupIntent()).toBe('Newer request');
});
