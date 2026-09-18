/**
 * @vitest-environment jsdom
 *
 * The Glass primitive, and the reference conversion that consumes it.
 *
 * What is worth asserting here is not that a div renders. It is that the two
 * halves of the material stay together: a `backdrop-filter` is glass only if
 * what sits on top of it is translucent, and the bug this primitive shipped
 * with for its entire life was exactly that pairing coming apart — a real blur
 * under an opaque fill. Invisible, and not free: the compositing pass is
 * charged whether or not a single blurred pixel survives to the screen.
 */

import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

import { Glass, glassSurface } from './Glass';
import { NotificationHost } from '../notifications/NotificationHost';
import { THEME_GLASS, setTheme, setNativeReduceTransparency } from '../../styles/tokens';
import { setTrayOpen, toast } from '../../lib/notifications';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  setTheme('dark');
  setNativeReduceTransparency(false);
  setTrayOpen(false);
});

function draw(node: React.ReactElement) {
  act(() => root.render(node));
}

/**
 * Every element in the tree that actually asked for a backdrop filter.
 *
 * jsdom's CSS implementation does not know `backdrop-filter`, so it never
 * reaches the style ATTRIBUTE — but React's `style[key] = value` assignment
 * still lands on the declaration object as a plain property, which is what we
 * read here. The consequence to remember: an element that never set it reads
 * back `undefined`, not `''`, so absence is a falsy check and not an equality
 * one. (`getPropertyValue('backdrop-filter')` is empty either way, and
 * `getAttribute('style')` omits it entirely.)
 */
function filtered(): HTMLElement[] {
  return [...container.querySelectorAll<HTMLElement>('*')]
    .filter(el => Boolean(el.style.backdropFilter));
}

describe('glassSurface', () => {
  it('carries a translucent fill alongside the filter', () => {
    const style = glassSurface(THEME_GLASS.dark.glass, false);
    expect(style.background).toBe('rgba(30,36,51,0.82)');
    expect(style.backdropFilter).toBe('blur(20px) saturate(180%)');
  });

  it('ships the -webkit- twin', () => {
    // Unprefixed `backdrop-filter` only landed in Safari 18, and our
    // `minimumSystemVersion` is macOS 11 — Safari 14. Centralising the
    // material is what stops a detail like this going missing on the
    // thirtieth hand-rolled copy.
    const style = glassSurface(THEME_GLASS.dark.glass, false) as Record<string, unknown>;
    expect(style.WebkitBackdropFilter).toBe('blur(20px) saturate(180%)');
  });

  it('drops the filter entirely under Reduce Transparency, not just the alpha', () => {
    // Raising the opacity and keeping the filter would leave the whole cost in
    // place for no visible effect — and the cost is half of why people turn
    // the setting on in the first place.
    const style = glassSurface(THEME_GLASS.dark.glass, true) as Record<string, unknown>;
    expect(style.background).toBe('#1E2433');
    expect(style.backdropFilter).toBeUndefined();
    expect(style.WebkitBackdropFilter).toBeUndefined();
    // Depth survives: a drop shadow is elevation, not transparency.
    expect(style.boxShadow).toBe(THEME_GLASS.dark.glass.boxShadow);
  });
});

describe('<Glass>', () => {
  it('applies the material to itself and not to its children', () => {
    // "Make sure to apply the material directly to the control, not its inner
    // views" (WWDC25/356). A glass toolbar is one element with plain children,
    // not five glass buttons — which is also the rule that keeps the
    // compositing passes down.
    draw(<Glass><span id="child">inner</span></Glass>);
    const child = container.querySelector<HTMLElement>('#child')!;
    const box = child.parentElement!;
    expect(box.style.backdropFilter).toBe('blur(20px) saturate(180%)');
    expect(box.style.background).toBe('rgba(30, 36, 51, 0.82)');
    expect(child.style.backdropFilter).toBeFalsy();
    expect(filtered()).toHaveLength(1);
  });

  it('takes the more opaque step for large surfaces', () => {
    draw(<Glass variant="glassHi"><span id="hi">inner</span></Glass>);
    const box = container.querySelector<HTMLElement>('#hi')!.parentElement!;
    expect(box.style.background).toBe('rgba(30, 36, 51, 0.9)');
    expect(box.style.backdropFilter).toBe('blur(24px) saturate(170%)');
  });

  it('follows the theme', () => {
    setTheme('silver');
    draw(<Glass><span id="s">inner</span></Glass>);
    const box = container.querySelector<HTMLElement>('#s')!.parentElement!;
    expect(box.style.background).toBe('rgba(255, 255, 255, 0.82)');
  });

  it('goes opaque when the native bridge reports Reduce Transparency', () => {
    setNativeReduceTransparency(true);
    draw(<Glass><span id="rt">inner</span></Glass>);
    const box = container.querySelector<HTMLElement>('#rt')!.parentElement!;
    expect(box.style.backdropFilter).toBeFalsy();
    expect(box.style.background).toBe('rgb(30, 36, 51)');
  });

  it('keeps the wizard inherited geometry as its defaults', () => {
    // Promoting this out of `wizard/atoms.tsx` changes the material and
    // nothing else; the six Moments importing it are untouched by this lane.
    draw(<Glass><span id="g">inner</span></Glass>);
    const box = container.querySelector<HTMLElement>('#g')!.parentElement!;
    expect(box.style.borderRadius).toBe('14px');
    expect(box.style.padding).toBe('18px');
  });
});

describe('the notification tray and toast — the reference conversion', () => {
  it('renders the tray on real glass, not a blur over an opaque fill', () => {
    setTrayOpen(true);
    draw(<NotificationHost />);

    const glass = filtered();
    expect(glass.length).toBeGreaterThan(0);
    for (const el of glass) {
      expect(el.style.backdropFilter).toBe('blur(20px) saturate(180%)');
      // The half that was missing before: the fill underneath the filter has
      // to let something through, or the blur is decoration on an invoice.
      expect(el.style.background).toBe('rgba(30, 36, 51, 0.82)');
    }
  });

  it('renders a toast on the same material', () => {
    toast('Build finished', 'permagent-app v1.31.0');
    draw(<NotificationHost />);

    const glass = filtered();
    expect(glass.length).toBeGreaterThan(0);
    expect(container.textContent).toContain('Build finished');
    for (const el of glass) {
      expect(el.style.backdropFilter).toBe('blur(20px) saturate(180%)');
      expect(el.style.background).toBe('rgba(30, 36, 51, 0.82)');
    }
  });

  it('honours Reduce Transparency on the floating surfaces too', () => {
    // The whole point of the bridge: a user who told macOS to reduce
    // transparency gets opaque chrome, in a webview where CSS cannot see the
    // setting at all.
    setNativeReduceTransparency(true);
    toast('Build finished', 'permagent-app v1.31.0');
    draw(<NotificationHost />);

    expect(filtered()).toEqual([]);
    expect(container.textContent).toContain('Build finished');
  });
});
