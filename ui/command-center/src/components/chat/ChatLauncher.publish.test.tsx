/** @vitest-environment jsdom */
/**
 * The launcher pill publishes its own measured size to the store, and the
 * Browser subtracts that corner from the native webview's bounds (#553) — the
 * webview composites above all DOM, so the only way the pill stays visible is
 * for the webview's rect not to reach it.
 *
 * That makes the publication load-bearing for a VISUAL pass in a way that is
 * easy to miss: a polish change that alters the pill's geometry, or one that
 * animates a property the measurement reads, silently moves the hole the
 * browser leaves for it. This pins the contract.
 */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const { storeState, setChatLauncherSize } = vi.hoisted(() => ({
  setChatLauncherSize: vi.fn(),
  storeState: {
    chatWindowOpen: false,
    chatDockOpen: false,
    identityRev: 0,
    setChatWindowOpen: vi.fn(),
    openChatDock: vi.fn(),
    setChatLauncherSize: vi.fn(),
  },
}));
storeState.setChatLauncherSize = setChatLauncherSize;

vi.mock('../../lib/store', () => ({
  useCommandCenter: (sel: (s: typeof storeState) => unknown) => sel(storeState),
}));
vi.mock('../../lib/api', () => ({
  api: { getIdentity: () => Promise.resolve({ first_name: 'Henry' }) },
}));

import { ChatLauncher, CHAT_LAUNCHER_MARGIN } from './ChatLauncher';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/** jsdom has no ResizeObserver and no layout; supply both. */
let observed: Element[] = [];
class StubResizeObserver {
  constructor(private cb: () => void) {}
  observe(el: Element) { observed.push(el); StubResizeObserver.last = this; }
  disconnect() { /* noop */ }
  fire() { this.cb(); }
  static last: StubResizeObserver | null = null;
}

let container: HTMLDivElement;
let root: Root;
const RECT = { width: 186, height: 44 };

beforeEach(() => {
  observed = [];
  StubResizeObserver.last = null;
  setChatLauncherSize.mockClear();
  storeState.chatWindowOpen = false;
  storeState.chatDockOpen = false;
  (globalThis as Record<string, unknown>).ResizeObserver = StubResizeObserver;
  Object.defineProperty(HTMLElement.prototype, 'getBoundingClientRect', {
    configurable: true,
    value: () => ({ ...RECT, x: 0, y: 0, top: 0, left: 0, right: 0, bottom: 0, toJSON: () => ({}) }),
  });
  container = document.createElement('div');
  document.body.appendChild(container);
});

afterEach(() => {
  act(() => root?.unmount());
  container.remove();
});

async function render() {
  root = createRoot(container);
  // `async`: the agent name resolves after the first commit, and the resulting
  // re-render is exactly the one the ResizeObserver exists to catch.
  await act(async () => { root.render(<ChatLauncher />); });
}

describe('chat launcher size publication', () => {
  it('publishes the measured pill on mount, and observes it for later changes', async () => {
    await render();
    expect(setChatLauncherSize).toHaveBeenCalledWith(RECT);
    // The agent name arrives asynchronously and changes the pill's width — the
    // ResizeObserver is what keeps the reserved corner honest after that.
    expect(observed).toHaveLength(1);
    expect(observed[0].tagName).toBe('BUTTON');
  });

  it('re-publishes when the observer fires', async () => {
    await render();
    setChatLauncherSize.mockClear();
    act(() => { StubResizeObserver.last?.fire(); });
    expect(setChatLauncherSize).toHaveBeenCalledWith(RECT);
  });

  it('publishes null while a chat surface is already open, so no corner is reserved', async () => {
    storeState.chatDockOpen = true;
    await render();
    expect(setChatLauncherSize).toHaveBeenCalledWith(null);
    expect(container.querySelector('button')).toBeNull();
  });

  it('keeps the anchor the reserved corner is computed from', async () => {
    await render();
    const pill = container.querySelector('button') as HTMLElement;
    // Browser.tsx derives the reserved corner from this margin plus the
    // published size; the pill has to actually sit there.
    expect(pill.style.position).toBe('fixed');
    expect(pill.style.bottom).toBe(`${CHAT_LAUNCHER_MARGIN}px`);
    expect(pill.style.right).toBe(`${CHAT_LAUNCHER_MARGIN}px`);
  });

  it('animates only properties that cannot change the published box', async () => {
    await render();
    const pill = container.querySelector('button') as HTMLElement;
    // `all` would put padding, border-width and font in the transition — every
    // one of which changes the measured size, and the reserved corner with it.
    // Peel the parenthesised easings off (they contain commas of their own),
    // then the property name is the head of each remaining entry.
    let list = pill.style.transition;
    while (/\([^()]*\)/.test(list)) list = list.replace(/\([^()]*\)/g, '');
    const animated = list.split(',').map(part => part.trim().split(/\s+/)[0]).sort();
    expect(animated).toEqual(['border-color', 'transform']);
  });
});
