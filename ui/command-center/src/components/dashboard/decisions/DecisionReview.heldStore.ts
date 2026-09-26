/**
 * Held answers outlive the page. A hold lives in memory for the Undo window
 * and is sent by its timer, by the inbox closing, or on `pagehide`; none of
 * those run when the app quits inside the window (Cmd+Q ends the process
 * without any page event reaching the webview) or a webview is torn down.
 * So every hold is also written here the moment it is made, and removed once
 * it is undone or its send settles. An entry left behind by a page that is
 * gone is sent by the next page that reads the inbox (`sendOrphanedHolds`,
 * from useDecisions): an answer the user saw acknowledged is sent late rather
 * than silently dropped, and they are told which way it went.
 *
 * localStorage is shared by every window of the app, so each entry names the
 * page that made it. A page's own holds are its own business; another page's
 * are kept out of this page's queue while they wait, and are only sent once
 * they are past any send their own page could still be making.
 */
import { useMemo, useSyncExternalStore } from 'react';
import { toast } from '../../../lib/notifications';
import type { AnswerBody } from './types';
import type { AnswerResult } from './useDecisions';

/** Matches the toast's own lifetime (NotificationHost TOAST_MS). */
export const UNDO_MS = 6000;
/** Past its Undo window plus this, a hold is no longer its page's send in flight. */
const SEND_GRACE_MS = 10_000;
export const ORPHAN_AFTER_MS = UNDO_MS + SEND_GRACE_MS;

export const HELD_STORE_KEY = 'permagent.decisions.held.v1';
/** This page; a reload or a relaunch is a new one. */
const PAGE = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;

export interface StoredHold {
  id: string;
  body: AnswerBody;
  /** What the toast acknowledged it as ("Allowed"). */
  label: string;
  /** The decision's title, to say later which answer this was. */
  title: string;
  /** The page holding it. */
  owner: string;
  heldAt: number;
}

function isStoredHold(value: unknown): value is StoredHold {
  const h = value as StoredHold | null;
  return !!h && typeof h === 'object' && typeof h.id === 'string' && typeof h.label === 'string'
    && typeof h.title === 'string' && typeof h.owner === 'string' && typeof h.heldAt === 'number'
    && !!h.body && typeof h.body === 'object' && typeof h.body.answer === 'string';
}

function read(): StoredHold[] {
  try {
    const raw = globalThis.localStorage?.getItem(HELD_STORE_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : [];
    return Array.isArray(parsed) ? parsed.filter(isStoredHold) : [];
  } catch {
    return [];
  }
}

function write(holds: StoredHold[]): void {
  try {
    if (holds.length) globalThis.localStorage?.setItem(HELD_STORE_KEY, JSON.stringify(holds));
    else globalThis.localStorage?.removeItem(HELD_STORE_KEY);
  } catch { /* storage unavailable: the in-memory hold still sends */ }
  changed();
}

const listeners = new Set<() => void>();
function changed(): void { for (const fn of listeners) fn(); }
function onStorage(event: StorageEvent): void {
  if (event.key === HELD_STORE_KEY || event.key === null) changed();
}

/** Ids this page is sending on behalf of a gone page. */
const recovering = new Set<string>();

export function rememberHold(id: string, body: AnswerBody, label: string, title: string): void {
  write([...read().filter(h => h.id !== id), { id, body, label, title, owner: PAGE, heldAt: Date.now() }]);
}

export function forgetHold(id: string): void {
  const holds = read();
  if (holds.some(h => h.id === id)) write(holds.filter(h => h.id !== id));
}

/** Another page's holds, and the ones this page is sending for a gone page. */
function heldElsewhereKey(): string {
  return read().filter(h => h.owner !== PAGE || recovering.has(h.id)).map(h => h.id).sort().join('\n');
}

function subscribe(fn: () => void): () => void {
  listeners.add(fn);
  if (listeners.size === 1 && typeof window !== 'undefined') window.addEventListener('storage', onStorage);
  return () => {
    listeners.delete(fn);
    if (listeners.size === 0 && typeof window !== 'undefined') window.removeEventListener('storage', onStorage);
  };
}

/** Decisions answered by a hold this page does not own: out of its queue until that answer settles. */
export function useHeldElsewhere(): ReadonlySet<string> {
  const key = useSyncExternalStore(subscribe, heldElsewhereKey, () => '');
  return useMemo(() => new Set(key ? key.split('\n') : []), [key]);
}

function waiting(now: number): StoredHold[] {
  return read().filter(h => h.owner !== PAGE && !recovering.has(h.id)).map(h => ({ ...h, heldAt: Math.min(h.heldAt, now) }));
}

/** Ms until another page's hold is due for recovery; null when none waits. */
export function nextOrphanDueIn(now = Date.now()): number | null {
  const holds = waiting(now);
  return holds.length ? Math.max(0, Math.min(...holds.map(h => h.heldAt + ORPHAN_AFTER_MS - now))) : null;
}

export type OrphanOutcome =
  | { hold: StoredHold; kind: 'landed' }
  | { hold: StoredHold; kind: 'sent'; result: Extract<AnswerResult, { ok: true }> }
  | { hold: StoredHold; kind: 'refused' }
  | { hold: StoredHold; kind: 'failed'; error: unknown; stillOpen: boolean };

/** What the user is told about an answer a gone page left held. */
export function reportOrphan(outcome: OrphanOutcome): void {
  const { hold } = outcome;
  switch (outcome.kind) {
    case 'landed':
      return;
    case 'sent':
      if (outcome.result.effect_error) {
        toast('Answer recorded — but the follow-through failed', outcome.result.effect_error, undefined, { tone: 'danger', force: true });
      } else {
        toast('Sent your earlier answer', `${hold.title}: ${hold.label}. The app closed before it went, so it was sent now.`, undefined, { force: true });
      }
      return;
    case 'refused':
      toast("Your earlier answer wasn't accepted", `${hold.title}: ${hold.label}. It is back in your queue.`, undefined, { force: true });
      return;
    case 'failed': {
      const why = outcome.error instanceof Error && outcome.error.message ? `${outcome.error.message}. ` : '';
      toast("Couldn't send your earlier answer", `${why}${hold.title}: ${hold.label}.${outcome.stillOpen ? ' It is back in your queue.' : ''}`, undefined, { tone: 'danger', force: true });
      return;
    }
  }
}

/**
 * Send the holds a gone page left behind, now that the daemon has answered a
 * read. `queue` is that read: a hold whose decision is no longer open when the
 * queue is `complete` already landed (a `pagehide` keepalive, or a send its
 * page never heard back from) and is dropped without a second POST.
 */
export async function sendOrphanedHolds(
  queue: { open: ReadonlySet<string>; complete: boolean },
  send: (id: string, body: AnswerBody) => Promise<AnswerResult>,
  report: (outcome: OrphanOutcome) => void = reportOrphan,
  now = Date.now(),
): Promise<void> {
  const due = waiting(now).filter(h => now - h.heldAt >= ORPHAN_AFTER_MS);
  await Promise.all(due.map(async hold => {
    const stillOpen = queue.open.has(hold.id);
    if (queue.complete && !stillOpen) {
      forgetHold(hold.id);
      report({ hold, kind: 'landed' });
      return;
    }
    // Claim it, so another window reading the inbox now leaves it to us.
    recovering.add(hold.id);
    write(read().map(h => (h.id === hold.id ? { ...h, owner: PAGE, heldAt: now } : h)));
    let outcome: OrphanOutcome;
    try {
      const result = await send(hold.id, hold.body);
      outcome = result.ok ? { hold, kind: 'sent', result }
        // 409: it was resolved already (most likely by this very answer)
        // unless the queue still listed it as open.
        : stillOpen ? { hold, kind: 'refused' } : { hold, kind: 'landed' };
    } catch (error) {
      outcome = { hold, kind: 'failed', error, stillOpen };
    }
    recovering.delete(hold.id);
    forgetHold(hold.id);
    changed();
    report(outcome);
  }));
}
