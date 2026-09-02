/** @vitest-environment jsdom
 *
 * Splash — the boot logo. Covers the tagline reveal sequence, the click-to-
 * skip affordance, and the reduce-motion gate this lane's brief calls out:
 * Mobius only stops animating on its own for the `idle` state, so Splash has
 * to route through that state under reduce motion rather than trust the
 * `thinking` loop to respect the setting (it does not — see Mobius.tsx).
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

import { Splash } from './Splash';
import { setReduceMotion } from '../../styles/tokens';

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
  setReduceMotion(false);
});

async function advance(ms: number) {
  await act(async () => {
    vi.advanceTimersByTime(ms);
    for (let i = 0; i < 4; i += 1) await Promise.resolve();
  });
}

describe('Splash', () => {
  it('reveals both tagline lines in sequence (via opacity) and calls onDone once the pass completes', async () => {
    const onDone = vi.fn();
    act(() => { root.render(<Splash onDone={onDone} />); });
    const spans = () => container.querySelectorAll('p span');

    expect((spans()[0] as HTMLElement).style.opacity).toBe('0');
    expect((spans()[1] as HTMLElement).style.opacity).toBe('0');

    await advance(300);
    expect((spans()[0] as HTMLElement).style.opacity).toBe('1');
    expect((spans()[1] as HTMLElement).style.opacity).toBe('0');

    await advance(900); // -> 1200ms total
    expect((spans()[1] as HTMLElement).style.opacity).toBe('1');

    expect(onDone).not.toHaveBeenCalled();
    await advance(1300); // -> 2500ms, phase flips to 'out'
    await advance(500); // exit-fade timer
    expect(onDone).toHaveBeenCalledTimes(1);
  });

  it('clicking skips straight to the exit phase without waiting out the sequence', async () => {
    const onDone = vi.fn();
    act(() => { root.render(<Splash onDone={onDone} />); });
    const el = container.firstElementChild as HTMLElement;
    act(() => { el.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    await advance(500);
    expect(onDone).toHaveBeenCalledTimes(1);
  });

  it('default motion: the orb runs the thinking loop', () => {
    act(() => { root.render(<Splash onDone={() => {}} />); });
    const img = container.querySelector('img');
    expect(img?.getAttribute('src')).toContain('mobius/frame_000.webp');
  });

  it('reduce motion: the orb is static (routed through the idle-gated state), not the thinking loop', () => {
    setReduceMotion(true);
    act(() => { root.render(<Splash onDone={() => {}} />); });
    const img = container.querySelector('img');
    // Mobius disables animation for `idle` under reduce motion and falls back
    // to the static `logo.webp`, which is the whole point of routing through
    // it — see Mobius.tsx `idleDisabled`.
    expect(img?.getAttribute('src')).toContain('mobius/logo.webp');
  });

  it('reduce motion: the exit fade is instant (no transition string)', () => {
    setReduceMotion(true);
    act(() => { root.render(<Splash onDone={() => {}} />); });
    const el = container.firstElementChild as HTMLElement;
    expect(el.style.transition).toBe('none');
  });
});
