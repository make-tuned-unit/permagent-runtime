/**
 * Daemon auth state — the one place that knows whether this client currently
 * holds a working credential.
 *
 * It lives outside `api.ts` on purpose: the api client is the thing that
 * *detects* the failure, the store and the UI are the things that *show* it,
 * and neither should have to import the other. (It also keeps this signal out
 * of the api-client mocks that most component tests install.)
 *
 * Why it exists at all: losing the daemon token used to be silent. A bare
 * `.catch(() => null)` in the token loader memoized a resolved-null promise, so
 * one early failure left every later request unauthenticated for the life of
 * the window — no log, no retry, no UI. A tokenless client still renders and
 * still polls; it just gets 401 on everything, including the `/events` stream
 * the agent's replies ride. The agent looks quiet when it is actually dead.
 */

let currentError: string | null = null;
const listeners = new Set<(message: string | null) => void>();

/** The current auth problem, or null when the credential is healthy. */
export function getDaemonTokenError(): string | null {
  return currentError;
}

/** Subscribe to auth-state changes. Returns an unsubscribe function. */
export function onDaemonTokenError(listener: (message: string | null) => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

/** Publish an auth problem (or null to clear it). Deduped, and always logged —
 *  the silence is the defect, so this never fails quietly. */
export function setDaemonTokenError(message: string | null): void {
  if (currentError === message) return;
  currentError = message;
  if (message) console.error(`[api] daemon auth: ${message}`);
  for (const listener of listeners) {
    try { listener(message); } catch { /* a bad subscriber must not break auth */ }
  }
}

/** Test seam: forget the published state. */
export function resetDaemonTokenError(): void {
  currentError = null;
}

export const BROWSER_UNPAIRED =
  'This browser is not paired with the daemon — every request is unauthenticated (401). ' +
  'Open the app, or pair this device from Settings → Devices.';
