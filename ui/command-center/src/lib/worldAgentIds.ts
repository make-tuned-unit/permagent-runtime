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
  financier: 'financier',
};

/**
 * Dispatch-persona keys that name a worker under a SECOND spelling. agent.yaml
 * coined `steward` while the worker registry coined `git_steward`, so a surface
 * that resolved only one drew the same character with a portrait on one page and
 * none on the other.
 *
 * NOTE the collision: the string `steward` is a WORLD id AND a persona key at
 * once, and they happen to mean the same character — which is exactly why this
 * resolves persona keys BEFORE the world map is consulted, never after.
 *
 * Mirrors `agent_identity::descriptor_id_for_worker_key` on the Rust side. Both
 * are pinned by tests; nothing pins them to each other.
 */
const AGENT_ALIASES: Readonly<Record<string, string>> = { steward: 'git_steward' };

/** The agents-API id an id-or-persona-key names. Identity for everything else. */
export function canonicalAgentId(agentId: string): string {
  return AGENT_ALIASES[agentId] ?? agentId;
}

/** null when this in-world character is not an agent the API knows. */
export function agentIdForWorldAgent(worldId: string): string | null {
  return WORLD_TO_AGENT_ID[worldId] ?? null;
}

/** null when this agent has no in-world character to fly to. */
export function worldAgentIdForAgent(agentId: string): string | null {
  const canonical = canonicalAgentId(agentId);
  const hit = Object.entries(WORLD_TO_AGENT_ID).find(([, id]) => id === canonical);
  return hit ? hit[0] : null;
}
