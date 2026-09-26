/**
 * @vitest-environment jsdom
 *
 * dagK-pressable: a timeline row that deep-links to evidence (goal, decision,
 * memory, url) opened only on click, with no role/tabIndex/keyboard path. A
 * row with nowhere to link (`onClick` undefined) must stay a plain, inert
 * div — converting it would announce a control that does nothing.
 */
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';
import { TimelineCard } from './TimelineCard';
import { useCommandCenter } from '../../../lib/store';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const apiFetch = vi.fn();
vi.mock('../../../lib/api', () => ({
  apiFetch: (...a: unknown[]) => apiFetch(...a),
  eventsWsUrl: () => 'ws://test',
}));

let container: HTMLDivElement;
let root: Root;
const initialState = useCommandCenter.getState();

const items = [
  {
    id: 'j1', ts: new Date().toISOString(), kind: 'goal_state_changed', actor: 'henry',
    title: 'Shipped the pressable sweep', detail: null,
    ref_kind: 'goal', ref_id: 'card-1', goal_project_id: 'proj-1',
  },
  {
    id: 'j2', ts: new Date().toISOString(), kind: 'librarian_describe_completed', actor: 'librarian',
    title: 'Indexed a document', detail: null,
    ref_kind: null, ref_id: null, goal_project_id: null,
  },
];

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  apiFetch.mockReset();
  apiFetch.mockResolvedValue({ items, next_before: null });
  useCommandCenter.setState({ ...initialState });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  useCommandCenter.setState(initialState);
});

async function render() {
  await act(async () => { root.render(<TimelineCard />); });
  await act(async () => { await Promise.resolve(); });
  await act(async () => { await Promise.resolve(); });
}

it('a row with evidence to open is a keyboard-operable control (role, tabIndex, pa-pressable)', async () => {
  await render();
  const row = container.querySelector('[aria-label^="View evidence"]') as HTMLElement;
  expect(row).toBeTruthy();
  expect(row.getAttribute('role')).toBe('button');
  expect(row.tabIndex).toBe(0);
  expect(row.className).toContain('pa-pressable');
});

it('Enter on a goal row opens the goal detail, same as a click', async () => {
  await render();
  const row = container.querySelector('[aria-label^="View evidence"]') as HTMLElement;
  await act(async () => {
    row.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
  });
  expect(useCommandCenter.getState().goalDetail).toEqual({ projectId: 'proj-1', cardId: 'card-1' });
});

it('a row with no evidence stays a plain, non-interactive row', async () => {
  await render();
  const rows = Array.from(container.querySelectorAll('[data-card-row]')) as HTMLElement[];
  const inertRow = rows.find(r => r.textContent?.includes('Indexed a document'));
  expect(inertRow).toBeTruthy();
  expect(inertRow!.getAttribute('role')).toBeNull();
  expect(inertRow!.tabIndex).toBe(-1);
});
