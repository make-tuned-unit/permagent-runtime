/**
 * Coding harnesses a growth Action can be sent to, and the Build → Projects
 * dropdown launches.
 *
 * Command strings are the same literals ProjectChip.tsx hands to `onLaunch`
 * (the desktop crate greps those literals — keep this list in lockstep).
 *
 * Order is the ranking goal dispatch uses: subscription CLIs first ($0 at the
 * margin), then the Permagent harness for routed cheap models.
 */
export const CODING_AGENTS = [
  {
    id: 'claude',
    label: 'Claude',
    command: 'claude',
    costTier: 'subscription',
    tooltip:
      'Claude Code — subscription, $0 extra at the margin. Prefer this over the Anthropic API for coding.',
  },
  {
    id: 'codex',
    label: 'Codex',
    command: 'codex',
    costTier: 'subscription',
    tooltip:
      'Codex — subscription, $0 extra at the margin. Prefer this over a metered OpenAI key for coding.',
  },
  {
    id: 'cursor',
    label: 'Cursor',
    command: 'cursor-agent',
    costTier: 'subscription',
    tooltip:
      'Cursor CLI — subscription, $0 extra at the margin. Install with: curl https://cursor.com/install -fsS | bash',
  },
  {
    id: 'permagent',
    label: 'Permagent',
    command: 'permagent run --recipe permagent-coding --interactive',
    costTier: 'cheap_api',
    tooltip:
      'Permagent harness — routed cheap models when you want per-role spend, or when no subscription CLI is installed. Not cheaper than Claude/Codex if you already pay for those.',
  },
] as const;

export type CodingAgentId = (typeof CODING_AGENTS)[number]['id'];

export type CodingAgent = (typeof CODING_AGENTS)[number];

export function codingAgentById(id: string): CodingAgent | undefined {
  return CODING_AGENTS.find((agent) => agent.id === id);
}

export function codingAgentByCommand(command: string): CodingAgent | undefined {
  return CODING_AGENTS.find((agent) => agent.command === command);
}

const NO_ROOT = 'Add a root path to launch a terminal here.';

/** Tooltip on a Projects launch button. Root-path gate wins; otherwise the cost policy. */
export function launchTooltip(command: string, hasRoot: boolean): string {
  if (!hasRoot) return NO_ROOT;
  return codingAgentByCommand(command)?.tooltip ?? NO_ROOT;
}

/** Grow's Send-to-agent select: subscription CLIs read as $0 extra, harness as routed. */
export function codingAgentSelectLabel(agent: CodingAgent): string {
  return agent.costTier === 'subscription'
    ? `${agent.label} · $0 extra`
    : `${agent.label} · routed`;
}

export const SUBSCRIPTION_FIRST_HINT =
  'Claude / Codex / Cursor first if you have a subscription — they cost nothing extra. Permagent is for routed cheap models.';
