/**
 * @vitest-environment jsdom
 *
 * SessionsList rows — R4/R8: a row that opened a session on click only, with
 * no role, no tabIndex and no keyboard path. Converted to `pressableEnterOnly`
 * (see components/common/pressable.ts); this pins that a row is now reachable
 * by keyboard and fires the same `handleSelect` Enter does.
 */
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';
import { SessionsList } from './SessionsList';
import { useCommandCenter } from '../../lib/store';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;
const initialState = useCommandCenter.getState();

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  useCommandCenter.setState({
    ...initialState,
    sessions: [{ id: 'sess-1', name: 'Planning', message_count: 3, updated_at: new Date().toISOString() } as never],
    loadSessions: vi.fn(async () => {}),
    switchToSession: vi.fn(async () => {}),
    openChatDock: vi.fn(),
    setActivePanel: vi.fn(),
  });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  useCommandCenter.setState(initialState);
});

it('a session row is a keyboard-operable control (role, tabIndex) with the shared pressable class', async () => {
  await act(async () => { root.render(<SessionsList />); });
  const row = container.querySelector('[role="button"]') as HTMLElement;
  expect(row).toBeTruthy();
  expect(row.tabIndex).toBe(0);
  expect(row.className).toContain('pa-pressable');
});

it('Enter on a row selects the session, same as a click', async () => {
  await act(async () => { root.render(<SessionsList />); });
  const row = container.querySelector('[role="button"]') as HTMLElement;
  const switchToSession = useCommandCenter.getState().switchToSession;
  await act(async () => {
    row.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
  });
  expect(switchToSession).toHaveBeenCalledWith('sess-1');
});
