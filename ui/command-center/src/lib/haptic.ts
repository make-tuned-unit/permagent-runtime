/**
 * Trackpad haptic after a People save lands. No-op in the browser.
 */

export function hapticSuccess(): void {
  if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) return;
  import('@tauri-apps/api/core')
    .then(({ invoke }) => invoke('haptic_success'))
    .catch(() => {});
}
