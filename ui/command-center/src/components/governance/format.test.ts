import { describe, expect, it } from 'vitest';
import { bandColor, bandLabel, formatTokens, formatUsd, timeAgo } from './format';
import type { ThemeColors } from '../../styles/tokens';

const colors = {
  warning: '#WARN',
  danger: '#DANGER',
  textDim: '#DIM',
} as unknown as ThemeColors;

describe('formatUsd', () => {
  it('shows two decimals for normal amounts', () => {
    expect(formatUsd(0)).toBe('$0.00');
    expect(formatUsd(3.5)).toBe('$3.50');
    expect(formatUsd(12.005)).toBe('$12.01');
  });
  it('keeps sub-cent spend visible (never rounds a real cost to $0.00)', () => {
    expect(formatUsd(0.003)).toBe('$0.0030');
  });
  it('is defensive against non-finite input', () => {
    expect(formatUsd(NaN)).toBe('$0.00');
  });
});

describe('formatTokens', () => {
  it('compacts thousands and millions', () => {
    expect(formatTokens(0)).toBe('0');
    expect(formatTokens(950)).toBe('950');
    expect(formatTokens(1500)).toBe('1.5k');
    expect(formatTokens(2_400_000)).toBe('2.4M');
  });
});

describe('bandColor', () => {
  it('maps bands to semantic colors, ok stays muted', () => {
    expect(bandColor('ok', colors)).toBe('#DIM');
    expect(bandColor('soft', colors)).toBe('#WARN');
    expect(bandColor('gate', colors)).toBe('#DANGER');
    expect(bandColor('hard', colors)).toBe('#DANGER');
  });
});

describe('bandLabel', () => {
  it('gives a human sentence per band', () => {
    expect(bandLabel('ok')).toMatch(/within budget/);
    expect(bandLabel('gate')).toMatch(/gate/);
  });
});

describe('timeAgo', () => {
  it('returns empty string for unparseable input', () => {
    expect(timeAgo('')).toBe('');
    expect(timeAgo('not-a-date')).toBe('');
  });
  it('handles SQLite space-separated UTC timestamps', () => {
    const recent = new Date(Date.now() - 5 * 60 * 1000)
      .toISOString()
      .replace('T', ' ')
      .replace(/\.\d+Z$/, '');
    expect(timeAgo(recent)).toBe('5m ago');
  });
});
