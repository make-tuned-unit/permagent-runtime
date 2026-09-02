/**
 * @vitest-environment jsdom
 *
 * The board's half of the Apple glass pass (R12), pinned as behaviour rather
 * than as a screenshot.
 *
 * Four rules from the design directive are each independently breakable by a
 * later edit, and none of them is visible to the existing board tests:
 *
 *   D1  a column is CONTENT, so it is opaque in every state — the old fill was
 *       a translucent white wash that was also invisible on the silver theme;
 *   D4  the corner ladder is derived (column → card → chip), not three
 *       separately-chosen numbers;
 *   D10 a card has a real pointer ladder — hover lifts, press settles and
 *       scales, which is what says "press to drag" before the drag travels;
 *   D11 the pinned column header's scroll edge appears only once there is
 *       content passing under it ("do not use it where nothing is floating").
 *
 * The board drags on raw pointer events, and jsdom ships no PointerEvent, so
 * the helper dispatches MouseEvents under pointer type names — the same trick
 * `ProjectKanban.cardDetail.test.tsx` uses, and what React's `onPointerDown`
 * actually subscribes to.
 */
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('../../lib/api', () => ({
  api: { readConfig: vi.fn(() => new Promise(() => {})) },
  apiFetch: vi.fn(),
  extractText: vi.fn(() => ''),
  extractThinking: vi.fn(() => ''),
  fileToBase64: vi.fn(),
  readerIngest: vi.fn(),
  getApiBaseUrl: vi.fn(() => 'http://localhost:1234'),
}));
vi.mock('../../lib/useGoalEvents', () => ({ useGoalEvents: () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(() => Promise.resolve()) }));

import { ProjectKanban } from './ProjectsView';
import { apiFetch } from '../../lib/api';
import { concentric, radius, space, THEME_GLASS } from '../../styles/tokens';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
const apiFetchMock = vi.mocked(apiFetch);

let storage: Record<string, string> = {};
vi.stubGlobal('localStorage', {
  getItem: (k: string) => (k in storage ? storage[k] : null),
  setItem: (k: string, v: string) => { storage[k] = v; },
  removeItem: (k: string) => { delete storage[k]; },
  clear: () => { storage = {}; },
});

const project = {
  id: 'p1', slug: 'p1', name: 'Kinros', description: '', status: 'active',
  rootPath: null, siteUrl: null, repoUrl: null, tags: [],
  metadataJson: {}, createdAt: '', updatedAt: '', lastOpenedAt: '',
};

const column = {
  id: 'col1', projectId: 'p1', name: 'Backlog', position: 0,
  columnKind: 'manual', stateBinding: null, wipLimit: null,
};

const card = {
  id: 'c1', projectId: 'p1', columnId: 'col1', title: 'Renew the domain',
  description: 'Before it lapses', cardType: 'standard', position: 0,
  metadataJson: {}, createdAt: '', updatedAt: '',
};

let container: HTMLDivElement;
let root: Root;

function pointer(type: string, target: EventTarget, x: number, y: number) {
  const e = new MouseEvent(type, { bubbles: true, cancelable: true, clientX: x, clientY: y });
  Object.defineProperty(e, 'pointerId', { value: 1 });
  target.dispatchEvent(e);
}

beforeEach(() => {
  storage = {};
  apiFetchMock.mockReset();
  apiFetchMock.mockImplementation((url: string) => {
    if (url.endsWith('/columns')) return Promise.resolve([column]);
    if (url.endsWith('/cards')) return Promise.resolve([card]);
    return Promise.resolve({});
  });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function renderBoard() {
  await act(async () => { root.render(<ProjectKanban project={project} />); });
  for (let i = 0; i < 4; i++) await act(async () => { await Promise.resolve(); });
}

/** The column box — the element the board registers as a drop target, i.e. the
 *  one whose corner the cards' corner is derived from. */
function columnEl(): HTMLElement {
  const el = Array.from(container.querySelectorAll('div'))
    .find(n => n.style.borderRadius === `${radius.xl}px` && n.textContent?.includes('Backlog'));
  if (!el) throw new Error('column not rendered');
  return el as HTMLElement;
}

function cardEl(): HTMLElement {
  const el = Array.from(container.querySelectorAll('[role="button"]'))
    .find(n => n.textContent?.includes('Renew the domain'));
  if (!el) throw new Error('card not rendered');
  return el as HTMLElement;
}

/** The scroller the cards live in — the element that owns the scroll the
 *  header's hard edge responds to. */
function cardScroller(): HTMLElement {
  return cardEl().parentElement as HTMLElement;
}

function columnHeader(): HTMLElement {
  const el = Array.from(columnEl().querySelectorAll('div'))
    .find(n => n.textContent?.trim().startsWith('Backlog'));
  if (!el) throw new Error('column header not rendered');
  return el as HTMLElement;
}

describe('kanban: the content layer stays opaque (D1)', () => {
  it('gives the column a solid theme fill and no backdrop filter at rest', async () => {
    await renderBoard();
    const col = columnEl();
    // A solid hex, not an `rgba(…, 0.02)` wash: the fill is the theme's own
    // recessed surface, so the cards read as sitting in a well.
    expect(col.style.background).toMatch(/^(#[0-9a-fA-F]{6}|rgb\(\d+, ?\d+, ?\d+\))$/);
    expect(col.style.backdropFilter || '').toBe('');
  });

  it('keeps the card an opaque surface with no backdrop filter', async () => {
    await renderBoard();
    const el = cardEl();
    expect(el.style.background).toMatch(/^(#[0-9a-fA-F]{6}|rgb\(\d+, ?\d+, ?\d+\))$/);
    expect(el.style.backdropFilter || '').toBe('');
  });

  it('puts the glass on the card menu — the one floating control here', async () => {
    await renderBoard();
    await act(async () => {
      container.querySelector<HTMLElement>('[data-testid="card-menu-c1"]')!.click();
    });
    const menu = container.querySelector<HTMLElement>('[role="menu"]');
    expect(menu, 'the card menu should be open').toBeTruthy();
    // The default (dark) theme's `glass` material, straight off the token — not
    // a hand-rolled blur, and not the opaque dropdown fill it used to use.
    expect(menu!.style.backdropFilter).toBe(THEME_GLASS.dark.glass.backdropFilter);
    expect(menu!.style.background).toBeTruthy();
  });
});

describe('kanban: concentric radii (D4)', () => {
  it('derives the card corner from the column, and the chip from the card', async () => {
    await renderBoard();
    const cardRadius = concentric(radius.xl, space.xs);
    expect(columnEl().style.borderRadius).toBe(`${radius.xl}px`);
    expect(cardEl().style.borderRadius).toBe(`${cardRadius}px`);

    // The card-type chip is the innermost rect on the card; its corner is the
    // card's minus the card's padding. (This board's only standard card has no
    // type chip, so the derived value is asserted against the menu trigger,
    // which sits at the same inset.)
    const trigger = container.querySelector<HTMLElement>('[data-testid="card-menu-c1"]')!;
    expect(trigger.style.getPropertyValue('--pa-btn-radius'))
      .toBe(`${concentric(cardRadius, space.md)}px`);
  });
});

describe('kanban: the card carries a pointer ladder (D10)', () => {
  it('lifts on hover and settles under the press, with the press-to-drag cursor', async () => {
    await renderBoard();
    const el = cardEl();
    const rest = el.style.boxShadow;
    expect(el.style.cursor).toBe('grab');

    await act(async () => { el.dispatchEvent(new MouseEvent('mouseover', { bubbles: true })); });
    const hovered = el.style.boxShadow;
    expect(hovered, 'hover must lift the card, not only recolour its border').not.toBe(rest);

    await act(async () => { pointer('pointerdown', el, 40, 40); });
    expect(el.style.transform).toBe('scale(0.98)');
    expect(el.style.cursor).toBe('grabbing');
    expect(el.style.boxShadow, 'press compresses the shadow back to flat').toBe(rest);

    await act(async () => { pointer('pointerup', el, 40, 40); });
    expect(el.style.transform).toBe('none');
    expect(el.style.cursor).toBe('grab');
  });

  it('makes every one of those transitions instant under Reduce Motion', async () => {
    storage['permagent-reduce-motion'] = 'true';
    await renderBoard();
    expect(cardEl().style.transition).toBe('none');
    expect(columnEl().style.transition).toBe('none');
  });

  it('keeps every card transition under Apple‘s 500ms ceiling', async () => {
    await renderBoard();
    const durations = [...cardEl().style.transition.matchAll(/(\d+)ms/g)].map(m => Number(m[1]));
    expect(durations.length).toBeGreaterThan(0);
    for (const ms of durations) expect(ms).toBeLessThan(500);
  });
});

describe('kanban: hard scroll edges (D11)', () => {
  it('shows the column header boundary only once the scroller has moved', async () => {
    await renderBoard();
    expect(columnHeader().style.borderBottom).toBe('1px solid transparent');

    const scroller = cardScroller();
    // jsdom never lays out, so `scrollTop` is a plain 0 — set it, then fire the
    // scroll the component listens for.
    Object.defineProperty(scroller, 'scrollTop', { value: 24, configurable: true });
    await act(async () => { scroller.dispatchEvent(new Event('scroll', { bubbles: true })); });

    const edge = columnHeader().style.borderBottom;
    expect(edge).not.toBe('1px solid transparent');
    expect(edge).toContain('1px solid');
  });

  it('contains the column scroll rather than chaining it to the board', async () => {
    await renderBoard();
    expect(cardScroller().style.overscrollBehavior).toBe('contain');
  });
});
