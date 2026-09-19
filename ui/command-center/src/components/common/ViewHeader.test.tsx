// @vitest-environment jsdom
//
// The four main tabs each used to invent their own header — Projects at 16px,
// Build at 14px, Automate at 20px with no tracking, Home with no title at all —
// and they disagreed structurally too, with Automate's header scrolling away
// inside its own content. Visual consistency is exactly the kind of thing that
// re-drifts one well-meaning edit at a time, so it is pinned here.

import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { hardScrollEdgeSurface, ViewHeader } from './ViewHeader';
import { THEME_GLASS, setNativeReduceTransparency, setTheme, type } from '../../styles/tokens';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/** A colour as jsdom's style object reads it back (`rgba(1, 2, 3, 0.5)`). */
function normalised(cssColor: string): string {
  const probe = document.createElement('div');
  probe.style.background = cssColor;
  return probe.style.background;
}

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
});

function render(ui: React.ReactElement) {
  act(() => root.render(ui));
}

describe('ViewHeader', () => {
  it('renders the title at the ramp size, not an ad hoc one', () => {
    render(<ViewHeader title="Projects" />);
    const title = container.querySelector('[data-testid="view-title"]') as HTMLElement;
    expect(title.textContent).toBe('Projects');
    expect(title.style.fontSize).toBe(`${type.title.fontSize}px`);
    expect(title.style.letterSpacing).toBe(type.title.letterSpacing);
  });

  it('renders a subtitle when given', () => {
    render(<ViewHeader title="Projects" subtitle="3 active" />);
    expect(container.textContent).toContain('3 active');
  });

  it('omits the subtitle element entirely when there is nothing to say', () => {
    render(<ViewHeader title="Automate" />);
    // Title block holds only the title — no empty second line taking space.
    const titleBlock = container.querySelector('[data-testid="view-title-block"]') as HTMLElement;
    expect(titleBlock.children).toHaveLength(1);
  });

  it('keeps a stable height with or without a subtitle, so tabs do not jump', () => {
    render(<ViewHeader title="Home" />);
    const bare = (container.querySelector('[data-testid="view-header"]') as HTMLElement).style.minHeight;
    render(<ViewHeader title="Projects" subtitle="3 active" />);
    const withSub = (container.querySelector('[data-testid="view-header"]') as HTMLElement).style.minHeight;
    expect(bare).toBe(withSub);
    expect(bare).not.toBe('');
  });

  it('renders leading and action slots', () => {
    render(
      <ViewHeader
        title="Build"
        leading={<span data-testid="lead">M</span>}
        actions={<button data-testid="act">Take over</button>}
      />,
    );
    expect(container.querySelector('[data-testid="lead"]')).not.toBeNull();
    expect(container.querySelector('[data-testid="act"]')).not.toBeNull();
  });

  // ── Glass by default, D11 hard edge on request ──
  // "There is no liquid glass anywhere": the toolbar every view wears used to
  // be bare with no `surface`, so the one surface Apple names first for glass
  // never was. With no `surface` it now wears the `glass` token's fill and rim.
  //
  // jsdom has no compositor, so this proves the MATERIAL IS REQUESTED — the
  // translucent fill, the rim, the opaque collapse under Reduce Transparency,
  // and the ABSENCE of a blur — not how any of it looks on screen.
  it('wears the glass fill and rim with no `surface` prop', () => {
    render(<ViewHeader title="Home" />);
    const header = container.querySelector('[data-testid="view-header"]') as HTMLElement;
    const token = THEME_GLASS.dark.glass;
    expect(header.style.background).toBe(normalised(token.background));
    expect(header.style.background).toMatch(/^rgba\(.*, 0\.\d+\)$/);
    expect(header.style.boxShadow).not.toBe('');
    // One hairline, from the rim — not a border doubled up beside it.
    expect(header.style.borderBottom).toBe('');
  });

  it('draws its hairline on the bottom seam only, not as a four-sided ring', () => {
    // The glass token's ring put a line under the titlebar band and a second
    // one beside the rail's own right-edge hairline — a doubled seam in the
    // header band. A docked bar has one seam: the edge the content meets.
    render(<ViewHeader title="Home" />);
    const header = container.querySelector('[data-testid="view-header"]') as HTMLElement;
    expect(header.style.boxShadow).not.toMatch(/inset 0 0 0 1px/);
    expect(header.style.boxShadow).toContain('inset 0 -1px 0 0 rgba(255,255,255,0.07)');
  });

  it('does not pay for a blur: nothing ever scrolls behind a fixed sibling bar', () => {
    // The header sits ABOVE the scroll container over an opaque view wrapper,
    // so a backdrop-filter would only blur a static fill into itself — a
    // compositing pass on every tab for zero visible pixels.
    render(<ViewHeader title="Home" />);
    const header = container.querySelector('[data-testid="view-header"]') as HTMLElement;
    expect(header.style.backdropFilter).toBeFalsy();
    expect((header.style as unknown as Record<string, unknown>).WebkitBackdropFilter).toBeFalsy();
  });

  it('collapses to the flat surface under Reduce Transparency, dropping the filter', () => {
    setNativeReduceTransparency(true);
    render(<ViewHeader title="Home" />);
    const header = container.querySelector('[data-testid="view-header"]') as HTMLElement;
    expect(header.style.background).toBe(normalised(THEME_GLASS.dark.glass.opaque));
    expect(header.style.backdropFilter).toBeFalsy();
  });

  it('`surface` still wins, and never puts a blur under its opaque fill', () => {
    render(<ViewHeader title="Grow" surface="rgb(1, 2, 3)" />);
    const header = container.querySelector('[data-testid="view-header"]') as HTMLElement;
    expect(header.style.backdropFilter).toBeFalsy();
    expect(header.style.borderBottom).not.toBe('');
  });

  it('`surface` paints the opaque fill and pins the header above scrolling content', () => {
    render(<ViewHeader title="Grow" surface="rgb(1, 2, 3)" />);
    const header = container.querySelector('[data-testid="view-header"]') as HTMLElement;
    expect(header.style.background).toBe('rgb(1, 2, 3)');
    expect(header.style.position).toBe('relative');
    expect(header.style.zIndex).toBe('1');
  });

  it('hardScrollEdgeSurface is exactly what `surface` applies', () => {
    render(<ViewHeader title="Grow" surface="rgb(1, 2, 3)" />);
    const header = container.querySelector('[data-testid="view-header"]') as HTMLElement;
    const expected = hardScrollEdgeSurface('rgb(1, 2, 3)');
    expect(header.style.background).toBe(expected.background);
    expect(header.style.position).toBe(expected.position);
    expect(header.style.zIndex).toBe(String(expected.zIndex));
  });
});

// ── Source guard ──
// A render test cannot catch someone hand-rolling a fresh header in a view.
// This does: every top-level tab must go through ViewHeader.

const VIEWS: Array<[string, string]> = [
  ['Home', 'dashboard/Dashboard.tsx'],
  ['Projects', 'projects/ProjectsView.tsx'],
  ['Build', 'build/BuildView.tsx'],
  ['Automate', 'automate/AutomateView.tsx'],
];

describe('every top-level view uses the shared header', () => {
  for (const [name, relative] of VIEWS) {
    it(`${name} renders <ViewHeader>`, () => {
      const source = readFileSync(join(__dirname, '..', relative), 'utf8');
      expect(source).toContain('<ViewHeader');
      expect(source).toContain("from '../common/ViewHeader'");
    });
  }
});

// The two screens that used to paint their own hard-scroll-edge fill by hand
// (#1180's grow/ wrapper div, #1164's chat dock header) now go through
// ViewHeader's `surface` prop / `hardScrollEdgeSurface` instead — this pins
// that they stayed on the shared mechanic rather than drifting back to a
// hand-rolled `background` + `position: 'relative'` + `zIndex: 1` trio.
describe('grow/ and chat/ adopted the shared hard-scroll-edge mechanic', () => {
  it('GrowView passes `surface` to ViewHeader instead of wrapping it in its own filled div', () => {
    const source = readFileSync(join(__dirname, '..', 'grow', 'GrowView.tsx'), 'utf8');
    expect(source).toMatch(/<ViewHeader[\s\S]{0,80}surface=/);
  });

  it('ChatDock uses hardScrollEdgeSurface for its own (non-ViewHeader) header', () => {
    const source = readFileSync(join(__dirname, '..', 'chat', 'ChatDock.tsx'), 'utf8');
    expect(source).toContain("import { hardScrollEdgeSurface } from '../common/ViewHeader'");
    expect(source).toMatch(/\.\.\.hardScrollEdgeSurface\(/);
  });
});
