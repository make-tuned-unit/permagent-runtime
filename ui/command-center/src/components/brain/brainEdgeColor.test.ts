import { describe, expect, it } from 'vitest';
import { predicateColor, predicateLabel } from './brainEdgeColor';

describe('predicateColor', () => {
  it('is stable for the same predicate', () => {
    expect(predicateColor('works_at')).toEqual(predicateColor('works_at'));
  });

  it('gives distinct predicates distinct colors, most of the time', () => {
    const preds = ['works_at', 'member_of', 'knows', 'lives_in', 'founded'];
    const colors = new Set(preds.map(p => predicateColor(p).join(',')));
    // A 6-bucket palette can collide, but 5 unrelated real predicates should
    // not all pile into the same one.
    expect(colors.size).toBeGreaterThan(1);
  });

  it('falls back to the default bucket for no predicate', () => {
    expect(predicateColor(null)).toEqual(predicateColor(undefined));
    expect(predicateColor('')).toEqual(predicateColor(null));
  });
});

describe('predicateLabel', () => {
  it('humanizes snake_case', () => {
    expect(predicateLabel('works_at')).toBe('works at');
    expect(predicateLabel('member_of')).toBe('member of');
  });
  it('never surfaces a raw identifier when the predicate is missing', () => {
    expect(predicateLabel(null)).toBe('connected');
    expect(predicateLabel(undefined)).toBe('connected');
    expect(predicateLabel('')).toBe('connected');
  });
});
