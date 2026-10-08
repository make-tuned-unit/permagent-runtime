import { useSyncExternalStore } from 'react';
import { getReduceMotion, onThemeChange } from '../../../styles/tokens';

function subscribe(notify: () => void) {
  const unsubscribe = onThemeChange(notify);
  const media = typeof window.matchMedia === 'function'
    ? window.matchMedia('(prefers-reduced-motion: reduce)') : null;
  media?.addEventListener('change', notify);
  return () => { unsubscribe(); media?.removeEventListener('change', notify); };
}

/** Live OS changes and Settings overrides share the app's existing precedence. */
export function useForumReducedMotion() {
  return useSyncExternalStore(subscribe, getReduceMotion, () => true);
}
