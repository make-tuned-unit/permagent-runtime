// The Grow lens contract.
//
// Actions leads deliberately: the point of collecting analytics is deciding
// what to do about it, so "so what" comes before "what happened". This is easy
// to undo by accident when adding a lens, so the order and the default are
// pinned here rather than left to whoever edits the array next.

import { describe, expect, it } from 'vitest';

import { GROW_SOURCE as SOURCE } from './growSource';

describe('Grow lenses', () => {
  it('lists Actions first', () => {
    const match = SOURCE.match(/const LENSES: GrowLens\[\] = \[([^\]]+)\]/);
    expect(match, 'LENSES array not found').toBeTruthy();
    const lenses = match![1]
      .split(',')
      .map((s) => s.trim().replace(/'/g, ''))
      .filter(Boolean);
    expect(lenses[0]).toBe('actions');
    expect(lenses[1]).toBe('results');
    expect(lenses).toContain('analytics');
  });

  it('opens on Actions', () => {
    expect(SOURCE).toContain("useState<GrowLens>('actions')");
  });

  it('renders the Actions lens', () => {
    expect(SOURCE).toContain("lens === 'actions' && <GrowActions");
  });

  it('renders the Results lens', () => {
    expect(SOURCE).toContain("lens === 'results' && <GrowResults");
  });

  // The growth inbox moved to Actions. Leaving a copy on Analytics would put
  // the same actions in two places, which is how they drift apart.
  it('shows the growth inbox in Actions only, not Analytics', () => {
    const occurrences = SOURCE.split('<GrowthInboxSection').length - 1;
    expect(occurrences).toBe(1);
  });

  // Permagent's own collector keeps the data on the user's infrastructure, so
  // a vendor connection must not sit at equal visual weight and steer people
  // away from it for no reason.
  it('keeps third-party providers behind an opt-in disclosure', () => {
    expect(SOURCE).toContain('providersOpen');
    expect(SOURCE).toMatch(/Already use Plausible.*Connect it read-only instead/);
  });

  // Every number the first-party panel shows is qualified, because the visitor
  // hash undercounts and the bot filter removes rows.
  it('labels the visitor figure as devices, never visitors', () => {
    expect(SOURCE).toContain('deviceSignatures');
    expect(SOURCE).toContain('undercounts');
    expect(SOURCE).not.toContain('fpStats!.visitors');
  });

  // An empty Actions panel must say why: silence is indistinguishable from
  // breakage, and this panel is allowed to have nothing to say.
  it('explains an empty action list', () => {
    expect(SOURCE).toContain('actions?.reason');
  });
});
