/**
 * The formatter's job is not to be pretty. It is to make a converted figure
 * impossible to mistake for a recorded one, and to refuse to render a
 * conversion it cannot back.
 */

import { describe, expect, it } from 'vitest';

import {
  BASE_CURRENCY,
  BASE_RATES,
  convert,
  currencyLabel,
  formatMoney,
  formatPercent,
  formatSigned,
  isDisplayCurrency,
  makeMoney,
  normalizeCurrency,
  rateLine,
  USD_MONEY,
} from './money';

const RATES = { USD: 1, CAD: 1.37 };

describe('formatMoney — the prefix is the mark', () => {
  it('renders US dollars with the bare dollar sign', () => {
    expect(formatMoney(1234.5)).toBe('$1,234.50');
  });

  it('renders a converted figure with a prefix that cannot be read as USD', () => {
    // The whole honesty claim: 1000 US dollars and the same money shown in
    // Canadian dollars are different strings on screen.
    expect(formatMoney(1000, { display: 'CAD', rates: RATES })).toBe('CA$1,370.00');
    expect(formatMoney(1000)).toBe('$1,000.00');
  });

  it('does not depend on the machine it runs on', () => {
    // With the browser's own locale, en-CA renders USD as "US$" and CAD as
    // "$" — the cue inverted, on exactly the reader most likely to pick CAD.
    // The locale is pinned, so this holds wherever the test runs.
    expect(formatMoney(1, { display: 'CAD', rates: RATES })).toMatch(/^CA\$/);
    expect(formatMoney(1)).toMatch(/^\$/);
  });

  it('rounds the converted value, not an already-rounded one', () => {
    // 10.005 → 13.70685 in CAD. Rounding first would show CA$13.71 from
    // $10.01; rounding once shows CA$13.71 from the true product. The figure
    // that matters is that only one rounding happened.
    expect(formatMoney(10.005, { display: 'CAD', rates: RATES })).toBe('CA$13.71');
    expect(formatMoney(0.004, { display: 'CAD', rates: RATES })).toBe('CA$0.01');
    expect(formatMoney(0.001, { display: 'CAD', rates: RATES })).toBe('CA$0.00');
  });

  it('says nothing rather than zero when there is no number', () => {
    expect(formatMoney(null)).toBe('—');
    expect(formatMoney(undefined)).toBe('—');
    expect(formatMoney(Number.NaN)).toBe('—');
  });
});

describe('formatMoney — a rate it does not have is not a rate', () => {
  it('renders the original currency when the pair is unknown', () => {
    expect(formatMoney(1000, { display: 'CAD', rates: BASE_RATES })).toBe('$1,000.00');
    expect(formatMoney(1000, { display: 'CAD', rates: null })).toBe('$1,000.00');
    expect(formatMoney(1000, { display: 'EUR', rates: RATES })).toBe('$1,000.00');
  });

  it('leaves a figure that already carries its own currency alone', () => {
    // A TSX quote arrives priced in CAD. Asked for CAD, nothing is converted.
    expect(formatMoney(50, { source: 'CAD', display: 'CAD', rates: RATES })).toBe('CA$50.00');
    // Asked for USD, it converts back through the same rate.
    expect(formatMoney(137, { source: 'CAD', display: 'USD', rates: RATES })).toBe('$100.00');
  });

  it('treats a figure with no stated currency as the base', () => {
    expect(formatMoney(10, { source: null, display: 'CAD', rates: RATES })).toBe('CA$13.70');
    expect(formatMoney(10, { source: 'usd', display: 'CAD', rates: RATES })).toBe('CA$13.70');
  });
});

describe('formatSigned', () => {
  it('leads with the sign, using a minus and not a hyphen', () => {
    expect(formatSigned(12.5)).toBe('+$12.50');
    expect(formatSigned(-12.5)).toBe('−$12.50');
    expect(formatSigned(0)).toBe('$0.00');
    expect(formatSigned(null)).toBe('—');
  });

  it('converts through the same rules as the unsigned figure', () => {
    expect(formatSigned(-100, { display: 'CAD', rates: RATES })).toBe('−CA$137.00');
    expect(formatSigned(-100, { display: 'CAD', rates: null })).toBe('−$100.00');
  });
});

describe('formatPercent — a percentage has no currency', () => {
  it('is untouched by the display currency', () => {
    expect(formatPercent(2.5)).toBe('+2.50%');
    expect(formatPercent(-2.5)).toBe('-2.50%');
    expect(formatPercent(null)).toBe('');
  });
});

describe('convert', () => {
  it('goes both ways through the base', () => {
    expect(convert(100, 'USD', 'CAD', RATES)).toBeCloseTo(137);
    expect(convert(137, 'CAD', 'USD', RATES)).toBeCloseTo(100);
    expect(convert(5, 'CAD', 'CAD', RATES)).toBe(5);
  });

  it('returns null rather than a guess', () => {
    expect(convert(100, 'USD', 'EUR', RATES)).toBeNull();
    expect(convert(100, 'USD', 'CAD', null)).toBeNull();
    expect(convert(100, 'USD', 'CAD', { USD: 1, CAD: 0 })).toBeNull();
  });
});

describe('rateLine', () => {
  it('states the rate the figures were converted at', () => {
    expect(rateLine('CAD', RATES)).toBe('1 USD = 1.37 CAD');
    expect(rateLine('CAD', { USD: 1, CAD: 1.3712 })).toBe('1 USD = 1.3712 CAD');
  });

  it('has nothing to say about the base currency, or about a missing rate', () => {
    expect(rateLine('USD', BASE_RATES)).toBeNull();
    expect(rateLine('CAD', BASE_RATES)).toBeNull();
  });
});

describe('makeMoney — the whole board falls back together', () => {
  it('converts every figure once a rate is in hand', () => {
    const money = makeMoney('CAD', RATES);
    expect(money.display).toBe('CAD');
    expect(money.converting).toBe(true);
    expect(money.fmt(100)).toBe('CA$137.00');
    expect(money.signed(-100)).toBe('−CA$137.00');
  });

  it('falls back to the base currency entirely when the rate is missing', () => {
    // Not half a board: no CA$ prefix appears anywhere, because no figure was
    // converted anywhere.
    const money = makeMoney('CAD', null);
    expect(money.requested).toBe('CAD');
    expect(money.display).toBe(BASE_CURRENCY);
    expect(money.converting).toBe(false);
    expect(money.fmt(100)).toBe('$100.00');
    expect(money.signed(-100)).toBe('−$100.00');
  });

  it('asks nothing of a reader who never left US dollars', () => {
    expect(USD_MONEY.converting).toBe(false);
    expect(USD_MONEY.fmt(1)).toBe('$1.00');
  });
});

describe('the currency list', () => {
  it('recognises what it offers and rejects what it does not', () => {
    expect(isDisplayCurrency('CAD')).toBe(true);
    expect(isDisplayCurrency('cad')).toBe(true);
    expect(isDisplayCurrency('EUR')).toBe(false);
    expect(isDisplayCurrency('')).toBe(false);
    expect(isDisplayCurrency(null)).toBe(false);
  });

  it('normalises only what looks like a currency code', () => {
    expect(normalizeCurrency(' cad ')).toBe('CAD');
    expect(normalizeCurrency('dollars')).toBeNull();
    expect(normalizeCurrency(undefined)).toBeNull();
  });

  it('names each currency in words, for the fallback sentence', () => {
    expect(currencyLabel('USD')).toBe('US dollars');
    expect(currencyLabel('CAD')).toBe('Canadian dollars');
  });
});
