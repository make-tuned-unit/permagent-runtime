/** @vitest-environment jsdom */
/**
 * The launcher pill floats over every screen, and on Grow → Analytics it hid
 * the last row of the EVENTS column (found driving the real app, 2026-09-18).
 * The EVENTS list (FirstPartyAnalyticsPanel) does not scroll by itself: it
 * sits inside the Grow tabpanel (GrowView's `flex: 1; overflowY: auto`
 * column-flex panel), whose scroll range ended at the viewport's bottom,
 * under the pill, so no amount of scrolling revealed that row.
 *
 * The launcher now marks every height-limited scroll region under its box and
 * gives each a trailing block as tall as the pill plus its margins. jsdom has
 * no layout and no hit-testing, so `elementsFromPoint` is stubbed to return
 * the stack a real browser would, and scrollHeight/clientHeight are stubbed
 * per element; this proves WHICH regions get clearance (and that sideways-only
 * table wrappers do not), how much, that scrolling re-marks, and that it is
 * withdrawn when the pill hides. It does not prove the pixels line up in
 * WebKit — that needs the real app.
 */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const { storeState } = vi.hoisted(() => ({
  storeState: {
    chatWindowOpen: false,
    chatDockOpen: false,
    identityRev: 0,
    setChatWindowOpen: vi.fn(),
    openChatDock: vi.fn(),
    setChatLauncherSize: vi.fn(),
  },
}));
vi.mock('../../lib/store', () => ({
  useCommandCenter: (sel: (s: typeof storeState) => unknown) => sel(storeState),
}));
vi.mock('../../lib/api', () => ({
  api: { getIdentity: () => Promise.resolve({ first_name: 'Henry' }) },
}));

import * as Launcher from './ChatLauncher';
const { ChatLauncher, CHAT_LAUNCHER_MARGIN } = Launcher;
const ATTR = 'data-pa-launcher-clearance';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

class StubResizeObserver { observe() {} disconnect() {} }

const PILL_HEIGHT = 44;
let container: HTMLDivElement;
let page: HTMLDivElement;
let column: HTMLDivElement;
let row: HTMLDivElement;
let hscroller: HTMLDivElement;
let tableWrap: HTMLDivElement;
let root: Root;

/** jsdom reports 0 for both; give an element the box a real layout would. */
function setBox(el: HTMLElement, scrollHeight: number, clientHeight: number) {
  Object.defineProperty(el, 'scrollHeight', { configurable: true, get: () => scrollHeight });
  Object.defineProperty(el, 'clientHeight', { configurable: true, get: () => clientHeight });
}

beforeEach(() => {
  storeState.chatWindowOpen = false;
  storeState.chatDockOpen = false;
  (globalThis as Record<string, unknown>).ResizeObserver = StubResizeObserver;
  Object.defineProperty(HTMLElement.prototype, 'getBoundingClientRect', {
    configurable: true,
    value: () => ({ width: 186, height: PILL_HEIGHT, x: 0, y: 0, top: 0, left: 0, right: 186, bottom: PILL_HEIGHT, toJSON: () => ({}) }),
  });

  // A scrolling shell page holding the Grow tabpanel (a column-flex scroller
  // whose content overflows it), a plain row inside it (an EVENTS row), a
  // horizontal chip strip, and a sideways-only table wrapper like Settings'
  // egress log / Finance's holdings (`overflowX: 'auto'`, auto height — CSS
  // computes its overflow-y to `auto` too, but its box fits its content).
  page = document.createElement('div');
  page.style.overflowY = 'auto';
  setBox(page, 2000, 900);
  column = document.createElement('div');
  column.setAttribute('role', 'tabpanel');
  column.style.overflowY = 'auto';
  column.style.display = 'flex';
  column.style.flexDirection = 'column';
  setBox(column, 1600, 700);
  row = document.createElement('div');
  hscroller = document.createElement('div');
  hscroller.style.overflowY = 'auto';
  hscroller.style.display = 'flex';
  hscroller.style.flexDirection = 'row';
  setBox(hscroller, 120, 40);
  tableWrap = document.createElement('div');
  tableWrap.style.overflowX = 'auto';
  tableWrap.style.overflowY = 'auto'; // what getComputedStyle reports for it
  setBox(tableWrap, 320, 320);
  column.appendChild(row);
  page.append(column, hscroller, tableWrap);
  document.body.appendChild(page);

  container = document.createElement('div');
  document.body.appendChild(container);
  (document as unknown as { elementsFromPoint: (x: number, y: number) => Element[] }).elementsFromPoint = () => {
    const pill: Element | null = container.querySelector('button');
    const stack: Array<Element | null> = [pill, row, column, hscroller, tableWrap, page, document.body, document.documentElement];
    return stack.filter((e): e is Element => e !== null);
  };
});

afterEach(() => {
  act(() => root?.unmount());
  container.remove();
  page.remove();
});

async function render() {
  root = createRoot(container);
  await act(async () => { root.render(<ChatLauncher />); });
}

describe('chat launcher scroll clearance', () => {
  it('marks every vertical scroll region under the pill, and nothing else', async () => {
    await render();
    expect(column.hasAttribute(ATTR), 'the Grow tabpanel must get room to scroll EVENTS clear').toBe(true);
    expect(page.hasAttribute(ATTR), 'the page scroller holding it too').toBe(true);
    expect(row.hasAttribute(ATTR), 'a plain row does not scroll').toBe(false);
    expect(hscroller.hasAttribute(ATTR), 'a row flex box would grow sideways, not down').toBe(false);
    expect(document.body.hasAttribute(ATTR)).toBe(false);
    expect(document.documentElement.hasAttribute(ATTR)).toBe(false);
  });

  it('reserves the pill height plus a margin above and below it', async () => {
    await render();
    const style = container.querySelector('style[data-testid="chat-launcher-clearance"]');
    expect(style?.textContent).toContain(`[${ATTR}]::after`);
    expect(style?.textContent).toContain(`height:${PILL_HEIGHT + 2 * CHAT_LAUNCHER_MARGIN}px`);
    expect(style?.textContent).toContain('display:block');
  });

  it('withdraws every mark when a chat surface opens and the pill hides', async () => {
    await render();
    expect(column.hasAttribute(ATTR)).toBe(true);
    storeState.chatDockOpen = true;
    await act(async () => { root.render(<ChatLauncher />); });
    expect(container.querySelector('button')).toBeNull();
    expect(column.hasAttribute(ATTR)).toBe(false);
    expect(page.hasAttribute(ATTR)).toBe(false);
  });

  it('picks up a scroll region that mounts after the pill (a tab switch)', async () => {
    await render();
    const late = document.createElement('div');
    late.style.overflowY = 'scroll';
    setBox(late, 900, 300);
    const prev = document.elementsFromPoint;
    (document as unknown as { elementsFromPoint: typeof prev }).elementsFromPoint =
      (x: number, y: number) => [late, ...prev.call(document, x, y)];
    await act(async () => {
      document.body.appendChild(late);
      await new Promise(r => setTimeout(r, 50)); // MutationObserver + frame
    });
    expect(late.hasAttribute(ATTR)).toBe(true);
    late.remove();
  });

  it('leaves a sideways-only table wrapper alone, even though its overflow-y computes to auto', async () => {
    await render();
    // Marking it would put an 84px block INSIDE the table's auto-height box
    // and push everything below the table down by that much.
    expect(tableWrap.hasAttribute(ATTR)).toBe(false);
  });

  it('takes the mark straight back off a wrapper that the block would make taller', async () => {
    // Real WebKit (2026-09-19): a sideways-only table wrapper under the pill
    // reported scrollHeight 90 over clientHeight 84 on its first layout, so it
    // passed the overflow test, got marked, and stayed 84px taller for good:
    // setting an attribute is not a childList mutation, so nothing re-checked
    // it. Here the stubbed wrapper overflows the same way, and its box grows by
    // the block whenever the mark is on — what an auto-height box does. A
    // height-limited region (the column) keeps its box and keeps its mark.
    const wrap = document.createElement('div');
    wrap.style.overflowY = 'auto';
    setBox(wrap, 90, 84);
    Object.defineProperty(wrap, 'getBoundingClientRect', {
      configurable: true,
      value: () => {
        const h = 96 + (wrap.hasAttribute(ATTR) ? PILL_HEIGHT + 2 * CHAT_LAUNCHER_MARGIN : 0);
        return { width: 600, height: h, x: 0, y: 0, top: 0, left: 0, right: 600, bottom: h, toJSON: () => ({}) };
      },
    });
    page.appendChild(wrap);
    const prev = document.elementsFromPoint;
    (document as unknown as { elementsFromPoint: typeof prev }).elementsFromPoint =
      (x: number, y: number) => [wrap, ...prev.call(document, x, y)];
    await render();
    expect(wrap.hasAttribute(ATTR), 'an auto-height wrapper must not keep the block').toBe(false);
    expect(column.hasAttribute(ATTR), 'the limited Grow tabpanel still gets it').toBe(true);
  });

  it('re-marks on scroll: a nested scroller that scrolls under the pill gets room, one that leaves loses it', async () => {
    await render();
    const inner = document.createElement('div');
    inner.style.overflowY = 'auto';
    setBox(inner, 900, 300);
    column.appendChild(inner);
    // Let the MutationObserver pass for that append settle BEFORE the inner
    // scroller reaches the pill, so only the scroll event can mark it.
    await act(async () => { await new Promise(r => setTimeout(r, 50)); });
    expect(inner.hasAttribute(ATTR)).toBe(false);

    const prev = document.elementsFromPoint;
    (document as unknown as { elementsFromPoint: typeof prev }).elementsFromPoint =
      (x: number, y: number) => [inner, ...prev.call(document, x, y)];
    await act(async () => {
      column.dispatchEvent(new Event('scroll')); // scroll does not bubble
      await new Promise(r => setTimeout(r, 50));
    });
    expect(inner.hasAttribute(ATTR), 'scrolled under the pill').toBe(true);

    (document as unknown as { elementsFromPoint: typeof prev }).elementsFromPoint = prev;
    await act(async () => {
      column.dispatchEvent(new Event('scroll'));
      await new Promise(r => setTimeout(r, 50));
    });
    expect(inner.hasAttribute(ATTR), 'scrolled away from the pill').toBe(false);
  });

  it('stops listening for scroll once the pill hides', async () => {
    await render();
    storeState.chatDockOpen = true;
    await act(async () => { root.render(<ChatLauncher />); });
    const late = document.createElement('div');
    late.style.overflowY = 'auto';
    setBox(late, 900, 300);
    const prev = document.elementsFromPoint;
    (document as unknown as { elementsFromPoint: typeof prev }).elementsFromPoint =
      (x: number, y: number) => [late, ...prev.call(document, x, y)];
    await act(async () => {
      document.dispatchEvent(new Event('scroll'));
      await new Promise(r => setTimeout(r, 50));
    });
    expect(late.hasAttribute(ATTR)).toBe(false);
  });
});
