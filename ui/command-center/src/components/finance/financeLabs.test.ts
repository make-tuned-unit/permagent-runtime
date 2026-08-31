import { describe, expect, it } from 'vitest';
import {
  MAX_UNIVERSE,
  PICKER_DISCLAIMER,
  POLYBOT_DISCLAIMER,
  parseUniverse,
  appendUniverse,
  killSummary,
  loopTagLabel,
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

  it('appends without replacing or duplicating', () => {
    expect(appendUniverse(['AAPL'], 'shop.to aapl MSFT')).toEqual(['AAPL', 'SHOP.TO', 'MSFT']);
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

describe('killSummary', () => {
  // The exact sentences the daemon emits (crates/goose/src/pick_loop.rs).
  it('turns each server kill reason into a readable phrase', () => {
    expect(killSummary(['not enough daily history to run the loop (need ~40 paired days)']))
      .toBe('too little history');
    expect(killSummary(['in-sample ICIR 0.12 is below 0.3 — likely noise']))
      .toBe('looks like noise');
    expect(killSummary(['signal half-life 2.4d is under 5d'])).toBe('fades too fast');
    expect(killSummary(['out-of-sample ICIR 0.05 dropped more than 50% from in-sample 0.40 — overfit']))
      .toBe('did not hold up');
    expect(killSummary(['failed Bonferroni gate for 8 picks tested in this batch']))
      .toBe('too many names tested');
    expect(killSummary(['in-sample ICIR could not be computed'])).toBe('not measurable');
  });

  it('falls back rather than showing an empty tag', () => {
    expect(killSummary([])).toBe('filtered out');
    expect(killSummary(['   '])).toBe('filtered out');
    expect(killSummary(['something new from the daemon'])).toBe('filtered out');
  });
});

describe('loopTagLabel', () => {
  it('says what happened instead of "loop kill"', () => {
    expect(loopTagLabel({ passed: false, kills: ['in-sample ICIR 0.12 is below 0.3 — likely noise'] }))
      .toBe('filtered: looks like noise');
    expect(loopTagLabel({ passed: true, kills: [] })).toBe('signal checked');
    expect(loopTagLabel(null)).toBe('');
  });
});
