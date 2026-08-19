import { describe, expect, it } from 'vitest';
import { agentIdForWorldAgent, canonicalAgentId, worldAgentIdForAgent } from './worldAgentIds';

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

  // The dispatch-persona key is a THIRD namespace beyond the world/agents
  // bridge: agent.yaml calls the Steward `steward`, the worker registry calls it
  // `git_steward`. Before the alias, `worldAgentIdForAgent('steward')` was null,
  // so the Steward's persona row resolved to no character while its worker row
  // resolved to one — the same agent, drawn two different ways.
  it('resolves a dispatch-persona key to its worker character', () => {
    expect(canonicalAgentId('steward')).toBe('git_steward');
    expect(canonicalAgentId('strix')).toBe('strix');
    expect(worldAgentIdForAgent('steward')).toBe('steward');
    // The alias must not invent a character for an agent that has none.
    expect(canonicalAgentId('claude_code')).toBe('claude_code');
    expect(worldAgentIdForAgent('claude_code')).toBeNull();
  });

  it('round-trips every mapped id', () => {
    for (const worldId of ['librarian', 'watcher', 'steward', 'strix']) {
      const agentId = agentIdForWorldAgent(worldId);
      expect(agentId).not.toBeNull();
      expect(worldAgentIdForAgent(agentId as string)).toBe(worldId);
    }
  });
});
