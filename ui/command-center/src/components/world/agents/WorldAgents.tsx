// World agents composition — drop-in successor to WorldCharacters.
// Mounts the real state sources (Henry poll, Librarian /events, sim roster),
// drives behavior + locomotion, advances the motion store once per frame, and
// renders the 5 merged-geometry characters (≤ 6 draw calls each, bible §8).

import { useFrame } from '@react-three/fiber';
import { useAgentRuntimeStates } from '../shared/agentStatus';
import { useOrchestratorName } from '../shared/useOrchestratorName';
import type { AgentHudState } from '../shared/palette';
import { ROSTER } from './roster';
import { AgentStateSources } from './stateSources';
import { useAgentBehavior } from './behavior';
import { advanceMotion } from './motion';
import { AgentCharacterV2 } from './AgentCharacterV2';
import { HenryToolSigil } from './HenryToolSigil';

interface WorldAgentsProps {
  hoveredAgent: string | null;
  onHoverAgent: (id: string | null) => void;
  onSelectAgent: (id: string) => void;
}

export function WorldAgents({ hoveredAgent, onHoverAgent, onSelectAgent }: WorldAgentsProps) {
  const states = useAgentRuntimeStates();
  useAgentBehavior(states);

  // Resolve the orchestrator's nameplate through the configured identity (e.g.
  // "Henry") instead of the roster's hardcoded placeholder — same source the
  // Chat launcher uses. Only the orchestrator (isHenry) is overridden; the named
  // sim workers keep their roster names.
  const orchestratorName = useOrchestratorName();

  // Single per-frame advance for all agents — no RAF + setState-per-frame.
  useFrame((_, dt) => advanceMotion(Math.min(dt, 0.1)));

  const hudFor = (id: string): AgentHudState =>
    states.find((s) => s.id === id)?.hudState ?? 'idle';

  return (
    <group>
      <AgentStateSources />
      {/* The REAL in-flight tool name over Henry while he works (#84). */}
      <HenryToolSigil />
      {ROSTER.map((identity) => {
        const resolved =
          identity.isHenry && orchestratorName
            ? { ...identity, name: orchestratorName }
            : identity;
        return (
          <AgentCharacterV2
            key={identity.id}
            identity={resolved}
            hudState={hudFor(identity.id)}
            hovered={hoveredAgent === identity.id}
            hoveredAgentId={hoveredAgent}
            onPointerOver={() => onHoverAgent(identity.id)}
            onPointerOut={() => onHoverAgent(null)}
            onClick={() => onSelectAgent(identity.id)}
          />
        );
      })}
    </group>
  );
}
