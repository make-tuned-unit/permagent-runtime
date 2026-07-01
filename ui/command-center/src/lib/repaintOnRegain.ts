// Throttle-immune render self-heal (Phase 2.5 S1, see
// docs/architecture/WEBVIEW_LIFECYCLE.md).
//
// The in-app browser is a native sibling WKWebView composited over the main
// app webview. When it is forward (or the window is backgrounded in
// fullscreen), macOS marks the MAIN webview occluded and WebKit throttles its
// requestAnimationFrame / timers. The native browser keeps its own paint, but
// the main webview's rAF-driven surfaces — the xterm terminal (DOM renderer)
// and React paints (sidebar) — freeze and present a stale or blank frame until
// something forces a repaint (#517 blank-on-focus, #551 CC-pane-blank).
//
// This module is the pure-TS half of the fix (ruling D3): on focus/visibility
// regain, fire registered callbacks so each frozen surface can force itself to
// repaint. The native App-Nap assertion (which would prevent the throttle
// entirely) is a conditional follow-up, NOT part of this slice.

type RepaintCallback = () => void;

const subscribers = new Set<RepaintCallback>();

/**
 * Fire every registered repaint callback. A throwing callback (e.g. a pane
 * mid-teardown) must not prevent the others from running. Exported for the
 * regain wiring below and for unit tests.
 */
export function fireRepaint(): void {
  // Snapshot so a callback that unsubscribes during iteration is safe.
  for (const cb of [...subscribers]) {
    try {
      cb();
    } catch {
      /* one stale surface shouldn't break the rest */
    }
  }
}

let wired = false;

// Attach the global focus/visibility listeners exactly once, lazily on first
// subscribe. Guarded for non-DOM environments (unit tests run under node).
function ensureWired(): void {
  if (wired || typeof window === 'undefined') return;
  wired = true;
  window.addEventListener('focus', fireRepaint);
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible') fireRepaint();
  });
}

/**
 * Run `cb` whenever the webview regains focus or becomes visible again, so a
 * surface that froze under occlusion throttling can force a repaint. Returns an
 * unsubscribe function.
 */
export function onRepaintRegain(cb: RepaintCallback): () => void {
  ensureWired();
  subscribers.add(cb);
  return () => {
    subscribers.delete(cb);
  };
}

/**
 * Force the compositor to repaint a (possibly frozen) subtree. A momentary,
 * visually-imperceptible opacity nudge dirties paint, then restores on the next
 * frame — by which point rAF has resumed. Opacity (not transform) is used so we
 * never create a containing block for `position: fixed` descendants such as the
 * chat launcher.
 */
export function forceCompositorRepaint(el: HTMLElement): void {
  const prev = el.style.opacity;
  el.style.opacity = '0.9999';
  void el.offsetHeight; // flush the style change synchronously
  requestAnimationFrame(() => {
    el.style.opacity = prev;
  });
}
