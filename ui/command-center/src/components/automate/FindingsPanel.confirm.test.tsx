/** @vitest-environment jsdom
 * ST1: current eligibility controls cleanup; busy and managed rows offer no
 * override. Existing assertions were updated because a second confirmation
 * cannot make a live build safe to remove.
 */

import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { BulkConfirmDialog, FindingRow, type Finding, type BulkSweepSummary } from './AutomateView';
import { idleJob } from '../../hooks/useLongRunningJob';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function finding(overrides: Partial<Finding> & Pick<Finding, 'id' | 'path'>): Finding {
  return {
    type: 'dev_cache',
    size_bytes: 1000,
    age_days: null,
    recommendation: 'Safe to remove',
    action_taken: null,
    actioned_at: null,
    size_recovered_bytes: null,
    error_message: null,
    category: null,
    consequence: null,
    action_source: null,
    ...overrides,
  };
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe('BulkConfirmDialog', () => {
  const pending: Finding[] = [
    finding({ id: 'safe-1', path: '/Users/j/Downloads/old.zip', category: 'safe_to_remove', size_bytes: 2_000_000 }),
    finding({ id: 'safe-2', path: '/Users/j/Downloads/old2.zip', category: 'safe_to_remove', size_bytes: 3_000_000 }),
    finding({
      id: 'inuse-1',
      path: '/Users/j/dev/permagent-runtime/target',
      category: 'in_use',
      size_bytes: 133_000_000_000,
      consequence: '5 rustc processes are compiling here',
    }),
  ];

  it('renders the total size to be trashed and the per-category counts', async () => {
    await act(async () => {
      root.render(
        <BulkConfirmDialog
          pending={pending}
          includeRegenerable={false}
          onToggleRegenerable={() => {}}
          onCancel={() => {}}
          onConfirm={() => {}}
          sweep={idleJob<BulkSweepSummary>()}
        />,
      );
    });
    // 2,000,000 + 3,000,000 bytes = ~4.8 MB total for the eligible (safe_to_remove) set.
    expect(container.textContent).toContain('2 items');
    expect(container.textContent).toContain('4.8 MB');
    // Per-category breakdown line for Safe to remove.
    expect(container.textContent).toContain('Safe to remove');
  });

  it('lists the excluded in_use item with its consequence text, and never counts it toward the trashed total', async () => {
    await act(async () => {
      root.render(
        <BulkConfirmDialog
          pending={pending}
          includeRegenerable={false}
          onToggleRegenerable={() => {}}
          onCancel={() => {}}
          onConfirm={() => {}}
          sweep={idleJob<BulkSweepSummary>()}
        />,
      );
    });
    expect(container.textContent).toContain('Excluded — will not be removed');
    expect(container.textContent).toContain('/Users/j/dev/permagent-runtime/target');
    expect(container.textContent).toContain('5 rustc processes are compiling here');
    // The 133 GB in_use finding must never be folded into the "will be trashed" total.
    expect(container.textContent).not.toContain('133');
  });

  it('confirming calls onConfirm with only the eligible (safe_to_remove) findings, excluding in_use', async () => {
    const onConfirm = vi.fn();
    await act(async () => {
      root.render(
        <BulkConfirmDialog
          pending={pending}
          includeRegenerable={false}
          onToggleRegenerable={() => {}}
          onCancel={() => {}}
          onConfirm={onConfirm}
          sweep={idleJob<BulkSweepSummary>()}
        />,
      );
    });
    const confirmBtn = [...container.querySelectorAll('button')].find(b => b.textContent?.startsWith('Move'));
    expect(confirmBtn).toBeTruthy();
    await act(async () => confirmBtn!.dispatchEvent(new MouseEvent('click', { bubbles: true })));
    expect(onConfirm).toHaveBeenCalledTimes(1);
    const eligible = onConfirm.mock.calls[0][0] as Finding[];
    expect(eligible.map(f => f.id).sort()).toEqual(['safe-1', 'safe-2']);
  });

  it('a sweep the server refuses stays on screen, in the server\'s own words', async () => {
    const sweep = {
      ...idleJob<BulkSweepSummary>(),
      phase: 'failed' as const,
      error: 'Some findings cannot be bulk-removed. (1 item blocked)',
    };
    await act(async () => {
      root.render(
        <BulkConfirmDialog
          pending={pending}
          includeRegenerable={false}
          onToggleRegenerable={() => {}}
          onCancel={() => {}}
          onConfirm={() => {}}
          sweep={sweep}
        />,
      );
    });
    // The dialog is still up, the reason is the server's, and the Move button
    // is still there to try again with.
    expect(container.textContent).toContain('Some findings cannot be bulk-removed. (1 item blocked)');
    expect(container.textContent).toContain('Excluded — will not be removed');
    expect([...container.querySelectorAll('button')].some(b => b.textContent?.startsWith('Move'))).toBe(true);
  });

  it('while the sweep runs the Move button is held and the job strip is the one indicator', async () => {
    const sweep = {
      ...idleJob<BulkSweepSummary>(),
      phase: 'running' as const,
      running: true,
      reading: { stage: 'trashing', status: 'Moving 2 items to Trash' },
    };
    await act(async () => {
      root.render(
        <BulkConfirmDialog
          pending={pending}
          includeRegenerable={false}
          onToggleRegenerable={() => {}}
          onCancel={() => {}}
          onConfirm={() => {}}
          sweep={sweep}
        />,
      );
    });
    const move = [...container.querySelectorAll('button')].find(b => b.textContent?.includes('Moving to Trash…'));
    expect(move).toBeTruthy();
    expect((move as HTMLButtonElement).disabled).toBe(true);
    expect(container.textContent).toContain('Moving 2 items to Trash');
  });
});

describe('acceptance 1: FindingRow has no busy or managed override', () => {
  it('offers Recheck availability for a busy finding without issuing Trash', async () => {
    const onAction = vi.fn();
    const onRecheck = vi.fn();
    await act(async () => root.render(<FindingRow finding={finding({ id: 'busy', path: '/repo/target', category: 'in_use' })}
      loading={false} onAction={onAction} onRecheck={onRecheck} />));
    const button = [...container.querySelectorAll('button')].find(b => b.textContent === 'Recheck availability');
    await act(async () => button!.click());
    expect(onRecheck).toHaveBeenCalledOnce();
    expect(onAction).not.toHaveBeenCalled();
    expect(container.textContent).not.toContain('Trash anyway');
  });
  it('keeps the per-item Trash flow for an eligible finding', async () => {
    const onAction = vi.fn();
    await act(async () => root.render(<FindingRow finding={finding({ id: 'safe', path: '/Downloads/old.zip', category: 'safe_to_remove' })}
      loading={false} onAction={onAction} />));
    const button = [...container.querySelectorAll('button')].find(b => b.textContent === 'Trash');
    await act(async () => button!.click());
    expect(onAction).toHaveBeenCalledWith('trash');
  });
  it('never offers a managed-path override', async () => {
    const onAction = vi.fn();
    await act(async () => root.render(<FindingRow finding={finding({ id: 'managed', path: '/Library/Caches/com.apple.cache', category: 'managed_by_macos' })}
      loading={false} onAction={onAction} />));
    const button = [...container.querySelectorAll('button')].find(b => b.textContent === 'Managed by macOS');
    expect(button?.disabled).toBe(true);
    expect(container.textContent).not.toContain('Trash anyway');
  });
});
