/**
 * QA 2026-09-24: Spend → By project listed "/" (373 sessions), "tmp" and "j"
 * as projects — the server groups by working directory and labels an unknown
 * one with its basename. These pin the UI's fold into one "No project" row.
 */
import { describe, expect, it } from 'vitest';
import { groupProjectSpend } from './spendProjects';

const row = (path: string, label: string, costUsd: number, sessionCount = 1) =>
  ({ path, label, costUsd, tokens: 100 * sessionCount, sessionCount });

const ROWS = [
  row('/', '/', 68.6, 373),
  row('/Users/j/dev/permagent', 'Permagent', 40, 90),
  row('/tmp', 'tmp', 3.1, 181),
  row('/Users/j', 'j', 1.2, 12),
  row('/Users/j/dev/permagent/ui', 'ui', 0.5, 4),
];

describe('groupProjectSpend', () => {
  it('folds directories outside every project root into one No project group', () => {
    const g = groupProjectSpend(ROWS, ['/Users/j/dev/permagent']);
    expect(g.projects.map(p => p.path)).toEqual(['/Users/j/dev/permagent', '/Users/j/dev/permagent/ui']);
    expect(g.noProject?.dirs.map(d => d.path)).toEqual(['/', '/tmp', '/Users/j']);
    expect(g.noProject?.sessionCount).toBe(373 + 181 + 12);
    expect(g.noProject?.costUsd).toBeCloseTo(68.6 + 3.1 + 1.2);
  });

  it('a parent of a project root is not inside it ("/Users/j" is not Permagent)', () => {
    const g = groupProjectSpend([row('/Users/j', 'j', 1)], ['/Users/j/dev/permagent']);
    expect(g.projects).toHaveLength(0);
    expect(g.noProject?.dirs).toHaveLength(1);
  });

  it('a sibling with a shared prefix is not inside the root', () => {
    const g = groupProjectSpend([row('/Users/j/dev/permagent-old', 'x', 1)], ['/Users/j/dev/permagent']);
    expect(g.projects).toHaveLength(0);
  });

  it('leaves the rows alone when the project roots are unknown', () => {
    const g = groupProjectSpend(ROWS, null);
    expect(g.projects).toBe(ROWS);
    expect(g.noProject).toBeNull();
  });
});
