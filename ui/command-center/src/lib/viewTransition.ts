/**
 * runViewTransition — same-document View Transitions for main-view and tab
 * switches (R8 item 5), with a plain no-op fallback.
 *
 * `document.startViewTransition` ships in WKWebView on macOS 26 / Safari 18
 * (our floor is macOS 11 for the CLI, but the desktop shell this feeds runs
 * on current Tauri/WKWebView builds only); everywhere else — an older
 * WKWebView, jsdom in tests — it is `undefined`, and this just runs the DOM
 * update synchronously with no transition at all. Callers therefore never
 * branch on availability themselves.
 *
 * Deliberately does NOT replace `viewCrossfade` where that already exists and
 * is doing its job (per the brief: "keep the existing viewCrossfade if
 * better") — this is for a switch that has no bespoke crossfade of its own
 * yet. Respects the app's reduce-motion setting: with motion reduced, the
 * update still runs (so the view still switches) but through the
 * non-transition path, since `startViewTransition` has no "instant" mode of
 * its own.
 *
 * Focus and scroll restoration: `startViewTransition` snapshots the DOM
 * before/after the callback but does not touch focus or scroll position
 * itself, so a caller's existing focus-restore / scroll-restore logic inside
 * `update` continues to run exactly as before — this only wraps it.
 */
import { getReduceMotion } from '../styles/tokens';

type StartViewTransition = (callback: () => void | Promise<void>) => { ready?: Promise<void> };

export function runViewTransition(update: () => void): void {
  const doc = document as Document & { startViewTransition?: StartViewTransition };
  if (typeof doc.startViewTransition !== 'function' || getReduceMotion()) {
    update();
    return;
  }
  doc.startViewTransition(update);
}
