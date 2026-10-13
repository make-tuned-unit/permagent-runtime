import { useEffect, useState } from 'react';

export const DAEMON_FAILURE_EVENT = 'daemon-failure';

/**
 * The detail string the Tauri shell emits as `daemon-failure` when starting
 * the background service fails, or null. Display-only: the boot screen's
 * deadline and Retry logic never depend on it. A no-op outside Tauri.
 */
export function useDaemonFailure(): string | null {
  const [detail, setDetail] = useState<string | null>(null);
  useEffect(() => {
    if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void import('@tauri-apps/api/event')
      .then(({ listen }) => listen<unknown>(DAEMON_FAILURE_EVENT, event => {
        const text = typeof event.payload === 'string' ? event.payload.trim() : '';
        if (text) setDetail(text);
      }))
      .then(fn => { if (disposed) fn(); else unlisten = fn; })
      .catch(() => {});
    return () => { disposed = true; unlisten?.(); };
  }, []);
  return detail;
}
