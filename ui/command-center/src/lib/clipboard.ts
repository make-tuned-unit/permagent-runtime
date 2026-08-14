import { useCallback, useEffect, useRef, useState } from 'react';

/** Copy `text`, reporting honestly whether it landed.
 *
 * The async Clipboard API is only defined in a secure context. Inside the Tauri
 * shell the Command Center is a localhost origin, so it is there — but the SAME
 * bundle is served to paired devices over plain HTTP on the LAN
 * (`http://<host>:3001/ui/`, SettingsView's pairing URL), where
 * `navigator.clipboard` is `undefined`. Every existing copy button in this app
 * reaches for `navigator.clipboard.writeText(...)` directly, so on a paired
 * device they throw into a dangling promise and the button does nothing, with
 * nothing on screen to say so — the exact failure this helper exists to end.
 *
 * `document.execCommand('copy')` is deprecated but is the only synchronous copy
 * path an insecure context has, so it stays as the fallback rather than the
 * primary. Returns false — never throws — when neither path works, so callers
 * can say "couldn't copy" instead of pretending.
 */
export async function copyText(text: string): Promise<boolean> {
  if (!text) return false;

  if (navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      // Permission denied, or a non-focused document. Fall through.
    }
  }

  try {
    const scratch = document.createElement('textarea');
    scratch.value = text;
    // Off-screen rather than display:none / hidden — an unrendered element is
    // not selectable, and execCommand copies the selection.
    scratch.setAttribute('readonly', '');
    scratch.style.position = 'fixed';
    scratch.style.top = '-9999px';
    scratch.style.opacity = '0';
    document.body.appendChild(scratch);
    scratch.select();
    const ok = document.execCommand('copy');
    scratch.remove();
    return ok;
  } catch {
    return false;
  }
}

export type CopyState = 'idle' | 'copied' | 'failed';

/** Copy plus the transient on-screen acknowledgement, in one place.
 *
 * Ten call sites had each re-implemented copy + a `copied` boolean + a reset
 * timer, with three different error postures and no failure state at all. The
 * state is deliberately three-valued: a button that shows nothing when the copy
 * fails is indistinguishable from a button that is broken.
 */
export function useCopyToClipboard(resetMs = 2000) {
  const [state, setState] = useState<CopyState>('idle');
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => () => { if (timer.current) clearTimeout(timer.current); }, []);

  const copy = useCallback(async (text: string) => {
    const ok = await copyText(text);
    setState(ok ? 'copied' : 'failed');
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => setState('idle'), resetMs);
    return ok;
  }, [resetMs]);

  return { state, copy };
}
