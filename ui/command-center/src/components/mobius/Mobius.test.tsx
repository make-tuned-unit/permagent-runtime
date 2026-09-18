/** @vitest-environment jsdom
 *
 * Mobius — the orb. Reduce Motion must freeze every state, not just `idle`:
 * before this fix, `thinking` / `speaking` / `calibrating` kept cycling
 * frames under `prefers-reduced-motion` because the gate only checked
 * `isIdle && reduceMotion`. Splash.tsx and BootScreen.tsx used to work
 * around that by routing through `state="idle"` whenever reduce motion was
 * on; that workaround is gone now that the gate covers every state at the
 * source (see `motionDisabled` in Mobius.tsx).
 */

import { afterEach, describe, expect, it } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

import { Mobius, type MobiusState } from './Mobius';
import { setReduceMotion } from '../../styles/tokens';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

afterEach(() => {
  if (root) act(() => root.unmount());
  container?.remove();
  setReduceMotion(false);
});

function render(state: MobiusState) {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  act(() => { root.render(<Mobius state={state} />); });
  return container.querySelector('img');
}

describe('Mobius reduce-motion gate', () => {
  const activeStates: MobiusState[] = ['thinking', 'speaking', 'calibrating'];

  it.each(activeStates)('default motion: %s animates (frame_000, not the static logo)', (state) => {
    const img = render(state);
    expect(img?.getAttribute('src')).toContain('mobius/frame_000.webp');
  });

  it.each(activeStates)('reduce motion: %s freezes on the static logo, not a cycling frame', (state) => {
    setReduceMotion(true);
    const img = render(state);
    expect(img?.getAttribute('src')).toContain('mobius/logo.webp');
  });

  it('reduce motion: idle also freezes on the static logo (unchanged behaviour)', () => {
    setReduceMotion(true);
    const img = render('idle');
    expect(img?.getAttribute('src')).toContain('mobius/logo.webp');
  });

  it('sleeping is always the static logo, motion setting aside', () => {
    const img = render('sleeping');
    expect(img?.getAttribute('src')).toContain('mobius/logo.webp');
  });
});
