/**
 * The chrome every Permagent window wears, in one place.
 *
 * The main window gets its chrome from `tauri.conf.json` — `titleBarStyle:
 * "Overlay"`, `hiddenTitle`, `trafficLightPosition`, with `decorations: true`
 * so macOS keeps drawing the window's corner radius and drop shadow for us.
 * The windows the frontend opens at runtime (the chat window, detached
 * terminal/browser panes) cannot: they are built through the Rust
 * `WebviewWindowBuilder`, and the A1a spike proved that path silently DROPS
 * `trafficLightPosition` while the `unstable` cargo feature is on — which it
 * permanently is, because the in-app browser is built on `Window::add_child`.
 *
 * So a runtime window is chromed in two steps: the options below (which the
 * builder does honour) and then one `reinsetTrafficLights` call once the window
 * exists, which reapplies the position against the live NSWindow from Rust.
 * `ui/desktop/src-tauri/src/chrome.rs` is the other half and carries the
 * measurements.
 */
import { shell } from '../styles/tokens';

/**
 * Whether the native window is opaque. It is, deliberately and measurably:
 * `transparent: true` is the prerequisite for vibrancy / `NSVisualEffectView`,
 * and A1a measured it at ~+6 points of whole-GPU utilisation at idle on a
 * static page on this M4 (0.12% -> 6.1-6.3%, WindowServer +2.3pts) because
 * macOS alpha-composites the entire window every frame (tauri#15471). For an
 * always-on desktop agent that is a product decision, not a styling tweak.
 *
 * This is a named constant rather than an inline assumption so the calls that
 * depend on it — `setBackgroundColor` in App.tsx, most of all — are greppable
 * the day someone decides to reverse it with their own measurement.
 */
export const NATIVE_WINDOW_IS_OPAQUE = true;

/**
 * Window options that give a runtime-created window the same titlebar as the
 * main window. `trafficLightPosition` is deliberately NOT here: passing it
 * would read as if it worked. It does not, on this path — `reinsetTrafficLights`
 * is what actually places the buttons.
 */
export const OVERLAY_TITLEBAR = {
  // Lowercase here, `"Overlay"` in tauri.conf.json. Not a typo either side:
  // the JS `WindowOptions` type spells the enum in camelCase and the JSON
  // schema spells it capitalised, and each rejects the other's spelling.
  titleBarStyle: 'overlay',
  hiddenTitle: true,
} as const;

/** Height to reserve at the top of a window that wears `OVERLAY_TITLEBAR`. */
export const TITLEBAR_HEIGHT = shell.titlebar;

/**
 * Put the window's traffic lights where the CSS expects them. Idempotent, and
 * a no-op both outside Tauri and for windows Rust does not own the chrome of
 * (the browser's sign-in popups keep a real system titlebar so the user can
 * read the site's title). Failure is silent by design: a window with slightly
 * misplaced buttons is a blemish, never a reason to fail an open.
 */
export async function reinsetTrafficLights(windowLabel: string): Promise<void> {
  if (!('__TAURI_INTERNALS__' in window)) return;
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('reinset_traffic_lights', { windowLabel });
  } catch { /* older shell, or a window that closed while we were asking */ }
}
