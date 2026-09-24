import { describe, expect, it } from 'vitest';

import { classifyQuoteError, unknownSymbolHint } from './quoteError';

describe('classifyQuoteError', () => {
  it('reads the backend 404 sentence as an unknown symbol', () => {
    expect(classifyQuoteError(
      'market data source answered 404 Not Found: No data found, symbol may be delisted',
    )).toBe('unknown-symbol');
  });

  it('keeps every other failure generic', () => {
    expect(classifyQuoteError('market data source answered 429 Too Many Requests: no detail given')).toBe('other');
    expect(classifyQuoteError('market data source answered 500 Internal Server Error: no detail given')).toBe('other');
    expect(classifyQuoteError('could not reach the market data source: timed out')).toBe('other');
    // A 404 inside some other number is not a 404 status.
    expect(classifyQuoteError('market data source answered 4040 odd')).toBe('other');
  });

  it('has nothing to classify when there is no error', () => {
    expect(classifyQuoteError(null)).toBeNull();
    expect(classifyQuoteError(undefined)).toBeNull();
    expect(classifyQuoteError('')).toBeNull();
  });
});

describe('unknownSymbolHint', () => {
  it('suggests the exchange suffixes for the symbol it was given, and points at Edit', () => {
    const hint = unknownSymbolHint('png');
    expect(hint).toContain('PNG.TO');
    expect(hint).toContain('PNG.V');
    expect(hint).toContain('Edit');
    expect(unknownSymbolHint('ABC')).toContain('ABC.V');
  });

  it('does not stack a second suffix on a symbol that already has one', () => {
    const hint = unknownSymbolHint('PNG.V');
    expect(hint).not.toContain('PNG.V.');
    expect(hint).toContain('Edit');
  });
});
