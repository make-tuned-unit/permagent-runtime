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
 * That exclusion list is now gone. Each roster entry declares its OWN `wire`,
 * and `stateSources` simulates exactly the entries that say `sim` — so adding
 * an agent can no longer leave two files disagreeing about whether its state
 * is real. The pins below are on the invariant, not on any one id:
 *
 *   daemon — something really reports this agent's state.
 *   sim    — nothing does, and the §4 clamp holds it to idle/available.
 *   static — nothing does YET, and it must not be animated in the meantime.
 *            A roster seat whose emitter is queued renders as a fixed pose
 *            rather than a plausible one.
 */

import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { ROSTER, SIM_AGENT_IDS } from './roster';

const STATE_SOURCES = readFileSync(
  fileURLToPath(new URL('./stateSources.tsx', import.meta.url)),
  'utf8',
);

/**
 * The agents that emit `agent_state_changed` from the daemon today. Steward
 * announces as `git_steward` and is mapped to the world id on arrival.
 */
const WIRED = ['henry', 'librarian', 'strix', 'steward', 'financier', 'forecaster'];

/**
 * Seats whose emitter does not exist yet (agent-QA D-N5-1 Council, D22
 * Polybot/Picker). They are on screen because a user turns each of them on and
 * reasons about it (J11) — and they render honestly static until the emitter
 * lane lands.
 */
const AWAITING_EMITTER = ['council', 'polybot', 'picker'];

describe('ambient simulation', () => {
  it.each(WIRED)('never simulates %s, which reports its own state', id => {
    const entry = ROSTER.find(a => a.id === id);
    expect(entry, `${id} must be in the roster`).toBeTruthy();
    expect(entry!.wire, `${id} reports its own state`).toBe('daemon');
    expect(SIM_AGENT_IDS).not.toContain(id);
  });

  it.each(AWAITING_EMITTER)('never simulates %s, whose emitter has not shipped', id => {
    const entry = ROSTER.find(a => a.id === id);
    expect(entry, `${id} must be in the roster (J11)`).toBeTruthy();
    expect(entry!.wire).toBe('static');
    expect(SIM_AGENT_IDS).not.toContain(id);
  });

  it('keeps the exclusions in ONE place — the roster entry itself', () => {
    // No hand-maintained id list in stateSources: the previous shape let a
    // roster entry claim a wire that the filter had never heard of.
    expect(STATE_SOURCES).not.toMatch(/a\.id !== '[a-z_]+'/);
    expect(STATE_SOURCES).toContain("wire === 'sim'");
  });

  it('simulates exactly the entries that admit they are simulated', () => {
    expect([...SIM_AGENT_IDS].sort()).toEqual(
      ROSTER.filter(a => a.wire === 'sim').map(a => a.id).sort(),
    );
  });

  it('gives every roster entry a declared wire', () => {
    for (const a of ROSTER) {
      expect(['daemon', 'sim', 'static'], `${a.id} must declare its wire`).toContain(a.wire);
    }
  });
});
