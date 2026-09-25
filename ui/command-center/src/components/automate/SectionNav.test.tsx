/** @vitest-environment jsdom
 *
 * A5: Automate is one long scroll, and Learned was reachable only by
 * scrolling past everything above it. This pins the jump bar's own rules:
 * it renders nothing for a single section (nothing to jump between), and
 * clicking an item scrolls its target into view (jsdom stubs
 * `scrollIntoView`, so this checks the call, not real layout — see
 * LANE_RULES on jsdom having no layout).
 */
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';
import { createRef } from 'react';

import { SectionNav, SECTION_NAV_OFFSET } from './SectionNav';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;
beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});
afterEach(() => { act(() => root.unmount()); container.remove(); });

function Harness({ items }: { items: { id: string; label: string; count?: number }[] }) {
  const ref = createRef<HTMLDivElement>();
  return (
    <div ref={ref} data-testid="scroll-root">
      <SectionNav items={items} scrollRoot={ref} />
      {items.map(i => (
        <div key={i.id} data-section={i.id} style={{ height: 400 }}>{i.label} content</div>
      ))}
    </div>
  );
}

it('renders nothing when there is only one (or no) section — nothing to jump between', () => {
  act(() => root.render(<Harness items={[{ id: 'only', label: 'Only' }]} />));
  expect(container.querySelector('[data-testid="automate-section-nav"]')).toBeNull();

  act(() => root.render(<Harness items={[]} />));
  expect(container.querySelector('[data-testid="automate-section-nav"]')).toBeNull();
});

it('lists every section with its count, and jumps to the clicked one', () => {
  const items = [
    { id: 'running', label: 'Running Now', count: 2 },
    { id: 'learned', label: 'Learned', count: 5 },
  ];
  act(() => root.render(<Harness items={items} />));
  const nav = container.querySelector('[data-testid="automate-section-nav"]')!;
  expect(nav.textContent).toContain('Running Now');
  expect(nav.textContent).toContain('2');
  expect(nav.textContent).toContain('Learned');
  expect(nav.textContent).toContain('5');

  const target = container.querySelector('[data-section="learned"]') as HTMLElement;
  const scrollIntoView = vi.fn();
  target.scrollIntoView = scrollIntoView;

  const learnedButton = [...nav.querySelectorAll('button')].find(b => b.textContent?.includes('Learned'))!;
  act(() => { learnedButton.dispatchEvent(new MouseEvent('click', { bubbles: true })); });

  expect(scrollIntoView).toHaveBeenCalledWith(expect.objectContaining({ block: 'start' }));
  expect(learnedButton.getAttribute('aria-current')).toBe('true');
});

it('omits the count badge for an empty section', () => {
  act(() => root.render(<Harness items={[{ id: 'a', label: 'A', count: 0 }, { id: 'b', label: 'B', count: 3 }]} />));
  const nav = container.querySelector('[data-testid="automate-section-nav"]')!;
  const aButton = [...nav.querySelectorAll('button')].find(b => b.textContent?.startsWith('A'))!;
  expect(aButton.textContent).toBe('A');
});

it('exports the offset used for both the sticky bar height and each section\'s scroll-margin', () => {
  expect(SECTION_NAV_OFFSET).toBeGreaterThan(0);
});
