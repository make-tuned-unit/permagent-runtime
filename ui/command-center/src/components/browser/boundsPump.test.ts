/**
 * @vitest-environment jsdom
 *
 * The nap-safe bounds pump, and the signal that had no way in.
 *
 * Reported 2026-08-19: the owner minimised the window for ten minutes, and on
 * restore the native browser surface was painting at coordinates that no longer
 * matched its container. Nothing was broken about the pump's SUSPENSION — that
 * is the #562 C1 nap-safety property and it has to stay. What was broken is
 * that only the Page Visibility API could resume it, and on macOS the
 * `visibilitychange` that says "the window is back" does not reliably arrive: a
 * miniaturised window's WebContent process is suspended across the minimise.
 *
 * So the property under test is precisely: a NATIVE window event re-aligns the
 * surface and restarts the pump WITHOUT `document.hidden` ever changing. Every
 * test here drives the pump through signals only — `document.hidden` is never
 * touched, which is the whole point.
 */
import { describe, it, expect, vi } from 'vitest';
import {
  createBoundsPump,
  pumpTransition,
  PUMP_INTERVAL_MS,
  SUSPENDED_PROBE_MS,
  type PumpSignal,
} from './boundsPump';

/** A hand-driven timer pair, so no test waits on a real clock. */
function fakeTimers() {
  let next = 1;
  const timers = new Map<number, { fn: () => void; ms: number }>();
  return {
    set: (fn: () => void, ms: number) => {
      const id = next++;
      timers.set(id, { fn, ms });
      return id as unknown as ReturnType<typeof setInterval>;
    },
    clear: (handle: ReturnType<typeof setInterval>) => {
      timers.delete(handle as unknown as number);
    },
    /** Fire every live timer once. */
    tick: () => {
      for (const { fn } of [...timers.values()]) fn();
    },
    intervals: () => [...timers.values()].map(t => t.ms),
  };
}

describe('pump policy', () => {
  it('resumes and re-syncs on a native window event', () => {
    expect(pumpTransition('window-active')).toEqual({ run: true, resync: true });
  });

  it('suspends only for a page-hidden or a genuinely minimised window', () => {
    expect(pumpTransition('page-hidden')).toEqual({ run: false, resync: false });
    expect(pumpTransition('window-occluded')).toEqual({ run: false, resync: false });
  });

  it('never suspends without being told to — nap-safety is a decision, not a default', () => {
    const signals: PumpSignal[] = ['page-visible', 'window-active'];
    for (const s of signals) expect(pumpTransition(s).run).toBe(true);
  });
});

describe('a window event rescues a pump the Page Visibility API stranded', () => {
  it('re-aligns and restarts, with document.hidden never changing', () => {
    const sync = vi.fn();
    const timers = fakeTimers();
    const hiddenBefore = document.hidden;

    const pump = createBoundsPump({ sync, setTimer: timers.set, clearTimer: timers.clear });

    // The window is minimised: WebKit does deliver THIS one.
    pump.signal('page-hidden');
    expect(pump.isPumping()).toBe(false);
    sync.mockClear();

    // Ten minutes later the window is restored. On macOS the matching
    // `visibilitychange` may never arrive — so no 'page-visible' signal is sent
    // here, deliberately. The old code did nothing at all at this point, and
    // the surface stayed where it was.
    pump.signal('window-active');

    expect(sync).toHaveBeenCalledTimes(1);
    expect(pump.isPumping()).toBe(true);
    expect(document.hidden).toBe(hiddenBefore);

    // And it is a real pump again, at the real interval.
    sync.mockClear();
    timers.tick();
    expect(sync).toHaveBeenCalledTimes(1);
    expect(timers.intervals()).toContain(PUMP_INTERVAL_MS);

    pump.dispose();
  });

  /**
   * The regression this replaces. A visibility-only policy — what shipped —
   * has no input for a native window event, so the stranded state is terminal.
   */
  it('the old visibility-only policy had no way back', () => {
    let running = true;
    const visibilityOnly = (hidden: boolean) => {
      running = !hidden;
    };
    visibilityOnly(true);
    expect(running).toBe(false);
    // A window event is not an input this policy accepts. Nothing to call.
    expect(running).toBe(false);
  });
});

describe('nap safety is preserved', () => {
  it('does not run the fast pump while suspended', () => {
    const sync = vi.fn();
    const timers = fakeTimers();
    const pump = createBoundsPump({ sync, setTimer: timers.set, clearTimer: timers.clear });

    pump.signal('page-visible');
    expect(pump.isPumping()).toBe(true);
    pump.signal('page-hidden');
    expect(pump.isPumping()).toBe(false);
    expect(timers.intervals()).not.toContain(PUMP_INTERVAL_MS);
    pump.dispose();
  });

  it('a minimised window cannot restart the pump, even though minimising emits a resize', () => {
    const sync = vi.fn();
    const timers = fakeTimers();
    const pump = createBoundsPump({ sync, setTimer: timers.set, clearTimer: timers.clear });

    pump.signal('page-visible');
    // macOS emits Resized as the window miniaturises. The caller asks the
    // WINDOW whether it is minimised and reports occlusion, not activity.
    pump.signal('window-occluded');

    expect(pump.isPumping()).toBe(false);
    pump.dispose();
  });

  it('the suspended state runs a slow probe that does no native work until the surface drifts', () => {
    const sync = vi.fn();
    const probe = vi.fn(() => false);
    const timers = fakeTimers();
    const pump = createBoundsPump({
      sync,
      probe,
      setTimer: timers.set,
      clearTimer: timers.clear,
    });

    pump.signal('page-hidden');
    expect(pump.isProbing()).toBe(true);
    expect(timers.intervals()).toEqual([SUSPENDED_PROBE_MS]);

    sync.mockClear();
    timers.tick();
    expect(probe).toHaveBeenCalled();
    expect(sync).not.toHaveBeenCalled(); // no drift: no native op at all
    expect(pump.isPumping()).toBe(false);

    // Now the container has moved out from under the surface — the reported
    // symptom, expressed as a checkable fact. This is the last-resort path for
    // when no window event arrives either.
    probe.mockReturnValue(true);
    timers.tick();
    expect(sync).toHaveBeenCalled();
    expect(pump.isPumping()).toBe(true);
    expect(pump.isProbing()).toBe(false);

    pump.dispose();
  });

  it('a throwing probe leaves the pump suspended rather than wedged or spinning', () => {
    const sync = vi.fn();
    const timers = fakeTimers();
    const pump = createBoundsPump({
      sync,
      probe: () => {
        throw new Error('container mid-teardown');
      },
      setTimer: timers.set,
      clearTimer: timers.clear,
    });

    pump.signal('page-hidden');
    expect(() => timers.tick()).not.toThrow();
    expect(pump.isPumping()).toBe(false);
    pump.dispose();
  });

  it('dispose stops everything and ignores later signals', () => {
    const sync = vi.fn();
    const timers = fakeTimers();
    const pump = createBoundsPump({ sync, setTimer: timers.set, clearTimer: timers.clear });

    pump.signal('page-visible');
    pump.dispose();
    expect(pump.isPumping()).toBe(false);

    sync.mockClear();
    pump.signal('window-active');
    expect(sync).not.toHaveBeenCalled();
    expect(pump.isPumping()).toBe(false);
  });
});
