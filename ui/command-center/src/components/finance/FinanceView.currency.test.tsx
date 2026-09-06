/**
 * @vitest-environment jsdom
 *
 * The display currency, end to end: the choice is remembered, a rate the
 * daemon cannot give is said out loud rather than faked, and no figure ever
 * wears a CA$ prefix it did not earn.
 */

import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const { apiFetch } = vi.hoisted(() => ({ apiFetch: vi.fn() }));

vi.mock('../../lib/api', () => ({
  apiFetch,
  uploadFinanceStatement: vi.fn(),
  api: {
    readConfig: vi.fn(async () => false),
    upsertConfig: vi.fn(async () => ({})),
    readSecretConfig: vi.fn(async () => null),
    removeConfig: vi.fn(async () => ({})),
  },
}));

vi.mock('../../lib/store', () => ({
  navigateToTool: vi.fn(),
  useCommandCenter: vi.fn((selector: (state: { financeRev: number }) => unknown) => selector({ financeRev: 0 })),
}));

import { FinanceView } from './FinanceView';
import { DISPLAY_CURRENCY_KEY, FX_ENDPOINT } from './displayCurrency';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

async function flush() {
  await act(async () => { await Promise.resolve(); await Promise.resolve(); await Promise.resolve(); });
}

const BOARD = {
  polybot: { found: true, paused: false, stale: false, credentialsReady: false },
  polybotEnabled: false,
  holdings: {
    source: 'ledger', openCount: 1, netUnrealized: 100, netRealized: 0, netPnl: 100, trend: [], rows: [],
  },
  watchlist: [],
  notes: [],
  positions: [],
  picker: { reachable: false, baseUrl: 'x', scanInProgress: false },
  pickerEnabled: false,
  pickerUniverse: [],
  picks: [],
  sellSignals: [],
  rsiThreshold: 74,
  dailyPick: null,
  household: {
    recent: [],
    forecast: {
      daysUsed: 30, spend90d: 0, runRate30d: 1000, runRate90d: 0,
      byCategory: [], recurring: [], method: 'trailing run-rate, not a model',
    },
  },
};

/** The board always answers; the rate answers however the test says. */
function routes(fx: (() => Promise<unknown>) | null) {
  apiFetch.mockImplementation(async (endpoint: string) => {
    if (endpoint.startsWith(FX_ENDPOINT)) {
      if (!fx) {
        const err = new Error('HTTP 404') as Error & { status?: number };
        err.status = 404;
        throw err;
      }
      return fx();
    }
    return BOARD;
  });
}

const note = () => container.querySelector('[data-testid="finance-currency-note"]') as HTMLElement | null;
const picker = () => container.querySelector('[data-testid="finance-currency"]') as HTMLSelectElement;

beforeEach(() => {
  localStorage.clear();
  apiFetch.mockReset();
  routes(null);
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  localStorage.clear();
});

it('shows US dollars by default and asks for no rate at all', async () => {
  await act(async () => { root.render(<FinanceView />); });
  await flush();

  expect(picker().value).toBe('USD');
  // The default reader never makes a request they did not need.
  expect(apiFetch.mock.calls.every(([e]) => !String(e).startsWith(FX_ENDPOINT))).toBe(true);
  // Nothing to explain, so nothing is said.
  expect(note()).toBeNull();
  expect(container.textContent).toContain('+$100.00');
});

it('remembers the choice the way the app remembers its other display settings', async () => {
  await act(async () => { root.render(<FinanceView />); });
  await flush();

  await act(async () => {
    const select = picker();
    select.value = 'CAD';
    select.dispatchEvent(new Event('change', { bubbles: true }));
  });
  await flush();

  expect(localStorage.getItem(DISPLAY_CURRENCY_KEY)).toBe('CAD');

  // And it is still there on the next mount, without a fetch or a daemon.
  act(() => root.unmount());
  root = createRoot(container);
  await act(async () => { root.render(<FinanceView />); });
  await flush();
  expect(picker().value).toBe('CAD');
});

it('says the rate is unavailable, and keeps showing US dollars, rather than faking one', async () => {
  localStorage.setItem(DISPLAY_CURRENCY_KEY, 'CAD');
  await act(async () => { root.render(<FinanceView />); });
  await flush();

  expect(apiFetch.mock.calls.some(([e]) => String(e).startsWith(FX_ENDPOINT))).toBe(true);
  expect(note()?.getAttribute('data-status')).toBe('unavailable');
  expect(note()?.textContent).toContain('Rate unavailable');
  expect(note()?.textContent).toContain('US dollars');

  // The reader asked for CAD; the board did not pretend to give it. No figure
  // on screen carries a prefix that was never converted.
  expect(container.textContent).toContain('+$100.00');
  expect(container.textContent).not.toContain('CA$');
  // The choice itself survives the missing rate.
  expect(picker().value).toBe('CAD');
});

it('converts, marks every figure, and states the rate once with its age', async () => {
  const asOf = new Date(Date.now() - 60_000).toISOString();
  routes(async () => ({ base: 'USD', rates: { CAD: 1.37 }, asOf, source: 'yahoo:CAD=X' }));
  localStorage.setItem(DISPLAY_CURRENCY_KEY, 'CAD');

  await act(async () => { root.render(<FinanceView />); });
  await flush();

  expect(note()?.getAttribute('data-status')).toBe('ready');
  // The rate and its age, once per view — not beside every figure.
  expect(note()?.textContent).toContain('1 USD = 1.37 CAD');
  expect(note()?.textContent).toMatch(/as of/);
  expect(container.querySelectorAll('[data-testid="finance-currency-note"]').length).toBe(1);

  // Converted, and marked as converted.
  expect(container.textContent).toContain('+CA$137.00');
  expect(container.textContent).toContain('CA$1,370.00');
});

it('tells the journal apart from the display — the form still records US dollars', async () => {
  routes(async () => ({ base: 'USD', rates: { CAD: 1.37 }, asOf: new Date().toISOString() }));
  localStorage.setItem(DISPLAY_CURRENCY_KEY, 'CAD');

  await act(async () => { root.render(<FinanceView />); });
  await flush();

  const openForm = Array.from(container.querySelectorAll('button'))
    .find((b) => b.textContent?.includes('Record a trade')) as HTMLButtonElement;
  await act(async () => { openForm.click(); });

  const form = container.querySelector('#finance-holdings-form') as HTMLElement;
  expect(form.textContent).toContain('Entry price (USD)');
  expect(form.textContent).toContain('Exit price (USD)');
});
