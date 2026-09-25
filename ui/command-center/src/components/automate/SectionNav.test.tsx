/** @vitest-environment jsdom
 *
 * A5: Automate is one long scroll, and Learned was reachable only by
 * scrolling past everything above it. This pins the jump bar's own rules:
 * it renders nothing for a single section (nothing to jump between), and
 * clicking an item scrolls its target into view (jsdom stubs
 * `scrollIntoView`, so this checks the call, not real layout — see
 * LANE_RULES on jsdom having no layout).
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';
import { createRef } from 'react';

import { SectionNav, SECTION_NAV_OFFSET, pickActiveSection } from './SectionNav';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

describe('pickActiveSection (pure)', () => {
  const items = [
    { id: 'running', label: 'Running Now' },
    { id: 'automations', label: 'Automations' },
    { id: 'learned', label: 'Learned' },
  ];

  it('is the first item when nothing is in the reading band — true at scroll top', () => {
    expect(pickActiveSection(items, [])).toBe('running');
    expect(pickActiveSection(items, new Set())).toBe('running');
  });

  it('is the first item when the LIST itself changes and nothing has been observed yet', () => {
    // The exact QA repro: before data loads, `items` is just [automations,
    // learned, installed] (loading placeholder), so "automations" is
    // genuinely items[0] at that instant. This asserts the fix's actual
    // contract — the function has no memory, so a caller that re-derives
    // `items` after data loads and calls this again gets the NEW first item.
    const loading = [{ id: 'automations', label: 'Automations' }, { id: 'learned', label: 'Learned' }];
    expect(pickActiveSection(loading, [])).toBe('automations');
    const loaded = [{ id: 'running', label: 'Running Now' }, ...loading];
    expect(pickActiveSection(loaded, [])).toBe('running');
  });

  it('picks the entry in the reading band, in nav order — not observer callback order', () => {
    expect(pickActiveSection(items, ['learned'])).toBe('learned');
    // Two "intersecting" at once (a short section) — nav order decides, not
    // Set/array insertion order.
    expect(pickActiveSection(items, ['learned', 'automations'])).toBe('automations');
  });

  it('is null for an empty item list', () => {
    expect(pickActiveSection([], ['x'])).toBeNull();
  });
});

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

describe('scroll-spy wiring (QA 2026-09-25: nav stuck on "Automations" at scroll top)', () => {
  // jsdom has no IntersectionObserver — install a controllable stand-in and
  // capture the callback so the test can fire intersection changes itself.
  let instances: Array<{ callback: IntersectionObserverCallback; observed: Element[]; disconnect: () => void }>;
  beforeEach(() => {
    instances = [];
    (globalThis as { IntersectionObserver?: unknown }).IntersectionObserver = class {
      callback: IntersectionObserverCallback;
      observed: Element[] = [];
      constructor(cb: IntersectionObserverCallback) {
        this.callback = cb;
        instances.push(this);
      }
      observe(el: Element) { this.observed.push(el); }
      unobserve() {}
      disconnect() {}
      takeRecords() { return []; }
    } as unknown as typeof IntersectionObserver;
  });
  afterEach(() => { delete (globalThis as { IntersectionObserver?: unknown }).IntersectionObserver; });

  it('is active on the FIRST item before the observer has reported anything (scroll top)', () => {
    const items = [{ id: 'running', label: 'Running Now' }, { id: 'learned', label: 'Learned' }];
    act(() => root.render(<Harness items={items} />));
    const nav = container.querySelector('[data-testid="automate-section-nav"]')!;
    const runningBtn = [...nav.querySelectorAll('button')].find(b => b.textContent?.includes('Running Now'))!;
    expect(runningBtn.getAttribute('aria-current')).toBe('true');
  });

  it('recomputes from the item list alone when it changes — no scroll or observer callback needed', () => {
    // Mount with the "loading placeholder" shape (Automations is items[0],
    // exactly as AutomateView renders before /schedule/list resolves).
    const loading = [{ id: 'automations', label: 'Automations' }, { id: 'learned', label: 'Learned' }];
    function Rerenderable({ items }: { items: typeof loading }) {
      return <Harness items={items} />;
    }
    act(() => root.render(<Rerenderable items={loading} />));
    let nav = container.querySelector('[data-testid="automate-section-nav"]')!;
    expect([...nav.querySelectorAll('button')].find(b => b.getAttribute('aria-current') === 'true')?.textContent)
      .toContain('Automations');

    // Data loads: "Running Now" appears ahead of it. No scroll event, no
    // IntersectionObserver callback — the old bug left "Automations" active
    // forever because nothing re-ran the calculation.
    const loaded = [{ id: 'running', label: 'Running Now' }, ...loading];
    act(() => root.render(<Rerenderable items={loaded} />));
    nav = container.querySelector('[data-testid="automate-section-nav"]')!;
    expect([...nav.querySelectorAll('button')].find(b => b.getAttribute('aria-current') === 'true')?.textContent)
      .toContain('Running Now');
  });

  it('follows the observer once a later section enters the reading band', () => {
    const items = [{ id: 'running', label: 'Running Now' }, { id: 'learned', label: 'Learned' }];
    act(() => root.render(<Harness items={items} />));
    expect(instances.length).toBeGreaterThan(0);
    const observer = instances[0];
    const learnedEl = container.querySelector('[data-section="learned"]')!;
    act(() => {
      observer.callback(
        [{ target: learnedEl, isIntersecting: true } as IntersectionObserverEntry],
        observer as unknown as IntersectionObserver,
      );
    });
    const nav = container.querySelector('[data-testid="automate-section-nav"]')!;
    expect([...nav.querySelectorAll('button')].find(b => b.getAttribute('aria-current') === 'true')?.textContent)
      .toContain('Learned');
  });
});
