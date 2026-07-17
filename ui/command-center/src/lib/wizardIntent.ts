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
