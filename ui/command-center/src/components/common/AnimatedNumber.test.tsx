/**
 * @vitest-environment jsdom
 *
 * AnimatedNumber — counts on change, jumps on mount, jumps under reduced
 * motion (D14), and never reflows past the formatted target.
 */
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';
import { AnimatedNumber } from './AnimatedNumber';
import { setReduceMotion } from '../../styles/tokens';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;
// eslint-disable-next-line @typescript-eslint/no-explicit-any -- vi.spyOn's
// inferred MockInstance type for `requestAnimationFrame` doesn't unify with a
// module-scope `ReturnType<typeof vi.spyOn>` declared ahead of the call.
let rafSpy: any;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  // Anchored to the REAL performance.now(), like the browser's own rAF would
  // be — the component measures elapsed time against `performance.now()` at
  // the moment its effect starts, so a counter beginning at 0 reads as
  // "already ~1.7 trillion ms behind schedule" and the progress fraction
  // never reaches 1, spinning forever.
  let now = performance.now();
  rafSpy = vi.spyOn(globalThis, 'requestAnimationFrame').mockImplementation((cb) => {
    now += 16;
    cb(now);
    return 1;
  });
  vi.spyOn(globalThis, 'cancelAnimationFrame').mockImplementation(() => {});
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  rafSpy.mockRestore();
  setReduceMotion(false);
});

it('shows the initial value immediately with no count-up on first mount', () => {
  act(() => { root.render(<AnimatedNumber value={42} />); });
  expect(container.textContent).toBe('42');
});

it('formats with the given Intl options', () => {
  act(() => {
    root.render(<AnimatedNumber value={1234} format={{ style: 'currency', currency: 'USD' }} />);
  });
  expect(container.textContent).toMatch(/\$1,234/);
});

it('renders prefix and suffix inside the same tabular-nums span', () => {
  act(() => { root.render(<AnimatedNumber value={7} prefix="+" suffix="%" />); });
  expect(container.textContent).toBe('+7%');
  const span = container.querySelector('span')!;
  expect(span.style.fontVariantNumeric).toBe('tabular-nums');
});

it('animates toward a new value and settles exactly on it', async () => {
  act(() => { root.render(<AnimatedNumber value={0} durationMs={50} />); });
  act(() => { root.render(<AnimatedNumber value={100} durationMs={50} />); });
  // The mocked rAF advances a fake clock 16ms per frame until t>=1; by the
  // time updates flush, the value must have settled exactly on the target.
  await act(async () => { await Promise.resolve(); });
  expect(container.textContent).toBe('100');
});

it('jumps straight to the target with no animation under reduced motion', () => {
  setReduceMotion(true);
  act(() => { root.render(<AnimatedNumber value={0} />); });
  act(() => { root.render(<AnimatedNumber value={500} />); });
  expect(container.textContent).toBe('500');
  expect(rafSpy).not.toHaveBeenCalled();
});

it('continues from the value actually on screen when interrupted mid-animation, instead of snapping to the previous target', () => {
  // A dashboard tile polling frequently can retarget before a count-up
  // finishes. The next animation's `from` must be wherever it visually
  // stopped, not the target the interrupted animation was still chasing.
  let pendingCb: FrameRequestCallback | null = null;
  rafSpy.mockImplementation((cb: FrameRequestCallback) => {
    pendingCb = cb;
    return 1;
  });

  // Mount (no animation), then start an animation toward 100.
  act(() => { root.render(<AnimatedNumber value={0} durationMs={10000} />); });
  act(() => { root.render(<AnimatedNumber value={100} durationMs={10000} />); });
  expect(pendingCb).not.toBeNull();

  // Advance partway (~30%) through that animation.
  act(() => { pendingCb!(performance.now() + 3000); });
  const partial = parseFloat(container.textContent!);
  expect(partial).toBeGreaterThan(0);
  expect(partial).toBeLessThan(100);

  // Interrupt with a new target before the first animation finishes.
  act(() => { root.render(<AnimatedNumber value={200} durationMs={10000} />); });
  expect(pendingCb).not.toBeNull();

  // The very first frame of the new animation (t≈0) must read from wherever
  // the display actually was (`partial`), not jump to the old target (100).
  act(() => { pendingCb!(performance.now()); });
  const resumed = parseFloat(container.textContent!);
  expect(Math.abs(resumed - partial)).toBeLessThan(5);
  expect(Math.abs(resumed - 100)).toBeGreaterThan(20);
});
