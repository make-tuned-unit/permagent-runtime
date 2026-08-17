import { describe, expect, it } from 'vitest';
import { agentIdForWorldAgent, worldAgentIdForAgent } from './worldAgentIds';

describe('world ↔ agents id bridge', () => {
  it('maps the Steward across its two different ids', () => {
    expect(agentIdForWorldAgent('steward')).toBe('git_steward');
    expect(worldAgentIdForAgent('git_steward')).toBe('steward');
  });

  it('reports no mapping for in-world characters that are not agents', () => {
    // Henry is the orchestrator and the Reader is a surface — neither has a
    // roster entry, so a "manage this agent" link would 404.
    expect(agentIdForWorldAgent('henry')).toBeNull();
    expect(agentIdForWorldAgent('reader')).toBeNull();
  });

  it('reports no in-world character for agents that have none', () => {
    expect(worldAgentIdForAgent('claude_code')).toBeNull();
    expect(worldAgentIdForAgent('scheduler')).toBeNull();
  });

  it('round-trips every mapped id', () => {
    for (const worldId of ['librarian', 'watcher', 'steward', 'strix']) {
      const agentId = agentIdForWorldAgent(worldId);
      expect(agentId).not.toBeNull();
      expect(worldAgentIdForAgent(agentId as string)).toBe(worldId);
    }
  });
});
