/**
 * The World ROSTER and the agents API are SEPARATE id namespaces, and the drift
 * is real: the Steward is `steward` in-world and `git_steward` in the worker
 * registry, while Henry (the orchestrator) and the Reader (a surface, not a
 * worker) have no agents-API entry at all. Both deep-link directions resolve
 * through this one map so neither side offers a control that lands nowhere.
 *
 * Kept dependency-free so it can be unit-tested without pulling in the API
 * client or the 3D world.
 */
const WORLD_TO_AGENT_ID: Readonly<Record<string, string>> = {
  librarian: 'librarian',
  watcher: 'watcher',
  steward: 'git_steward',
  strix: 'strix',
};

/** null when this in-world character is not an agent the API knows. */
export function agentIdForWorldAgent(worldId: string): string | null {
  return WORLD_TO_AGENT_ID[worldId] ?? null;
}

/** null when this agent has no in-world character to fly to. */
export function worldAgentIdForAgent(agentId: string): string | null {
  const hit = Object.entries(WORLD_TO_AGENT_ID).find(([, id]) => id === agentId);
  return hit ? hit[0] : null;
}
