/** @vitest-environment jsdom
 *
 * usePollWhenVisible — poll-cadence tests for the Automate tab's
 * `/schedule/list` backstop poll (2026-08-25 "schedule polling storm" fix:
 * a 5s indefinite poll drove an unindexed SQL query, 97 slow-query bursts
 * in 9 minutes — see AutomateView's `SCHEDULE_LIST_POLL_MS`).
 *
 * Pins the three guarantees the fix depends on: the callback never fires
 * more often than the given interval while the tab is visible, it fires
 * nothing at all while the tab is hidden, and it fires nothing after the
 * consuming component unmounts.
 */

import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { usePollWhenVisible } from './usePollWhenVisible';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function setVisibility(state: DocumentVisibilityState) {
  Object.defineProperty(document, 'visibilityState', { value: state, configurable: true });
  document.dispatchEvent(new Event('visibilitychange'));
}

function Harness({ onTick, intervalMs }: { onTick: () => void; intervalMs: number }) {
  usePollWhenVisible(onTick, intervalMs);
  return null;
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.useFakeTimers();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  Object.defineProperty(document, 'visibilityState', { value: 'visible', configurable: true });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

describe('usePollWhenVisible', () => {
  it('never fires more often than the interval while the tab stays visible', () => {
    const onTick = vi.fn();
    act(() => {
      root.render(<Harness onTick={onTick} intervalMs={60_000} />);
    });
    // No immediate fire on mount — the caller does its own initial fetch;
    // this hook is purely the backstop interval.
    expect(onTick).not.toHaveBeenCalled();

    act(() => { vi.advanceTimersByTime(59_999); });
    expect(onTick).not.toHaveBeenCalled();

    act(() => { vi.advanceTimersByTime(1); });
    expect(onTick).toHaveBeenCalledTimes(1);

    // Three more full intervals (180s / 60s) — never more than once per tick.
    act(() => { vi.advanceTimersByTime(180_000); });
    expect(onTick).toHaveBeenCalledTimes(4);
  });

  it('fires nothing while the tab is hidden', () => {
    const onTick = vi.fn();
    act(() => {
      root.render(<Harness onTick={onTick} intervalMs={60_000} />);
    });

    act(() => setVisibility('hidden'));
    act(() => { vi.advanceTimersByTime(10 * 60_000); });

    expect(onTick).not.toHaveBeenCalled();
  });

  it('mounting while already hidden never starts the interval', () => {
    setVisibility('hidden');
    const onTick = vi.fn();
    act(() => {
      root.render(<Harness onTick={onTick} intervalMs={60_000} />);
    });

    act(() => { vi.advanceTimersByTime(10 * 60_000); });

    expect(onTick).not.toHaveBeenCalled();
  });

  it('fires nothing after the component unmounts', () => {
    const onTick = vi.fn();
    act(() => {
      root.render(<Harness onTick={onTick} intervalMs={60_000} />);
    });

    act(() => { vi.advanceTimersByTime(60_000); });
    expect(onTick).toHaveBeenCalledTimes(1);

    act(() => root.unmount());
    act(() => { vi.advanceTimersByTime(10 * 60_000); });

    expect(onTick).toHaveBeenCalledTimes(1);
  });

  it('catches up once on regaining visibility, then resumes the interval', () => {
    const onTick = vi.fn();
    act(() => {
      root.render(<Harness onTick={onTick} intervalMs={60_000} />);
    });

    act(() => setVisibility('hidden'));
    act(() => { vi.advanceTimersByTime(5 * 60_000); });
    expect(onTick).not.toHaveBeenCalled();

    act(() => setVisibility('visible'));
    expect(onTick).toHaveBeenCalledTimes(1); // catch-up fire, not a burst

    act(() => { vi.advanceTimersByTime(60_000); });
    expect(onTick).toHaveBeenCalledTimes(2);
  });
});
