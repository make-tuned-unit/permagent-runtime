/** @vitest-environment jsdom */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';
import type { GuardFinding, GuardSummary } from '../../lib/api';

const setPendingTerminalLaunch = vi.fn();
const navigateToTool = vi.fn(() => true);
vi.mock('../../lib/store', async () => {
  const actual = await vi.importActual<Record<string, unknown>>('../../lib/store');
  return {
    ...actual,
    useCommandCenter: (sel: (s: Record<string, unknown>) => unknown) => sel({ setPendingTerminalLaunch }),
    navigateToTool: (...a: unknown[]) => (navigateToTool as (...x: unknown[]) => boolean)(...a),
  };
});

const getGuard = vi.fn();
const markGuardFindingSent = vi.fn();
const verifyGuardFinding = vi.fn();
const setGuardFindingStatus = vi.fn();
const requestGuardRescan = vi.fn();
vi.mock('../../lib/api', () => ({
  api: {
    getGuard: (...a: unknown[]) => getGuard(...a),
    markGuardFindingSent: (...a: unknown[]) => markGuardFindingSent(...a),
    verifyGuardFinding: (...a: unknown[]) => verifyGuardFinding(...a),
    setGuardFindingStatus: (...a: unknown[]) => setGuardFindingStatus(...a),
    requestGuardRescan: (...a: unknown[]) => requestGuardRescan(...a),
  },
}));

import { GuardPanel } from './GuardPanel';
import type { Project } from './types';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const project = {
  id: 'p1', slug: 'shop', name: 'Shop', rootPath: '/dev/shop', metadataJson: {},
} as unknown as Project;

const finding = (over: Partial<GuardFinding>): GuardFinding => ({
  id: 'f:1', title: 'Hardcoded key', severity: 'high', found_at: '2026-10-01T00:00:00Z', ...over,
});

const summary = (findings: GuardFinding[], over: Partial<GuardSummary> = {}): GuardSummary => ({
  findings,
  counts: {
    active: findings.filter(f => f.status !== 'fixed' && f.status !== 'dismissed').length,
    open: 0, sent: 0, awaiting_rescan: 0, unconfirmed: 0,
    fixed: findings.filter(f => f.status === 'fixed').length,
    dismissed: findings.filter(f => f.status === 'dismissed').length,
  },
  last_scan: '2026-10-02T00:00:00Z', last_attempt: '2026-10-02T00:00:00Z',
  rescan_requested_at: null, enabled: true, ...over,
});

let container: HTMLDivElement;
let root: Root;

async function mount(g: GuardSummary) {
  getGuard.mockResolvedValue(g);
  await act(async () => { root.render(<GuardPanel project={project} />); });
  await act(async () => { await Promise.resolve(); });
}
const btn = (label: string) =>
  Array.from(container.querySelectorAll('button')).find(b => b.textContent?.includes(label)) as HTMLButtonElement | undefined;
const click = async (el: HTMLElement | undefined) => {
  expect(el).toBeTruthy();
  await act(async () => { el!.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
  await act(async () => { await Promise.resolve(); });
};

beforeEach(() => {
  vi.clearAllMocks();
  navigateToTool.mockReturnValue(true);
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});
afterEach(() => { act(() => root.unmount()); container.remove(); });

describe('GuardPanel empty states', () => {
  it('says the Guard is off', async () => {
    await mount(summary([], { enabled: false, last_scan: null, last_attempt: null }));
    expect(container.textContent).toContain('The Guard is off');
  });
  it('says never scanned', async () => {
    await mount(summary([], { last_scan: null, last_attempt: null }));
    expect(container.textContent).toContain('Never scanned');
  });
  it('says the last sweep did not finish', async () => {
    await mount(summary([], { last_scan: '2026-09-01T00:00:00Z', last_attempt: '2026-10-05T00:00:00Z' }));
    expect(container.textContent).toContain('Last sweep did not finish');
  });
  it('says scanned clean', async () => {
    await mount(summary([]));
    expect(container.textContent).toContain('no open findings');
  });
});

describe('GuardPanel status chips and summary', () => {
  it('shows each plain-language chip and a count line', async () => {
    await mount(summary([
      finding({ id: 'a', status: 'open' }),
      finding({ id: 'b', status: 'sent', sent_agent: 'claude' }),
      finding({ id: 'c', status: 'fix_committed', fix_commit: 'abcdef1234', fix_commit_subject: 'Fix it' }),
      finding({ id: 'd', status: 'not_seen' }),
      finding({ id: 'e', status: 'reopened' }),
    ]));
    const chips = Array.from(container.querySelectorAll('[data-testid="guard-chip"]')).map(c => c.textContent);
    expect(chips).toEqual([
      'Open', 'Sent to Claude', 'Fix committed (waiting for rescan)',
      'Not seen in latest scan (unconfirmed)', 'Reopened',
    ]);
    expect(container.querySelector('[data-testid="guard-summary"]')?.textContent).toContain('5 active');
  });

  it('keeps closed items collapsed with counts, and shows the fix commit when opened', async () => {
    await mount(summary([
      finding({ id: 'a', status: 'open' }),
      finding({ id: 'z', status: 'fixed', fix_commit: 'abcdef1234567', fix_commit_subject: 'Remove key' }),
      finding({ id: 'y', status: 'dismissed', dismissed_reason: 'test fixture' }),
    ]));
    expect(container.querySelector('[data-testid="guard-closed"]')).toBeNull();
    expect(btn('closed (1 fixed, 1 dismissed)')).toBeTruthy();
    await click(btn('closed ('));
    const closed = container.querySelector('[data-testid="guard-closed"]')!;
    expect(closed.textContent).toContain('abcdef1');
    expect(closed.textContent).not.toContain('abcdef12');
    expect(closed.textContent).toContain('Remove key');
    expect(closed.textContent).toContain('Reopen');
  });
});

describe('GuardPanel actions', () => {
  it('Fix with agent queues a Build launch, then marks the finding sent', async () => {
    await mount(summary([finding({ id: 'a/b:1' })]));
    await click(btn('Fix with agent'));
    expect(setPendingTerminalLaunch).toHaveBeenCalledTimes(1);
    const launch = setPendingTerminalLaunch.mock.calls[0][0];
    expect(launch.rootPath).toBe('/dev/shop');
    expect(launch.command).toBe('claude');
    expect(launch.followUpInput).toContain('Guard: a/b:1');
    expect(navigateToTool).toHaveBeenCalledWith('build');
    expect(markGuardFindingSent).toHaveBeenCalledWith('p1', 'a/b:1', 'claude');
    expect(getGuard).toHaveBeenCalledTimes(2);
  });

  it('does not mark sent when Build cannot open', async () => {
    navigateToTool.mockReturnValue(false);
    await mount(summary([finding({ id: 'a' })]));
    await click(btn('Fix with agent'));
    expect(markGuardFindingSent).not.toHaveBeenCalled();
    expect(container.textContent).toContain('Open the Build workspace');
  });

  it('shows a failed verify as checks, not an error', async () => {
    verifyGuardFinding.mockResolvedValue({
      verified: false, finding: finding({}), detail: 'No commit touches src/a.ts yet.', commit: null,
      rescan_requested: false,
      checks: [{ id: 'commit', label: 'A commit since it was sent', passed: false, detail: 'none found' }],
    });
    await mount(summary([finding({ id: 'a' })]));
    await click(btn('Verify fix'));
    const v = container.querySelector('[data-testid="guard-verify"]')!;
    expect(v.textContent).toContain('No fix found yet.');
    expect(v.textContent).toContain('A commit since it was sent');
    expect(v.textContent).toContain('none found');
    expect(getGuard).toHaveBeenCalledTimes(2);
  });

  it('validates the dismiss reason (1-280 characters) before posting', async () => {
    await mount(summary([finding({ id: 'a' })]));
    await click(btn('Dismiss'));
    expect(btn('Confirm dismiss')!.disabled).toBe(true);
    const input = container.querySelector('input[aria-label="Reason for dismissing"]') as HTMLInputElement;
    const setValue = async (v: string) => {
      await act(async () => {
        const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
        setter.call(input, v);
        input.dispatchEvent(new Event('input', { bubbles: true }));
      });
    };
    await setValue('x'.repeat(281));
    expect(btn('Confirm dismiss')!.disabled).toBe(true);
    expect(container.querySelector('[role="alert"]')?.textContent).toContain('280');
    await setValue('Test fixture only');
    expect(btn('Confirm dismiss')!.disabled).toBe(false);
    await click(btn('Confirm dismiss'));
    expect(setGuardFindingStatus).toHaveBeenCalledWith('p1', 'a', { status: 'dismissed', reason: 'Test fixture only' });
  });

  it('reopens a closed item', async () => {
    await mount(summary([finding({ id: 'z', status: 'dismissed', dismissed_reason: 'r' })]));
    await click(btn('closed ('));
    await click(btn('Reopen'));
    expect(setGuardFindingStatus).toHaveBeenCalledWith('p1', 'z', { status: 'open' });
  });
});

describe('GuardPanel rescan', () => {
  it('shows queued', async () => {
    requestGuardRescan.mockResolvedValue({ queued: true, requested_at: 'now', reason: null });
    await mount(summary([]));
    await click(btn('Rescan now'));
    expect(container.querySelector('[data-testid="guard-rescan-note"]')?.textContent).toContain('queued');
  });
  it('shows the server reason when refused', async () => {
    requestGuardRescan.mockResolvedValue({ queued: false, requested_at: null, reason: 'Scanned 10 minutes ago.' });
    await mount(summary([]));
    await click(btn('Rescan now'));
    expect(container.querySelector('[data-testid="guard-rescan-note"]')?.textContent).toContain('Scanned 10 minutes ago.');
  });
  it('disables Rescan now when the Guard is off', async () => {
    await mount(summary([], { enabled: false }));
    expect(btn('Rescan now')!.disabled).toBe(true);
  });
});

describe('compact overview', () => {
  it('shows the full risk count and opens all existing review controls on demand', async () => {
    getGuard.mockResolvedValue(summary(Array.from({length: 10}, (_, i) => finding({id: `f${i}`, title: `Finding ${i}`}))));
    await act(async () => root.render(<GuardPanel project={project} compact />));
    expect(container.textContent).toContain('10 active findings · 10 high or critical');
    expect(container.textContent).not.toContain('Finding 2');
    expect(container.querySelector('select')).toBeNull();
    await click(btn('Review security'));
    const dialog = document.querySelector('[role="dialog"]')!;
    expect(dialog).toBeTruthy();
    expect(dialog.querySelectorAll('[data-testid="guard-finding"]')).toHaveLength(8);
    expect(dialog.textContent).toContain('Rescan now');
    expect(dialog.querySelector('select[aria-label="Coding agent"]')).toBeTruthy();
    expect(markGuardFindingSent).not.toHaveBeenCalled();
    expect(requestGuardRescan).not.toHaveBeenCalled();
  });
});
