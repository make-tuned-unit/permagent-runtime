/**
 * @vitest-environment jsdom
 *
 * QA 2026-09-20 (dark theme, Spend open): two tabs looked selected at once.
 * History styled its own segmented control and painted the selected tab with
 * `colors.surface`, which is DARKER than the strip in the dark theme, so the
 * chosen tab read as recessed while a hovered one read as raised. It now wears
 * the app's shared segmented control.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('../../lib/notifications', () => ({ useNotifications: () => ({ items: [], unread: 0 }) }));
vi.mock('../../lib/api', () => ({ api: { getIncidents: vi.fn().mockResolvedValue([]) } }));
vi.mock('../sessions/SessionsList', () => ({ SessionsList: () => <div data-testid="history-sessions-panel" /> }));
vi.mock('../inbox/InboxPanel', () => ({ InboxPanel: () => <div data-testid="history-inbox-panel" /> }));
vi.mock('../trace/ExecutionTrace', () => ({ ExecutionTrace: () => <div data-testid="history-activity-panel" /> }));
vi.mock('./SpendPanel', () => ({ SpendPanel: () => <div data-testid="history-spend-panel" /> }));

import { HistoryView, HISTORY_PAGE_PADDING } from './HistoryView';
import { space } from '../../styles/tokens';
import { segmentedTab } from '../common/segmented';
import { getThemedColors, setTheme } from '../../styles/tokens';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;
beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});
afterEach(() => { act(() => root.unmount()); container.remove(); });

const tabs = () => Array.from(container.querySelectorAll('[role="tab"]')) as HTMLElement[];

describe('History segmented control', () => {
  it('marks exactly one tab selected, in the shared selected look', () => {
    for (const theme of ['dark', 'silver'] as const) {
      setTheme(theme);
      const colors = getThemedColors();
      act(() => root.render(<HistoryView initialTab="spend" />));
      const selected = tabs().filter(t => t.getAttribute('aria-selected') === 'true');
      expect(selected, theme).toHaveLength(1);
      expect(selected[0].textContent, theme).toContain('Spend');
      const want = segmentedTab(colors, true)['--pa-btn-bg' as keyof ReturnType<typeof segmentedTab>];
      expect(selected[0].style.getPropertyValue('--pa-btn-bg'), theme).toBe(want);
      // ...and an unselected tab's HOVER must not be the selected fill, which
      // is what made a hovered tab look chosen.
      const other = tabs().find(t => t.getAttribute('aria-selected') !== 'true')!;
      expect(other.style.getPropertyValue('--pa-btn-bg-hover'), theme).not.toBe(want);
    }
  });

  it('switches to the matching records panel when a tab is chosen', () => {
    act(() => root.render(<HistoryView initialTab="sessions" />));
    const expectedPanels = [
      ['sessions', 'history-sessions-panel'],
      ['inbox', 'history-inbox-panel'],
      ['activity', 'history-activity-panel'],
      ['spend', 'history-spend-panel'],
    ] as const;

    for (const [tab, panel] of expectedPanels) {
      const button = container.querySelector('[data-testid="history-tab-' + tab + '"]') as HTMLButtonElement;
      act(() => button.click());
      expect(container.querySelector('[data-testid="' + panel + '"]'), tab).not.toBeNull();
      const panelRegion = container.querySelector('[role="tabpanel"]')!;
      expect(panelRegion.id).toBe('history-panel-' + tab);
      expect(panelRegion.getAttribute('aria-labelledby')).toBe('history-tab-' + tab);
      expect(button.getAttribute('aria-controls')).toBe('history-panel-' + tab);
      expect(container.querySelectorAll('[role="tab"][aria-selected="true"]')).toHaveLength(1);
      expect(button.getAttribute('aria-selected')).toBe('true');
      for (const [, otherPanel] of expectedPanels.filter(([key]) => key !== tab)) {
        expect(container.querySelector('[data-testid="' + otherPanel + '"]'), tab + ' hides ' + otherPanel).toBeNull();
      }
    }
  });
});

describe('History page padding (QA 2026-09-24)', () => {
  it('insets the header, segments and list from the rail like every other view', () => {
    act(() => root.render(<HistoryView initialTab="sessions" />));
    const page = container.querySelector<HTMLElement>('[data-testid="history-view"]')!;
    expect(page.style.padding).toBe(HISTORY_PAGE_PADDING);
    expect(page.style.paddingLeft).toBe(`${space.huge}px`);
    expect(page.style.boxSizing).toBe('border-box');
  });
});
