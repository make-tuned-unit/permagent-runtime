/**
 * The World's ambient toggler flips an avatar between idle and available on a
 * fabricated 20–40 second timer. That is honest for agents whose real state
 * nothing reports, and a lie for any agent that DOES report — because the sim
 * fires unconditionally and buries the announcement within half a minute.
 *
 * The Forecaster was in the second group and filtered as if it were in the
 * first. Its roster entry said, correctly, that tools announce on its id; the
 * exclusion list in stateSources was never updated to match, so the screen
 * showed a coin flip while the daemon was telling the truth.
 *
 * The pin is on the invariant rather than on that one id: an agent with a wire
 * is never also simulated.
 */

import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const STATE_SOURCES = readFileSync(
  fileURLToPath(new URL('./stateSources.tsx', import.meta.url)),
  'utf8',
);

/**
 * The agents that emit `agent_state_changed` from the daemon today. Steward
 * announces as `git_steward` and is mapped to the world id on arrival.
 */
const WIRED = ['henry', 'librarian', 'strix', 'steward', 'financier', 'forecaster'];

describe('ambient simulation', () => {
  it.each(WIRED)('never simulates %s, which reports its own state', id => {
    expect(STATE_SOURCES, `${id} must be excluded from the sim roster`)
      .toContain(`a.id !== '${id}'`);
  });

  it('keeps the exclusions in one place, so a new wire has one thing to update', () => {
    const filters = STATE_SOURCES.match(/a\.id !== '[a-z_]+'/g) ?? [];
    expect(filters).toHaveLength(WIRED.length);
  });
});
