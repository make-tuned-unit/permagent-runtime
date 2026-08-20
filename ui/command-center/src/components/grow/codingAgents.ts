/**
 * Coding harnesses a growth Action can be sent to.
 *
 * Command strings are the same literals ProjectChip.tsx hands to `onLaunch`.
 * The desktop crate greps those literals to prove user-installed CLIs are
 * named, not bundled — keep these in lockstep with that file.
 */
export const CODING_AGENTS = [
  { id: 'claude', label: 'Claude', command: 'claude' },
  { id: 'codex', label: 'Codex', command: 'codex' },
  { id: 'cursor', label: 'Cursor', command: 'cursor-agent' },
  {
    id: 'permagent',
    label: 'Permagent',
    command: 'permagent run --recipe permagent-coding --interactive',
  },
] as const;

export type CodingAgentId = (typeof CODING_AGENTS)[number]['id'];

export function codingAgentById(id: string): (typeof CODING_AGENTS)[number] | undefined {
  return CODING_AGENTS.find((agent) => agent.id === id);
}
