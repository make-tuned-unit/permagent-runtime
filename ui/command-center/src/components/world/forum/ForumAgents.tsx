import { ensureForumMotion, ForumInhabitants, getForumMotion } from './ForumInhabitants';
import { AgentCharacterV2, AgentStateSources, ROSTER } from '../agents';
import { useAgentRuntimeStates } from '../shared/agentStatus';
import { useOrchestratorName } from '../shared/useOrchestratorName';

// This standalone study shares identity and telemetry, but not the rotunda's
// mezzanine/path graph. Forum-specific seats occupy actual floors in this scene;
// the existing live rigs and telemetry remain unchanged.
export function ForumAgents({ hoveredAgent, onHoverAgent, onSelectAgent }: {
  hoveredAgent: string | null;
  onHoverAgent: (id: string | null) => void;
  onSelectAgent: (id: string) => void;
}) {
  const states = useAgentRuntimeStates();
  const name = useOrchestratorName();
  return <group><AgentStateSources /><ForumInhabitants attended={hoveredAgent} />{ROSTER.map(a => <AgentCharacterV2
    key={a.id} armorVariant="forum" identity={a.isHenry && name ? { ...a, name } : a}
    motion={ensureForumMotion(a.id, a.home)}
    motionForAgent={getForumMotion}
    hudState={states.find(s => s.id === a.id)?.hudState ?? 'idle'}
    hovered={hoveredAgent === a.id} hoveredAgentId={hoveredAgent}
    onPointerOver={() => onHoverAgent(a.id)} onPointerOut={() => onHoverAgent(null)}
    onClick={() => onSelectAgent(a.id)}
  />)}</group>;
}
