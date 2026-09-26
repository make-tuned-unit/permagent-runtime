/**
 * @vitest-environment jsdom
 *
 * "What Henry learns from" (Settings → Memory, memory-feed lane 2026-09-25;
 * layout pass 2026-09-25 after a coordinator review at 1440px; toggle pass
 * 2026-09-25 after a second review found the "Off" chip's Settings link led
 * to a page with no actual control — R5 says the control belongs in the
 * panel itself).
 *
 * Pins: loading skeleton, error + retry, a source reading "not yet" at zero
 * count, a never-fed source reading "Never" (not "date unknown"), a fed
 * source showing its count and last-fed time, a failing source showing its
 * error, the summary line's counts and failing-filter toggle, the
 * source/Brain navigation links, and — this pass — the per-row Toggle:
 * flipping it writes the disabled-sources config key, a failed write rolls
 * back and toasts, and the one locked-on source (chat) cannot be flipped.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const hooks = vi.hoisted(() => ({
  getBrainFeedStatus: vi.fn(),
  upsertConfig: vi.fn(),
  navigateToTool: vi.fn(),
  toast: vi.fn(),
}));

vi.mock('../../lib/api', () => ({
  api: { getBrainFeedStatus: hooks.getBrainFeedStatus, upsertConfig: hooks.upsertConfig },
}));

vi.mock('../../lib/store', () => ({
  navigateToTool: hooks.navigateToTool,
  // The panel's copy names the agent from the persona (the store's agentName),
  // never a hardcoded default — a neutral configured name proves the plumbing.
  useCommandCenter: (select: (s: { agentName: string }) => unknown) => select({ agentName: 'Nova' }),
}));

vi.mock('../../lib/notifications', () => ({
  toast: hooks.toast,
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
  await act(async () => { await Promise.resolve(); await Promise.resolve(); await Promise.resolve(); });
}

function toggleIn(row: Element): HTMLButtonElement {
  return row.querySelector('[role="switch"]') as HTMLButtonElement;
}

beforeEach(() => {
  hooks.getBrainFeedStatus.mockReset();
  hooks.upsertConfig.mockReset();
  hooks.navigateToTool.mockReset();
  hooks.toast.mockReset();
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

  it('reads "Never" (not "date unknown") for a source with no last_fed_at', async () => {
    hooks.getBrainFeedStatus.mockResolvedValueOnce({
      sources: [row('lesson')],
      chat_outbox: { pending: 0, dead: 0 },
    });
    act(() => { root.render(<MemoryFeedPanel />); });
    await flush();
    const r = container.querySelector('[data-testid="memory-feed-row-lesson"]');
    expect(r?.textContent).toContain('Never');
    expect(r?.textContent).not.toContain('date unknown');
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

  it('renders a disabled source\'s toggle in the off position', async () => {
    hooks.getBrainFeedStatus.mockResolvedValueOnce({
      sources: [row('finance', { enabled: false })],
      chat_outbox: { pending: 0, dead: 0 },
    });
    act(() => { root.render(<MemoryFeedPanel />); });
    await flush();
    const r = container.querySelector('[data-testid="memory-feed-row-finance"]') as HTMLElement;
    expect(toggleIn(r).getAttribute('aria-checked')).toBe('false');
  });

  it('flipping a source\'s toggle on writes the disabled-sources config without it, and commits the new state', async () => {
    hooks.upsertConfig.mockResolvedValueOnce(undefined);
    hooks.getBrainFeedStatus.mockResolvedValueOnce({
      sources: [row('finance', { enabled: false }), row('meeting', { enabled: false })],
      chat_outbox: { pending: 0, dead: 0 },
    });
    act(() => { root.render(<MemoryFeedPanel />); });
    await flush();
    const r = container.querySelector('[data-testid="memory-feed-row-finance"]') as HTMLElement;
    await act(async () => { toggleIn(r).click(); await flush(); });

    expect(hooks.upsertConfig).toHaveBeenCalledWith('BRAIN_FEED_DISABLED_SOURCES', 'meeting', false);
    expect(toggleIn(r).getAttribute('aria-checked')).toBe('true');
  });

  it('flipping a source off writes it into the disabled-sources list', async () => {
    hooks.upsertConfig.mockResolvedValueOnce(undefined);
    hooks.getBrainFeedStatus.mockResolvedValueOnce({
      sources: [row('finance', { enabled: true })],
      chat_outbox: { pending: 0, dead: 0 },
    });
    act(() => { root.render(<MemoryFeedPanel />); });
    await flush();
    const r = container.querySelector('[data-testid="memory-feed-row-finance"]') as HTMLElement;
    await act(async () => { toggleIn(r).click(); await flush(); });
    expect(hooks.upsertConfig).toHaveBeenCalledWith('BRAIN_FEED_DISABLED_SOURCES', 'finance', false);
    expect(toggleIn(r).getAttribute('aria-checked')).toBe('false');
  });

  it('serializes two fast toggles so the second write does not clobber the first (lost-update regression)', async () => {
    // Two enabled sources. Disable both, back to back, before EITHER
    // network round trip resolves — the exact race that used to compute
    // the second toggle's disabled-set from a stale view that still
    // thought the first source was enabled.
    let resolveFirst: () => void = () => {};
    let resolveSecond: () => void = () => {};
    const firstWrite = new Promise<void>(resolve => { resolveFirst = resolve; });
    const secondWrite = new Promise<void>(resolve => { resolveSecond = resolve; });
    hooks.upsertConfig.mockImplementationOnce(() => firstWrite);
    hooks.upsertConfig.mockImplementationOnce(() => secondWrite);

    hooks.getBrainFeedStatus.mockResolvedValueOnce({
      sources: [row('finance', { enabled: true }), row('meeting', { enabled: true })],
      chat_outbox: { pending: 0, dead: 0 },
    });
    act(() => { root.render(<MemoryFeedPanel />); });
    await flush();

    const financeRow = container.querySelector('[data-testid="memory-feed-row-finance"]') as HTMLElement;
    const meetingRow = container.querySelector('[data-testid="memory-feed-row-meeting"]') as HTMLElement;

    // Click finance off, then meeting off, with no await in between — both
    // fire before either request has had a chance to settle.
    act(() => { toggleIn(financeRow).click(); });
    act(() => { toggleIn(meetingRow).click(); });
    await flush();

    // The second write must not have gone out yet: it is queued behind the
    // first, not fired from a stale snapshot of the (still "enabled")
    // finance row.
    expect(hooks.upsertConfig).toHaveBeenCalledTimes(1);
    expect(hooks.upsertConfig).toHaveBeenNthCalledWith(1, 'BRAIN_FEED_DISABLED_SOURCES', 'finance', false);

    await act(async () => { resolveFirst(); await flush(); });

    // Once the first write lands, the queued second write fires — and it
    // must carry BOTH disabled sources, not just 'meeting'.
    expect(hooks.upsertConfig).toHaveBeenCalledTimes(2);
    expect(hooks.upsertConfig).toHaveBeenNthCalledWith(2, 'BRAIN_FEED_DISABLED_SOURCES', 'finance,meeting', false);

    await act(async () => { resolveSecond(); await flush(); });

    expect(toggleIn(financeRow).getAttribute('aria-checked')).toBe('false');
    expect(toggleIn(meetingRow).getAttribute('aria-checked')).toBe('false');
  });

  it('recomputes the queued second write from the rolled-back state when the first write fails (lost-update regression)', async () => {
    // Same fast double-toggle as above, but this time the FIRST write fails.
    // The second write is already queued behind it when that happens; it
    // must not go out carrying the stale, pre-rollback disabled-set that was
    // true at the moment it was queued — it has to see finance back on.
    let rejectFirst: (e: Error) => void = () => {};
    let resolveSecond: () => void = () => {};
    const firstWrite = new Promise<void>((_resolve, reject) => { rejectFirst = reject; });
    const secondWrite = new Promise<void>(resolve => { resolveSecond = resolve; });
    hooks.upsertConfig.mockImplementationOnce(() => firstWrite);
    hooks.upsertConfig.mockImplementationOnce(() => secondWrite);

    hooks.getBrainFeedStatus.mockResolvedValueOnce({
      sources: [row('finance', { enabled: true }), row('meeting', { enabled: true })],
      chat_outbox: { pending: 0, dead: 0 },
    });
    act(() => { root.render(<MemoryFeedPanel />); });
    await flush();

    const financeRow = container.querySelector('[data-testid="memory-feed-row-finance"]') as HTMLElement;
    const meetingRow = container.querySelector('[data-testid="memory-feed-row-meeting"]') as HTMLElement;

    // Click finance off, then meeting off, before either request settles.
    act(() => { toggleIn(financeRow).click(); });
    act(() => { toggleIn(meetingRow).click(); });
    await flush();

    expect(hooks.upsertConfig).toHaveBeenCalledTimes(1);
    expect(hooks.upsertConfig).toHaveBeenNthCalledWith(1, 'BRAIN_FEED_DISABLED_SOURCES', 'finance', false);

    // The first write (finance) fails: it rolls back and toasts.
    await act(async () => { rejectFirst(new Error('daemon unreachable')); await flush(); });

    expect(toggleIn(financeRow).getAttribute('aria-checked')).toBe('true');
    expect(hooks.toast).toHaveBeenCalledTimes(1);

    // The queued second write (meeting) must now fire — and it must carry
    // ONLY 'meeting', recomputed after finance's rollback, never the stale
    // 'finance,meeting' snapshot taken back when it was queued.
    expect(hooks.upsertConfig).toHaveBeenCalledTimes(2);
    expect(hooks.upsertConfig).toHaveBeenNthCalledWith(2, 'BRAIN_FEED_DISABLED_SOURCES', 'meeting', false);

    await act(async () => { resolveSecond(); await flush(); });

    expect(toggleIn(financeRow).getAttribute('aria-checked')).toBe('true');
    expect(toggleIn(meetingRow).getAttribute('aria-checked')).toBe('false');
  });

  it('rolls back and toasts when the config write fails', async () => {
    hooks.upsertConfig.mockRejectedValueOnce(new Error('daemon unreachable'));
    hooks.getBrainFeedStatus.mockResolvedValueOnce({
      sources: [row('finance', { enabled: true })],
      chat_outbox: { pending: 0, dead: 0 },
    });
    act(() => { root.render(<MemoryFeedPanel />); });
    await flush();
    const r = container.querySelector('[data-testid="memory-feed-row-finance"]') as HTMLElement;
    await act(async () => { toggleIn(r).click(); await flush(); });

    expect(toggleIn(r).getAttribute('aria-checked')).toBe('true');
    expect(hooks.toast).toHaveBeenCalledTimes(1);
    expect(hooks.toast.mock.calls[0][0]).toContain("Couldn't");
  });

  it('locks the toggle on for a source that must never be disabled (chat)', async () => {
    hooks.getBrainFeedStatus.mockResolvedValueOnce({
      sources: [row('chat', { count: 5 })],
      chat_outbox: { pending: 0, dead: 0 },
    });
    act(() => { root.render(<MemoryFeedPanel />); });
    await flush();
    const r = container.querySelector('[data-testid="memory-feed-row-chat"]') as HTMLElement;
    expect(toggleIn(r).disabled).toBe(true);
  });

  it('flags a failing source with its error and a "failed" count when dead > 0', async () => {
    hooks.getBrainFeedStatus.mockResolvedValueOnce({
      sources: [row('briefing', { last_error: 'brain unavailable', dead: 3 })],
      chat_outbox: { pending: 0, dead: 0 },
    });
    act(() => { root.render(<MemoryFeedPanel />); });
    await flush();
    const chip = container.querySelector('[data-testid="memory-feed-error-briefing"]');
    expect(chip).toBeTruthy();
    expect(chip?.textContent).toContain('3 failed');
    // Static chips with an explanation get a tabIndex so the tooltip (the
    // error text) is keyboard-reachable too — see Chip's own contract.
    expect(chip?.getAttribute('tabindex')).toBe('0');
  });

  it('folds the chat outbox depth into the chat row instead of reading its own (always-zero) pending/dead', async () => {
    hooks.getBrainFeedStatus.mockResolvedValueOnce({
      sources: [row('chat', { count: 5 })],
      chat_outbox: { pending: 4, dead: 2 },
    });
    act(() => { root.render(<MemoryFeedPanel />); });
    await flush();
    const r = container.querySelector('[data-testid="memory-feed-row-chat"]');
    expect(r?.textContent).toContain('2 failed');
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

  it('summarizes source count, total facts, failing count and off count', async () => {
    hooks.getBrainFeedStatus.mockResolvedValueOnce({
      sources: [
        row('goal', { count: 100 }),
        row('card', { count: 50, last_error: 'boom' }),
        row('finance', { count: 0, enabled: false }),
      ],
      chat_outbox: { pending: 0, dead: 0 },
    });
    act(() => { root.render(<MemoryFeedPanel />); });
    await flush();
    expect(container.textContent).toContain('3 sources');
    expect(container.textContent).toContain('150 facts remembered');
    expect(container.textContent).toContain('1 failing');
    expect(container.textContent).toContain('1 turned off');
  });

  it('the failing-count chip filters the list down to only failing rows, and toggles back', async () => {
    hooks.getBrainFeedStatus.mockResolvedValueOnce({
      sources: [
        row('goal', { count: 100 }),
        row('card', { count: 50, last_error: 'boom' }),
      ],
      chat_outbox: { pending: 0, dead: 0 },
    });
    act(() => { root.render(<MemoryFeedPanel />); });
    await flush();
    expect(container.querySelector('[data-testid="memory-feed-row-goal"]')).toBeTruthy();
    expect(container.querySelector('[data-testid="memory-feed-row-card"]')).toBeTruthy();

    const filterChip = container.querySelector('[data-testid="memory-feed-failing-filter"]') as HTMLElement;
    expect(filterChip).toBeTruthy();
    act(() => { filterChip.click(); });
    expect(container.querySelector('[data-testid="memory-feed-row-goal"]')).toBeFalsy();
    expect(container.querySelector('[data-testid="memory-feed-row-card"]')).toBeTruthy();

    act(() => { filterChip.click(); });
    expect(container.querySelector('[data-testid="memory-feed-row-goal"]')).toBeTruthy();
  });

  it('does not show a failing-count chip when nothing is failing', async () => {
    hooks.getBrainFeedStatus.mockResolvedValueOnce({
      sources: [row('goal', { count: 1 })],
      chat_outbox: { pending: 0, dead: 0 },
    });
    act(() => { root.render(<MemoryFeedPanel />); });
    await flush();
    expect(container.querySelector('[data-testid="memory-feed-failing-filter"]')).toBeFalsy();
  });
});
