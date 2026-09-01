/** @vitest-environment jsdom
 *
 * `useToolOnScreen` — the gate that stops "mounted" from being mistaken for
 * "on screen". The app hides inactive workspaces instead of unmounting them,
 * so without this every tab's backstop poll would run at once, forever, on a
 * window that is genuinely visible.
 */

import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it } from 'vitest';

import { useCommandCenter } from './store';
import { useToolOnScreen } from './useToolOnScreen';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;
const seen: boolean[] = [];

function Probe() {
  seen.push(useToolOnScreen('grow'));
  return null;
}

function workspace(id: string, tool: string) {
  return {
    id,
    name: id,
    layoutJson: { type: 'panel', tool, config: {} },
    orderIndex: 0,
  } as never;
}

beforeEach(() => {
  seen.length = 0;
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  act(() => {
    useCommandCenter.setState({
      activePanel: 'chat',
      workspaces: [workspace('ws-grow', 'grow'), workspace('ws-chat', 'chat')],
      activeWorkspaceId: 'ws-grow',
    });
  });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function latest(): boolean {
  return seen[seen.length - 1]!;
}

it('is true when the active workspace hosts the tool', () => {
  act(() => root.render(<Probe />));
  expect(latest()).toBe(true);
});

it('is false while another workspace is the active one', () => {
  act(() => root.render(<Probe />));
  act(() => { useCommandCenter.setState({ activeWorkspaceId: 'ws-chat' }); });
  expect(latest()).toBe(false);
});

it('is false while a full-bleed overlay covers the workspaces', () => {
  act(() => root.render(<Probe />));
  act(() => { useCommandCenter.setState({ activePanel: 'settings' }); });
  expect(latest()).toBe(false);
  act(() => { useCommandCenter.setState({ activePanel: 'skills' }); });
  expect(latest()).toBe(false);
  act(() => { useCommandCenter.setState({ activePanel: 'chat' }); });
  expect(latest()).toBe(true);
});

it('is false when there is no active workspace at all', () => {
  act(() => root.render(<Probe />));
  act(() => { useCommandCenter.setState({ activeWorkspaceId: null }); });
  expect(latest()).toBe(false);
});
