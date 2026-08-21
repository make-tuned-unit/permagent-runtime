import { describe, expect, it } from 'vitest';
import { ROSTER } from '../../world/agents/roster';
import { agentIdForWorldAgent } from '../../../lib/worldAgentIds';
import { portraitSpec } from './portrait';

describe('agent portrait spec', () => {
  // Identity trim is the world's, not this module's. WORLD_VIEW_BIBLE §2/§4:
  // there is exactly one place a character's trim is decided, and a second
  // palette would drift from it silently the first time a hue was tuned.
  it('borrows the world identity trim and never invents one', () => {
    for (const identity of ROSTER) {
      const agentId = agentIdForWorldAgent(identity.id);
      if (agentId === null) continue;
      expect(portraitSpec(agentId).trimColor).toBe(identity.trimColor);
      expect(portraitSpec(agentId).weathering).toBe(identity.weathering);
    }
    // A dispatch persona with no in-world character has no colour to borrow.
    expect(portraitSpec('reviewer').trimColor).toBeNull();
  });

  // REGRESSION. The Steward is `steward` in agent.yaml and `git_steward` in the
  // worker registry, and Settings → Agents lists it under BOTH — as a dispatch
  // persona and as a background worker. Without the persona-key alias in
  // worldAgentIds, `portraitSpec('steward')` resolves to no world character at
  // all and this test fails on `variant` being 'unknown': one of the Steward's
  // two rows would draw the real character and the other a blank silhouette.
  it('draws the same character for both of the Steward ids', () => {
    expect(portraitSpec('steward')).toEqual(portraitSpec('git_steward'));
    expect(portraitSpec('steward').variant).toBe('steward');
  });

  it('still gives a portrait to an agent with no in-world character', () => {
    expect(portraitSpec('claude_code')).toEqual({
      worldId: null,
      variant: 'unknown',
      trimColor: null,
      weathering: 0,
      label: 'claude_code',
    });
  });

  // Every variant this module can produce must be an id the agents API can
  // actually hand it. A variant for an unreachable id is gear no screen draws,
  // and the renderer would carry an arm nothing can reach.
  it('produces a gear variant only for agents the API can name', () => {
    const reachable = ROSTER
      .map(identity => agentIdForWorldAgent(identity.id))
      .filter((id): id is string => id !== null);
    const variants = reachable.map(id => portraitSpec(id).variant).sort();
    expect(variants).toEqual(['financier', 'librarian', 'steward', 'strix', 'watcher']);
    // …and the world's own characters that no agent id reaches get no variant.
    expect(portraitSpec('henry').variant).toBe('unknown');
    expect(portraitSpec('reader').variant).toBe('unknown');
  });
});
