/**
 * @vitest-environment jsdom
 *
 * dagK-pressable: an In Flight goal row opened a goal's detail on click only,
 * with no role, tabIndex or keyboard path. Converted to `pressable` (see
 * components/common/pressable.ts); this pins that a row is now reachable by
 * keyboard and opens the same goal Enter does.
 */
import { afterEach, beforeEach, expect, it } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';
import { InFlightCard } from './InFlightCard';
import { useCommandCenter } from '../../../lib/store';
import type { ActiveGoal } from '../../../lib/useLiveGoals';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;
const initialState = useCommandCenter.getState();

const goal: ActiveGoal = {
  id: 'goal-1', title: 'Sweep the pressable sites', project_id: 'proj-1',
  state: 'in_progress', assigned_to: 'Henry',
  created_at: new Date().toISOString(), updated_at: new Date().toISOString(),
  routing_note: null, hold_note: null, auto_approve: false,
};

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  useCommandCenter.setState({ ...initialState });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  useCommandCenter.setState(initialState);
});

it('a goal row is a keyboard-operable control (role, tabIndex, pa-pressable)', async () => {
  await act(async () => { root.render(<InFlightCard goals={[goal]} />); });
  const row = container.querySelector('[data-testid="inflight-row"]') as HTMLElement;
  expect(row).toBeTruthy();
  expect(row.getAttribute('role')).toBe('button');
  expect(row.tabIndex).toBe(0);
  expect(row.className).toContain('pa-pressable');
});

it('Enter on a goal row opens the same goal a click would', async () => {
  await act(async () => { root.render(<InFlightCard goals={[goal]} />); });
  const row = container.querySelector('[data-testid="inflight-row"]') as HTMLElement;
  await act(async () => {
    row.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
  });
  expect(useCommandCenter.getState().goalDetail).toEqual({ projectId: 'proj-1', cardId: 'goal-1' });
});
