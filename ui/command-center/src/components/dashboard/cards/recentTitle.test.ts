import { describe, expect, it } from 'vitest';
import { recentTitle, UNTITLED_SESSION } from './recentTitle';

describe('recentTitle', () => {
  it('names a row whose title is a truncated tool-args object after its name field', () => {
    // The exact shape QA saw on Home: dashboard.rs cut an object literal at 40.
    expect(recentTitle("{name: 'scroll_depth', depth: 0.75, sel...")).toBe('Scroll depth');
    expect(recentTitle('{"tool":"browser_navigate","args":{"url":"htt')).toBe('Browser navigate');
    expect(recentTitle('{ "goal": "Ship the release notes", "prio')).toBe('Ship the release notes');
  });

  it('falls back to the first quoted string, then to a plain label', () => {
    expect(recentTitle("{depth: 3, selector: 'main-article', x: 1}")).toBe('Main article');
    expect(recentTitle('{"depth": 3, "x": 4}')).toBe(UNTITLED_SESSION);
    expect(recentTitle('{"depth": 3, "label": "Top of the fo')).toBe('Top of the fo');
    expect(recentTitle('{depth: 3, x: 4, y: 5}')).toBe(UNTITLED_SESSION);
    expect(recentTitle('[1, 2, 3]')).toBe(UNTITLED_SESSION);
    expect(recentTitle('   ')).toBe(UNTITLED_SESSION);
  });

  it('leaves ordinary titles alone, including ones that mention braces', () => {
    expect(recentTitle('Plan the Atlas launch')).toBe('Plan the Atlas launch');
    expect(recentTitle('Why does {x} render twice?')).toBe('Why does {x} render twice?');
    expect(recentTitle('Session 20260924_12')).toBe('Session 20260924_12');
    expect(recentTitle('{curly} braces in a sentence')).toBe('{curly} braces in a sentence');
  });
});
