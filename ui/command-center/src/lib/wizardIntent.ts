/**
 * Wizard intent hand-off (2026-07 wiring audit).
 *
 * The onboarding wizard's "What will you build together?" step collected an
 * intent and then dropped it on the floor — the step claimed it would prepare
 * context for the first conversation, but the text was never persisted or
 * sent anywhere. This module is the real hand-off: the wizard stashes the
 * intent, and the chat composer consumes it exactly once, pre-filling the
 * user's first message so they can edit or send it as-is.
 */

const KEY = 'permagent-wizard-intent';
export const CHAT_SETUP_INTENT_KEY = 'permagent-chat-setup-intent';
export const CHAT_SETUP_INTENT_EVENT = 'permagent-chat-setup-intent';

export interface ChatSetupIntentRecord { v: 1; id: string; text: string; issuedAt: number }
export type ChatSetupIntentChange =
  | { kind: 'upsert'; record: ChatSetupIntentRecord }
  | { kind: 'clear'; id: string };

let lastIssuedAt = 0;
let lastSeenId: string | null = null;

function legacyRecord(text: string): ChatSetupIntentRecord {
  let hash = 2166136261;
  for (const char of text) hash = Math.imul(hash ^ char.charCodeAt(0), 16777619);
  return { v: 1, id: `legacy-${(hash >>> 0).toString(16)}`, text, issuedAt: 0 };
}

function parseRecord(raw: string | null): ChatSetupIntentRecord | null {
  if (!raw) return null;
  try {
    const value: unknown = JSON.parse(raw);
    if (value && typeof value === 'object') {
      const record = value as Partial<ChatSetupIntentRecord>;
      if (record.v === 1 && typeof record.id === 'string' && record.id &&
          typeof record.text === 'string' && record.text.trim() &&
          typeof record.issuedAt === 'number' && Number.isFinite(record.issuedAt)) {
        return record as ChatSetupIntentRecord;
      }
      return null;
    }
  } catch { /* the old storage format was plain text */ }
  return legacyRecord(raw);
}

export function peekChatSetupIntentRecord(): ChatSetupIntentRecord | null {
  try {
    const record = parseRecord(localStorage.getItem(CHAT_SETUP_INTENT_KEY));
    if (record) lastSeenId = record.id;
    return record;
  } catch { return null; }
}

export function lastSeenChatSetupIntentId(): string | null {
  return peekChatSetupIntentRecord()?.id ?? lastSeenId;
}

function notifyLocal(): void {
  if (typeof window !== 'undefined') window.dispatchEvent(new Event(CHAT_SETUP_INTENT_EVENT));
}

function notifyOtherWindow(change: ChatSetupIntentChange): void {
  if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) return;
  const target = new URLSearchParams(window.location.search).get('view') === 'chat' ? 'main' : 'chat';
  void import('@tauri-apps/api/event')
    .then(({ emitTo }) => emitTo(target, CHAT_SETUP_INTENT_EVENT, change))
    .catch(() => {});
}

function putRecord(record: ChatSetupIntentRecord): boolean {
  try {
    localStorage.setItem(CHAT_SETUP_INTENT_KEY, JSON.stringify(record));
    lastIssuedAt = Math.max(lastIssuedAt, record.issuedAt);
    lastSeenId = record.id;
    notifyLocal();
    return true;
  } catch { return false; }
}

function clearRecord(id: string): boolean {
  const current = peekChatSetupIntentRecord();
  if (!current || current.id !== id) return false;
  try {
    localStorage.removeItem(CHAT_SETUP_INTENT_KEY);
    lastSeenId = id;
    notifyLocal();
    return true;
  } catch { return false; }
}

/** Apply a native event inside this webview without broadcasting it again. */
export function receiveChatSetupIntentChange(change: ChatSetupIntentChange): void {
  if (!change || typeof change !== 'object') return;
  if (change.kind === 'clear' && typeof change.id === 'string') {
    clearRecord(change.id);
  } else if (change.kind === 'upsert' && change.record?.v === 1 &&
      typeof change.record.id === 'string' && typeof change.record.text === 'string' &&
      typeof change.record.issuedAt === 'number') {
    const current = peekChatSetupIntentRecord();
    if (!current || (current.id !== change.record.id && current.issuedAt <= change.record.issuedAt)) {
      putRecord(change.record);
    }
  }
}

/** Reuse the origin ID when a detached window receives its initial handoff. */
export function adoptChatSetupIntentRecord(record: ChatSetupIntentRecord | null): void {
  if (record) receiveChatSetupIntentChange({ kind: 'upsert', record });
}

/** Final detached-window reconciliation; a clear may only remove its own ID. */
export function reconcileChatSetupIntentFinal(record: ChatSetupIntentRecord | null, seenId: string | null): void {
  if (record) adoptChatSetupIntentRecord(record);
  else if (seenId) receiveChatSetupIntentChange({ kind: 'clear', id: seenId });
}

/** Persist the wizard intent for the first chat composer. No-op when blank. */
export function stashWizardIntent(text: string): void {
  const t = text.trim();
  if (!t) return;
  try {
    localStorage.setItem(KEY, t);
  } catch {
    /* storage unavailable — the hand-off is best-effort */
  }
}

/** Take the stashed intent (one-shot: returns it and clears the stash). */
export function takeWizardIntent(): string | null {
  try {
    const v = localStorage.getItem(KEY);
    if (v !== null) localStorage.removeItem(KEY);
    return v;
  } catch {
    return null;
  }
}

/** Read without removing: React may run state initializers more than once. */
export function peekWizardIntent(): string | null {
  try { return localStorage.getItem(KEY); }
  catch { return null; }
}

/** Queue an editable setup request for Chat. The button never sends it. */
export function stashChatSetupIntent(text: string): boolean {
  const intent = text.trim();
  if (!intent) return false;
  const issuedAt = lastIssuedAt = Math.max(Date.now(), lastIssuedAt + 1);
  const record: ChatSetupIntentRecord = {
    v: 1, id: globalThis.crypto?.randomUUID?.() ?? `${issuedAt}-${Math.random()}`,
    text: intent, issuedAt,
  };
  if (!putRecord(record)) return false;
  notifyOtherWindow({ kind: 'upsert', record });
  return true;
}

export function peekChatSetupIntent(): string | null {
  return peekChatSetupIntentRecord()?.text ?? null;
}

/** Consume only when the composer can show it without replacing user text. */
export function takeChatSetupIntent(expectedId?: string): string | null {
  const record = peekChatSetupIntentRecord();
  if (!record || (expectedId && record.id !== expectedId) || !clearRecord(record.id)) return null;
  notifyOtherWindow({ kind: 'clear', id: record.id });
  return record.text;
}

export function discardChatSetupIntent(expectedId?: string): boolean {
  const record = peekChatSetupIntentRecord();
  if (!record || (expectedId && record.id !== expectedId) || !clearRecord(record.id)) return false;
  notifyOtherWindow({ kind: 'clear', id: record.id });
  return true;
}
