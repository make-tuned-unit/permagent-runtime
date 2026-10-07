/** @vitest-environment jsdom */
/**
 * QA at real size (~245px tile): every to-do title was cut to 3-4 characters
 * ("Set …", "Publi…") because the red "16 days overdue" label sat on the
 * title's row, nowrap, and took the width. The due label now lives on the
 * second line with the provenance, and the title may wrap to two lines.
 *
 * jsdom has no layout, so this pins the structure: the title's row holds only
 * the text column and the dismiss button; the due label is inside the second
 * (meta) line; the title clamps at two lines instead of one nowrap line.
 */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

vi.mock('../../../lib/store', () => ({
  useCommandCenter: (sel: (s: Record<string, unknown>) => unknown) =>
    sel({ openCardOnBoard: () => {} }),
}));

import { TodosCard } from './TodosCard';
import type { DueTodo, UseDueTodos } from '../../../lib/useDueTodos';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

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

const todo: DueTodo = {
  id: 't1', title: 'Set up the Atlantic shipping calendar for October',
  projectId: 'p1', projectName: 'Northwind', columnId: 'c1', columnName: 'Doing',
  dueDate: '2026-09-08', assignedTo: null, updatedAt: '2026-09-01T00:00:00Z',
};
const todos: UseDueTodos = {
  todos: [todo],
  groups: [{ bucket: 'overdue', label: 'Overdue', todos: [todo] }],
  today: '2026-09-24',
  loading: false, error: null,
  refresh: () => {}, setDueDate: async () => {}, dismiss: async () => {},
};

it('puts the overdue label on the second line, giving the title the row', () => {
  act(() => root.render(<TodosCard todos={todos} />));
  const text = container.querySelector('[data-testid="todo-row-text"]') as HTMLElement;
  const meta = container.querySelector('[data-testid="todo-row-meta"]') as HTMLElement;
  const due = container.querySelector('[data-testid="todo-row-due"]') as HTMLElement;
  expect(due.textContent).toBe('16 days overdue');
  // The due label is on the meta line, with the provenance, inside the text column.
  expect(meta.contains(due)).toBe(true);
  expect(text.contains(meta)).toBe(true);
  expect(meta.textContent).toContain('Northwind · Doing');
  // The row itself is just [text column][dismiss]: nothing else competes with the title.
  const row = text.parentElement as HTMLElement;
  expect(row.children).toHaveLength(2);
  expect(text.style.flex).toMatch(/^1/);
  expect(text.style.minWidth).toBe('0');
  // A too-long provenance wraps the due label under it rather than squeezing it.
  expect(meta.style.flexWrap).toBe('wrap');
});

it('lets the title wrap to two lines instead of one ellipsised nowrap line', () => {
  act(() => root.render(<TodosCard todos={todos} />));
  const title = container.querySelector('[data-testid="todo-row-title"]') as HTMLElement;
  const span = title.firstElementChild as HTMLElement;
  expect(span.textContent).toBe(todo.title);
  expect(span.style.whiteSpace).not.toBe('nowrap');
  expect(span.getAttribute('style') ?? '').toMatch(/line-clamp: 2/);
});
