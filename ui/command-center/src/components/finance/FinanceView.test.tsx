/**
 * @vitest-environment jsdom
 *
 * Finance tab: Polybot and Picker stay hidden until opt-in; picks are a
 * compact list; the Financier's approved name is labelled for review.
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
  // FinanceView reads `financeRev` (bumped by livenessSync on finance_changed)
  // only to re-trigger its poll effect; a stable value is enough here.
  useCommandCenter: vi.fn(() => 0),
}));

import { FinanceView } from './FinanceView';
import { POLYBOT_DISCLAIMER } from './financeLabs';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

async function flush() {
  await act(async () => { await Promise.resolve(); await Promise.resolve(); });
}

const emptyHousehold = {
  recent: [],
  forecast: {
    daysUsed: 0, spend90d: 0, runRate30d: 0, runRate90d: 0,
    byCategory: [], recurring: [], method: 'trailing run-rate, not a model',
  },
};

function board(over: Record<string, unknown> = {}) {
  return {
    polybot: {
      found: true, paused: false, stale: false, credentialsReady: false,
    },
    polybotEnabled: false,
    holdings: {
      source: 'ledger', openCount: 0, netUnrealized: 0, netRealized: 0, netPnl: 0, trend: [], rows: [],
    },
    watchlist: [],
    notes: [],
    positions: [],
    picker: {
      reachable: false, baseUrl: 'http://127.0.0.1:8080', scanInProgress: false,
      paused: false, stale: false,
    },
    pickerEnabled: false,
    pickerUniverse: [],
    picks: [],
    sellSignals: [],
    rsiThreshold: 74,
    dailyPick: null,
    household: emptyHousehold,
    ...over,
  };
}

beforeEach(() => {
  apiFetch.mockReset();
  apiFetch.mockResolvedValue(board());
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

it('hides Polybot and Picker cards until the user turns them on', async () => {
  await act(async () => { root.render(<FinanceView />); });
  await flush();
  expect(container.querySelector('[data-testid="finance-polybot-card"]')).toBeNull();
  expect(container.querySelector('[data-testid="finance-picker-card"]')).toBeNull();
  expect(container.querySelector('[data-testid="finance-picks-card"]')).toBeNull();
  expect(container.querySelector('[data-testid="finance-enable-polybot"]')).toBeTruthy();
  expect(container.querySelector('[data-testid="finance-enable-picker"]')).toBeTruthy();
  expect(container.textContent).not.toMatch(/POLYBOT/);
});

it('requires the risk checkbox before Polybot can turn on', async () => {
  await act(async () => { root.render(<FinanceView />); });
  await flush();
  await act(async () => {
    (container.querySelector('[data-testid="finance-enable-polybot"]') as HTMLButtonElement).click();
  });
  expect(container.querySelector('[data-testid="finance-disclaimer"]')?.textContent).toContain('real orders');
  expect(container.textContent).toContain(POLYBOT_DISCLAIMER.slice(0, 40));
  const confirm = Array.from(container.querySelectorAll('button')).find((b) =>
    b.textContent?.includes('lose real money'),
  ) as HTMLButtonElement;
  expect(confirm.disabled).toBe(true);
});

it('shows a Financier review badge on the approved pick and previews six rows', async () => {
  const picks = ['LEE', 'MAIA', 'CDZI', 'NAK', 'PMN', 'IAF', 'FOO', 'BAR'].map((ticker, i) => ({
    ticker,
    companyName: `${ticker} Inc`,
    rank: i + 1,
    score: 1,
    priceMismatch: false,
    fundamentals: { available: false },
    loop: { passed: false, kills: ['loop kill'], batchSize: 8 },
  }));
  apiFetch.mockResolvedValue(board({
    pickerEnabled: true,
    pickerUniverse: picks.map((p) => p.ticker),
    picks,
    dailyPick: {
      day: '2026-08-26',
      asOf: '2026-08-26T19:30:00Z',
      ticker: 'NAK',
      companyName: 'NAK Inc',
      why: 'Cleared the gate.',
      candidateCount: 8,
    },
  }));
  await act(async () => { root.render(<FinanceView />); });
  await flush();
  const rows = container.querySelectorAll('[data-testid="pick-row"]');
  expect(rows.length).toBe(6);
  expect(container.querySelector('[data-testid="pick-financier-badge"]')?.textContent).toMatch(/Agent approved/);
  expect(container.querySelector('[data-testid="financier-approved"]')).toBeNull();
  expect(container.querySelector('[data-testid="finance-financier-card"]')).toBeNull();
  expect((rows[0] as HTMLElement).getAttribute('data-approved')).toBe('true');
  expect(rows[0].textContent).toMatch(/NAK/);
  expect(container.textContent).toMatch(/Show 2 more/);
});

it('draws a holdings sparkline when a trend series is present', async () => {
  apiFetch.mockResolvedValue(board({
    holdings: {
      source: 'ledger', openCount: 1, netUnrealized: 10, netRealized: 0, netPnl: 10,
      trend: [1, 2, 4, 3],
      rows: [],
    },
  }));
  await act(async () => { root.render(<FinanceView />); });
  await flush();
  expect(container.querySelector('[data-testid="holdings-sparkline"]')).toBeTruthy();
  expect(container.querySelector('[data-testid="fundamentals-key"]')).toBeTruthy();
});
