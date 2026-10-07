import { describe, expect, it } from 'vitest';
import { catalogue, describeModel, matchesQuery, mergeModelLists } from './modelCatalog';

describe('never losing a model the app supports', () => {
  it('keeps a known model the live list dropped', () => {
    // The reported bug: Haiku is a known Anthropic model and the cost router
    // routes to it, but the live list is filtered through the canonical
    // registry and the picker replaced known with live. Union, not replace.
    const live = ['claude-sonnet-5', 'claude-opus-4-8'];
    const known = ['claude-sonnet-5', 'claude-haiku-4-5', 'claude-haiku-4-5-20251001'];
    const merged = mergeModelLists(live, known);
    expect(merged).toContain('claude-haiku-4-5');
    expect(merged).toContain('claude-haiku-4-5-20251001');
  });

  it('follows the live order, because the daemon sorts it newest first', () => {
    expect(mergeModelLists(['b', 'a'], ['a', 'b', 'c'])).toEqual(['b', 'a', 'c']);
  });

  it('does not repeat a model that is in both lists', () => {
    const merged = mergeModelLists(['x', 'x'], ['x']);
    expect(merged).toEqual(['x']);
  });

  it('survives an empty live list, which is what a failed fetch looks like', () => {
    expect(mergeModelLists([], ['claude-haiku-4-5'])).toEqual(['claude-haiku-4-5']);
  });

  it('ignores blank ids rather than rendering an empty row', () => {
    expect(mergeModelLists(['', '  '], ['claude-sonnet-5'])).toEqual(['claude-sonnet-5']);
  });
});

describe('reading a model id as a name', () => {
  it('names the hyphenated Anthropic form', () => {
    const m = describeModel('claude-haiku-4-5');
    expect(m.label).toBe('Claude Haiku 4.5');
    expect(m.family).toBe('haiku');
    expect(m.dated).toBe(false);
  });

  it('names a dated pin and keeps its date', () => {
    const m = describeModel('claude-haiku-4-5-20251001');
    expect(m.label).toBe('Claude Haiku 4.5');
    expect(m.released).toBe('20251001');
    expect(m.dated).toBe(true);
  });

  it('names a single-digit version', () => {
    expect(describeModel('claude-sonnet-5').label).toBe('Claude Sonnet 5');
    expect(describeModel('claude-opus-4-8').label).toBe('Claude Opus 4.8');
  });

  it('falls back to the id rather than inventing a name', () => {
    const m = describeModel('some-internal-build');
    expect(m.label).toBe('some-internal-build');
    expect(m.family).toBeNull();
  });
});

describe('the order the menu offers models in', () => {
  it('puts the newest generation first', () => {
    const order = catalogue(['claude-haiku-4-5', 'claude-sonnet-5', 'claude-opus-4-8']).map(m => m.label);
    expect(order[0]).toBe('Claude Sonnet 5');
  });

  it('keeps a dated pin next to the alias it pins, not at the far end', () => {
    const ids = catalogue(['claude-sonnet-5', 'claude-haiku-4-5-20251001', 'claude-haiku-4-5']).map(m => m.id);
    const alias = ids.indexOf('claude-haiku-4-5');
    const pin = ids.indexOf('claude-haiku-4-5-20251001');
    expect(pin).toBe(alias + 1);
  });

  it('sorts an unrecognised id last instead of dropping it', () => {
    const ids = catalogue(['mystery-model', 'claude-sonnet-5']).map(m => m.id);
    expect(ids[ids.length - 1]).toBe('mystery-model');
    expect(ids).toHaveLength(2);
  });
});

describe('searching for a model the way a person types it', () => {
  const haiku = describeModel('claude-haiku-4-5-20251001');

  it('finds it by the name on screen', () => {
    expect(matchesQuery(haiku, 'Haiku 4.5')).toBe(true);
  });

  it('finds it however the version is punctuated', () => {
    for (const q of ['haiku45', 'haiku-4-5', 'haiku 4.5', 'HAIKU']) {
      expect(matchesQuery(haiku, q), `missed "${q}"`).toBe(true);
    }
  });

  it('finds it by raw id', () => {
    expect(matchesQuery(haiku, 'claude-haiku-4-5-20251001')).toBe(true);
  });

  it('does not match something else', () => {
    expect(matchesQuery(haiku, 'opus')).toBe(false);
  });

  it('matches everything on an empty query', () => {
    expect(matchesQuery(haiku, '   ')).toBe(true);
  });
});

describe('naming models from every vendor, not just Anthropic', () => {
  it('names an OpenAI model instead of printing its serial number', () => {
    // This is what the user saw in the menu: a column of these, verbatim.
    expect(describeModel('gpt-5.4-nano-2026-03-17').label).toBe('GPT 5.4 Nano');
    expect(describeModel('gpt-5.4-nano-2026-03-17').released).toBe('20260317');
    expect(describeModel('gpt-5-mini').label).toBe('GPT 5 Mini');
  });

  it('keeps a vendor own casing', () => {
    expect(describeModel('MiniMax-M2.7').label).toBe('MiniMax M2.7');
  });

  it('reads a size suffix as a unit', () => {
    expect(describeModel('examplemodel3-27b').label).toBe('Examplemodel3 27B');
  });

  it('still leaves an id with no version completely alone', () => {
    expect(describeModel('some-internal-build').label).toBe('some-internal-build');
  });
});
