/**
 * Reading a quote failure the backend hands the Finance tab.
 *
 * The API gives the UI only a string (`quoteError`), built in
 * `crates/goose/src/market_data.rs` as
 * `market data source answered {status}: {detail}` — `{status}` is reqwest's
 * `StatusCode` display, so an unknown symbol reads
 * `market data source answered 404 Not Found: No data found, symbol may be delisted`.
 * There is no structured status field, so the match is on the status code in
 * that one sentence, and it lives here, in one tested place.
 */

export type QuoteFailureKind = 'unknown-symbol' | 'other';

const NOT_FOUND = /\banswered 404\b/;

/** `unknown-symbol` when the market data source said 404 for this symbol. */
export function classifyQuoteError(error: string | null | undefined): QuoteFailureKind | null {
  if (!error) return null;
  return NOT_FOUND.test(error) ? 'unknown-symbol' : 'other';
}

/**
 * The one-line fix for an unknown symbol on a lot. Yahoo keys non-US listings
 * by an exchange suffix (`.TO` Toronto, `.V` TSX Venture), and a bare ticker
 * typed from a Canadian broker statement is the commonest way to get here.
 * A symbol that already carries a suffix gets the plain version.
 */
export function unknownSymbolHint(symbol: string): string {
  const s = symbol.trim().toUpperCase();
  if (!s || s.includes('.')) {
    return 'Symbol not found. Check the ticker and its exchange suffix with Edit.';
  }
  return `Not found. Use Edit to add the exchange suffix: ${s}.TO (Toronto), ${s}.V (TSX Venture).`;
}
