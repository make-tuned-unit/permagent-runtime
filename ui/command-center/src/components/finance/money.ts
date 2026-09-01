/**
 * Money — one formatter for every figure on the Finance tab, and the rules
 * that keep a converted number from pretending to be a recorded one.
 *
 * Finance renders money in nineteen places across six sections: a hero
 * balance, four run-rates, a P&L column, two prices on every pick row, a
 * transaction ledger. They already shared `fmtMoney`/`fmtSigned` — this module
 * is where those move so that adding a display currency is one change rather
 * than nineteen, and so the conversion rules below are impossible to skip at a
 * call site.
 *
 * ## The rules, which are honesty rules and not formatting preferences
 *
 * 1. **Conversion is display, never data.** Nothing here writes anything. The
 *    journal, the imported transactions and the daemon's own figures stay in
 *    the currency they were recorded in; a reader who switches to CAD changes
 *    what they are shown, not what is stored.
 *
 * 2. **A converted figure says so in its own prefix.** `$1,000.00` and
 *    `CA$1,370.00` are different strings, so a converted number can never be
 *    mistaken for the recorded one at a glance. This is why the locale is
 *    **pinned**: with the browser's own locale, a machine set to en-CA renders
 *    USD as `US$` and CAD as `$` — the exact inversion of the cue, on the one
 *    reader most likely to want CAD. Pinned, the mark is the same everywhere.
 *
 * 3. **A missing rate is never guessed.** If the rate for a pair is not known,
 *    the amount renders in the currency it is already in. There is no
 *    "approximately", no last-known-good silently reused as current (the
 *    caller decides what is fresh enough — see `displayCurrency.ts`), and
 *    never a CA$ prefix on a number that was not converted.
 *
 * 4. **A figure carrying its own currency keeps it.** A quote for a TSX name
 *    arrives priced in CAD; the daemon says so on the quote. That number is
 *    not a USD amount and is not converted as one — `source` is the currency
 *    the number is *in*, and when it already matches the display currency
 *    nothing is converted at all.
 */

/** ISO 4217, uppercase. */
export type CurrencyCode = string;

/**
 * Everything the daemon reports without saying otherwise is USD, and the rate
 * table is expressed as "units of X per 1 USD" so a second currency is one
 * number rather than a matrix.
 */
export const BASE_CURRENCY = 'USD';

export interface CurrencyOption {
  code: CurrencyCode;
  /** The name, in words, for the picker and for the fallback sentence. */
  label: string;
}

/**
 * The list is deliberately open: adding a currency is one line here plus the
 * daemon knowing its rate. It is not a list of currencies Finance *supports*
 * in any deeper sense — every one of them is a display conversion over the
 * same stored USD.
 */
export const DISPLAY_CURRENCIES: readonly CurrencyOption[] = [
  { code: 'USD', label: 'US dollars' },
  { code: 'CAD', label: 'Canadian dollars' },
];

/** Units of the keyed currency per 1 USD. `{ USD: 1, CAD: 1.37 }`. */
export type RateTable = Readonly<Record<string, number>>;

/** What is always true, and needs no network to know. */
export const BASE_RATES: RateTable = { [BASE_CURRENCY]: 1 };

/**
 * Pinned so the prefix is a reliable cue rather than a function of the
 * reader's OS — see rule 2. English is also the only language the rest of this
 * interface speaks, so this borrows nothing it wasn't already assuming.
 */
const MONEY_LOCALE = 'en-US';

export function currencyLabel(code: CurrencyCode): string {
  return DISPLAY_CURRENCIES.find((c) => c.code === code)?.label ?? code;
}

/** A code we can hand to `Intl`, or null. Never a silent fallback to USD. */
export function normalizeCurrency(raw: string | null | undefined): CurrencyCode | null {
  if (!raw) return null;
  const code = raw.trim().toUpperCase();
  return /^[A-Z]{3}$/.test(code) ? code : null;
}

export function isDisplayCurrency(raw: string | null | undefined): boolean {
  const code = normalizeCurrency(raw);
  return code != null && DISPLAY_CURRENCIES.some((c) => c.code === code);
}

/**
 * `from` → `to` through the USD base. Returns null when either leg is unknown,
 * which is the whole point: the caller then renders the original rather than
 * inventing a number.
 */
export function convert(
  amount: number,
  from: CurrencyCode,
  to: CurrencyCode,
  rates: RateTable | null | undefined,
): number | null {
  if (from === to) return amount;
  const perUsdFrom = from === BASE_CURRENCY ? 1 : rates?.[from];
  const perUsdTo = to === BASE_CURRENCY ? 1 : rates?.[to];
  if (!Number.isFinite(perUsdFrom) || !Number.isFinite(perUsdTo)) return null;
  if (!perUsdFrom || !perUsdTo) return null;
  return (amount / (perUsdFrom as number)) * (perUsdTo as number);
}

export interface MoneyOptions {
  /** The currency the number is already in. Defaults to USD. */
  source?: string | null;
  /** What the reader asked to see. Defaults to `source` — no conversion. */
  display?: string | null;
  /** Units per 1 USD. Absent or incomplete means "do not convert". */
  rates?: RateTable | null;
}

/** The em-dash every Finance surface already uses for "no number". */
const NOTHING = '—';

function render(amount: number, currency: CurrencyCode): string {
  try {
    return new Intl.NumberFormat(MONEY_LOCALE, {
      style: 'currency',
      currency,
      maximumFractionDigits: 2,
    }).format(amount);
  } catch {
    // An unknown code is not a reason to lose the figure.
    return amount.toFixed(2);
  }
}

/**
 * The one money formatter. Rounds once, at render — a converted figure is
 * rounded from the converted value, never from an already-rounded one.
 */
export function formatMoney(
  amount: number | null | undefined,
  options: MoneyOptions = {},
): string {
  if (amount == null || Number.isNaN(amount)) return NOTHING;
  const source = normalizeCurrency(options.source) ?? BASE_CURRENCY;
  const display = normalizeCurrency(options.display) ?? source;
  if (display === source) return render(amount, source);
  const converted = convert(amount, source, display, options.rates);
  // Rule 3: no rate, no claim. The figure renders as what it is.
  if (converted == null) return render(amount, source);
  return render(converted, display);
}

/** P&L, where the sign is the headline. `−` is U+2212, not a hyphen. */
export function formatSigned(
  amount: number | null | undefined,
  options: MoneyOptions = {},
): string {
  if (amount == null || Number.isNaN(amount)) return NOTHING;
  const abs = formatMoney(Math.abs(amount), options);
  if (amount < 0) return `−${abs}`;
  if (amount > 0) return `+${abs}`;
  return abs;
}

/** A percentage change. Never converted — a percentage has no currency. */
export function formatPercent(n: number | null | undefined): string {
  if (n == null || Number.isNaN(n)) return '';
  const sign = n > 0 ? '+' : '';
  return `${sign}${n.toFixed(2)}%`;
}

/** "1 USD = 1.37 CAD" — the rate itself, stated once per view. */
export function rateLine(
  display: CurrencyCode,
  rates: RateTable | null | undefined,
): string | null {
  const rate = convert(1, BASE_CURRENCY, display, rates);
  if (rate == null || display === BASE_CURRENCY) return null;
  const figure = new Intl.NumberFormat(MONEY_LOCALE, {
    minimumFractionDigits: 2,
    maximumFractionDigits: 4,
  }).format(rate);
  return `1 ${BASE_CURRENCY} = ${figure} ${display}`;
}

/**
 * A formatter bound to one reader's choice, so a call site renders a figure
 * without restating the conversion rules — and cannot forget them.
 */
export interface Money {
  /** What the reader asked for. */
  requested: CurrencyCode;
  /** What is actually being rendered — falls back to USD with no rate. */
  display: CurrencyCode;
  /** True when figures on screen are converted rather than recorded. */
  converting: boolean;
  rates: RateTable;
  fmt(amount: number | null | undefined, options?: Omit<MoneyOptions, 'display' | 'rates'>): string;
  signed(amount: number | null | undefined, options?: Omit<MoneyOptions, 'display' | 'rates'>): string;
  pct(n: number | null | undefined): string;
}

/**
 * `display` is what was asked for; the rate table decides what is possible.
 * When the rate is missing the whole view falls back to USD together — a
 * half-converted board, some rows in one currency and some in another, is
 * worse than either currency alone.
 */
export function makeMoney(requested: CurrencyCode, rates: RateTable | null | undefined): Money {
  const table: RateTable = rates ?? BASE_RATES;
  const reachable = convert(1, BASE_CURRENCY, requested, table) != null;
  const display = reachable ? requested : BASE_CURRENCY;
  const opts = (o?: Omit<MoneyOptions, 'display' | 'rates'>): MoneyOptions =>
    ({ ...o, display, rates: table });
  return {
    requested,
    display,
    converting: display !== BASE_CURRENCY,
    rates: table,
    fmt: (amount, o) => formatMoney(amount, opts(o)),
    signed: (amount, o) => formatSigned(amount, opts(o)),
    pct: formatPercent,
  };
}

/** The plain USD formatter — what every surface got before there was a choice. */
export const USD_MONEY: Money = makeMoney(BASE_CURRENCY, BASE_RATES);
