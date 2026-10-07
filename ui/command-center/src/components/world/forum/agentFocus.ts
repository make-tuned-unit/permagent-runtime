/** Shared camera-focus revision bump for world click, dock and desk picker. */
export type AgentFocusRequest = { id: string; revision: number };

export function nextAgentFocus(
  previous: AgentFocusRequest | null,
  id: string,
): AgentFocusRequest {
  return { id, revision: (previous?.revision ?? 0) + 1 };
}
