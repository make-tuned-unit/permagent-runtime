/**
 * runViewTransition — same-document View Transitions for main-view and tab
 * switches (R8 item 5), with a plain no-op fallback.
 *
 * `document.startViewTransition` ships in WKWebView on macOS 26 / Safari 18
 * (our floor is macOS 11 for the CLI, but the desktop shell this feeds runs
 * on current Tauri/WKWebView builds only) and in Chromium (Playwright can
 * exercise it); everywhere else — an older WKWebView, jsdom in tests — it is
 * `undefined`, and this just runs the DOM update synchronously with no
 * transition at all. Callers therefore never branch on availability
 * themselves.
 *
 * Deliberately does NOT replace `viewCrossfade` where that already exists and
 * is doing its job (per the brief: "keep the existing viewCrossfade if
 * better") — this is for a switch that has no bespoke crossfade of its own
 * yet. Respects the app's reduce-motion setting: with motion reduced, the
 * update still runs (so the view still switches) but through the
 * non-transition path, since `startViewTransition` has no "instant" mode of
 * its own.
 *
 * `flushSync` (coordinator ruling 2026-09-25): `startViewTransition`'s
 * callback must produce the "new" DOM state SYNCHRONOUSLY before the browser
 * takes its after-snapshot — the API has no way to know when a React update
 * has actually committed otherwise. Without it, `update()` (a `setState`)
 * schedules a render that lands one or more microtasks after the callback
 * returns, the browser snapshots the OLD DOM as "new", and the transition
 * plays over no visible change at all. `flushSync` forces React to commit
 * before the callback returns.
 *
 * Direction: `opts.direction` sets `data-vt-direction` on `<html>` for the
 * transition's duration (index.css keys the default root transition's slide
 * off it) and is cleared once the transition finishes, so it never leaks into
 * an unrelated later transition that doesn't pass one.
 *
 * Focus and scroll restoration: `startViewTransition` snapshots the DOM
 * before/after the callback but does not touch focus or scroll position
 * itself, so a caller's existing focus-restore / scroll-restore logic inside
 * `update` continues to run exactly as before — this only wraps it.
 */
import { flushSync } from 'react-dom';
import { getReduceMotion } from '../styles/tokens';

export type ViewTransitionDirection = 'forward' | 'backward';

interface MinimalViewTransition {
  finished?: Promise<unknown>;
}
type StartViewTransition = (callback: () => void | Promise<void>) => MinimalViewTransition;

export function runViewTransition(update: () => void, opts?: { direction?: ViewTransitionDirection }): void {
  // `store.ts` and other lib modules run under plain node-environment tests
  // (no `@vitest-environment jsdom`), where `document` itself doesn't exist —
  // not just `startViewTransition`. Same fallback either way: just run it.
  if (typeof document === 'undefined') {
    update();
    return;
  }
  const doc = document as Document & { startViewTransition?: StartViewTransition };
  if (typeof doc.startViewTransition !== 'function' || getReduceMotion()) {
    update();
    return;
  }
  const html = doc.documentElement;
  const clearDirection = () => { delete html.dataset.vtDirection; };
  if (opts?.direction) html.dataset.vtDirection = opts.direction;
  else clearDirection();

  const transition = doc.startViewTransition(() => flushSync(update));
  // `finished` isn't on every minimal test double; guard rather than require
  // it. Direction is cleared on completion OR failure — a transition that
  // gets interrupted by a second one mid-flight must not leave a stale
  // direction attribute for whichever one runs next.
  void transition.finished?.then(clearDirection, clearDirection);
}
