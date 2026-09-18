/**
 * @vitest-environment jsdom
 *
 * ToastCard — the per-toast spring, dismiss timer, hover-pause and keyboard
 * dismissal. Split out of NotificationHost precisely so this behaviour could
 * be pinned in isolation, against fake timers, without three siblings and a
 * daemon event stream in the way.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

import { ToastCard } from './Toast';
import type { AppNotification } from '../../lib/notifications';
import { duration, setReduceMotion } from '../../styles/tokens';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

function makeNotification(over: Partial<AppNotification> = {}): AppNotification {
  return {
    id: 'n1', kind: 'system', title: 'Build finished', body: 'v1.31.0',
    ts: Date.now(), read: false,
    ...over,
  };
}

beforeEach(() => {
  vi.useFakeTimers();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
  setReduceMotion(false);
});

async function advance(ms: number) {
  await act(async () => {
    vi.advanceTimersByTime(ms);
    // Let any microtask chains queued by the timer settle too.
    for (let i = 0; i < 4; i += 1) await Promise.resolve();
  });
}

function card() {
  return container.querySelector<HTMLDivElement>('[role="status"]')!;
}

describe('ToastCard', () => {
  it('auto-dismisses ttlMs after mount, once the exit spring finishes', async () => {
    const onDismiss = vi.fn();
    await act(async () => {
      root.render(
        <ToastCard notification={makeNotification()} ttlMs={1000} onDismiss={onDismiss} onActivate={vi.fn()} />,
      );
    });

    await advance(999);
    expect(onDismiss).not.toHaveBeenCalled();

    // The countdown elapses — the card starts leaving, but the exit spring
    // (`duration.snappy`, <500ms) still has to play before it's actually gone.
    await advance(1);
    expect(onDismiss).not.toHaveBeenCalled();

    await advance(duration.snappy);
    expect(onDismiss).toHaveBeenCalledWith('n1');
  });

  it('hovering pauses the countdown, and it resumes from where it left off', async () => {
    const onDismiss = vi.fn();
    await act(async () => {
      root.render(
        <ToastCard notification={makeNotification()} ttlMs={1000} onDismiss={onDismiss} onActivate={vi.fn()} />,
      );
    });

    // Half the countdown elapses, then the pointer lands on the toast.
    // React's onMouseEnter/onMouseLeave are synthesised from the BUBBLING
    // native `mouseover`/`mouseout` (its EnterLeaveEventPlugin), not from the
    // native non-bubbling `mouseenter`/`mouseleave` — so those are what a
    // jsdom dispatch has to send.
    await advance(500);
    await act(async () => {
      card().dispatchEvent(new MouseEvent('mouseover', { bubbles: true, relatedTarget: document.body }));
    });

    // Whatever would have been the rest of the 1000ms window passes while
    // hovered — nothing fires, because the timer is paused, not merely slow.
    await advance(600);
    expect(onDismiss).not.toHaveBeenCalled();

    // Pointer leaves: the remaining ~500ms resumes from where it was paused,
    // not from a fresh 1000ms.
    await act(async () => {
      card().dispatchEvent(new MouseEvent('mouseout', { bubbles: true, relatedTarget: document.body }));
    });
    await advance(499);
    expect(onDismiss).not.toHaveBeenCalled();
    await advance(1 + duration.snappy);
    expect(onDismiss).toHaveBeenCalledWith('n1');
  });

  it('Escape dismisses immediately, without waiting for the countdown', async () => {
    const onDismiss = vi.fn();
    await act(async () => {
      root.render(
        <ToastCard notification={makeNotification()} ttlMs={60_000} onDismiss={onDismiss} onActivate={vi.fn()} />,
      );
    });

    await act(async () => {
      card().dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });
    await advance(duration.snappy);
    expect(onDismiss).toHaveBeenCalledWith('n1');
  });

  it('activating the toast body fires onActivate and starts the exit', async () => {
    const onDismiss = vi.fn();
    const onActivate = vi.fn();
    await act(async () => {
      root.render(
        <ToastCard notification={makeNotification()} ttlMs={60_000} onDismiss={onDismiss} onActivate={onActivate} />,
      );
    });

    const button = card().querySelector('button')!;
    await act(async () => {
      button.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    });
    expect(onActivate).toHaveBeenCalledTimes(1);

    await advance(duration.snappy);
    expect(onDismiss).toHaveBeenCalledWith('n1');
  });

  it('under Reduce Motion, only opacity transitions — no transform, and the exit is the fast fade', async () => {
    setReduceMotion(true);
    const onDismiss = vi.fn();
    await act(async () => {
      root.render(
        <ToastCard notification={makeNotification()} ttlMs={1000} onDismiss={onDismiss} onActivate={vi.fn()} />,
      );
    });

    // No rAF flip needed — Reduce Motion opens straight into the settled
    // state, and never sets a transform at all.
    expect(card().style.transform).toBeFalsy();
    expect(card().style.transition).not.toMatch(/transform/);
    expect(card().style.transition).toMatch(/opacity/);
    expect(card().style.opacity).toBe('1');

    await advance(1000);
    expect(onDismiss).not.toHaveBeenCalled();
    // The fast, fade-only exit — not the full snappy spring.
    await advance(duration.fast);
    expect(onDismiss).toHaveBeenCalledWith('n1');
  });

  it('without Reduce Motion, mounts unentered and springs in on the next frame', async () => {
    await act(async () => {
      root.render(
        <ToastCard notification={makeNotification()} ttlMs={60_000} onDismiss={vi.fn()} onActivate={vi.fn()} />,
      );
    });
    // requestAnimationFrame is faked alongside the timers; nothing has fired
    // the frame yet, so the card is still in its pre-entry pose.
    expect(card().style.opacity).toBe('0');

    await advance(32); // one faked animation frame's worth
    expect(card().style.opacity).toBe('1');
  });
});
