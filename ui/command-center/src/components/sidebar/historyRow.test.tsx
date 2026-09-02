/**
 * @vitest-environment jsdom
 *
 * The History promotion, from both ends.
 *
 * #1177 made Sessions/Downloads/Activity/Spend one component but had to leave
 * it reachable only from inside Settings, because the rail row and the
 * destination switch belonged to another lane. This is that half, and the two
 * ways it can silently be wrong are:
 *
 *   1. The rail row exists but does not open anything — a control that does
 *      nothing, which is worse than no control.
 *   2. The rail row works, but the deep links that have been saying
 *      "Settings -> Spend" for months now land somewhere else. Those callers
 *      live outside this repo (app_conductor.rs, the agent's own phrasing)
 *      and none of them are versioned, so nothing but a test protects them.
 *
 * `settingsReach.test.ts` holds the routing table. This holds the wiring: the
 * row is really in the rail, clicking it really sets the destination, and the
 * destination really toggles back rather than trapping you.
 */

import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

import { Sidebar } from './Sidebar';
import { useCommandCenter } from '../../lib/store';
import {
  HISTORY_SECTIONS, isHistorySection, panelForSection,
} from '../settings/sections';
import { HISTORY_TAB_KEYS } from '../history/HistoryView';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  act(() => { useCommandCenter.setState({ activePanel: 'chat' }); });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  useCommandCenter.setState({ activePanel: 'chat' });
});

/** The rail row whose visible label starts with "History". */
function historyRow(): HTMLElement {
  const match = Array.from(container.querySelectorAll('button, [role="button"]'))
    .find(el => (el.textContent ?? '').trim().startsWith('History'));
  expect(match, 'no History row in the sidebar rail').toBeTruthy();
  return match as HTMLElement;
}

describe('History rail row', () => {
  it('is in the rail, and opens the History destination', () => {
    act(() => root.render(<Sidebar />));
    expect(useCommandCenter.getState().activePanel).toBe('chat');

    act(() => { historyRow().click(); });
    expect(useCommandCenter.getState().activePanel).toBe('history');
  });

  it('toggles back to chat rather than trapping you on it', () => {
    // Every other overlay row in this rail behaves this way; a destination you
    // can only leave by picking something else is the odd one out.
    act(() => root.render(<Sidebar />));
    act(() => { historyRow().click(); });
    act(() => { historyRow().click(); });
    expect(useCommandCenter.getState().activePanel).toBe('chat');
  });

  it('is the only entry point — Settings does not list History', () => {
    act(() => root.render(<Sidebar />));
    const railLabels = Array.from(container.querySelectorAll('button, [role="button"]'))
      .map(el => (el.textContent ?? '').trim())
      .filter(t => t.startsWith('History'));
    expect(railLabels, 'History appears twice in the rail').toHaveLength(1);
  });
});

describe('History deep links', () => {
  it('claims exactly the four record keys, plus its own', () => {
    // Pinned against HistoryView's own list so the router and the view cannot
    // drift — the router lives in `sections.ts` precisely so that nothing
    // navigational has to import a view.
    expect([...HISTORY_SECTIONS].sort()).toEqual(
      ['history', ...HISTORY_TAB_KEYS].sort(),
    );
  });

  it('routes every legacy record key to the destination, not to Settings', () => {
    for (const key of HISTORY_TAB_KEYS) {
      expect(isHistorySection(key)).toBe(true);
      expect(panelForSection(key)).toBe('history');
    }
  });

  it('leaves every other deep link going to Settings', () => {
    // The retired-pane keys #1177 absorbed still open Settings; only the
    // records moved out.
    for (const key of ['agent', 'models', 'keys', 'features', 'search', 'sources', 'data', 'sovereignty']) {
      expect(panelForSection(key)).toBe('settings');
    }
    // An unknown or absent section must not accidentally become a destination.
    expect(panelForSection(null)).toBe('settings');
    expect(panelForSection(undefined)).toBe('settings');
    expect(panelForSection('does-not-exist')).toBe('settings');
  });
});
