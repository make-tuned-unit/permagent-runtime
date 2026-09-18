/**
 * The macOS "Reduce transparency" bridge.
 *
 * System Settings > Accessibility > Display > Reduce transparency is the one
 * accessibility setting our glass has to obey and cannot see. `prefers-reduced
 * -motion` and `prefers-contrast` both work in WebKit; `prefers-reduced
 * -transparency` does not, and is not coming — it was objected to on
 * fingerprinting grounds, so inside a WKWebView the setting is invisible to
 * CSS and to `matchMedia` alike. There is nothing to query.
 *
 * So we push it in from the other side. Rust reads
 * `NSWorkspace.accessibilityDisplayShouldReduceTransparency`, hands it over
 * once on startup, and emits `accessibility-reduce-transparency` whenever
 * AppKit posts an accessibility-options change — the user flipping the switch
 * in System Settings, live, with our window open. Both paths land in
 * `setNativeReduceTransparency`, which notifies the theme listeners, so every
 * glass surface re-renders opaque without a reload.
 *
 * Everything here is best-effort and silent. Outside Tauri — a browser, a
 * test, a `vite preview` — the dynamic import fails and glass simply stays on,
 * which is the correct behaviour for a platform that has no such setting.
 * A failure to read an accessibility preference must never be a failure to
 * render the app.
 */

import { setNativeReduceTransparency } from './tokens';

let _started = false;

/**
 * Start the bridge. Idempotent and safe to call from anywhere — `<Glass>`
 * calls it on mount, so the bridge is live exactly when there is glass on
 * screen to be affected by it, and an app with no glass pays nothing.
 *
 * (A1c owns App.tsx and should hoist this to app startup once it lands, so
 * that non-glass surfaces can honour the setting too. Calling it from both
 * places is harmless — that is what `_started` is for.)
 */
export function initReduceTransparencyBridge(): void {
  if (_started) return;
  if (typeof window === 'undefined') return;
  _started = true;

  import('@tauri-apps/api/core')
    .then(({ invoke }) => invoke<boolean>('reduce_transparency'))
    .then(v => setNativeReduceTransparency(Boolean(v)))
    .catch(() => { /* not in Tauri, or the read failed — glass stays on */ });

  import('@tauri-apps/api/event')
    .then(({ listen }) =>
      listen<boolean>('accessibility-reduce-transparency', e => {
        setNativeReduceTransparency(Boolean(e.payload));
      }),
    )
    .catch(() => { /* no event bus — the startup read above still applies */ });
}

/** Test seam: forget that the bridge was started. */
export function _resetReduceTransparencyBridgeForTests(): void {
  _started = false;
}
