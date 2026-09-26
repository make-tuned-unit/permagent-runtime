import { api } from '../../lib/api';

export const FORGET_UNDO_MS = 8_000;
export type ForgetOutcome = 'deleted' | 'restored';
export type PendingForget = (() => boolean) & { commit: () => void; expiresAt: number };

/** The toast and deletion share one deadline. Once committed, Undo must not
 * restore a row or suppress the server's eventual outcome. */
export function scheduleForget(key: string, onSettled: (outcome: ForgetOutcome) => void): PendingForget {
  let cancelled = false;
  let started = false;
  const expiresAt = Date.now() + FORGET_UNDO_MS;
  const commit = () => {
    if (cancelled || started) return;
    started = true;
    clearTimeout(timer);
    api.forgetBrainMemory(key)
      .then(result => onSettled(result.verified ? 'deleted' : 'restored'))
      .catch(() => onSettled('restored'));
  };
  const timer = setTimeout(commit, FORGET_UNDO_MS);
  return Object.assign(() => {
    if (started || Date.now() >= expiresAt) return false;
    cancelled = true; clearTimeout(timer); return true;
  }, { commit, expiresAt });
}
