// world/agents — W3 lane public surface.
export { WorldAgents } from './WorldAgents';
export { AgentCharacterV2 } from './AgentCharacterV2';
export { AgentStateSources } from './stateSources';
export { getAgentPosition } from './motion';
export { ROSTER, getIdentity } from './roster';
export type { AgentIdentity } from './roster';
// W3 → W4 coordinate: Henry's live position so atmosphere can gather light where he
// stands (bible §4). Read-only; never mutate the returned record.
export { getHenryPresence } from './henryPresence';
export type { HenryPresence } from './henryPresence';
