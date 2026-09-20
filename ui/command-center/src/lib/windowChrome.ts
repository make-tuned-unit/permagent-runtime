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
 * Whether the MAIN native window is opaque. It is not: since 2026-09-19 it is
 * `transparent: true` with macOS's `sidebar` material behind the web layer
 * (tauri.conf.json `windowEffects`), so the sidebar rail is real glass over the
 * desktop. That reverses the A1a default on purpose. A1a measured the
 * transparent window at ~+6 points of whole-GPU utilisation at idle on this M4
 * (0.12% -> 6.1-6.3%, WindowServer +2.3pts, tauri#15471), and the user took
 * that cost after "There is no liquid glass anywhere".
 *
 * What depends on it: App.tsx stops painting the NSWindow fill and moves the
 * shell gradient off the window onto the content column, the page backgrounds
 * clear (`native-vibrancy` on <html>), and the Sidebar thins its fill to
 * `RAIL_FILL_OVER_VIBRANCY` so the material is not buried. The content column
 * stays opaque (glass on the control layer only, never under content).
 * Runtime windows (chat, detached panes) are opaque and never read this.
 */
export const NATIVE_WINDOW_IS_OPAQUE = false;

/** Set on <html> in the main window while it is transparent; index.css clears
 *  the page backgrounds under it. */
export const NATIVE_VIBRANCY_CLASS = 'native-vibrancy';

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

/**
 * Give a frontend-created window the transparent treatment the main window gets
 * from Rust's `setup`. Same two facts as there: the builder's `transparent`
 * does not survive the signed bundle, and the webview keeps its own opaque
 * background under the `unstable` add_child path.
 */
export async function applyWindowTransparency(windowLabel: string): Promise<void> {
  if (NATIVE_WINDOW_IS_OPAQUE || !('__TAURI_INTERNALS__' in window)) return;
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('make_window_transparent_cmd', { windowLabel });
  } catch { /* older shell, or the window closed while we were asking */ }
}

/**
 * Clear the page background so the native material shows through, and put it
 * back on unmount. index.html's anti-flash script paints <html> INLINE before
 * any module runs and an inline style outranks the stylesheet, so clearing the
 * class alone is not enough. Returns a cleanup function; a no-op when the
 * window is opaque or outside Tauri.
 */
export function clearPageBackgroundForVibrancy(): () => void {
  if (NATIVE_WINDOW_IS_OPAQUE || !('__TAURI_INTERNALS__' in window)) return () => {};
  const root = document.documentElement;
  const painted = root.style.background;
  root.classList.add(NATIVE_VIBRANCY_CLASS);
  root.style.background = 'transparent';
  return () => {
    root.style.background = painted;
    root.classList.remove(NATIVE_VIBRANCY_CLASS);
  };
}
