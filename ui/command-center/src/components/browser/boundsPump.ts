// Nap-safe bounds pump for the native browser surface (#562 C1, and the
// 2026-08-19 report "the browser was in the wrong place after I un-minimised
// the window").
//
// WHY THIS FILE EXISTS
//
// The in-app browser is a native child WKWebView positioned by absolute
// coordinates over the HTML. Nothing in the compositor keeps it attached to its
// container, so Browser.tsx re-applies `update_browser_bounds` on a 500 ms
// pump plus a ResizeObserver.
//
// That pump drives a MAIN-THREAD native op on the child webview every tick.
// Left running while the window is hidden or occluded it keeps the app's AppKit
// main thread coupled to a throttled/App-Napped WebContent process — the
// mechanism of the idle wedge fixed in #562 C1. So the pump has to SUSPEND
// while the surface is not on screen. That much was right and must stay right.
//
// What was wrong is WHO gets to say "not on screen". Browser.tsx trusted the
// Page Visibility API alone. On macOS a miniaturised window's WKWebView does
// not reliably deliver the `visibilitychange` that says it is BACK: the
// WebContent process was suspended across the minimise, and the owner who
// minimised for ten minutes came back to a pump that never resumed and a
// surface parked at coordinates that no longer matched its container.
//
// The fix is to let NATIVE window events (focus, resize, move, and the
// deminiaturise that shows up as those) resume the pump, and to ask the window
// itself — not the page — whether it is minimised. `document.hidden` is exactly
// the value that cannot be trusted here, so `window-active` deliberately does
// not consult it.
//
// The suspended state also carries a low-frequency PROBE. It is not the pump:
// it performs no native op at all unless the caller's pure predicate says the
// container has actually moved out from under the surface. That covers the case
// where no window event arrives either — the failure the owner actually hit —
// without putting the main thread back in a loop against a napped surface.

/** The 500 ms alignment tick. Only ever runs while the surface is on screen. */
export const PUMP_INTERVAL_MS = 500;

/**
 * How often the SUSPENDED state asks the caller's predicate whether the surface
 * has drifted. Deliberately slow and deliberately native-op-free: this is the
 * last resort when neither the Page Visibility API nor a window event fires.
 */
export const SUSPENDED_PROBE_MS = 2000;

/**
 * Why the pump is being told to change state.
 *
 * - `page-visible` / `page-hidden` — the Page Visibility API. Correct when it
 *   fires; the whole problem is that on macOS it sometimes does not.
 * - `window-active` — a NATIVE window event (focus gained, resized, moved,
 *   deminiaturised). The window is demonstrably on screen, whatever
 *   `document.hidden` still claims.
 * - `window-occluded` — the window itself reports it is minimised. The only
 *   signal allowed to suspend on the basis of a window event, so that the
 *   `Resized` macOS emits ON minimise cannot restart the pump behind a
 *   miniaturised window.
 */
export type PumpSignal = 'page-visible' | 'page-hidden' | 'window-active' | 'window-occluded';

/**
 * The policy, as a pure function: should the pump run after `signal`, and
 * should the caller re-align once immediately?
 *
 * The load-bearing line is `window-active`: it resumes and re-syncs WITHOUT
 * reading `document.hidden`. A policy that consulted page visibility here would
 * be exactly the broken one — the stale `hidden` flag is the bug.
 */
export function pumpTransition(signal: PumpSignal): { run: boolean; resync: boolean } {
  switch (signal) {
    case 'page-hidden':
    case 'window-occluded':
      return { run: false, resync: false };
    case 'page-visible':
    case 'window-active':
      return { run: true, resync: true };
  }
}

type TimerHandle = ReturnType<typeof setInterval>;

export interface BoundsPumpOptions {
  /** Re-apply the native bounds. Called on every tick and on every resume. */
  sync: () => void;
  /**
   * While SUSPENDED, called every `probeMs`. Return true when the surface is
   * demonstrably out of position (the container rect no longer matches the
   * bounds last applied). Returning true resumes the pump and re-syncs.
   * Must be cheap and must not touch the native webview.
   */
  probe?: () => boolean;
  pumpMs?: number;
  probeMs?: number;
  /** Injectable for tests; defaults to the DOM timers. */
  setTimer?: (fn: () => void, ms: number) => TimerHandle;
  clearTimer?: (handle: TimerHandle) => void;
}

export interface BoundsPump {
  signal(signal: PumpSignal): void;
  /** True while the 500 ms alignment tick is live. */
  isPumping(): boolean;
  /** True while the suspended-state drift probe is live. */
  isProbing(): boolean;
  dispose(): void;
}

/**
 * A pump that is running or suspended, never both, and that always has exactly
 * one timer outstanding — the fast alignment tick when on screen, the slow
 * drift probe when not.
 */
export function createBoundsPump(options: BoundsPumpOptions): BoundsPump {
  const {
    sync,
    probe,
    pumpMs = PUMP_INTERVAL_MS,
    probeMs = SUSPENDED_PROBE_MS,
    setTimer = ((fn, ms) => setInterval(fn, ms)) as NonNullable<BoundsPumpOptions['setTimer']>,
    clearTimer = ((handle) => clearInterval(handle)) as NonNullable<BoundsPumpOptions['clearTimer']>,
  } = options;

  let pump: TimerHandle | null = null;
  let probeTimer: TimerHandle | null = null;
  let disposed = false;

  const stopPump = () => {
    if (pump !== null) {
      clearTimer(pump);
      pump = null;
    }
  };
  const stopProbe = () => {
    if (probeTimer !== null) {
      clearTimer(probeTimer);
      probeTimer = null;
    }
  };

  const run = () => {
    if (disposed) return;
    stopProbe();
    if (pump === null) pump = setTimer(() => sync(), pumpMs);
  };

  const suspend = () => {
    stopPump();
    if (disposed || !probe || probeTimer !== null) return;
    probeTimer = setTimer(() => {
      let drifted = false;
      try {
        drifted = probe();
      } catch {
        // A container mid-teardown must not leave the pump wedged off.
        drifted = false;
      }
      if (drifted) apply('window-active');
    }, probeMs);
  };

  function apply(signal: PumpSignal) {
    if (disposed) return;
    const { run: shouldRun, resync } = pumpTransition(signal);
    if (resync) sync();
    if (shouldRun) run();
    else suspend();
  }

  return {
    signal: apply,
    isPumping: () => pump !== null,
    isProbing: () => probeTimer !== null,
    dispose() {
      disposed = true;
      stopPump();
      stopProbe();
    },
  };
}
