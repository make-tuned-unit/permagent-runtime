/** @vitest-environment jsdom
 *
 * Chat streaming smoothness (2026-08-11 report: "the arrival of new lines can
 * stutter the sidebar chat view as text pours in").
 *
 * Two costs were measured on the pre-fix code, both paid once per streamed
 * delta rather than once per frame:
 *
 *   markdownRendersPerDelta: 5   (in an 8-message conversation — one full
 *                                 react-markdown parse per assistant bubble,
 *                                 growing linearly with history length)
 *   scrollIntoViewPerDelta:  1   with { behavior: 'smooth' } — every call
 *                                 cancels the in-flight smooth animation and
 *                                 restarts it from wherever it had reached
 *
 * These pin both back down. They are cost assertions, not paint assertions:
 * jsdom has no layout, so the scroll container's metrics are supplied here.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';
import { create } from 'zustand';

interface Msg {
  id: string;
  role: 'user' | 'assistant' | 'system';
  content: string;
  timestamp: string;
}

const hoisted = vi.hoisted(() => ({ store: null as unknown as ReturnType<typeof makeStore> }));

function makeStore() {
  return create<{
    chatMessages: Msg[];
    isStreaming: boolean;
    agentName: string;
    sessionLoadError: string | null;
    chatSessionId: string | null;
    chatHistoryLoaded: boolean;
    loadSessionMessages: () => void;
  }>(() => ({
    chatMessages: [],
    isStreaming: true,
    agentName: 'Henry',
    sessionLoadError: null,
    chatSessionId: 's1',
    chatHistoryLoaded: true,
    loadSessionMessages: () => {},
  }));
}

vi.mock('../../lib/store', () => {
  const s = makeStore();
  hoisted.store = s;
  return { useCommandCenter: s };
});
vi.mock('../settings/useSettings', () => ({ usePersona: () => ({ data: undefined }) }));
vi.mock('../../lib/useVoices', () => ({ useVoicePreview: () => ({ preview: vi.fn(), playingId: null }) }));
vi.mock('../../lib/speakReplies', () => ({
  hasSpokenKey: () => true,
  markReplySpoken: vi.fn(),
  replyDedupeKey: () => 'k',
}));

// react-markdown is the expensive child; count how often it is asked to parse.
const markdownParses: string[] = [];
vi.mock('./MarkdownContent', () => ({
  MarkdownContent: ({ content }: { content: string }) => {
    markdownParses.push(content);
    return null;
  },
}));

import { MessageList, isAtBottom, isSelfScroll } from './MessageList';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;
let frames: FrameRequestCallback[];
let scrollIntoViewCalls: unknown[];

/** Metrics + a write log for the scroll container, which jsdom cannot provide. */
let scroller: { el: HTMLElement; scrollTop: number; scrollHeight: number; clientHeight: number; writes: number[] };

function instrumentScroller() {
  const el = container.querySelector('[data-testid="message-scroller"]') as HTMLElement;
  scroller = { el, scrollTop: 0, scrollHeight: 2000, clientHeight: 500, writes: [] };
  Object.defineProperty(el, 'scrollTop', {
    configurable: true,
    get: () => scroller.scrollTop,
    set: (v: number) => { scroller.scrollTop = v; scroller.writes.push(v); },
  });
  Object.defineProperty(el, 'scrollHeight', { configurable: true, get: () => scroller.scrollHeight });
  Object.defineProperty(el, 'clientHeight', { configurable: true, get: () => scroller.clientHeight });
  return scroller;
}

function flushFrames() {
  const queued = frames;
  frames = [];
  queued.forEach(cb => cb(0));
}

beforeEach(() => {
  markdownParses.length = 0;
  scrollIntoViewCalls = [];
  frames = [];
  scroller = undefined as unknown as typeof scroller;
  vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => { frames.push(cb); return frames.length; });
  vi.stubGlobal('cancelAnimationFrame', () => {});
  Element.prototype.scrollIntoView = function (arg?: unknown) { scrollIntoViewCalls.push(arg); };
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});

const SETTLED: Msg[] = Array.from({ length: 8 }, (_, i) => ({
  id: `m${i}`,
  role: (i % 2 === 0 ? 'user' : 'assistant') as Msg['role'],
  content: `settled message ${i} with \`code\` in it`,
  timestamp: new Date(2026, 0, 1, 0, i).toISOString(),
}));

async function mountWithStream() {
  hoisted.store.setState({
    chatMessages: [...SETTLED, { id: 'live', role: 'assistant', content: 'Hel', timestamp: new Date(2026, 0, 1, 1).toISOString() }],
  });
  await act(async () => { root.render(<MessageList />); });
}

/** One streamed delta: the store replaces exactly one message object, and the
 *  transcript gets taller. */
async function streamDelta(text: string, grewBy = 40) {
  if (scroller) scroller.scrollHeight += grewBy;
  await act(async () => {
    hoisted.store.setState(s => ({
      chatMessages: s.chatMessages.map(m => (m.id === 'live' ? { ...m, content: m.content + text } : m)),
    }));
  });
}

describe('streaming does not re-parse the whole transcript', () => {
  it('a delta re-renders only the message that changed', async () => {
    await mountWithStream();
    const assistantBubbles = SETTLED.filter(m => m.role === 'assistant').length + 1;
    expect(markdownParses).toHaveLength(assistantBubbles); // 5, on mount

    markdownParses.length = 0;
    await act(async () => {
      hoisted.store.setState(s => ({
        chatMessages: s.chatMessages.map(m => (m.id === 'live' ? { ...m, content: m.content + 'lo' } : m)),
      }));
    });

    // Was 5 — one full markdown parse per assistant bubble, per token.
    expect(markdownParses).toEqual(['Hello']);
  });
});

describe('autoscroll pins without animating', () => {
  it('never uses smooth scrollIntoView, and writes scrollTop at most once per frame', async () => {
    await mountWithStream();
    const s = instrumentScroller();
    flushFrames();               // the mount pin
    s.writes.length = 0;

    // Four deltas inside a single frame — the burst that used to restart the
    // smooth animation four times.
    await streamDelta('l');
    await streamDelta('o');
    await streamDelta(' t');
    await streamDelta('here');
    expect(s.writes).toHaveLength(0); // nothing written until the frame runs

    flushFrames();
    // One write, and it lands at the bottom as of the LATEST delta — coalescing
    // must not pin to a height that four deltas ago was current.
    expect(s.writes).toEqual([s.scrollHeight - s.clientHeight]);
    expect(scrollIntoViewCalls).toHaveLength(0);
  });

  it('does not disengage on the scroll event its own pin provokes', async () => {
    await mountWithStream();
    const s = instrumentScroller();
    flushFrames();
    act(() => { s.el.dispatchEvent(new Event('scroll')); });

    s.writes.length = 0;
    await streamDelta('lo');
    flushFrames();
    expect(s.writes).toHaveLength(1); // still following
  });

  it('stops pinning once the reader scrolls up, and does not yank them back', async () => {
    await mountWithStream();
    const s = instrumentScroller();
    flushFrames();
    act(() => { s.el.dispatchEvent(new Event('scroll')); });

    // The reader drags up. Not through the setter — this is not our write.
    s.scrollTop = 200;
    act(() => { s.el.dispatchEvent(new Event('scroll')); });

    s.writes.length = 0;
    await streamDelta('lo');
    await streamDelta('ng reply');
    flushFrames();
    expect(s.writes).toHaveLength(0);
    expect(s.scrollTop).toBe(200);
    expect(container.textContent).toContain('Jump to latest');
  });
});

describe('scroll-position predicates', () => {
  it('treats a reader within the slack of the end as following the stream', () => {
    expect(isAtBottom({ scrollHeight: 2000, scrollTop: 1500, clientHeight: 500 })).toBe(true);
    expect(isAtBottom({ scrollHeight: 2000, scrollTop: 1450, clientHeight: 500 })).toBe(true);
    expect(isAtBottom({ scrollHeight: 2000, scrollTop: 1400, clientHeight: 500 })).toBe(false);
  });

  it('recognises the position it just wrote, and nothing else', () => {
    expect(isSelfScroll(1500, 1500)).toBe(true);
    expect(isSelfScroll(1500, 1499)).toBe(true);  // sub-pixel clamping
    expect(isSelfScroll(1400, 1500)).toBe(false);
    expect(isSelfScroll(1500, null)).toBe(false); // no write outstanding
  });
});
