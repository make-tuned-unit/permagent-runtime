/** @vitest-environment jsdom */
/**
 * QA walk 2026-09-24 (1456px): To-dos sliced its third row through the middle
 * of the text at the card's bottom edge, with nothing saying there was more.
 * Every Home card list now sits in one CardScrollBody that fades the bottom
 * edge and counts the hidden rows while content continues below.
 *
 * jsdom has no layout, so the geometry is stubbed per element: this proves the
 * measuring and the affordance, not WebKit's paint.
 */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it } from 'vitest';

import { CardScrollBody, rowsBelowFold } from './atoms';

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
});

function rect(top: number, bottom: number): DOMRect {
  return { top, bottom, left: 0, right: 100, width: 100, height: bottom - top, x: 0, y: top, toJSON: () => ({}) } as DOMRect;
}

/** Give the scroller and its rows a geometry: a 100px viewport, 40px rows. */
function stubGeometry(scroller: HTMLElement, rows: number, scrollTop = 0) {
  Object.defineProperty(scroller, 'clientHeight', { configurable: true, value: 100 });
  Object.defineProperty(scroller, 'scrollHeight', { configurable: true, value: rows * 40 });
  scroller.scrollTop = scrollTop;
  Object.defineProperty(scroller, 'scrollTop', { configurable: true, writable: true, value: scrollTop });
  scroller.getBoundingClientRect = () => rect(0, 100);
  scroller.querySelectorAll<HTMLElement>('[data-card-row]').forEach((row, i) => {
    row.getBoundingClientRect = () => rect(i * 40 - scrollTop, i * 40 + 40 - scrollTop);
  });
}

function renderRows(n: number) {
  act(() => root.render(
    <div style={{ height: 100 }}>
      <CardScrollBody testId="body">
        {Array.from({ length: n }, (_, i) => <div key={i} data-card-row="">Row {i}</div>)}
      </CardScrollBody>
    </div>,
  ));
  return container.querySelector('[data-testid="body"]') as HTMLElement;
}

it('fades the bottom edge and counts the hidden rows when the list overflows', () => {
  const body = renderRows(5);
  stubGeometry(body, 5);
  act(() => { body.dispatchEvent(new Event('scroll')); });
  // Rows at 0-40, 40-80 fit; 80-120 is cut in half; 120-160, 160-200 below.
  expect(rowsBelowFold(body)).toBe(3);
  expect(body.getAttribute('data-overflowing')).toBe('true');
  expect(body.style.maskImage || body.style.webkitMaskImage).toContain('linear-gradient');
  expect(container.querySelector('[data-testid="body-more"]')?.textContent).toContain('+3 more');
});

it('says nothing when everything fits, and drops the affordance at the end of the list', () => {
  const body = renderRows(5);
  stubGeometry(body, 5, 100);
  act(() => { body.dispatchEvent(new Event('scroll')); });
  expect(body.getAttribute('data-overflowing')).toBeNull();
  expect(container.querySelector('[data-testid="body-more"]')).toBeNull();

  const short = renderRows(2);
  stubGeometry(short, 2);
  act(() => { short.dispatchEvent(new Event('scroll')); });
  expect(container.querySelector('[data-testid="body-more"]')).toBeNull();
  expect(short.style.maskImage || '').toBe('');
});

it('the pill scrolls the list on toward the hidden rows', () => {
  const body = renderRows(5);
  stubGeometry(body, 5);
  act(() => { body.dispatchEvent(new Event('scroll')); });
  let scrolledTo: number | undefined;
  body.scrollTo = ((opts: ScrollToOptions) => { scrolledTo = opts.top; }) as typeof body.scrollTo;
  act(() => { (container.querySelector('[data-testid="body-more"]') as HTMLElement).click(); });
  expect(scrolledTo).toBeGreaterThan(0);
});
