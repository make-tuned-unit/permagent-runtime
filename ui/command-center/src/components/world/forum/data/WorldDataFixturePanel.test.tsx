/** @vitest-environment jsdom */
import { afterEach, beforeAll, describe, expect, it } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';
import { WorldDataFixturePanel } from './WorldDataFixturePanel';
import { WORLD_FIXTURES, FIXTURE_MARKER } from './fixtures/worldFixtures';
import { runFixture, type FixtureRun } from './fixtures/fixtureSnapshots';
import { WORLD_PLACE_IDS } from './identity';
import { MAX_REQUESTS_PER_PLACE_VISIT } from './bounds';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let runs: FixtureRun[] = [];
let root: Root | null = null;
let host: HTMLDivElement | null = null;

beforeAll(async () => {
  runs = [];
  for (const f of WORLD_FIXTURES) runs.push(await runFixture(f));
});

afterEach(() => {
  act(() => root?.unmount());
  host?.remove();
  root = null;
  host = null;
});

function mount() {
  // 1280×800 logical — the review size in Astra's ruling §5.
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: 1280 });
  Object.defineProperty(window, 'innerHeight', { configurable: true, value: 800 });
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => root!.render(<WorldDataFixturePanel runs={runs} />));
  return host;
}

describe('World W1 fixture panel', () => {
  it('renders at 1280×800 logical, marked as test data', () => {
    const el = mount();
    const panel = el.querySelector<HTMLElement>('[data-testid="world-fixture-panel"]')!;
    expect(panel.style.width).toBe('1280px');
    expect(panel.style.height).toBe('800px');
    expect(panel.textContent).toContain(FIXTURE_MARKER);
    expect(el.querySelectorAll('[role="group"] button')).toHaveLength(10);
  });

  it('shows all six places for each of the ten fixture states (DOM snapshot per state)', () => {
    const el = mount();
    const buttons = Array.from(el.querySelectorAll<HTMLButtonElement>('[role="group"] button'));
    buttons.forEach((button, i) => {
      act(() => button.click());
      expect(button.getAttribute('aria-pressed')).toBe('true');
      const places = el.querySelectorAll('[data-place]');
      expect(Array.from(places).map((p) => p.getAttribute('data-place'))).toEqual([...WORLD_PLACE_IDS]);
      for (const place of Array.from(places)) {
        const count = Number(place.textContent!.match(/(\d+) request\(s\) this visit/)![1]);
        expect(count).toBeLessThanOrEqual(MAX_REQUESTS_PER_PLACE_VISIT);
      }
      expect(el.querySelector('[data-testid="world-fixture-panel"]')!.innerHTML).toMatchSnapshot(`${i + 1}-${runs[i].fixture.id}`);
    });
  });

  it('the disconnect fixture shows last good data marked stale, not an empty world', () => {
    const el = mount();
    const button = Array.from(el.querySelectorAll<HTMLButtonElement>('[role="group"] button')).find((b) => b.textContent!.startsWith('10.'))!;
    act(() => button.click());
    const arrival = el.querySelector('[data-place="commons"]')!;
    const goals = Array.from(arrival.querySelectorAll('li[data-slot-state="error"]'));
    expect(goals.length).toBeGreaterThan(0);
    expect(goals.some((g) => g.getAttribute('data-stale') === 'true' && /disconnected — showing last good data/.test(g.textContent!))).toBe(true);
  });

  it('every fixture state yields at least one non-ready section somewhere or real records — never a blank place', () => {
    for (const run of runs) {
      for (const p of WORLD_PLACE_IDS) {
        expect(Object.keys(run.snapshots[p].sections).length).toBeGreaterThan(0);
      }
    }
  });
});
