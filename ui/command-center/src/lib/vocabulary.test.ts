/**
 * The vocabulary module's whole job is that two surfaces cannot drift apart on
 * one concept's name. So the pins are on the shape, not the strings: every
 * term is complete and glossed, and a plural is a plural.
 */

import { describe, expect, it } from 'vitest';
import { AUTOMATION, GLOSSARY, MEMORY_STRENGTH, SKILL, deJargonDaemonText, plural, type Term } from './vocabulary';

const TERMS: Array<[string, Term]> = [
  ['AUTOMATION', AUTOMATION],
  ['SKILL', SKILL],
  ['MEMORY_STRENGTH', MEMORY_STRENGTH],
];

describe('vocabulary', () => {
  it.each(TERMS)('%s carries every form a surface might need', (_name, term) => {
    expect(term.one).toBeTruthy();
    expect(term.many).toBeTruthy();
    expect(term.title).toBeTruthy();
    // A term with no gloss is jargon with a nicer name.
    expect(term.gloss.length).toBeGreaterThan(20);
  });

  it('counts in the right form', () => {
    expect(plural(AUTOMATION, 1)).toBe('1 automation');
    expect(plural(AUTOMATION, 0)).toBe('0 automations');
    expect(plural(AUTOMATION, 4)).toBe('4 automations');
  });

  it('glosses every borrowed term in a full sentence', () => {
    for (const [key, text] of Object.entries(GLOSSARY)) {
      expect(text.length, key).toBeGreaterThan(20);
      expect(text.trim().endsWith('.'), key).toBe(true);
    }
  });
});

describe('deJargonDaemonText', () => {
  // The activity journal is server-authored prose, not a string in this repo
  // — automationNaming.test.ts's source scan cannot see it, so this is the
  // catch for the same retired word when the DATA says it instead of the code.
  it('swaps the retired "recipe" for the ruled "automation"', () => {
    expect(deJargonDaemonText('Ran the daily digest recipe')).toBe('Ran the daily digest automation');
  });

  it('handles the plural and preserves capitalization', () => {
    expect(deJargonDaemonText('2 recipes ran')).toBe('2 automations ran');
    expect(deJargonDaemonText('Recipe failed')).toBe('Automation failed');
  });

  it('does not touch a word that merely contains "recipe"', () => {
    expect(deJargonDaemonText('recipebook.com was fetched')).toBe('recipebook.com was fetched');
  });

  it('passes text with nothing to swap through unchanged', () => {
    expect(deJargonDaemonText('Proposed deleting 2 merged branches')).toBe('Proposed deleting 2 merged branches');
  });
});
