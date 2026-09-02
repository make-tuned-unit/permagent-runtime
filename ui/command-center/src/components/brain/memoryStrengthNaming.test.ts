/**
 * The Brain rendered one field two ways. `mem.weight` was "reinforcement" in
 * the graph panel's stats footer and "signal" in the list's metadata row —
 * same memory, same tab, same number, two words, and neither of them defined
 * anywhere on screen.
 *
 * A reader who learns what "signal 62%" means in the list learns nothing that
 * transfers three feet to the right. The ruled word is "strength", which
 * neither surface had to invent and which says what the percentage measures
 * without a metaphor.
 *
 * Source-level because both surfaces sit inside a WebGL view the runner cannot
 * mount, and the failure guarded against — one of them drifting back to a
 * private word — is visible in the source.
 */

import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const read = (file: string) =>
  readFileSync(fileURLToPath(new URL(file, import.meta.url)), 'utf8');

const SURFACES = ['./BrainView.tsx', './BrainList.tsx'];

describe('a memory\'s weight has one name', () => {
  it.each(SURFACES)('%s takes the word from the shared vocabulary', file => {
    const source = read(file);
    expect(source).toContain("import { MEMORY_STRENGTH } from '../../lib/vocabulary'");
    expect(source).toContain('MEMORY_STRENGTH.one');
  });

  it.each(SURFACES)('%s glosses it, so the number is readable', file => {
    expect(read(file)).toContain('MEMORY_STRENGTH.gloss');
  });

  it('has retired both private words from the rendered text', () => {
    for (const file of SURFACES) {
      const source = read(file);
      // Comments explaining the history are fine; a rendered string is not.
      expect(source).not.toMatch(/<Stat label="reinforcement"/);
      expect(source).not.toMatch(/<span>signal \{/);
    }
  });
});
