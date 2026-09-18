/** @vitest-environment jsdom */
/**
 * The dock's width: how it is set, how it survives a restart, and — the part
 * that is easy to get wrong — how it reaches the native browser webview.
 *
 * The webview is a native child surface that composites ABOVE all DOM, so if
 * its bounds do not follow the dock's width it simply paints over the dock.
 * The census flagged the fixed 384px; making it draggable is only safe if the
 * bounds follow, and the last test here is the proof that they do.
 */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const { storeState } = vi.hoisted(() => ({
  storeState: {
    chatDockOpen: true,
    chatWindowOpen: false,
    voiceConversation: null as { exit: () => void } | null,
    closeChatDock: vi.fn(),
  },
}));

vi.mock('./ChatView', () => ({ ChatView: () => null }));
vi.mock('../../lib/chatWindow', () => ({ createChatWindow: vi.fn() }));
vi.mock('../../lib/speakReplies', () => ({
  useSpeakReplies: () => false,
  setSpeakReplies: vi.fn(),
}));
vi.mock('../../lib/store', () => ({
  useCommandCenter: Object.assign(
    (sel: (s: typeof storeState) => unknown) => sel(storeState),
    { getState: () => storeState },
  ),
}));

import {
  ChatDock,
  CHAT_DOCK_WIDTH,
  CHAT_DOCK_MIN_WIDTH,
  CHAT_DOCK_MAX_WIDTH,
  clampDockWidth,
  readDockWidth,
} from './ChatDock';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const WIDTH_KEY = 'permagent-chat-dock-width';

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  localStorage.clear();
  // Deterministic geometry: with Reduce Motion the dock opens at its full
  // width in the first commit instead of springing out over 320ms, so the
  // width assertions below measure the dock and not the animation.
  localStorage.setItem('permagent-reduce-motion', 'true');
  storeState.chatDockOpen = true;
  container = document.createElement('div');
  document.body.appendChild(container);
});

afterEach(() => {
  act(() => root?.unmount());
  container.remove();
});

function render() {
  root = createRoot(container);
  act(() => { root.render(<ChatDock />); });
}

/** The panel that holds the header and the transcript — it keeps its full
 *  width through the open/close animation so the transcript never reflows. */
function panel(): HTMLElement {
  const el = container.querySelector('[role="separator"]')?.parentElement;
  if (!el) throw new Error('dock panel not found');
  return el as HTMLElement;
}

/** The dock's outer box — the flex sibling of <main>, and therefore the
 *  element whose width shrinks the content (and the browser pane) beside it. */
function dockBox(): HTMLElement {
  const el = container.firstElementChild;
  if (!el) throw new Error('dock not rendered');
  return el as HTMLElement;
}

function pointer(type: string, clientX: number): MouseEvent {
  const e = new MouseEvent(type, { bubbles: true, cancelable: true, clientX, button: 0 });
  Object.defineProperty(e, 'pointerId', { value: 1 });
  return e;
}

describe('chat dock width', () => {
  it('clamps to the readable range and rounds to whole pixels', () => {
    expect(clampDockWidth(400)).toBe(400);
    expect(clampDockWidth(10)).toBe(CHAT_DOCK_MIN_WIDTH);
    expect(clampDockWidth(9000)).toBe(CHAT_DOCK_MAX_WIDTH);
    expect(clampDockWidth(383.6)).toBe(384);
    // A hand-edited or corrupt entry cannot wedge the dock off-screen.
    expect(clampDockWidth(Number.NaN)).toBe(CHAT_DOCK_WIDTH);
  });

  it('falls back to the default when nothing is stored, and clamps what is', () => {
    expect(readDockWidth()).toBe(CHAT_DOCK_WIDTH);
    localStorage.setItem(WIDTH_KEY, '480');
    expect(readDockWidth()).toBe(480);
    localStorage.setItem(WIDTH_KEY, '40');
    expect(readDockWidth()).toBe(CHAT_DOCK_MIN_WIDTH);
    localStorage.setItem(WIDTH_KEY, 'not a number');
    expect(readDockWidth()).toBe(CHAT_DOCK_WIDTH);
  });

  it('opens at the persisted width', () => {
    localStorage.setItem(WIDTH_KEY, '512');
    render();
    expect(panel().style.width).toBe('512px');
  });

  it('widens as the grab edge is dragged left, and persists on release', () => {
    render();
    const edge = container.querySelector('[role="separator"]') as HTMLElement;
    expect(panel().style.width).toBe(`${CHAT_DOCK_WIDTH}px`);

    act(() => { edge.dispatchEvent(pointer('pointerdown', 900)); });
    // Dragging LEFT (a smaller clientX) makes the right-hand dock wider.
    act(() => { edge.dispatchEvent(pointer('pointermove', 840)); });
    expect(panel().style.width).toBe(`${CHAT_DOCK_WIDTH + 60}px`);

    // Live, before release — nothing is committed to storage mid-gesture.
    expect(localStorage.getItem(WIDTH_KEY)).toBeNull();

    act(() => { edge.dispatchEvent(pointer('pointerup', 840)); });
    expect(localStorage.getItem(WIDTH_KEY)).toBe(String(CHAT_DOCK_WIDTH + 60));
  });

  it('refuses to drag past either end of the range', () => {
    render();
    const edge = container.querySelector('[role="separator"]') as HTMLElement;
    act(() => { edge.dispatchEvent(pointer('pointerdown', 900)); });
    act(() => { edge.dispatchEvent(pointer('pointermove', 100)); });
    expect(panel().style.width).toBe(`${CHAT_DOCK_MAX_WIDTH}px`);
    act(() => { edge.dispatchEvent(pointer('pointermove', 1600)); });
    expect(panel().style.width).toBe(`${CHAT_DOCK_MIN_WIDTH}px`);
    act(() => { edge.dispatchEvent(pointer('pointerup', 1600)); });
  });

  it('resizes from the keyboard too — a mouse-only resize is not a resize', () => {
    render();
    const edge = container.querySelector('[role="separator"]') as HTMLElement;
    expect(edge.getAttribute('aria-valuenow')).toBe(String(CHAT_DOCK_WIDTH));
    act(() => {
      edge.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', bubbles: true }));
    });
    expect(Number(panel().style.width.replace('px', ''))).toBeGreaterThan(CHAT_DOCK_WIDTH);
    expect(localStorage.getItem(WIDTH_KEY)).toBe(panel().style.width.replace('px', ''));
  });
});

/**
 * The propagation proof.
 *
 * The dock's width is deliberately NOT published to the store, and the native
 * webview follows it anyway, because the dock is a flex sibling of <main>
 * (App.tsx) — widening it narrows <main>, which narrows the browser pane's
 * container, which fires the ResizeObserver that Browser.tsx hangs on that
 * container. So the contract to hold is a NEGATIVE one plus a positive one:
 * syncBounds must take no dock-width term (subtracting one would shrink the
 * webview by a second dock width — the comment at Browser.tsx says exactly
 * this), and it must observe the container it measures.
 */
describe('a resized dock reaches the native webview', () => {
  // `import.meta.url` is an http URL under the jsdom environment, so the
  // project root is the anchor here.
  const src = (rel: string) => readFileSync(resolve(process.cwd(), 'src', rel), 'utf8');
  const browser = src('components/browser/Browser.tsx');

  it('does not take the dock width as an input to the bounds', () => {
    expect(browser).not.toMatch(/CHAT_DOCK_WIDTH|chatDockWidth/);
  });

  it('measures the container and re-measures whenever the container resizes', () => {
    expect(browser).toMatch(/const rect = containerRef\.current\.getBoundingClientRect\(\)/);
    expect(browser).toMatch(/new ResizeObserver\(\(\) => syncBounds\(\)\)/);
    expect(browser).toMatch(/observer\.observe\(containerRef\.current\)/);
  });

  it('sets the width on the element that is the flex sibling of <main>', () => {
    // The width lives in an inline style on the dock's outer box, so the flex
    // row re-lays-out on every change — which is the whole mechanism above.
    // The inner panel carries the same width so the transcript inside it never
    // reflows while the outer one animates.
    render();
    expect(dockBox().style.width).toBe(`${CHAT_DOCK_WIDTH}px`);
    expect(panel().style.width).toBe(`${CHAT_DOCK_WIDTH}px`);
    const app = src('App.tsx');
    expect(app).toMatch(/<main[\s\S]{0,400}<\/main>\s*\n\s*<ChatLauncher \/>\s*\n\s*<ChatDock \/>/);
  });
});
