/**
 * Undo instead of "are you sure" (U12, R4): a reversible verdict is HELD on
 * the client for the toast's lifetime and only then sent, because the daemon
 * has no retraction. The decision leaves the queue at once (auto-advance) and
 * comes back on Undo or Z.
 *
 * Closing the inbox is a commit, not a cancel: anything still held is sent
 * immediately on unmount, so a verdict is never silently lost. Leaving the
 * page is the same, except React never unmounts on a reload or navigation:
 * `pagehide` sends what is held as `keepalive` requests, which the browser
 * finishes after the page is gone. Quitting the app fires no page event at
 * all, so every hold is also stored (DecisionReview.heldStore) until it is
 * undone or its send settles, and the next page sends what a gone one left.
 * The page merely going hidden (minimised, covered) is not leaving: the hold
 * keeps its full Undo window and sends as usual.
 *
 * `onCommit` fires the moment a hold ends in a send, whatever ended it, so
 * the caller can retire the Undo it offered: after that there is nothing left
 * to undo.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import { forgetHold, rememberHold, UNDO_MS } from './DecisionReview.heldStore';
import type { AnswerBody } from './types';
import type { AnswerOptions, AnswerResult } from './useDecisions';

export { UNDO_MS };

/**
 * In-flight `keepalive` bodies share a 64 KiB quota, and fetch rejects past
 * it. What does not fit stays held and stored: its timer sends it if the page
 * lives on, the next page if it does not.
 */
const KEEPALIVE_BUDGET = 60 * 1024;

interface Held {
  id: string;
  body: AnswerBody;
  label: string;
  timer: ReturnType<typeof setTimeout>;
}

export interface HeldAnswerCallbacks {
  send: (id: string, body: AnswerBody, opts?: AnswerOptions) => Promise<AnswerResult>;
  /** The hold is over and the answer is on its way; Undo can no longer help. */
  onCommit?: (id: string, label: string) => void;
  /** Sent and accepted (effect_error may still be set). */
  onSent?: (id: string, result: Extract<AnswerResult, { ok: true }>, label: string) => void;
  /** 409: someone else resolved it first. */
  onConflict?: (id: string, label: string) => void;
  /** The send failed; the decision is back in the queue. */
  onFailed?: (id: string, error: unknown, label: string) => void;
}

export function useHeldAnswers(callbacks: HeldAnswerCallbacks) {
  const cb = useRef(callbacks);
  cb.current = callbacks;
  const held = useRef(new Map<string, Held>());
  const order = useRef<string[]>([]);
  /** Ids out of the queue: held for undo, or sent and awaiting the refresh. */
  const [hidden, setHidden] = useState<ReadonlySet<string>>(() => new Set());

  const unhide = useCallback((id: string) => {
    setHidden(prev => { if (!prev.has(id)) return prev; const next = new Set(prev); next.delete(id); return next; });
  }, []);

  const commit = useCallback(async (id: string, opts?: AnswerOptions) => {
    const entry = held.current.get(id);
    if (!entry) return;
    clearTimeout(entry.timer);
    held.current.delete(id);
    order.current = order.current.filter(x => x !== id);
    cb.current.onCommit?.(id, entry.label);
    // The stored copy goes only once the send settles: a page that dies
    // before then leaves it for the next page to send (or find landed).
    try {
      const result = await (opts ? cb.current.send(id, entry.body, opts) : cb.current.send(id, entry.body));
      forgetHold(id);
      if (result.ok) {
        cb.current.onSent?.(id, result, entry.label);
      } else {
        cb.current.onConflict?.(id, entry.label);
      }
    } catch (error) {
      forgetHold(id);
      unhide(id);
      cb.current.onFailed?.(id, error, entry.label);
    }
  }, [unhide]);

  /** `title` names the decision if a later page has to send this for us. */
  const hold = useCallback((id: string, body: AnswerBody, label: string, title = label) => {
    const existing = held.current.get(id);
    if (existing) clearTimeout(existing.timer);
    const timer = setTimeout(() => { void commit(id); }, UNDO_MS);
    held.current.set(id, { id, body, label, timer });
    rememberHold(id, body, label, title);
    order.current = [...order.current.filter(x => x !== id), id];
    setHidden(prev => new Set(prev).add(id));
  }, [commit]);

  /** Undo one held verdict (the latest when no id). Returns the id restored. */
  const undo = useCallback((id?: string): string | null => {
    const target = id ?? order.current[order.current.length - 1];
    if (!target) return null;
    const entry = held.current.get(target);
    if (!entry) return null;
    clearTimeout(entry.timer);
    held.current.delete(target);
    order.current = order.current.filter(x => x !== target);
    forgetHold(target);
    unhide(target);
    return target;
  }, [unhide]);

  /** Forget hidden ids the daemon no longer lists (the answer landed). */
  const settle = useCallback((openIds: ReadonlySet<string>) => {
    setHidden(prev => {
      let changed = false;
      const next = new Set<string>();
      for (const id of prev) {
        if (held.current.has(id) || openIds.has(id)) next.add(id); else changed = true;
      }
      return changed ? next : prev;
    });
  }, []);

  useEffect(() => {
    // Leaving the page: send what is still held, as requests that outlive it.
    const flush = () => {
      let budget = KEEPALIVE_BUDGET;
      for (const [id, entry] of [...held.current]) {
        const size = new TextEncoder().encode(JSON.stringify(entry.body)).length;
        if (size > budget) continue;
        budget -= size;
        void commit(id, { keepalive: true });
      }
    };
    window.addEventListener('pagehide', flush);
    return () => window.removeEventListener('pagehide', flush);
  }, [commit]);

  useEffect(() => () => {
    // Unmount = the inbox closed: send what is still held, now.
    for (const id of [...held.current.keys()]) void commit(id);
  }, [commit]);

  return { hidden, hold, undo, settle, release: unhide, canUndo: () => order.current.length > 0 };
}
