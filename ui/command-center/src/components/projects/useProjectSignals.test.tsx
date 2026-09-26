/** @vitest-environment jsdom */
/**
 * P3 (QA walk 2026-09-24): the Projects grid showed name/description/tags only
 * — no way to tell which project needed attention without opening it.
 * `useProjectSignals` derives open tasks / goals in flight / last activity
 * from each project's existing cards + columns endpoints (no new backend
 * route: a card carries no status of its own, only a `columnId`, and a
 * column's `stateBinding` says whether it is settled).
 */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

const apiFetch = vi.fn();
vi.mock('../../lib/api', () => ({ apiFetch: (...a: unknown[]) => apiFetch(...a) }));

import { useProjectSignals, type ProjectSignal } from './useProjectSignals';

let container: HTMLDivElement;
let root: Root;
let latest: Record<string, ProjectSignal> = {};

function Probe({ ids }: { ids: string[] }) {
  latest = useProjectSignals(ids);
  return null;
}

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  apiFetch.mockReset();
  latest = {};
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

const col = (id: string, stateBinding: string | null) => ({
  id, projectId: 'p1', name: id, position: 0, columnKind: 'todo', stateBinding, wipLimit: null,
});
const card = (id: string, over: Record<string, unknown> = {}) => ({
  id, projectId: 'p1', cardType: 'task', title: id, description: '', columnId: 'todo',
  position: 0, createdBy: 'user', assignedTo: null, metadataJson: {},
  createdAt: '2026-09-01T00:00:00Z', updatedAt: '2026-09-01T00:00:00Z', archivedAt: null,
  ...over,
});

async function mount(ids: string[]) {
  await act(async () => { root.render(<Probe ids={ids} />); });
  await act(async () => { await Promise.resolve(); await Promise.resolve(); });
}

it('counts open tasks and goals in flight, and finds the latest activity', async () => {
  apiFetch.mockImplementation((url: string) => {
    if (url.endsWith('/cards')) {
      return Promise.resolve([
        card('t1', { columnId: 'todo', updatedAt: '2026-09-01T00:00:00Z' }),
        card('t2', { columnId: 'done', updatedAt: '2026-09-10T00:00:00Z' }),
        card('g1', { cardType: 'goal', columnId: 'todo', updatedAt: '2026-09-05T00:00:00Z' }),
        card('archived', { columnId: 'todo', archivedAt: '2026-09-02T00:00:00Z', updatedAt: '2026-09-02T00:00:00Z' }),
      ]);
    }
    return Promise.resolve([col('todo', 'in_progress'), col('done', 'complete')]);
  });
  await mount(['p1']);
  expect(latest.p1).toEqual({ openTasks: 1, goalsInFlight: 1, lastActivityAt: '2026-09-10T00:00:00Z' });
});

it('a column with no stateBinding counts as open, not settled', async () => {
  apiFetch.mockImplementation((url: string) => {
    if (url.endsWith('/cards')) return Promise.resolve([card('t1', { columnId: 'triage' })]);
    return Promise.resolve([col('triage', null)]);
  });
  await mount(['p1']);
  expect(latest.p1.openTasks).toBe(1);
});

it('a project whose fetch fails reads as no signal, not a crash', async () => {
  apiFetch.mockRejectedValue(new Error('network'));
  await mount(['p1']);
  expect(latest.p1).toEqual({ openTasks: 0, goalsInFlight: 0, lastActivityAt: null });
});

it('an empty id list returns no signals and fetches nothing', async () => {
  await mount([]);
  expect(latest).toEqual({});
  expect(apiFetch).not.toHaveBeenCalled();
});
