/**
 * @vitest-environment jsdom
 *
 * "What Henry learns from" (Settings → Memory, memory-feed lane 2026-09-25).
 * Pins: loading skeleton, error + retry, a source reading "not yet" at zero
 * count, a fed source showing its count and last-fed time, a disabled source
 * showing "Off", an errored source showing its warning, and the source/Brain
 * navigation links actually calling `navigateToTool`.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const hooks = vi.hoisted(() => ({
  getBrainFeedStatus: vi.fn(),
  navigateToTool: vi.fn(),
}));

vi.mock('../../lib/api', () => ({
  api: { getBrainFeedStatus: hooks.getBrainFeedStatus },
}));

vi.mock('../../lib/store', () => ({
  navigateToTool: hooks.navigateToTool,
}));

import { MemoryFeedPanel } from './MemoryFeedPanel';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

function row(source: string, overrides: Record<string, unknown> = {}) {
  return {
    source,
    tag: `permagent.${source}`,
    label: source[0].toUpperCase() + source.slice(1),
    count: 0,
    last_fed_at: null,
    last_error: null,
    last_error_at: null,
    pending: 0,
    dead: 0,
    enabled: true,
    ...overrides,
  };
}

async function flush() {
  await act(async () => { await Promise.resolve(); await Promise.resolve(); });
}

beforeEach(() => {
  hooks.getBrainFeedStatus.mockReset();
  hooks.navigateToTool.mockReset();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => { root.unmount(); });
  container.remove();
});

describe('MemoryFeedPanel', () => {
  it('shows a skeleton before the response lands', () => {
    hooks.getBrainFeedStatus.mockReturnValue(new Promise(() => {}));
    act(() => { root.render(<MemoryFeedPanel />); });
    const panel = container.querySelector('[data-testid="memory-feed-panel"]');
    expect(panel).toBeTruthy();
    expect(container.querySelector('[data-testid^="memory-feed-row-"]')).toBeFalsy();
  });

  it('shows an error with retry when the fetch fails, and retry re-fetches', async () => {
    hooks.getBrainFeedStatus.mockRejectedValueOnce(new Error('network down'));
    act(() => { root.render(<MemoryFeedPanel />); });
    await flush();
    expect(container.textContent).toContain('network down');

    hooks.getBrainFeedStatus.mockResolvedValueOnce({
      sources: [row('goal', { count: 3, last_fed_at: '2026-09-24T00:00:00Z' })],
      chat_outbox: { pending: 0, dead: 0 },
    });
    const retryBtn = Array.from(container.querySelectorAll('button'))
      .find(b => b.textContent === 'Try again');
    expect(retryBtn).toBeTruthy();
    await act(async () => { retryBtn!.click(); await flush(); });
    expect(container.querySelector('[data-testid="memory-feed-row-goal"]')).toBeTruthy();
  });

  it('renders "not yet" for a zero-count source', async () => {
    hooks.getBrainFeedStatus.mockResolvedValueOnce({
      sources: [row('lesson')],
      chat_outbox: { pending: 0, dead: 0 },
    });
    act(() => { root.render(<MemoryFeedPanel />); });
    await flush();
    const r = container.querySelector('[data-testid="memory-feed-row-lesson"]');
    expect(r?.textContent).toContain('Not yet');
  });

  it('renders a fed source\'s count and never claims "not yet" for it', async () => {
    hooks.getBrainFeedStatus.mockResolvedValueOnce({
      sources: [row('card', { count: 12, last_fed_at: '2026-09-24T12:00:00Z' })],
      chat_outbox: { pending: 0, dead: 0 },
    });
    act(() => { root.render(<MemoryFeedPanel />); });
    await flush();
    const r = container.querySelector('[data-testid="memory-feed-row-card"]');
    expect(r?.textContent).toContain('12 facts remembered');
    expect(r?.textContent).not.toContain('Not yet');
  });

  it('flags a disabled source with an "Off" chip', async () => {
    hooks.getBrainFeedStatus.mockResolvedValueOnce({
      sources: [row('finance', { enabled: false })],
      chat_outbox: { pending: 0, dead: 0 },
    });
    act(() => { root.render(<MemoryFeedPanel />); });
    await flush();
    expect(container.querySelector('[data-testid="memory-feed-disabled-finance"]')).toBeTruthy();
  });

  it('flags an errored source with a warning indicator carrying the error text', async () => {
    hooks.getBrainFeedStatus.mockResolvedValueOnce({
      sources: [row('briefing', { last_error: 'brain unavailable' })],
      chat_outbox: { pending: 0, dead: 0 },
    });
    act(() => { root.render(<MemoryFeedPanel />); });
    await flush();
    const errIcon = container.querySelector('[data-testid="memory-feed-error-briefing"]');
    expect(errIcon).toBeTruthy();
  });

  it('folds the chat outbox depth into the chat row instead of reading its own (always-zero) pending/dead', async () => {
    hooks.getBrainFeedStatus.mockResolvedValueOnce({
      sources: [row('chat', { count: 5 })],
      chat_outbox: { pending: 4, dead: 2 },
    });
    act(() => { root.render(<MemoryFeedPanel />); });
    await flush();
    const r = container.querySelector('[data-testid="memory-feed-row-chat"]');
    expect(r?.textContent).toContain('2 failed permanently');
    expect(r?.textContent).toContain('4 queued');
  });

  it('navigates to the Brain view from a row\'s Brain link', async () => {
    hooks.getBrainFeedStatus.mockResolvedValueOnce({
      sources: [row('goal', { count: 1 })],
      chat_outbox: { pending: 0, dead: 0 },
    });
    act(() => { root.render(<MemoryFeedPanel />); });
    await flush();
    const r = container.querySelector('[data-testid="memory-feed-row-goal"]') as HTMLElement;
    const brainBtn = Array.from(r.querySelectorAll('button')).find(b => b.textContent === 'Brain');
    expect(brainBtn).toBeTruthy();
    act(() => { brainBtn!.click(); });
    expect(hooks.navigateToTool).toHaveBeenCalledWith('memory');
  });

  it('navigates to the owning surface from a row\'s Source link when one is mapped', async () => {
    hooks.getBrainFeedStatus.mockResolvedValueOnce({
      sources: [row('finance', { count: 2 })],
      chat_outbox: { pending: 0, dead: 0 },
    });
    act(() => { root.render(<MemoryFeedPanel />); });
    await flush();
    const r = container.querySelector('[data-testid="memory-feed-row-finance"]') as HTMLElement;
    const sourceBtn = Array.from(r.querySelectorAll('button')).find(b => b.textContent === 'Source');
    expect(sourceBtn).toBeTruthy();
    act(() => { sourceBtn!.click(); });
    expect(hooks.navigateToTool).toHaveBeenCalledWith('finance');
  });

  it('omits the Source link for a source with no dedicated surface (never a dead link)', async () => {
    hooks.getBrainFeedStatus.mockResolvedValueOnce({
      sources: [row('lesson', { count: 1 })],
      chat_outbox: { pending: 0, dead: 0 },
    });
    act(() => { root.render(<MemoryFeedPanel />); });
    await flush();
    const r = container.querySelector('[data-testid="memory-feed-row-lesson"]') as HTMLElement;
    const sourceBtn = Array.from(r.querySelectorAll('button')).find(b => b.textContent === 'Source');
    expect(sourceBtn).toBeFalsy();
  });
});
