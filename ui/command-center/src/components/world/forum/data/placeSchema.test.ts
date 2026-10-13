import { describe, expect, it } from 'vitest';
import { PLACE_SCHEMAS, placeSchema } from './placeSchema';
import { PLACE_HOST, WORLD_PLACE_IDS } from './identity';
import { PLACE_REQUEST_PLAN } from './snapshotLoader';

describe('six-place interaction schema', () => {
  it('covers exactly the six places, in order, with their hosts', () => {
    expect(PLACE_SCHEMAS.map((p) => p.id)).toEqual([...WORLD_PLACE_IDS]);
    for (const p of PLACE_SCHEMAS) expect(p.host).toBe(PLACE_HOST[p.id]);
    expect(() => placeSchema('nowhere' as never)).toThrow();
  });

  it('each place has one interaction object, one primary action, and at least one more meaningful action', () => {
    for (const p of PLACE_SCHEMAS) {
      expect(p.interactionObject.length).toBeGreaterThan(10);
      expect(p.primaryAction.label.length).toBeGreaterThan(3);
      expect(p.secondaryActions.length).toBeGreaterThanOrEqual(3);
      const ids = [p.primaryAction, ...p.secondaryActions].map((a) => a.id);
      expect(new Set(ids).size).toBe(ids.length);
    }
  });

  it('every place has evidence slots with honest empty copy, an empty state, and a non-spatial equivalent', () => {
    for (const p of PLACE_SCHEMAS) {
      expect(p.evidence.length).toBeGreaterThan(0);
      for (const s of p.evidence) expect(s.emptyCopy.length).toBeGreaterThan(10);
      expect(p.emptyState.body.length).toBeGreaterThan(20);
      expect(p.accessible.listLabel).toContain(p.name);
      expect(p.accessible.shortcutHint).toMatch(/^Places list → /);
      expect(PLACE_REQUEST_PLAN[p.id]).toBeDefined();
    }
  });

  it('navigate actions name a home view; demonstrations are labelled', () => {
    for (const p of PLACE_SCHEMAS) {
      for (const a of [p.primaryAction, ...p.secondaryActions]) {
        if (a.kind === 'navigate') expect(a.target).toBeDefined();
        if (a.demonstration) expect(a.description).toMatch(/demonstration/i);
      }
    }
    expect(PLACE_SCHEMAS.find((p) => p.id === 'gallery')!.secondaryActions.find((a) => a.id === 'try_example')?.demonstration).toBe(true);
  });
});
