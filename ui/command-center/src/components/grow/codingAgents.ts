/**
 * Coding harnesses a growth Action can be sent to, and the Build → Projects
 * dropdown launches.
 *
 * Command strings are the same literals ProjectChip.tsx hands to `onLaunch`
 * (the desktop crate greps those literals — keep this list in lockstep).
 *
 * Order is the ranking goal dispatch uses: subscription CLIs first, then the Permagent harness for routed API models.
 */
export const CODING_AGENTS = [
  {
    id: 'claude',
    label: 'Claude',
    command: 'claude',
    costTier: 'subscription',
    tooltip:
      'Claude Code — with your subscription. Uses your CLI sign-in; subscription usage limits apply.',
  },
  {
    id: 'codex',
    label: 'Codex',
    command: 'codex',
    costTier: 'subscription',
    tooltip:
      'Codex — with your subscription. Uses your CLI sign-in; subscription usage limits apply.',
  },
  {
    id: 'cursor',
    label: 'Cursor',
    command: 'cursor-agent',
    costTier: 'subscription',
    tooltip:
      'Cursor CLI — with your subscription. Uses your CLI sign-in; subscription usage limits apply.',
  },
  {
    id: 'permagent',
    label: 'Permagent',
    command: 'permagent run --recipe permagent-coding --interactive',
    costTier: 'cheap_api',
    tooltip:
      'Permagent harness — uses the configured model routes. API usage is billed by the selected provider; local models use your machine.',
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

/**
 * Cursor accepts an initial prompt as a positional CLI argument. Use that
 * entry point for Grow dispatch so the prompt is present when the CLI starts,
 * even when its TUI does not emit one of our readiness markers. POSIX single
 * quoting keeps the full directive a single argument, including quotes and
 * newlines. Other harnesses keep their existing readiness-gated paste path.
 */
export function codingAgentLaunchCommand(agent: CodingAgent, prompt: string): string {
  if (agent.id !== 'cursor') return agent.command;
  const quotedPrompt = `'${prompt.replace(/'/g, `'\\''`)}'`;
  // `--` prevents a directive that begins with `-` from being parsed as an
  // option by the CLI. It is consumed by the CLI parser, not part of prompt.
  return `${agent.command} -- ${quotedPrompt}`;
}

const NO_ROOT = 'Add a root path to launch a terminal here.';

/** Tooltip on a Projects launch button. Root-path gate wins; otherwise the cost policy. */
export function launchTooltip(command: string, hasRoot: boolean): string {
  if (!hasRoot) return NO_ROOT;
  return codingAgentByCommand(command)?.tooltip ?? NO_ROOT;
}

/** Explain the billing route without promising a zero price. */
export function codingAgentSelectLabel(agent: CodingAgent): string {
  return agent.costTier === 'subscription'
    ? `${agent.label} · w/ Subscription`
    : `${agent.label} · API / Local`;
}

export const SUBSCRIPTION_FIRST_HINT =
  'Use Claude, Codex or Cursor with your subscription. Permagent uses your configured API or local model routes.';
