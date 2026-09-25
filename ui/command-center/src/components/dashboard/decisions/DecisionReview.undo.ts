/**
 * Undo instead of "are you sure" (U12, R4): a reversible verdict is HELD on
 * the client for the toast's lifetime and only then sent, because the daemon
 * has no retraction. The decision leaves the queue at once (auto-advance) and
 * comes back on Undo or Z.
 *
 * Closing the inbox is a commit, not a cancel: anything still held is sent
 * immediately on unmount, so a verdict is never silently lost.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import type { AnswerBody } from './types';
import type { AnswerResult } from './useDecisions';

/** Matches the toast's own lifetime (NotificationHost TOAST_MS). */
export const UNDO_MS = 6000;

interface Held {
  id: string;
  body: AnswerBody;
  label: string;
  timer: ReturnType<typeof setTimeout>;
}

export interface HeldAnswerCallbacks {
  send: (id: string, body: AnswerBody) => Promise<AnswerResult>;
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

  const commit = useCallback(async (id: string) => {
    const entry = held.current.get(id);
    if (!entry) return;
    clearTimeout(entry.timer);
    held.current.delete(id);
    order.current = order.current.filter(x => x !== id);
    try {
      const result = await cb.current.send(id, entry.body);
      if (result.ok) {
        cb.current.onSent?.(id, result, entry.label);
      } else {
        cb.current.onConflict?.(id, entry.label);
      }
    } catch (error) {
      unhide(id);
      cb.current.onFailed?.(id, error, entry.label);
    }
  }, [unhide]);

  const hold = useCallback((id: string, body: AnswerBody, label: string) => {
    const existing = held.current.get(id);
    if (existing) clearTimeout(existing.timer);
    const timer = setTimeout(() => { void commit(id); }, UNDO_MS);
    held.current.set(id, { id, body, label, timer });
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

  useEffect(() => () => {
    // Unmount = the inbox closed: send what is still held, now.
    for (const id of [...held.current.keys()]) void commit(id);
  }, [commit]);

  return { hidden, hold, undo, settle, release: unhide, canUndo: () => order.current.length > 0 };
}
