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

  // The caption names the badge by its own words, not by a colour. It used to
  // say "Gold means…" — a description that only works for readers who see that
  // colour the same way, on a tag that has said "Agent approved" in words for
  // some time now, across three themes.
  const caption = container.querySelector('[data-testid="picks-caption"]')?.textContent ?? '';
  expect(caption).toContain('Agent approved');
  expect(caption).not.toMatch(/gold/i);
  // And the badge itself explains what approval meant.
  const badge = container.querySelector('[data-testid="pick-financier-badge"]') as HTMLElement;
  expect(badge).toBeTruthy();
  act(() => { (badge.parentElement as HTMLElement).focus(); });
  expect(document.querySelector('[role="tooltip"]')?.textContent).toMatch(/Financier/);
});

it('labels the cross-link with what it does, not with who lives there', async () => {
  apiFetch.mockResolvedValue(board({ pickerEnabled: true, picks: [] }));
  await act(async () => { root.render(<FinanceView />); });
  await flush();
  const link = container.querySelector('[data-testid="picks-world-link"]') as HTMLElement;
  // A bare agent name says who, never what pressing it does.
  expect(link?.textContent).toBe('View in World');
  act(() => { link.focus(); });
  expect(document.querySelector('[role="tooltip"]')?.textContent).toMatch(/Financier/);
});

it('separates the scanner pool from the tickers the user added', async () => {
  // Regression: the card said "Picker universe · 14,630 names" for a number
  // that is the scanner's own exchange listing cache, while the user's own
  // list was empty. Two lists, two labels.
  apiFetch.mockResolvedValue(board({
    pickerEnabled: true,
    pickerUniverse: [],
    pickerUniverseCount: 14630,
    picker: { reachable: true, baseUrl: 'x', scanInProgress: false },
  }));
  await act(async () => { root.render(<FinanceView />); });
  await flush();
  const pool = container.querySelector('[data-testid="picker-scanner-pool"]');
  const mine = container.querySelector('[data-testid="picker-your-tickers"]');
  expect(pool?.textContent).toContain('Scanner pool');
  expect(pool?.textContent).toContain('14,630');
  expect(pool?.textContent).not.toMatch(/universe/i);
  expect(mine?.textContent).toContain('none yet');
  expect(container.textContent).not.toMatch(/Picker universe/);
  // …and the affordance that says what to do about it opens the add form.
  const hint = container.querySelector('[data-testid="picker-add-hint"]') as HTMLButtonElement;
  expect(hint).toBeTruthy();
  await act(async () => { hint.click(); });
  expect(container.querySelector('[data-testid="picker-universe"]')).toBeTruthy();
});

it('counts the user\'s own tickers separately when they have some', async () => {
  apiFetch.mockResolvedValue(board({
    pickerEnabled: true,
    pickerUniverse: ['AAPL', 'NAK'],
    pickerUniverseCount: 14630,
    picker: { reachable: true, baseUrl: 'x', scanInProgress: false },
  }));
  await act(async () => { root.render(<FinanceView />); });
  await flush();
  expect(container.querySelector('[data-testid="picker-your-tickers"]')?.textContent)
    .toContain('2 you added');
  expect(container.querySelector('[data-testid="picker-add-hint"]')).toBeNull();
});

it('explains a filtered pick on the tag, and the tag opens the reason', async () => {
  apiFetch.mockResolvedValue(board({
    pickerEnabled: true,
    pickerUniverse: [],
    picks: [{
      ticker: 'LEE',
      companyName: 'Lee Inc',
      rank: 1,
      score: 1,
      priceMismatch: false,
      fundamentals: { available: false },
      loop: {
        passed: false,
        kills: ['in-sample ICIR 0.12 is below 0.3 — likely noise'],
        batchSize: 8,
      },
    }],
  }));
  await act(async () => { root.render(<FinanceView />); });
  await flush();

  // The caption no longer claims a personal universe the user never set up.
  expect(container.querySelector('[data-testid="picks-caption"]')?.textContent)
    .toMatch(/scanner’s own ranking/);
  expect(container.querySelector('[data-testid="picks-legend"]')).toBeTruthy();

  const tag = container.querySelector('[data-testid="pick-loop-tag"]') as HTMLButtonElement;
  expect(tag.textContent).toContain('filtered');
  expect(tag.textContent).not.toMatch(/loop kill/);
  // The reason is not in the label: nearly every row carries this tag, and a
  // per-row phrase turns the column into fifteen sentences to read.
  expect(tag.textContent).not.toContain('looks like noise');
  // Focus opens the Tooltip primitive (title no longer sets a native attribute).
  await act(async () => { tag.focus(); });
  const tipId = tag.getAttribute('aria-describedby');
  expect(tipId).toBeTruthy();
  const tip = tipId ? document.getElementById(tipId) : null;
  expect(tip?.getAttribute('role')).toBe('tooltip');
  expect(tip?.textContent).toContain('looks like noise');
  expect(tip?.textContent).toContain('likely noise');
  // And the common case whispers: a hairline, no fill.
  expect(tag.style.getPropertyValue('--pa-btn-bg')).toBe('transparent');
  // The row starts closed, and the tag is what opens it.
  expect(container.querySelector('[data-testid="pick-loop-detail"]')).toBeNull();
  await act(async () => { tag.click(); });
  expect(container.querySelector('[data-testid="pick-loop-detail"]')?.textContent)
    .toContain('Why it was filtered');
  expect(tag.getAttribute('aria-expanded')).toBe('true');
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

it('gives the staleness sentence the weight the rest of the card already has', async () => {
  // The card escalates correctly around this line — warning border, warning
  // tone on the hero figure — while the sentence that actually says the balance
  // is months old sat at the same muted caption weight as the routine "Live
  // file" line. The one line carrying the bad news was the quietest thing on
  // the card.
  apiFetch.mockResolvedValue(board({
    polybotEnabled: true,
    polybot: {
      found: true, paused: false, stale: true, staleDays: 110,
      credentialsReady: true, currentBalance: 1234.5,
      asOf: '2026-05-13T12:00:00Z',
    },
  }));
  await act(async () => { root.render(<FinanceView />); });
  await flush();
  const line = container.querySelector('[data-testid="finance-polybot-freshness"]') as HTMLElement;
  expect(line.textContent).toContain('110d stale');
  expect(line.getAttribute('data-stale')).toBe('true');
  // Not the quiet caption colour it used to share with "Live file".
  expect(line.style.color).toBeTruthy();
  expect(line.style.fontWeight).toBe('600');
  // A step up the ramp from the routine caption beside it.
  expect(line.style.fontSize).toBe('13px');
});

it('leaves a fresh reading quiet', async () => {
  apiFetch.mockResolvedValue(board({
    polybotEnabled: true,
    polybot: {
      found: true, paused: false, stale: false,
      credentialsReady: true, currentBalance: 1234.5,
      asOf: '2026-08-31T12:00:00Z',
    },
  }));
  await act(async () => { root.render(<FinanceView />); });
  await flush();
  const line = container.querySelector('[data-testid="finance-polybot-freshness"]') as HTMLElement;
  expect(line.getAttribute('data-stale')).toBe('false');
  // The caption ramp's own weight, not the emphasis the stale variant takes.
  expect(line.style.fontWeight).toBe('400');
  expect(line.style.fontSize).toBe('12px');
});

it('gives the pick row controls the primitive\'s press feedback', async () => {
  // Both controls on a pick row shipped in the same commit series as the Button
  // primitive and used none of it, so pressing either looked exactly like not
  // pressing it — an inline style cannot express :hover or :active at all.
  // They are disclosure toggles: what they need is the feedback, not the
  // pending/success machinery, and the aria pairing that says what they do.
  apiFetch.mockResolvedValue(board({
    pickerEnabled: true,
    picks: [{
      ticker: 'LEE', companyName: 'LEE Inc', rank: 1, score: 1,
      priceMismatch: false, fundamentals: { available: false },
      loop: { passed: true, kills: [], batchSize: 8 },
    }],
  }));
  await act(async () => { root.render(<FinanceView />); });
  await flush();
  const tag = container.querySelector('[data-testid="pick-loop-tag"]') as HTMLButtonElement;
  expect(tag.className).toContain('pa-btn');
  expect(tag.getAttribute('aria-expanded')).toBe('false');
  const row = container.querySelector('[data-testid="pick-row"]') as HTMLElement;
  const toggle = row.querySelector('button.pa-btn[aria-expanded]') as HTMLButtonElement;
  expect(toggle).toBeTruthy();
  expect(toggle.getAttribute('aria-controls')).toBeTruthy();
});

it('keeps a column of filtered rows quiet and spends the emphasis on the rest', async () => {
  // The screenshot this fixes: fifteen filled danger pills of ragged widths,
  // one per row, so the column read as a wall of alerts and the one row that
  // was different was lost inside it. The common case is now a dim hairline of
  // a uniform width; the fill is reserved for the rare name that passed, and
  // the Financier's approval stays the one loud thing in the row.
  const filtered = Array.from({ length: 15 }, (_, i) => ({
    ticker: `F${i}`, companyName: `F${i} Inc`, rank: i + 2, score: 1,
    priceMismatch: false, fundamentals: { available: false },
    loop: { passed: false, kills: ['in-sample ICIR 0.12 is below 0.3 — likely noise'], batchSize: 16 },
  }));
  apiFetch.mockResolvedValue(board({
    pickerEnabled: true,
    dailyPick: { day: '2026-08-31', asOf: '2026-08-31T12:00:00Z', ticker: 'WIN', why: 'held up', candidateCount: 16 },
    picks: [
      {
        ticker: 'WIN', companyName: 'Win Inc', rank: 1, score: 9,
        priceMismatch: false, fundamentals: { available: false },
        loop: { passed: true, kills: [], batchSize: 16 },
      },
      ...filtered,
    ],
  }));
  await act(async () => { root.render(<FinanceView />); });
  await flush();

  // Every row is on screen, not just the preview.
  const showAll = Array.from(container.querySelectorAll('button'))
    .find((b) => b.textContent?.startsWith('Show ')) as HTMLButtonElement;
  await act(async () => { showAll.click(); });

  const tags = Array.from(container.querySelectorAll('[data-testid="pick-loop-tag"]')) as HTMLElement[];
  expect(tags).toHaveLength(16);
  const quiet = tags.filter((t) => t.style.getPropertyValue('--pa-btn-bg') === 'transparent');
  const marked = tags.filter((t) => t.style.getPropertyValue('--pa-btn-bg') !== 'transparent');
  // Fifteen whisper; the one that passed does not.
  expect(quiet).toHaveLength(15);
  expect(marked).toHaveLength(1);
  expect(marked[0].textContent).toContain('signal checked');
  // Every filtered tag reads the same width, because it says the same word.
  expect(new Set(quiet.map((t) => t.textContent))).toEqual(new Set(['filteredⓘ']));

  // And the loudest mark in the column is still the rarest one.
  const badge = container.querySelector('[data-testid="pick-financier-badge"]') as HTMLElement;
  expect(badge).toBeTruthy();
  expect(badge.style.background).toBeTruthy();
});

it('a Picker scan reports the scan, not the accept-POST, and names what it found', async () => {
  vi.useFakeTimers();
  try {
    // The POST is accepted instantly; the scan itself takes two polls.
    let started = false;
    let ticks = 0;
    apiFetch.mockImplementation(async (path: string) => {
      if (path === '/api/finance/picker/scan') { started = true; return {}; }
      if (started) ticks += 1;
      const running = started && ticks <= 2;
      return board({
        pickerEnabled: true,
        picker: {
          reachable: true, baseUrl: 'http://127.0.0.1:8080',
          scanInProgress: running,
          detail: running ? 'Ranking the universe' : null,
          results: started && !running ? 41 : null,
          scanDate: '2026-09-01',
        },
      });
    });

    await act(async () => { root.render(<FinanceView />); });
    await act(async () => { await Promise.resolve(); await Promise.resolve(); });

    const scan = container.querySelector('[data-testid="picker-scan"]') as HTMLButtonElement;
    expect(scan).toBeTruthy();
    await act(async () => { scan.click(); });
    await act(async () => { await Promise.resolve(); await Promise.resolve(); });

    // In flight: the strip is up, and it does NOT claim a percentage the
    // scanner never gave.
    const strip = container.querySelector('[data-testid="job-progress"]') as HTMLElement;
    expect(strip).toBeTruthy();
    expect(strip.getAttribute('data-phase')).toBe('running');
    expect(container.querySelector('[data-testid="job-progress-percent"]')).toBeNull();
    expect(container.querySelector('[data-testid="job-progress-bar"]')?.getAttribute('data-determinate'))
      .toBe('false');

    // Two more polls and the scan lands — as a named success, not a vanish.
    for (let i = 0; i < 3; i++) {
      await act(async () => { await vi.advanceTimersByTimeAsync(5_000); });
    }
    const done = container.querySelector('[data-testid="job-progress"]') as HTMLElement;
    expect(done.getAttribute('data-phase')).toBe('succeeded');
    expect(done.textContent).toContain('41 ranked');
  } finally {
    vi.useRealTimers();
  }
});

// ── QA 2026-09-24 (real-size walk at a ~570px Finance column) ─────────────

const PNG_404 = 'market data source answered 404 Not Found: No data found, symbol may be delisted';

function lot(over: Record<string, unknown>) {
  return {
    id: String(over.symbol),
    symbol: 'VST',
    companyName: 'Vistra',
    entryDate: '2026-08-01',
    entryPrice: 150,
    shares: 10,
    source: 'ledger',
    quote: { symbol: String(over.symbol ?? 'VST'), price: 160, currency: 'USD', marketClosed: false },
    last: 160,
    unrealized: 100,
    unrealizedPct: 6.6,
    rsi: 55,
    ...over,
  };
}

function lotsBoard(rows: Record<string, unknown>[]) {
  return board({
    holdings: {
      source: 'ledger', openCount: rows.length, netUnrealized: 0, netRealized: 0, netPnl: 0, trend: [], rows,
    },
  });
}

it('shows "No price" in the mark cell, not the backend sentence, and hints the exchange suffix on a 404', async () => {
  apiFetch.mockResolvedValue(lotsBoard([
    lot({ symbol: 'VST' }),
    lot({ symbol: 'PNG', quote: null, last: null, unrealized: null, unrealizedPct: null, rsi: null, quoteError: PNG_404 }),
  ]));
  await act(async () => { root.render(<FinanceView />); });
  await flush();

  const cells = container.querySelectorAll('[data-testid="quote-no-price"]');
  expect(cells).toHaveLength(1);
  const cell = cells[0] as HTMLElement;
  // Visible text is the short label; the full reason is screen-reader text only.
  const visible = Array.from(cell.childNodes)
    .filter((n) => !(n instanceof HTMLElement && n.classList.contains('sr-only')))
    .map((n) => n.textContent)
    .join('');
  expect(visible.trim()).toBe('No price');
  expect(cell.querySelector('.sr-only')?.textContent).toContain('404 Not Found');
  expect(cell.hasAttribute('title')).toBe(false);
  expect(cell.tabIndex).toBe(0);

  const hints = container.querySelectorAll('[data-testid="lot-symbol-hint"]');
  expect(hints).toHaveLength(1);
  expect(hints[0].textContent).toContain('PNG.V');
  expect(hints[0].textContent).toContain('Edit');
});

it('does not suggest a suffix when the quote failed for another reason', async () => {
  apiFetch.mockResolvedValue(lotsBoard([
    lot({ symbol: 'IREN', quote: null, last: null, quoteError: 'could not reach the market data source: timed out' }),
  ]));
  await act(async () => { root.render(<FinanceView />); });
  await flush();
  expect(container.querySelector('[data-testid="quote-no-price"]')).toBeTruthy();
  expect(container.querySelector('[data-testid="lot-symbol-hint"]')).toBeNull();
});

it('keeps a lot\'s Edit / Close / Remove on one unwrapping line, with Remove named and still visible', async () => {
  apiFetch.mockResolvedValue(lotsBoard([lot({ symbol: 'BLNE' })]));
  await act(async () => { root.render(<FinanceView />); });
  await flush();
  const actions = container.querySelector('[data-testid="lot-actions"]') as HTMLElement;
  expect(actions.style.flexWrap).toBe('nowrap');
  const labels = Array.from(actions.querySelectorAll('button')).map((b) => b.textContent?.trim());
  expect(labels).toEqual(['Edit', 'Close', 'Remove']);
  expect(actions.querySelector('button[aria-label="Remove BLNE lot"]')).toBeTruthy();
});

it('does not reopen the top "Record a trade" form after cancelling a lot edit (regression: stale showForm)', async () => {
  apiFetch.mockResolvedValue(lotsBoard([lot({ symbol: 'BLNE' })]));
  await act(async () => { root.render(<FinanceView />); });
  await flush();

  // Open the top form (as if starting a brand-new trade), without submitting it.
  const recordBtn = Array.from(container.querySelectorAll('button'))
    .find((b) => b.textContent === 'Record a trade') as HTMLButtonElement;
  expect(recordBtn).toBeTruthy();
  await act(async () => { recordBtn.click(); });
  expect(container.querySelector('#finance-holdings-form')).toBeTruthy();

  // Switch to editing the existing lot instead — this used to leave the top
  // form's `showForm` flag stuck `true` even though the row's inline guard
  // hides it while `editing` is true.
  const editBtn = Array.from(container.querySelectorAll('button'))
    .find((b) => b.textContent === 'Edit') as HTMLButtonElement;
  await act(async () => { editBtn.click(); });
  expect(container.querySelector('#finance-holdings-form')).toBeNull();
  expect(container.querySelector('[data-testid="lot-edit-row"]')).toBeTruthy();

  // Cancel the edit via the inline row's own Cancel button.
  const cancelBtn = Array.from(container.querySelectorAll('[data-testid="lot-edit-row"] button'))
    .find((b) => b.textContent === 'Cancel') as HTMLButtonElement;
  expect(cancelBtn).toBeTruthy();
  await act(async () => { cancelBtn.click(); });

  // The top "Record a trade" form must stay closed — it must not silently
  // pop back open (with a blanked draft) just because editing ended.
  expect(container.querySelector('[data-testid="lot-edit-row"]')).toBeNull();
  expect(container.querySelector('#finance-holdings-form')).toBeNull();
  const toggleBtn = Array.from(container.querySelectorAll('button'))
    .find((b) => b.textContent === 'Record a trade' || b.textContent === 'Hide form') as HTMLButtonElement;
  expect(toggleBtn.textContent).toBe('Record a trade');
});

it('does not reopen the top form after cancelling a lot edit via the header "Hide form" button', async () => {
  apiFetch.mockResolvedValue(lotsBoard([lot({ symbol: 'BLNE' })]));
  await act(async () => { root.render(<FinanceView />); });
  await flush();

  const recordBtn = Array.from(container.querySelectorAll('button'))
    .find((b) => b.textContent === 'Record a trade') as HTMLButtonElement;
  await act(async () => { recordBtn.click(); });
  expect(container.querySelector('#finance-holdings-form')).toBeTruthy();

  const editBtn = Array.from(container.querySelectorAll('button'))
    .find((b) => b.textContent === 'Edit') as HTMLButtonElement;
  await act(async () => { editBtn.click(); });
  expect(container.querySelector('[data-testid="lot-edit-row"]')).toBeTruthy();

  // Cancel via the card header's own toggle button, which reads "Hide form"
  // while an edit is in progress.
  const hideFormBtn = Array.from(container.querySelectorAll('button'))
    .find((b) => b.textContent === 'Hide form') as HTMLButtonElement;
  expect(hideFormBtn).toBeTruthy();
  await act(async () => { hideFormBtn.click(); });

  expect(container.querySelector('[data-testid="lot-edit-row"]')).toBeNull();
  expect(container.querySelector('#finance-holdings-form')).toBeNull();
});

it('shows "—" for household figures when no transactions are imported, not a measured zero', async () => {
  await act(async () => { root.render(<FinanceView />); });
  await flush();
  expect(container.textContent).toContain('No imported transactions yet.');
  for (const label of ['30-day run-rate', '90-day run-rate', 'Spent (window)']) {
    const labelEl = Array.from(container.querySelectorAll('div')).find((d) => d.textContent === label);
    expect(labelEl, label).toBeTruthy();
    expect(labelEl!.nextElementSibling?.textContent).toBe('—');
  }
});

it('shows the household run-rate as money once transactions exist', async () => {
  apiFetch.mockResolvedValue(board({
    household: {
      recent: [{
        id: 't1', date: '2026-09-01', amount: -42.5, payee: 'Grocer', category: 'Groceries', createdAt: '2026-09-01T00:00:00Z',
      }],
      forecast: {
        daysUsed: 30, spend90d: 42.5, runRate30d: 42.5, runRate90d: 14.17,
        byCategory: [], recurring: [], method: 'trailing run-rate, not a model',
      },
    },
  }));
  await act(async () => { root.render(<FinanceView />); });
  await flush();
  const labelEl = Array.from(container.querySelectorAll('div')).find((d) => d.textContent === '30-day run-rate');
  expect(labelEl!.nextElementSibling?.textContent).toMatch(/42\.50/);
});

/**
 * F1 (dagK UX QA, 2026-09-25): the tab was one flex column capped at 1120px —
 * a ~690px column once the chat dock took its share — with 1,400+px of empty
 * gradient beside it on a wide display. jsdom has no layout, so this cannot
 * prove the grid activates at 1200px of *container* width (a real-browser
 * screenshot at 1100/1440/2560 does that — see harness-grow.html). What it can
 * prove: the two columns exist as distinct containers holding the right
 * sections, in the original top-to-bottom order, and the `@container` rule
 * that switches them into a grid is actually shipped.
 */
it('splits Holdings/Lots from Household/Watchlist/Notes into two column containers', async () => {
  await act(async () => { root.render(<FinanceView />); });
  await flush();
  const left = container.querySelector('[data-testid="finance-col-holdings"]');
  const right = container.querySelector('[data-testid="finance-col-household"]');
  expect(left, 'left column').toBeTruthy();
  expect(right, 'right column').toBeTruthy();
  expect(left!.textContent).toContain('Lots'); // the Holdings/Lots section heading
  expect(right!.textContent).toContain('Household');
  expect(right!.textContent).toContain('Watchlist');
  expect(right!.textContent).toContain('Notes');
  // Document order: the grid's left/right split only takes effect above
  // 1200px of container width; below it the two divs stack in DOM order,
  // which must still be Holdings first (unchanged behaviour, not a new one).
  expect(left!.compareDocumentPosition(right!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();

  const css = document.getElementById('pa-finance-view')?.textContent ?? '';
  expect(css).toContain('@container (min-width: 1200px)');
  expect(css).toContain('.pa-fin-cols { display: grid;');
});
