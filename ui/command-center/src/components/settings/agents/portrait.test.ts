import { describe, expect, it } from 'vitest';
import { ROSTER } from '../../world/agents/roster';
import { agentIdForWorldAgent } from '../../../lib/worldAgentIds';
import { portraitSpec } from './portrait';

describe('Settings portrait identity', () => {
  it('resolves every agent with a World character to that canonical wardrobe', () => {
    for (const character of ROSTER) {
      const agentId = agentIdForWorldAgent(character.id);
      if (!agentId) continue;
      expect(portraitSpec(agentId)).toEqual({
        worldId: character.id,
        trimColor: character.trimColor,
        label: character.name,
      });
    }
  });

  it('gives the Steward worker and dispatch persona the same World image key', () => {
    expect(portraitSpec('steward')).toEqual(portraitSpec('git_steward'));
    expect(portraitSpec('steward').worldId).toBe('steward');
  });

  it('does not invent a character for API agents with no World identity', () => {
    expect(portraitSpec('claude_code')).toEqual({
      worldId: null,
      trimColor: null,
      label: 'claude_code',
    });
  });
});
