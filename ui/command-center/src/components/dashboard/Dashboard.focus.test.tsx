/**
 * @vitest-environment jsdom
 *
 * Home focus mode, through the real Dashboard grid: one card expands into
 * focus, the rest go compact, Escape or Collapse restores the overview and
 * puts keyboard focus back where it started — and none of it touches the
 * saved layout. The data hooks and unrelated header chrome are mocked; the
 * grid, the controls and the focus logic are the real ones.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, useEffect, useState } from 'react';
import { createRoot, type Root } from 'react-dom/client';

vi.mock('../../lib/api', () => ({ apiFetch: vi.fn(async () => []), api: {} }));
vi.mock('./useDashboard', () => ({
  useDashboard: () => ({
    data: {
      agent: { name: 'Agent', state: 'idle', active_count: 0, summary: '' },
      stats: { sessions_today: 0, sessions_total: 0, memory_count: 0, memory_delta_today: 0 },
      in_flight: [], recent: [],
    },
    loading: false, error: null, lastOkAt: Date.now(), failing: false, retry: vi.fn(), refresh: vi.fn(),
  }),
}));
vi.mock('../../lib/useLiveGoals', () => ({ useLiveGoals: () => ({ goals: [], activeCount: 0 }) }));
vi.mock('../../lib/useDueTodos', () => ({ useDueTodos: () => [] }));
vi.mock('../sidebar/NavAgentStatus', () => ({ HeaderAgentStatus: () => null }));
vi.mock('./LearnNext', () => ({ LearnNext: () => null }));
vi.mock('./Echo', () => ({ Echo: () => null }));
vi.mock('../mobius/Mobius', () => ({ Mobius: () => null }));
vi.mock('./dashboardLayoutPreviewEvent', () => ({
  useDashboardLayoutPreviewEvent: () => null,
  clearDashboardLayoutPreview: vi.fn(),
}));

const persistLayout = vi.fn(async () => {});
const initialLayoutCards = [
  { id: 'alpha', type: 'alpha', position: { x: 0, y: 0 }, size: { w: 6, h: 4 }, visible: true },
  { id: 'beta', type: 'beta', position: { x: 6, y: 0 }, size: { w: 6, h: 4 }, visible: true },
  { id: 'gamma', type: 'gamma', position: { x: 0, y: 4 }, size: { w: 12, h: 3 }, visible: true },
];
let layoutCards = initialLayoutCards;
let layoutLoaded = true;
vi.mock('./useLayout', async (importOriginal) => ({
  ...(await importOriginal<typeof import('./useLayout')>()),
  useLayout: () => ({ layout: { cards: layoutCards }, layoutLoaded, persistLayout, revertLayout: vi.fn(async () => {}) }),
}));

import { useCardPresentation } from './homeFocus';

/** A card with its own action, that shows how it is being drawn, and — like
 *  Calendar's date header — can ask to be expanded itself. */
const cardMounted = vi.fn();
const cardUnmounted = vi.fn();
function FakeCard({ label }: { label: string }) {
  useEffect(() => { cardMounted(label); return () => { cardUnmounted(label); }; }, [label]);
  const home = useCardPresentation();
  const [clicks, setClicks] = useState(0);
  return (
    <div style={{ height: '100%' }}>
      <span data-testid={`${label}-presentation`}>{home?.presentation ?? 'none'}</span>
      <button type="button" onClick={() => setClicks(c => c + 1)}>{label} action ({clicks})</button>
      <button type="button" onClick={() => home?.expand()}>Open {label} from inside</button>
      <input aria-label={`${label} search`} />
    </div>
  );
}
const card = (label: string) => ({
  component: () => <FakeCard label={label} />, name: label[0].toUpperCase() + label.slice(1),
  description: '', defaultSize: { w: 6, h: 4 },
});
// Match production's stable component identities across Dashboard renders.
const fakeRegistry = { alpha: card('alpha'), beta: card('beta'), gamma: card('gamma'), calendar: card('alpha') };
let registryStatus: 'loading' | 'ready' | 'error' = 'ready';
vi.mock('./cards/useCardRegistry', () => ({
  useCardRegistry: () => ({ registry: fakeRegistry, status: registryStatus }),
}));

import { Dashboard } from './Dashboard';
import { LEGACY_MORNING_TEMPLATE_LAYOUT, upgradeHomeDefault } from './useLayout';
import { requestHomeFocus, useAppNavigationRequests } from '../../lib/appNavigationRequests';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: Root;

beforeEach(() => {
  persistLayout.mockClear();
  cardMounted.mockClear();
  cardUnmounted.mockClear();
  layoutCards = initialLayoutCards.map(c => ({ ...c }));
  layoutLoaded = true;
  registryStatus = 'ready';
  useAppNavigationRequests.setState({ homeFocus: null });
  host = document.createElement('div');
  document.body.appendChild(host);
  act(() => { root = createRoot(host); root.render(<Dashboard />); });
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  document.querySelectorAll('[data-test-modal]').forEach(n => n.remove());
  Reflect.deleteProperty(document, 'startViewTransition');
  localStorage.removeItem('permagent-reduce-motion');
});

const cell = (id: string) => host.querySelector<HTMLElement>(`[data-testid="dashboard-cell-${id}"]`)!;
const presentation = (id: string) => cell(id).dataset.presentation;
const button = (name: string) => Array.from(host.querySelectorAll<HTMLButtonElement>('button'))
  .find(b => b.getAttribute('aria-label') === name || b.textContent === name);

async function frame() {
  await act(async () => { await new Promise(r => requestAnimationFrame(() => r(null))); });
}
function press(key: string, target: Element = document.activeElement ?? document.body) {
  const ev = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true });
  act(() => { target.dispatchEvent(ev); });
  return ev;
}
async function expandCard(name: string) {
  act(() => button(`Expand ${name}`)!.click());
  await frame();
}

describe('Home focus mode', () => {
  it('renders the shipped daemon morning layout as the five-card daily Home', () => {
    layoutCards = upgradeHomeDefault(LEGACY_MORNING_TEMPLATE_LAYOUT).cards;
    act(() => root.render(<Dashboard />));

    const grid = host.querySelector('[data-testid="home-grid"]')!;
    expect(grid.classList.contains('pa-home-overview')).toBe(true);
    expect(host.querySelectorAll('[data-testid^="dashboard-cell-"]')).toHaveLength(5);
    expect(host.querySelectorAll('[data-testid="dashboard-cell-decisions"]')).toHaveLength(1);
    expect(host.querySelector('[data-testid="dashboard-cell-today_focus"]')).toBeNull();
    expect(host.querySelector('[data-testid="dashboard-cell-henry_status"]')).toBeNull();
    expect(host.querySelector('[data-testid="dashboard-cell-weather"]')).toBeNull();
    expect(host.querySelector('[data-testid="dashboard-cell-system_stats"]')).toBeNull();
    expect(host.querySelector('[data-testid="dashboard-cell-in_flight"]')?.getAttribute('style')).toContain('grid-column: span 6');
  });

  it('shows every card in the overview with a labelled, collapsed Expand control', () => {
    for (const [id, name] of [['alpha', 'Alpha'], ['beta', 'Beta'], ['gamma', 'Gamma']]) {
      expect(presentation(id)).toBe('overview');
      const b = button(`Expand ${name}`)!;
      expect(b.getAttribute('aria-expanded')).toBe('false');
      expect(b.getAttribute('aria-controls')).toBe(cell(id).id);
    }
  });

  it('keeps customized overview cell geometry unchanged across Customize and Done', async () => {
    const geometry = () => {
      const grid = host.querySelector<HTMLElement>('[data-testid="home-grid"]')!;
      return {
        rows: grid.style.gridAutoRows,
        flow: grid.style.gridAutoFlow,
        cells: ['alpha', 'beta', 'gamma'].map(id => ({
          id,
          column: cell(id).style.gridColumn,
          row: cell(id).style.gridRow,
          height: cell(id).style.height,
          align: cell(id).style.alignSelf,
          maxHeight: cell(id).style.maxHeight,
        })),
      };
    };
    const overview = geometry();
    expect(overview.rows).toBe('60px');
    expect(overview.flow).toBe('row');
    expect(overview.cells).toEqual([
      { id: 'alpha', column: '1 / span 6', row: '1 / span 4', height: '', align: 'stretch', maxHeight: '' },
      { id: 'beta', column: '7 / span 6', row: '1 / span 4', height: '', align: 'stretch', maxHeight: '' },
      { id: 'gamma', column: '1 / span 12', row: '5 / span 3', height: '', align: 'stretch', maxHeight: '' },
    ]);

    act(() => host.querySelector<HTMLButtonElement>('[data-testid="dashboard-customize"]')!.click());
    expect(geometry()).toEqual(overview);
    act(() => host.querySelector<HTMLButtonElement>('[data-testid="dashboard-customize"]')!.click());
    expect(geometry()).toEqual(overview);
    expect(persistLayout).not.toHaveBeenCalled();
  });

  it('expands one card into focus and makes the others compact', async () => {
    await expandCard('Alpha');
    expect(presentation('alpha')).toBe('focused');
    expect(presentation('beta')).toBe('compact');
    expect(presentation('gamma')).toBe('compact');
    // The cards themselves are told, so a card with a richer focused view can switch.
    expect(host.querySelector('[data-testid="alpha-presentation"]')!.textContent).toBe('focused');
    expect(host.querySelector('[data-testid="beta-presentation"]')!.textContent).toBe('compact');

    expect(cell('alpha').getAttribute('role')).toBe('region');
    expect(cell('alpha').getAttribute('aria-label')).toBe('Alpha, expanded');
    // Focused: full width; compact tiles a fixed height (their own body scrolls).
    expect(cell('alpha').style.gridColumn).toBe('1 / -1');
    expect(cell('beta').style.height).not.toBe('');

    const collapseBtn = button('Collapse Alpha')!;
    expect(collapseBtn.getAttribute('aria-expanded')).toBe('true');
    expect(document.activeElement).toBe(collapseBtn);
    expect(button('Expand Alpha')).toBeUndefined();
  });

  it('preserves every card instance and local state across expand, collapse and changing focus', async () => {
    const originalInput = host.querySelector<HTMLInputElement>('input[aria-label="alpha search"]')!;
    originalInput.value = 'keep this draft';
    act(() => button('alpha action (0)')!.click());
    act(() => button('beta action (0)')!.click());
    const assertPreserved = () => {
      expect(cardMounted.mock.calls.map(([label]) => label)).toEqual(['alpha', 'beta', 'gamma']);
      expect(cardUnmounted).not.toHaveBeenCalled();
      expect(button('alpha action (1)')).toBeDefined();
      expect(button('beta action (1)')).toBeDefined();
      expect(host.querySelector('input[aria-label="alpha search"]')).toBe(originalInput);
      expect(originalInput.value).toBe('keep this draft');
    };
    await expandCard('Alpha');
    assertPreserved();
    act(() => button('Collapse Alpha')!.click());
    await frame();
    assertPreserved();
    await expandCard('Alpha');
    await expandCard('Beta');
    assertPreserved();
    act(() => button('Collapse Beta')!.click());
    await frame();
    assertPreserved();
  });

  it('focuses one card at a time', async () => {
    await expandCard('Alpha');
    await expandCard('Beta');
    expect(presentation('beta')).toBe('focused');
    expect(presentation('alpha')).toBe('compact');
    expect(host.querySelectorAll('[data-presentation="focused"]')).toHaveLength(1);
  });

  it('Collapse restores the overview and returns keyboard focus to the Expand control', async () => {
    await expandCard('Beta');
    act(() => button('Collapse Beta')!.click());
    await frame();
    expect(['alpha', 'beta', 'gamma'].map(presentation)).toEqual(['overview', 'overview', 'overview']);
    expect(document.activeElement).toBe(button('Expand Beta'));
  });

  it('Escape collapses from inside the focused card and restores focus', async () => {
    await expandCard('Alpha');
    const action = button('alpha action (0)')!;
    action.focus();
    const ev = press('Escape');
    await frame();
    expect(ev.defaultPrevented).toBe(true);
    expect(presentation('alpha')).toBe('overview');
    expect(document.activeElement).toBe(button('Expand Alpha'));
  });

  it('leaves Escape alone while a modal is open, in a text field, or already handled', async () => {
    await expandCard('Alpha');

    const modal = document.createElement('div');
    modal.setAttribute('role', 'dialog');
    modal.setAttribute('aria-modal', 'true');
    modal.setAttribute('data-test-modal', '');
    document.body.appendChild(modal);
    press('Escape', button('Collapse Alpha')!);
    expect(presentation('alpha')).toBe('focused');
    modal.remove();

    const field = host.querySelector<HTMLInputElement>('input[aria-label="alpha search"]')!;
    field.focus();
    const typed = press('Escape', field);
    expect(typed.defaultPrevented).toBe(false);
    expect(presentation('alpha')).toBe('focused');

    const handled = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
    handled.preventDefault();
    act(() => { button('Collapse Alpha')!.dispatchEvent(handled); });
    expect(presentation('alpha')).toBe('focused');
  });

  it('moves keyboard focus after a delayed view-transition update creates the target', async () => {
    localStorage.setItem('permagent-reduce-motion', 'false');
    const updates: Array<() => void> = [];
    Object.defineProperty(document, 'startViewTransition', { configurable: true, value: (update: () => void) => {
      updates.push(update); return { finished: Promise.resolve() };
    } });
    act(() => button('Expand Alpha')!.focus());
    act(() => button('Expand Alpha')!.click());
    await frame(); // Deliberately let the old implementation's premature RAF fire.
    expect(presentation('alpha')).toBe('overview');
    act(() => updates.shift()!()); await frame();
    expect(presentation('alpha')).toBe('focused');
    expect(document.activeElement).toBe(button('Collapse Alpha'));
    act(() => button('Collapse Alpha')!.click()); await frame();
    act(() => updates.shift()!()); await frame();
    expect(document.activeElement).toBe(button('Expand Alpha'));
  });

  it("keeps every card's own actions working in focus and compact", async () => {
    await expandCard('Alpha');
    act(() => button('alpha action (0)')!.click());
    act(() => button('beta action (0)')!.click());
    expect(button('alpha action (1)')).toBeDefined();
    expect(button('beta action (1)')).toBeDefined();
    expect(presentation('alpha')).toBe('focused');
  });

  it('lets a card expand itself (as Calendar does from its date header)', async () => {
    act(() => button('Open gamma from inside')!.click());
    await frame();
    expect(presentation('gamma')).toBe('focused');
    expect(document.activeElement).toBe(button('Collapse Gamma'));
  });

  it('never saves the layout', async () => {
    await expandCard('Alpha');
    await expandCard('Gamma');
    act(() => button('Collapse Gamma')!.click());
    await frame();
    expect(persistLayout).not.toHaveBeenCalled();
  });

  it('Customize turns focus off and hides the Expand controls', async () => {
    await expandCard('Alpha');
    const customize = Array.from(host.querySelectorAll('button')).find(b => b.textContent?.includes('Customize'))!;
    act(() => customize.click());
    await frame();
    expect(host.querySelectorAll('[data-presentation="focused"]')).toHaveLength(0);
    expect(button('Expand Beta')).toBeUndefined();
    const grid = host.querySelector<HTMLElement>('[data-testid="home-grid"]')!;
    const overviewGeometry = ['alpha', 'beta', 'gamma'].map(id => [
      cell(id).style.gridColumn,
      cell(id).style.gridRow,
      cell(id).style.height,
      cell(id).style.alignSelf,
      cell(id).style.maxHeight,
    ]);
    act(() => host.querySelector<HTMLButtonElement>('[data-testid="dashboard-customize"]')!.click());
    await frame();
    expect(host.querySelectorAll('[data-presentation="focused"]')).toHaveLength(0);
    expect(grid.style.gridAutoRows).toBe('60px');
    expect(grid.style.gridAutoFlow).toBe('row');
    expect(['alpha', 'beta', 'gamma'].map(id => [
      cell(id).style.gridColumn,
      cell(id).style.gridRow,
      cell(id).style.height,
      cell(id).style.alignSelf,
      cell(id).style.maxHeight,
    ])).toEqual(overviewGeometry);
    expect(persistLayout).not.toHaveBeenCalled();
  });

  it('drops focus when the focused card leaves the layout', async () => {
    await expandCard('Beta');
    layoutCards = layoutCards.filter(c => c.id !== 'beta');
    act(() => root.render(<Dashboard />));
    await frame();
    expect(host.querySelectorAll('[data-presentation="focused"]')).toHaveLength(0);
    expect(presentation('alpha')).toBe('overview');
  });
});

describe('agent Home focus navigation', () => {
  function useCalendarType(visible = true) {
    layoutCards = layoutCards.map(c => c.id === 'alpha' ? { ...c, type: 'calendar', visible } : c);
    act(() => root.render(<Dashboard />));
  }
  it('resolves a visible card type to its real layout id and collapses through overview without saving', async () => {
    useCalendarType();
    act(() => { requestHomeFocus('calendar'); }); await frame();
    expect(presentation('alpha')).toBe('focused');
    expect(useAppNavigationRequests.getState().homeFocus).toBeNull();
    act(() => { requestHomeFocus('overview'); }); await frame();
    expect(presentation('alpha')).toBe('overview');
    expect(persistLayout).not.toHaveBeenCalled();
  });
  it('holds a queued focus while the saved layout is loading', async () => {
    layoutLoaded = false; useCalendarType();
    act(() => { requestHomeFocus('calendar'); }); await frame();
    expect(presentation('alpha')).toBe('overview');
    expect(useAppNavigationRequests.getState().homeFocus).toEqual({ section: 'calendar' });
    layoutLoaded = true; act(() => root.render(<Dashboard />)); await frame();
    expect(presentation('alpha')).toBe('focused');
    expect(useAppNavigationRequests.getState().homeFocus).toBeNull();
  });
  it('reports a hidden or absent card without adding it or changing layout', async () => {
    useCalendarType(false); const before = JSON.stringify(layoutCards);
    act(() => { requestHomeFocus('calendar'); }); await frame();
    expect(host.textContent).toContain('card is not available in this Home layout. No card was added.');
    expect(useAppNavigationRequests.getState().homeFocus).toBeNull();
    expect(JSON.stringify(layoutCards)).toBe(before); expect(persistLayout).not.toHaveBeenCalled();
  });
  it('focuses a registered code card and collapses even while unrelated manifests load', async () => {
    registryStatus = 'loading'; useCalendarType();
    act(() => { requestHomeFocus('calendar'); }); await frame();
    expect(presentation('alpha')).toBe('focused');
    expect(useAppNavigationRequests.getState().homeFocus).toBeNull();
    act(() => { requestHomeFocus('overview'); }); await frame();
    expect(presentation('alpha')).toBe('overview');
  });
  it('waits for a missing manifest renderer, then reports unavailability if loading fails', async () => {
    registryStatus = 'loading'; act(() => root.render(<Dashboard />));
    act(() => { requestHomeFocus('weather'); }); await frame();
    expect(useAppNavigationRequests.getState().homeFocus).toEqual({ section: 'weather' });
    registryStatus = 'error'; act(() => root.render(<Dashboard />)); await frame();
    expect(host.textContent).toContain('weather card is not available');
    expect(useAppNavigationRequests.getState().homeFocus).toBeNull();
    expect(persistLayout).not.toHaveBeenCalled();
  });
  it('does not leave Customize to satisfy an agent request', async () => {
    useCalendarType();
    registryStatus = 'loading';
    const customize = Array.from(host.querySelectorAll('button')).find(b => b.textContent?.includes('Customize'))!;
    act(() => customize.click()); await frame();
    act(() => { requestHomeFocus('weather'); }); await frame();
    expect(useAppNavigationRequests.getState().homeFocus).toBeNull();
    expect(host.textContent).toContain('Finish Customize before focusing a Home card');
    expect(button('Expand Alpha')).toBeUndefined();
    expect(host.querySelectorAll('[data-presentation="focused"]')).toHaveLength(0);
    expect(persistLayout).not.toHaveBeenCalled();
  });
});
