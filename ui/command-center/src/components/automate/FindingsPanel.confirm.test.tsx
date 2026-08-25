/** @vitest-environment jsdom
 *
 * Storage-safety bulk/second-confirm UI — component tests.
 *
 * Covers the two guarantees the incident (a bulk "Clean Up All" click that
 * trashed a live 133 GB cargo target dir alongside 32 genuinely-safe
 * findings) demands from the UI layer: the bulk confirm dialog must show an
 * honest total + per-category breakdown + an excluded list with the
 * consequence of NOT trashing an in_use/managed_by_macos item, and a single
 * Trash click on one of those two categories must never fire the request
 * without a second, explicit confirmation.
 */

import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { BulkConfirmDialog, FindingRow, type Finding } from './AutomateView';

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
          busy={false}
          error={null}
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
          busy={false}
          error={null}
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
          busy={false}
          error={null}
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
});

describe('FindingRow — second confirmation for in_use / managed_by_macos', () => {
  it('clicking Trash on an in_use row does NOT fire the action until the second confirmation is accepted', async () => {
    const onAction = vi.fn();
    const inUseFinding = finding({
      id: 'inuse-1',
      path: '/Users/j/dev/permagent-runtime/target',
      category: 'in_use',
      consequence: '5 rustc processes are compiling here',
    });
    await act(async () => {
      root.render(<FindingRow finding={inUseFinding} loading={false} onAction={onAction} />);
    });

    const trashBtn = [...container.querySelectorAll('button')].find(b => b.textContent === 'Trash');
    expect(trashBtn).toBeTruthy();
    await act(async () => trashBtn!.dispatchEvent(new MouseEvent('click', { bubbles: true })));

    // First click only reveals the inline second-confirm — no request yet.
    expect(onAction).not.toHaveBeenCalled();
    expect(container.textContent).toContain('Are you sure?');
    expect(container.textContent).toContain('5 rustc processes are compiling here');

    const confirmBtn = [...container.querySelectorAll('button')].find(b => b.textContent === 'Trash anyway');
    expect(confirmBtn).toBeTruthy();
    await act(async () => confirmBtn!.dispatchEvent(new MouseEvent('click', { bubbles: true })));

    expect(onAction).toHaveBeenCalledTimes(1);
    expect(onAction).toHaveBeenCalledWith('trash', true);
  });

  it('clicking Trash on a plain safe_to_remove row fires the action immediately with no second confirm', async () => {
    const onAction = vi.fn();
    const safeFinding = finding({ id: 'safe-1', path: '/Users/j/Downloads/old.zip', category: 'safe_to_remove' });
    await act(async () => {
      root.render(<FindingRow finding={safeFinding} loading={false} onAction={onAction} />);
    });
    const trashBtn = [...container.querySelectorAll('button')].find(b => b.textContent === 'Trash');
    await act(async () => trashBtn!.dispatchEvent(new MouseEvent('click', { bubbles: true })));
    expect(onAction).toHaveBeenCalledTimes(1);
    expect(onAction).toHaveBeenCalledWith('trash');
    expect(container.textContent).not.toContain('Are you sure?');
  });

  it('Cancel on the second confirmation dismisses it without ever calling onAction', async () => {
    const onAction = vi.fn();
    const macosFinding = finding({
      id: 'macos-1',
      path: '/Library/Caches/com.apple.something',
      category: 'managed_by_macos',
      consequence: 'macOS maintains this cache',
    });
    await act(async () => {
      root.render(<FindingRow finding={macosFinding} loading={false} onAction={onAction} />);
    });
    const trashBtn = [...container.querySelectorAll('button')].find(b => b.textContent === 'Trash');
    await act(async () => trashBtn!.dispatchEvent(new MouseEvent('click', { bubbles: true })));
    expect(container.textContent).toContain('Are you sure?');

    const cancelBtn = [...container.querySelectorAll('button')].find(b => b.textContent === 'Cancel');
    await act(async () => cancelBtn!.dispatchEvent(new MouseEvent('click', { bubbles: true })));

    expect(onAction).not.toHaveBeenCalled();
    expect(container.textContent).not.toContain('Are you sure?');
  });
});
