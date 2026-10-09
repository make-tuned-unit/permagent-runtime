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
  try {
    localStorage.setItem(CHAT_SETUP_INTENT_KEY, intent);
    // Storage events do not fire in the window doing the write. A mounted
    // dock needs this signal; a detached Chat window gets the storage event.
    if (typeof window !== 'undefined') window.dispatchEvent(new Event(CHAT_SETUP_INTENT_EVENT));
    // Native webviews do not consistently deliver storage events to each
    // other. The persisted key covers late mounts; this wakes an open pop-out.
    if (typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
      void import('@tauri-apps/api/event')
        .then(({ emit }) => emit(CHAT_SETUP_INTENT_EVENT, {}))
        .catch(() => {});
    }
    return true;
  } catch {
    return false;
  }
}

export function peekChatSetupIntent(): string | null {
  try { return localStorage.getItem(CHAT_SETUP_INTENT_KEY); }
  catch { return null; }
}

/** Consume only when the composer can show it without replacing user text. */
export function takeChatSetupIntent(): string | null {
  const intent = peekChatSetupIntent();
  if (!intent) return null;
  try { localStorage.removeItem(CHAT_SETUP_INTENT_KEY); }
  catch { return null; }
  return intent;
}

export function discardChatSetupIntent(): boolean {
  try { localStorage.removeItem(CHAT_SETUP_INTENT_KEY); }
  catch { return false; }
  if (typeof window !== 'undefined') window.dispatchEvent(new Event(CHAT_SETUP_INTENT_EVENT));
  return true;
}
