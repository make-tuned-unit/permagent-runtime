/** @vitest-environment jsdom
 *
 * Wiring test for Privacy → Backups (ST4, dagK settings pass):
 * `/api/backups` + `/api/backups/run` had no UI. Covers loading/empty/error/
 * populated states and that "Back up now" is confirmed before it runs.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react';

const allowed = vi.hoisted(() => ({
  getBackups: vi.fn(),
  runBackups: vi.fn(),
}));

vi.mock('../../lib/api', () => ({
  api: new Proxy(allowed, {
    get(target, key) {
      if (typeof key !== 'string') return undefined;
      if (key in target) return target[key as keyof typeof target];
      throw new Error(`BackupsSection touched api.${key}, which is not part of its surface`);
    },
  }),
  apiFetch: vi.fn(async (endpoint: string) => {
    throw new Error(`unexpected fetch ${endpoint}`);
  }),
}));

import { BackupsSection } from './BackupsSection';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => { root.unmount(); });
  container.remove();
  vi.clearAllMocks();
});

async function mount() {
  await act(async () => { root.render(<BackupsSection />); });
}

const SNAP = {
  db: 'brain', filename: 'memory-20260925.db', timestamp: '2026-09-25T09:00:00Z',
  tier: 'Daily' as const, size_bytes: 5_000_000, integrity_ok: true,
};

describe('BackupsSection', () => {
  it('shows a loading state before snapshots resolve', async () => {
    allowed.getBackups.mockReturnValue(new Promise(() => {}));
    await mount();
    expect(container.textContent).toContain('Loading snapshots');
  });

  it('shows an honest error state with retry, not empty', async () => {
    allowed.getBackups.mockRejectedValue(new Error('down'));
    await mount();
    expect(container.textContent).toContain("Couldn't read backup snapshots");
  });

  it('shows the empty state when there are no snapshots yet', async () => {
    allowed.getBackups.mockResolvedValue({ snapshots: [] });
    await mount();
    expect(container.textContent).toContain('No snapshots yet');
  });

  it('lists the newest snapshot per database', async () => {
    allowed.getBackups.mockResolvedValue({
      snapshots: [
        SNAP,
        { ...SNAP, filename: 'memory-old.db', timestamp: '2026-09-01T00:00:00Z' },
      ],
    });
    await mount();
    expect(container.querySelector('[data-testid="backup-row-brain"]')).toBeTruthy();
    // Only one row for `brain` even though two snapshots exist for it.
    expect(container.querySelectorAll('[data-testid="backup-row-brain"]')).toHaveLength(1);
  });

  it('confirms before running an immediate backup, then calls the endpoint', async () => {
    allowed.getBackups.mockResolvedValue({ snapshots: [] });
    allowed.runBackups.mockResolvedValue({
      brain: { status: 'ok', db: 'brain', filename: 'x', timestamp: 't', tier: 'Daily', size_bytes: 100, integrity_ok: true },
      brain_graph: { status: 'skipped', reason: 'does not exist' },
      brain_recognition: { status: 'skipped', reason: 'does not exist' },
      spectral: { status: 'error', message: 'locked' },
    });
    await mount();
    const runBtn = Array.from(container.querySelectorAll('button')).find(b => b.textContent === 'Back up now') as HTMLButtonElement;
    await act(async () => { runBtn.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    expect(allowed.runBackups).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain('Back up all databases now?');
    const confirmButtons = Array.from(document.querySelectorAll('button')).filter(b => b.textContent === 'Back up now');
    await act(async () => { confirmButtons[confirmButtons.length - 1].dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    expect(allowed.runBackups).toHaveBeenCalledOnce();
    expect(container.textContent).toContain('failed — locked');
  });
});
