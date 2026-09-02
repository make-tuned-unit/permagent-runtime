/**
 * @vitest-environment jsdom
 *
 * `loadBoard` here used to `catch { /* silently fail *\/ }`, so a board that
 * failed to load rendered "No tasks yet." — the same words as a project that
 * genuinely has none. Its sibling `ProjectKanban` fetches the same two
 * endpoints one lens away and has always handled this correctly.
 */

import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('../../lib/store', async () => {
  const actual = await vi.importActual<Record<string, unknown>>('../../lib/store');
  return {
    ...actual,
    useCommandCenter: (sel: (s: Record<string, unknown>) => unknown) =>
      sel({ openCardOnBoard: vi.fn(), openGoalDetail: vi.fn(), growProject: vi.fn() }),
    navigateToTool: vi.fn(),
  };
});

import { TasksPanel } from './ProjectOverview';

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

function render(props: Record<string, unknown>) {
  return act(() => {
    root.render(
      <TasksPanel
        columns={[]}
        cards={[]}
        loading={false}
        error={false}
        onRetry={() => {}}
        onOpenGoal={() => {}}
        {...props}
      />,
    );
  });
}

it('a board that failed to load is an error with a retry, not "no tasks yet"', () => {
  const onRetry = vi.fn();
  render({ error: true, onRetry });

  expect(container.textContent).toMatch(/Couldn't load/i);
  expect(container.textContent).not.toMatch(/No tasks yet/i);
  const retry = Array.from(container.querySelectorAll('button')).find((b) => /Try again/i.test(b.textContent ?? ''))!;
  expect(retry).toBeTruthy();
  act(() => { retry.click(); });
  expect(onRetry).toHaveBeenCalled();
});

it('says it is loading before the first fetch lands', () => {
  render({ loading: true });
  expect(container.textContent).toMatch(/Loading/i);
  expect(container.textContent).not.toMatch(/No tasks yet/i);
});

it('a project that really has no cards still reads as empty', () => {
  render({});
  expect(container.textContent).toMatch(/No tasks yet/i);
  expect(container.textContent).not.toMatch(/Couldn't load/i);
});
