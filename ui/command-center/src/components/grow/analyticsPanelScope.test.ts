/**
 * The analytics panels are PROJECT-SCOPED and must not carry state across a
 * project switch.
 *
 * Verifying Evntally and then switching to GetLadle left the previous project's
 * PASS on screen, which reads as "analytics is installed here" for a project
 * that was never verified (reported 2026-08-04). Every failure mode in this
 * surface is silent, so a false positive is the most damaging thing it can
 * show. The fix is a `key` on the panel, which remounts it — clearing every
 * field rather than the single one that happened to leak.
 */
import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';

const SRC = readFileSync(new URL('./GrowView.tsx', import.meta.url), 'utf8');

/** Extract the JSX props of a component usage, so we can assert on `key`. */
function usage(component: string): string {
  const at = SRC.indexOf(`<${component}`);
  expect(at, `${component} not found`).toBeGreaterThan(-1);
  return SRC.slice(at, SRC.indexOf('/>', at));
}

describe('analytics panels are remounted per project', () => {
  for (const panel of ['FirstPartyAnalyticsPanel', 'AnalyticsConnectionPanel']) {
    it(`${panel} is keyed on the project id`, () => {
      const props = usage(panel);
      expect(props).toContain('projectId={project.id}');
      // Without this, per-panel state (verify result, copied flags) survives a
      // project switch even though the fetched setup correctly reloads.
      expect(props).toMatch(/key=\{project\.id\}/);
    });
  }

  it('the key appears BEFORE other props, so it is never mistaken for data', () => {
    const props = usage('FirstPartyAnalyticsPanel');
    expect(props.indexOf('key={project.id}')).toBeLessThan(props.indexOf('projectId='));
  });
});
