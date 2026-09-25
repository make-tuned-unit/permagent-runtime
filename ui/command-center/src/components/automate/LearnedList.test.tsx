/** @vitest-environment jsdom
 *
 * A4: the Learned section used to be one card per raw call-signature skill
 * ("copy_to_clipboard:-te…", 37 of them) with a MASTERED/TRUSTED/LEARNED tier
 * read off a repetition count. It is now one row per tool, humanized, with
 * a pattern list underneath — see skillHumanize.ts for the parsing rule this
 * pins the wiring of.
 */
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

import { LearnedList } from './LearnedList';
import type { SkillProposal, SkillState } from '../../lib/store';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;
beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});
afterEach(() => { act(() => root.unmount()); container.remove(); });

const skill = (over: Partial<SkillState>): SkillState => ({
  id: 'x', name: 'x', description: 'write: path=/a/b.md', toolUsed: 'write', usageCount: 1, status: 'active', ...over,
});

function renderList(props: Partial<React.ComponentProps<typeof LearnedList>> = {}) {
  const defaults: React.ComponentProps<typeof LearnedList> = {
    skills: [], proposals: [], onOpenSkill: vi.fn(), onSaveProposal: vi.fn(async () => true),
    onDismissProposal: vi.fn(), proposalError: null,
  };
  act(() => root.render(<LearnedList {...defaults} {...props} />));
}

it('groups skills by tool and shows readable titles, never the raw signature as the heading', () => {
  renderList({
    skills: [
      skill({ id: '1', description: 'write: path=/Users/j/dev/permagent-runtime/docs/a.md', usageCount: 3 }),
      skill({ id: '2', description: 'write: path=/Users/j/dev/permagent-runtime/docs/b.md', usageCount: 7 }),
    ],
  });
  const list = container.querySelector('[data-testid="learned-list"]')!;
  expect(list.textContent).toContain('Write a file');
  expect(list.textContent).not.toContain('write:-path');
  // The group's combined match count, not a per-item MASTERED/TRUSTED tier.
  expect(list.textContent).toContain('matched 10×');
  expect(list.textContent).not.toContain('MASTERED');
  expect(list.textContent).not.toContain('TRUSTED');
});

it('opens the skill in the library when a single-pattern group is clicked', () => {
  const onOpenSkill = vi.fn();
  renderList({ skills: [skill({ id: 'solo', description: 'read: path=/a' })], onOpenSkill });
  const row = container.querySelector('[data-testid="learned-group"] button')!;
  act(() => { row.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
  expect(onOpenSkill).toHaveBeenCalledWith('solo');
});

it('expands a multi-pattern group to its patterns, each opening its own skill', () => {
  const onOpenSkill = vi.fn();
  renderList({
    skills: [
      skill({ id: 'a', description: 'read: path=/one' }),
      skill({ id: 'b', description: 'read: path=/two' }),
    ],
    onOpenSkill,
  });
  const header = container.querySelector('[data-testid="learned-group"] button')!;
  act(() => { header.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
  const patterns = container.querySelectorAll('[data-testid="learned-pattern"]');
  expect(patterns).toHaveLength(2);
  act(() => { patterns[0].dispatchEvent(new MouseEvent('click', { bubbles: true })); });
  expect(onOpenSkill).toHaveBeenCalledWith('a');
});

it('renders a proposal card and reports a failed save on that same card', async () => {
  const proposal: SkillProposal = {
    description: 'write: path=/a', tool_used: 'write', argument_shape_hash: 'hash-1',
    occurrence_count: 4, source_task_ids: [], timestamp: '2026-09-01T00:00:00Z',
  };
  const onSaveProposal = vi.fn(async () => false);
  renderList({ proposals: [proposal], onSaveProposal, proposalError: 'hash-1' });
  const card = container.querySelector('[data-testid="learned-proposal"]')!;
  expect(card.textContent).toContain('Seen 4 times');
  expect(card.textContent).toContain("Couldn't save this skill");

  const save = [...card.querySelectorAll('button')].find(b => b.textContent === 'Save')!;
  await act(async () => { save.dispatchEvent(new MouseEvent('click', { bubbles: true })); await Promise.resolve(); });
  expect(onSaveProposal).toHaveBeenCalledWith(proposal);
});

it('dismisses a proposal by its hash', () => {
  const proposal: SkillProposal = {
    description: 'x', tool_used: 'x', argument_shape_hash: 'hash-2', occurrence_count: 1, source_task_ids: [], timestamp: 'x',
  };
  const onDismissProposal = vi.fn();
  renderList({ proposals: [proposal], onDismissProposal });
  const dismiss = [...container.querySelectorAll('button')].find(b => b.textContent === 'Dismiss')!;
  act(() => { dismiss.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
  expect(onDismissProposal).toHaveBeenCalledWith('hash-2');
});

it('marks an archived group distinctly from an active one', () => {
  renderList({ skills: [skill({ id: 'old', status: 'archived' })] });
  expect(container.querySelector('[data-testid="learned-list"]')!.textContent).toContain('archived');
});
