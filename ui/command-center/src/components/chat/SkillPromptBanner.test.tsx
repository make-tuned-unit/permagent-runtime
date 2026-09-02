/**
 * @vitest-environment jsdom
 *
 * "Save as Skill" is one of only two ways a skill ever gets created. A failure
 * here used to be a `console.error` and a banner that simply stayed put, which
 * is indistinguishable from not having clicked at all.
 */

import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const mocks = vi.hoisted(() => ({
  saveSkillProposal: vi.fn(),
  dismissSkillProposal: vi.fn(),
}));

vi.mock('../../lib/store', () => {
  const state = {
    pendingSkillProposal: {
      description: 'Look up the release notes for a version',
      occurrence_count: 3,
      tool_used: 'shell',
      argument_shape_hash: 'abc',
      source_task_ids: ['t1'],
    },
    saveSkillProposal: mocks.saveSkillProposal,
    dismissSkillProposal: mocks.dismissSkillProposal,
  };
  const useCommandCenter = Object.assign(
    (selector: (value: typeof state) => unknown) => selector(state),
    { getState: () => state },
  );
  return { useCommandCenter };
});

import { SkillPromptBanner } from './SkillPromptBanner';
import { MIN_PENDING_MS } from '../common/Button';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

async function advance(ms: number) {
  await act(async () => {
    vi.advanceTimersByTime(ms);
    for (let i = 0; i < 8; i += 1) await Promise.resolve();
  });
}

function saveButton(): HTMLButtonElement {
  return Array.from(container.querySelectorAll('button'))
    .find((b) => /Save as Skill/i.test(b.textContent ?? ''))!;
}

beforeEach(() => {
  vi.useFakeTimers();
  mocks.saveSkillProposal.mockReset();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

it('says so at the control when the skill could not be saved', async () => {
  mocks.saveSkillProposal.mockResolvedValue(false);
  await act(async () => { root.render(<SkillPromptBanner />); });
  await act(async () => { saveButton().click(); });
  await advance(MIN_PENDING_MS + 50);

  expect(container.textContent).toMatch(/Couldn't save/i);
  expect(container.querySelector('[data-state="success"]')).toBeNull();
});

it('holds a pending phase and ticks when the skill is saved', async () => {
  mocks.saveSkillProposal.mockResolvedValue(true);
  await act(async () => { root.render(<SkillPromptBanner />); });
  await act(async () => { saveButton().click(); });

  expect(container.querySelector('[data-pending="true"]')).toBeTruthy();
  await advance(MIN_PENDING_MS + 50);
  expect(container.querySelector('[data-state="success"]')).toBeTruthy();
  expect(container.textContent).not.toMatch(/Couldn't save/i);
});
