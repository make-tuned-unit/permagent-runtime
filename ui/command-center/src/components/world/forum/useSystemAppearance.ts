import { useSyncExternalStore } from 'react';
const QUERY = '(prefers-color-scheme: dark)';
function subscribe(notify: () => void) {
  if (typeof window.matchMedia !== 'function') return () => {};
  const media = window.matchMedia(QUERY);
  media.addEventListener('change', notify);
  return () => media.removeEventListener('change', notify);
}
function snapshot() {
  // Match the World dark fallback when the host cannot expose appearance.
  return typeof window.matchMedia !== 'function' || window.matchMedia(QUERY).matches;
}
/** macOS automatic appearance is surfaced by WebKit/Chrome through this query.
 * No clock heuristic and no polling: a system change updates the scene live.
 * This does not overwrite the user's global Permagent theme preference.
 */
export function useSystemAppearance() {
  return useSyncExternalStore(subscribe, snapshot, () => true) ? 'night' : 'day';
}
