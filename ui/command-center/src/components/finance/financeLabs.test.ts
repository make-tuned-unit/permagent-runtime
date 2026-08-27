import { describe, expect, it } from 'vitest';
import {
  MAX_UNIVERSE,
  PICKER_DISCLAIMER,
  POLYBOT_DISCLAIMER,
  parseUniverse,
  pickIsApproved,
  requiredKeysSet,
  sortPicks,
} from './financeLabs';

describe('parseUniverse', () => {
  it('splits mixed separators, uppercases, and dedupes', () => {
    expect(parseUniverse('aapl, SHOP.TO\nbrk.b; $msft  aapl')).toEqual([
      'AAPL', 'SHOP.TO', 'BRK.B', 'MSFT',
    ]);
  });

  it('drops non-tickers', () => {
    expect(parseUniverse('!!! 123')).toEqual([]);
    expect(parseUniverse('OK')).toEqual(['OK']);
    expect(parseUniverse('123')).toEqual([]);
  });

  it('caps the dump', () => {
    const raw = Array.from({ length: MAX_UNIVERSE + 10 }, (_, i) => `T${i}`).join(',');
    expect(parseUniverse(raw)).toHaveLength(MAX_UNIVERSE);
  });
});

describe('Financier approval sort', () => {
  it('puts the approved ticker first even when it failed the loop', () => {
    const ranked = sortPicks([
      { ticker: 'LEE', loop: { passed: false }, rank: 1 },
      { ticker: 'SHOP.TO', loop: { passed: true }, rank: 8 },
      { ticker: 'CDZI', loop: { passed: false }, rank: 2 },
    ], 'SHOP.TO');
    expect(ranked.map((p) => p.ticker)).toEqual(['SHOP.TO', 'LEE', 'CDZI']);
  });

  it('matches approval case-insensitively', () => {
    expect(pickIsApproved('shop.to', 'SHOP.TO')).toBe(true);
    expect(pickIsApproved('LEE', null)).toBe(false);
  });
});

describe('disclaimer copy', () => {
  it('names real orders and real losses for Polybot', () => {
    expect(POLYBOT_DISCLAIMER).toMatch(/real orders/i);
    expect(POLYBOT_DISCLAIMER).toMatch(/lose the entire bankroll/i);
  });

  it('says Picker does not place brokerage orders', () => {
    expect(PICKER_DISCLAIMER).toMatch(/does not place brokerage orders/i);
  });
});

describe('requiredKeysSet', () => {
  it('counts only required fields', () => {
    const fields = [
      { key: 'A', required: true },
      { key: 'B', required: true },
      { key: 'C', required: false },
    ];
    expect(requiredKeysSet(fields, { A: 'xx**', B: '', C: 'yy**' })).toEqual({ have: 1, need: 2 });
  });
});
