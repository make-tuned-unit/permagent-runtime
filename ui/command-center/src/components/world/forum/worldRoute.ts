// Rollback switch for the shipping World route.
//
// The Solar Forum (`ForumAppView`) is the World the app renders. This is the
// one escape hatch back to the original `WorldView`: set
// `localStorage['permagent.world.legacy'] = '1'` in the app and reload. It is a
// rollback lever, not a feature flag — there is no UI for it and no second
// code path to maintain beyond the lazy branch in `WorkspaceRenderer`.
//
// Storage access can throw rather than return null (Safari private browsing, a
// WebView with site data disabled, an embedded frame with third-party storage
// blocked), so every failure falls back to the shipping route instead of
// breaking the World panel.

export const LEGACY_WORLD_KEY = 'permagent.world.legacy';

export function shouldUseLegacyWorld(): boolean {
  try {
    return globalThis.localStorage?.getItem(LEGACY_WORLD_KEY) === '1';
  } catch {
    return false;
  }
}
