/** Identity for Settings portraits comes from the same World roster and ID
 * bridge as navigation. The image itself is rendered from the current hero kit
 * and wardrobe by thumbnailCache, never redrawn as a second character design. */
import { worldAgentIdForAgent } from '../../../lib/worldAgentIds';
import { ROSTER } from '../../world/agents/roster';

export interface PortraitSpec {
  worldId: string | null;
  trimColor: string | null;
  label: string;
}

export function portraitSpec(agentId: string): PortraitSpec {
  const worldId = worldAgentIdForAgent(agentId);
  const identity = worldId === null ? undefined : ROSTER.find(agent => agent.id === worldId);
  return identity
    ? { worldId: identity.id, trimColor: identity.trimColor, label: identity.name }
    : { worldId: null, trimColor: null, label: agentId };
}
