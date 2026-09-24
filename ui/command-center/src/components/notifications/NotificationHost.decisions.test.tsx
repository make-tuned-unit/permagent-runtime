/**
 * @vitest-environment jsdom
 *
 * QA 2026-09-24: the bell's tray said "Nothing needs you — all quiet." while
 * the Decision Inbox held 23 pending decisions. The tray only knew about live
 * /events pings since launch; its empty state spoke for a queue it never read.
 * These pin that the tray now reads the Decision Inbox's own count and only
 * claims quiet when that count positively says so.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const inboxState = vi.hoisted(() => ({
  value: { data: null as unknown, loading: true, error: false },
}));
vi.mock('../dashboard/decisions/useDecisions', () => ({
  useDecisions: () => inboxState.value,
}));

import { NotificationHost } from './NotificationHost';
import { setTrayOpen, useTrayOpen } from '../../lib/notifications';
import { useCommandCenter } from '../../lib/store';
import { trayDecisionState, trayIsQuiet } from './trayAttention';
import { summarizeDecisions } from '../dashboard/decisions/summary';
import type { DecisionsResponse } from '../dashboard/decisions/types';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function payload(pending: number, attention = 0): DecisionsResponse {
  return {
    decisions: [],
    total_pending: pending,
    handled_count: 0,
    goals_needing_attention: attention,
    goals_in_flight: 2,
    oldest_pending_at: pending > 0 ? new Date(Date.now() - 3 * 86_400_000).toISOString() : null,
  } as unknown as DecisionsResponse;
}

let container: HTMLDivElement;
let root: Root;
const realOpen = useCommandCenter.getState().openDecisionInbox;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  useCommandCenter.setState({ agentName: 'Nova' });
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
  setTrayOpen(false);
  useCommandCenter.setState({ overlayBlockingBrowser: 0, openDecisionInbox: realOpen });
});

async function renderOpenTray() {
  setTrayOpen(true);
  await act(async () => { root.render(<NotificationHost />); });
}

describe('tray decision count (pure)', () => {
  it('23 pending is not quiet, and says so in Home\'s own sentence', () => {
    const s = trayDecisionState(summarizeDecisions(payload(23), 'Nova'), false);
    expect(s.kind).toBe('waiting');
    expect(s.kind === 'waiting' && s.headline).toBe('Nova needs 23 answers');
    expect(trayIsQuiet(0, s)).toBe(false);
  });
  it('loading and a failed read are never "all quiet"', () => {
    expect(trayIsQuiet(0, trayDecisionState(summarizeDecisions(null, 'Nova'), false))).toBe(false);
    expect(trayIsQuiet(0, trayDecisionState(summarizeDecisions(null, 'Nova'), true))).toBe(false);
  });
  it('a parked goal alone still counts as waiting', () => {
    const s = trayDecisionState(summarizeDecisions(payload(0, 1), 'Nova'), false);
    expect(s.kind).toBe('waiting');
  });
  it('quiet only when the queue is clear and there are no tray items', () => {
    const clear = trayDecisionState(summarizeDecisions(payload(0), 'Nova'), false);
    expect(trayIsQuiet(0, clear)).toBe(true);
    expect(trayIsQuiet(2, clear)).toBe(false);
  });
});

describe('NotificationHost tray — Decision Inbox row', () => {
  it('does not claim "all quiet" while decisions are pending', async () => {
    inboxState.value = { data: payload(23), loading: false, error: false };
    await renderOpenTray();
    expect(container.textContent).not.toContain('all quiet');
    expect(container.textContent).toContain('Nova needs 23 answers');
  });

  it('opens Home\'s Decision Review through the shared entry point and closes the tray', async () => {
    inboxState.value = { data: payload(23), loading: false, error: false };
    const open = vi.fn(() => true);
    useCommandCenter.setState({ openDecisionInbox: open });
    let trayOpen = true;
    function Probe() { trayOpen = useTrayOpen(); return null; }
    setTrayOpen(true);
    await act(async () => { root.render(<><NotificationHost /><Probe /></>); });
    const btn = Array.from(container.querySelectorAll('button')).find(b => b.textContent?.includes('Open Decisions'))!;
    await act(async () => { btn.click(); });
    expect(open).toHaveBeenCalledTimes(1);
    expect(trayOpen).toBe(false);
  });

  it('says so when Home is unreachable instead of looking like it worked', async () => {
    inboxState.value = { data: payload(3), loading: false, error: false };
    useCommandCenter.setState({ openDecisionInbox: () => false });
    await renderOpenTray();
    const btn = Array.from(container.querySelectorAll('button')).find(b => b.textContent?.includes('Open Decisions'))!;
    await act(async () => { btn.click(); });
    expect(container.textContent).toContain('no workspace holds Home');
  });

  it('still says "all quiet" when the queue really is empty', async () => {
    inboxState.value = { data: payload(0), loading: false, error: false };
    await renderOpenTray();
    expect(container.textContent).toContain('Nothing needs you — all quiet.');
  });
});
