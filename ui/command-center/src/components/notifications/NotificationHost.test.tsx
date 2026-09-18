/**
 * @vitest-environment jsdom
 *
 * NotificationHost's list-management: how many toasts stack, the browser
 * overlay wiring, and the glass/radius tokens on the two floating surfaces
 * (the per-toast spring/timer/keyboard behaviour is `Toast.test.tsx`'s).
 *
 * The browser-overlay half is the fix for the gap `lib/notifications.ts`
 * flagged in place: the native browser webview composites above every DOM
 * layer regardless of z-index, so a toast or the tray landing while the
 * in-app browser is full-bleed would otherwise render invisibly underneath
 * it. `pushBrowserOverlay`/`popBrowserOverlay` is the app's one fix for that
 * (see `MeetingRecorder`, `ProjectChip`) — this proves NotificationHost now
 * uses it too.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

import { NotificationHost } from './NotificationHost';
import { setTrayOpen, toast } from '../../lib/notifications';
import { radius, setReduceMotion } from '../../styles/tokens';
import { useCommandCenter } from '../../lib/store';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

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
  setTrayOpen(false);
  setReduceMotion(false);
  // The overlay counter is app-global Zustand state; a test that leaves it
  // non-zero would silently hide the browser for every test after it.
  useCommandCenter.setState({ overlayBlockingBrowser: 0 });
});

async function advance(ms: number) {
  await act(async () => {
    vi.advanceTimersByTime(ms);
    for (let i = 0; i < 4; i += 1) await Promise.resolve();
  });
}

function overlayCount(): number {
  return useCommandCenter.getState().overlayBlockingBrowser;
}

describe('NotificationHost — toast stack', () => {
  it('caps at 3 toasts, dropping nothing that is already on screen', async () => {
    await act(async () => {
      root.render(<NotificationHost />);
    });
    // Pushed one at a time, host already mounted — matches how it actually
    // arrives (a live daemon event), each one triggering the arrival effect
    // in turn rather than being backfilled from a single re-render.
    for (let i = 0; i < 5; i += 1) {
      await act(async () => { toast(`Toast ${i}`); });
    }
    expect(container.querySelectorAll('[role="status"]').length).toBe(3);
    // The three newest survive; the two oldest never got a card.
    expect(container.textContent).toContain('Toast 4');
    expect(container.textContent).toContain('Toast 3');
    expect(container.textContent).toContain('Toast 2');
    expect(container.textContent).not.toContain('Toast 0');
  });

  it('a toast never appears while the tray is open', async () => {
    setTrayOpen(true);
    toast('Should stay in the tray only');
    await act(async () => {
      root.render(<NotificationHost />);
    });
    expect(container.querySelectorAll('[role="status"]').length).toBe(0);
  });
});

describe('NotificationHost — browser overlay push', () => {
  it('pushes while a toast is up and pops once its exit spring finishes', async () => {
    toast('Deploy finished');
    await act(async () => {
      root.render(<NotificationHost />);
    });
    expect(overlayCount()).toBe(1);

    // The countdown, then the exit spring (well over both is safe here).
    await advance(10_000);
    expect(overlayCount()).toBe(0);
  });

  it('pushes independently for the tray, on top of any toast', async () => {
    await act(async () => {
      root.render(<NotificationHost />);
    });
    // A toast, mounted while the tray is closed, so it actually shows.
    await act(async () => { toast('One toast'); });
    expect(overlayCount()).toBe(1);

    // Opening the tray on top of it is a second, independent floating
    // surface — it does not replace the toast's push, it adds to it. (An
    // existing toast is unaffected by the tray opening; only a brand-new
    // arrival is suppressed while the tray is up.)
    await act(async () => { setTrayOpen(true); });
    expect(overlayCount()).toBe(2);

    await act(async () => {
      setTrayOpen(false);
    });
    expect(overlayCount()).toBe(1);
  });

  it('unmounting the host with a toast still up releases its push', async () => {
    toast('Still open when we tear down');
    await act(async () => {
      root.render(<NotificationHost />);
    });
    expect(overlayCount()).toBe(1);
    act(() => root.unmount());
    expect(overlayCount()).toBe(0);
  });
});

describe('NotificationHost — floating-glass radius (D4)', () => {
  it('the tray takes the outermost floating-glass step, not an uncoordinated radius', async () => {
    setTrayOpen(true);
    await act(async () => {
      root.render(<NotificationHost />);
    });
    const tray = container.querySelector<HTMLElement>('[data-notifications-ui]')!;
    expect(tray.style.borderRadius).toBe(`${radius.glass}px`);
  });

  it('a toast\'s outer surface takes the same floating-glass step', async () => {
    toast('Same material family as the tray');
    await act(async () => {
      root.render(<NotificationHost />);
    });
    const toastButton = container.querySelector<HTMLElement>('[role="status"] button')!;
    expect(toastButton.style.borderRadius).toBe(`${radius.glass}px`);
  });
});
