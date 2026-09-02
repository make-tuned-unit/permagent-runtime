/**
 * @vitest-environment jsdom
 *
 * Delete used to `console.error` and close nothing: the confirm block stayed
 * open, the skill stayed in the list, and nothing on screen said why.
 */

import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const mocks = vi.hoisted(() => ({
  deleteSkill: vi.fn(),
  setSelectedSkillId: vi.fn(),
}));

vi.mock('../../lib/store', () => {
  const state = {
    deleteSkill: mocks.deleteSkill,
    setSelectedSkillId: mocks.setSelectedSkillId,
  };
  const useCommandCenter = Object.assign(
    (selector: (value: typeof state) => unknown) => selector(state),
    { getState: () => state },
  );
  return { useCommandCenter };
});
vi.mock('./SkillEditor', () => ({ SkillEditor: () => null }));
vi.mock('./SkillExecutionHistory', () => ({ SkillExecutionHistory: () => null }));

import { SkillDetailPanel } from './SkillDetailPanel';
import { MIN_PENDING_MS } from '../common/Button';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const skill = { id: 's1', name: 'release-notes', status: 'active' } as never;

let container: HTMLDivElement;
let root: Root;

async function advance(ms: number) {
  await act(async () => {
    vi.advanceTimersByTime(ms);
    for (let i = 0; i < 8; i += 1) await Promise.resolve();
  });
}

function buttonNamed(re: RegExp): HTMLButtonElement {
  return Array.from(container.querySelectorAll('button'))
    .find((b) => re.test(b.textContent ?? ''))!;
}

beforeEach(() => {
  vi.useFakeTimers();
  mocks.deleteSkill.mockReset();
  mocks.setSelectedSkillId.mockReset();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

it('keeps the skill and says what happened when the delete fails', async () => {
  mocks.deleteSkill.mockResolvedValue(false);
  await act(async () => { root.render(<SkillDetailPanel skill={skill} />); });
  await act(async () => { container.querySelector('[title="Delete skill"]')!.dispatchEvent(new MouseEvent('click', { bubbles: true })); });

  await act(async () => { buttonNamed(/^Delete$/).click(); });
  await advance(MIN_PENDING_MS + 50);

  expect(container.textContent).toMatch(/Couldn't delete/i);
  expect(mocks.setSelectedSkillId).not.toHaveBeenCalled();
  expect(container.querySelector('[data-state="success"]')).toBeNull();
});

it('closes the panel when the delete lands', async () => {
  mocks.deleteSkill.mockResolvedValue(true);
  await act(async () => { root.render(<SkillDetailPanel skill={skill} />); });
  await act(async () => { container.querySelector('[title="Delete skill"]')!.dispatchEvent(new MouseEvent('click', { bubbles: true })); });

  await act(async () => { buttonNamed(/^Delete$/).click(); });
  await advance(MIN_PENDING_MS + 50);

  expect(mocks.setSelectedSkillId).toHaveBeenCalledWith(null);
  expect(container.textContent).not.toMatch(/Couldn't delete/i);
});
