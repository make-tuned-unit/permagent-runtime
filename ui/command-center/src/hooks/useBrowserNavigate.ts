import { useCallback } from 'react';
import { useCommandCenter } from '../lib/store';

/**
 * Open a URL in the in-app browser on the Build tab (and focus it), instead of
 * the system browser. Reusable seam: the web-search setup flow routes provider
 * key-page links here, and the self-knowledge Phase-2 tour (#353) uses the same
 * mechanism to open app surfaces. Returns a stable `(url) => void`.
 */
export function useBrowserNavigate() {
  const openInBrowser = useCommandCenter(s => s.openInBrowser);
  return useCallback((url: string) => openInBrowser(url), [openInBrowser]);
}
