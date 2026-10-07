import { describe, expect, it } from 'vitest';
import { humanizeToolIds, summarize } from './LearnNext';

describe('LearnNext copy', () => {
  it('drops parenthesised tool ids', () => {
    expect(humanizeToolIds('Keeps your contacts (create_person, associate_person_with_project) up to date')).toBe('Keeps your contacts up to date');
  });
  it('never exposes a snake_case id', () => {
    expect(summarize('Uses card_create to add work. More text.')).not.toMatch(/[a-z]_[a-z]/);
  });
  it('ends on a word boundary when long', () => {
    const long = `${'word '.repeat(80)}end`;
    const out = summarize(long);
    expect(out.endsWith('…')).toBe(true);
    expect(out.slice(0, -1)).toMatch(/word$/);
    expect(out.length).toBeLessThanOrEqual(201);
  });
});
