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
import { ViewHeader } from './ViewHeader';
import { type } from '../../styles/tokens';

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
