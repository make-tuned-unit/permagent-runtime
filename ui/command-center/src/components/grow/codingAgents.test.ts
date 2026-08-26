import { describe, expect, it } from 'vitest';
import {
  CODING_AGENTS,
  SUBSCRIPTION_FIRST_HINT,
  codingAgentByCommand,
  codingAgentSelectLabel,
  launchTooltip,
} from './codingAgents';

describe('CODING_AGENTS', () => {
  it('lists subscription CLIs before the metered harness', () => {
    expect(CODING_AGENTS.map(a => a.id)).toEqual(['claude', 'codex', 'cursor', 'permagent']);
    expect(CODING_AGENTS.filter(a => a.costTier === 'subscription').map(a => a.id)).toEqual([
      'claude',
      'codex',
      'cursor',
    ]);
    expect(CODING_AGENTS.at(-1)?.costTier).toBe('cheap_api');
  });

  it('names subscription cost in Claude and Codex tooltips so users do not have to dig', () => {
    const claude = codingAgentByCommand('claude');
    const codex = codingAgentByCommand('codex');
    expect(claude?.tooltip.toLowerCase()).toContain('subscription');
    expect(claude?.tooltip).toContain('$0');
    expect(codex?.tooltip.toLowerCase()).toContain('subscription');
    expect(codex?.tooltip).toContain('$0');
    expect(codingAgentByCommand('permagent run --recipe permagent-coding --interactive')?.tooltip)
      .toMatch(/not cheaper than Claude\/Codex/i);
  });

  it('launchTooltip prefers the root-path gate, then the cost policy', () => {
    expect(launchTooltip('claude', false)).toContain('root path');
    expect(launchTooltip('claude', true)).toContain('subscription');
    expect(SUBSCRIPTION_FIRST_HINT).toContain('nothing extra');
  });

  it('Grow select labels subscription CLIs as $0 extra and Permagent as routed', () => {
    expect(codingAgentSelectLabel(CODING_AGENTS[0])).toBe('Claude · $0 extra');
    expect(codingAgentSelectLabel(CODING_AGENTS.at(-1)!)).toBe('Permagent · routed');
  });
});
