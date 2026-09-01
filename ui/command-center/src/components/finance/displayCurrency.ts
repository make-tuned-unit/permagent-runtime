/**
 * Display currency — the reader's choice, and the rate that makes it possible.
 *
 * Two separate things live here because they fail separately:
 *
 *   * **The preference** is a client display setting, persisted exactly the way
 *     the app's other display settings are (theme, density, reduce-motion in
 *     `styles/tokens.ts`): localStorage, a listener set, and a `storage`
 *     listener so a second window follows the first. It never fails and never
 *     needs the daemon — picking CAD is remembered whether or not a rate is
 *     ever found.
 *
 *   * **The rate** is live data, and is therefore governed by R1: it carries an
 *     `asOf`, it is marked when it gets old, and when it cannot be had the
 *     surface says so rather than showing a figure it cannot back. There is no
 *     hard-coded rate anywhere in this file, and there never may be — a
 *     plausible-looking constant is precisely the lie the liveness contract
 *     exists to prevent.
 *
 * ## The rate endpoint
 *
 * The daemon prices every other figure on this tab through `market_data`
 * (Yahoo, no key), which quotes FX pairs under the same symbols it quotes
 * equities: `CAD=X` is USD→CAD. Nothing exposes that over HTTP yet — the
 * finance routes only quote symbols the user has already stored — so this
 * client asks for `GET /api/finance/fx?base=USD&quote=CAD` and treats the 404
 * it currently gets as "rate unavailable", which is a state the caption knows
 * how to render. The day the route lands, CAD starts working with no client
 * change.
 *
 * Expected shape:
 *   { "base": "USD", "rates": { "CAD": 1.3712 },
 *     "asOf": "2026-08-31T20:00:00Z", "source": "yahoo:CAD=X" }
 */

import { useCallback, useEffect, useState } from 'react';

import { apiFetch } from '../../lib/api';
import {
  BASE_CURRENCY, BASE_RATES, isDisplayCurrency, normalizeCurrency,
  type CurrencyCode, type RateTable,
} from './money';

// ── The preference ───────────────────────────────────────────────────

/** Namespaced like every other `permagent-*` display preference. */
export const DISPLAY_CURRENCY_KEY = 'permagent-finance-currency';

const _listeners = new Set<() => void>();

function _notify() { _listeners.forEach((fn) => fn()); }

function _read(): CurrencyCode {
  try {
    const raw = localStorage.getItem(DISPLAY_CURRENCY_KEY);
    // A code we no longer offer reads as the default rather than as a currency
    // with no rate and no entry in the picker.
    return isDisplayCurrency(raw) ? (normalizeCurrency(raw) as CurrencyCode) : BASE_CURRENCY;
  } catch {
    return BASE_CURRENCY;
  }
}

export function getDisplayCurrency(): CurrencyCode {
  return _read();
}

export function setDisplayCurrency(code: CurrencyCode) {
  const next = isDisplayCurrency(code) ? (normalizeCurrency(code) as CurrencyCode) : BASE_CURRENCY;
  try { localStorage.setItem(DISPLAY_CURRENCY_KEY, next); } catch { /* private mode */ }
  _notify();
}

export function onDisplayCurrencyChange(fn: () => void): () => void {
  _listeners.add(fn);
  return () => { _listeners.delete(fn); };
}

// A second window (a detached Finance pane) follows the one the choice was
// made in — the same cross-window rule the theme already keeps.
if (typeof window !== 'undefined') {
  window.addEventListener('storage', (e) => {
    if (e.key === DISPLAY_CURRENCY_KEY) _notify();
  });
}

/** The choice, and the setter, kept current across windows. */
export function useDisplayCurrency(): [CurrencyCode, (code: CurrencyCode) => void] {
  const [code, setCode] = useState(getDisplayCurrency);
  useEffect(() => onDisplayCurrencyChange(() => setCode(getDisplayCurrency())), []);
  return [code, useCallback((next: CurrencyCode) => setDisplayCurrency(next), [])];
}

// ── The rate ─────────────────────────────────────────────────────────

export const FX_ENDPOINT = '/api/finance/fx';

/** Re-ask this often while a screen is open. FX closes daily; this is polite. */
export const FX_REFRESH_MS = 30 * 60_000;

/** Past this the reading is still shown, and shown as old. */
export const FX_STALE_AFTER_MS = 24 * 60 * 60_000;

/**
 * Past this it is not shown at all. A rate a week old is not a rate — the
 * bounded ceiling is what keeps the cache from turning into a hard-coded
 * constant that ages out of sight.
 */
export const FX_MAX_AGE_MS = 7 * 24 * 60 * 60_000;

const FX_CACHE_KEY = 'permagent-finance-fx';

export type FxStatus = 'ready' | 'loading' | 'unavailable';

export interface FxReading {
  rates: RateTable | null;
  /** When the rate was true, straight from the source. */
  asOf: string | number | null;
  status: FxStatus;
}

interface FxEntry {
  code: CurrencyCode;
  rate: number;
  asOf: string | number | null;
  /** When this client read it — the ceiling is measured on our own clock. */
  at: number;
}

interface FxWire {
  base?: string;
  rates?: Record<string, number>;
  asOf?: string | null;
  as_of?: string | null;
  source?: string | null;
}

function readCache(code: CurrencyCode, now: number): FxEntry | null {
  try {
    const raw = localStorage.getItem(FX_CACHE_KEY);
    if (!raw) return null;
    const entry = JSON.parse(raw) as Partial<FxEntry>;
    if (entry.code !== code) return null;
    if (typeof entry.rate !== 'number' || !Number.isFinite(entry.rate) || entry.rate <= 0) return null;
    if (typeof entry.at !== 'number' || now - entry.at > FX_MAX_AGE_MS) return null;
    return { code, rate: entry.rate, asOf: entry.asOf ?? null, at: entry.at };
  } catch {
    return null;
  }
}

function writeCache(entry: FxEntry) {
  try { localStorage.setItem(FX_CACHE_KEY, JSON.stringify(entry)); } catch { /* private mode */ }
}

/** Turns one entry into the table `money.ts` converts through. */
function tableOf(entry: FxEntry | null): RateTable | null {
  return entry ? { ...BASE_RATES, [entry.code]: entry.rate } : null;
}

export async function fetchFxRate(code: CurrencyCode): Promise<FxEntry> {
  const wire = await apiFetch<FxWire>(
    `${FX_ENDPOINT}?base=${encodeURIComponent(BASE_CURRENCY)}&quote=${encodeURIComponent(code)}`,
  );
  const rate = wire?.rates?.[code];
  if (typeof rate !== 'number' || !Number.isFinite(rate) || rate <= 0) {
    throw new Error(`No ${BASE_CURRENCY}→${code} rate in the answer`);
  }
  return { code, rate, asOf: wire.asOf ?? wire.as_of ?? null, at: Date.now() };
}

/**
 * The rate for one display currency. USD needs none and asks for none — the
 * default reader never makes a request.
 */
export function useFxRates(code: CurrencyCode): FxReading {
  const base = code === BASE_CURRENCY;
  const [entry, setEntry] = useState<FxEntry | null>(() => (base ? null : readCache(code, Date.now())));
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    if (base) return;
    let live = true;
    const cached = readCache(code, Date.now());
    setEntry(cached);
    setFailed(false);

    const pull = () => {
      void fetchFxRate(code).then(
        (next) => {
          if (!live) return;
          writeCache(next);
          setEntry(next);
          setFailed(false);
        },
        () => {
          // A failed fetch does not discard a cached reading that is still
          // inside its ceiling — it just stops it getting any younger.
          if (live) setFailed(true);
        },
      );
    };

    pull();
    const timer = setInterval(pull, FX_REFRESH_MS);
    return () => { live = false; clearInterval(timer); };
  }, [base, code]);

  if (base) return { rates: BASE_RATES, asOf: null, status: 'ready' };
  if (entry) return { rates: tableOf(entry), asOf: entry.asOf ?? entry.at, status: 'ready' };
  return { rates: null, asOf: null, status: failed ? 'unavailable' : 'loading' };
}
