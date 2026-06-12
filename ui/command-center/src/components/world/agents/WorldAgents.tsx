// World agents composition — drop-in successor to WorldCharacters.
// Mounts the real state sources (Henry poll, Librarian /events, sim roster),
// drives behavior + locomotion, advances the motion store once per frame, and
// renders the 5 merged-geometry characters (≤ 6 draw calls each, bible §8).

import { useFrame } from '@react-three/fiber';
import { useAgentRuntimeStates } from '../shared/agentStatus';
import type { AgentHudState } from '../shared/palette';
import { ROSTER } from './roster';
import { AgentStateSources } from './stateSources';
import { useAgentBehavior } from './behavior';
import { advanceMotion } from './motion';
import { AgentCharacterV2 } from './AgentCharacterV2';

interface WorldAgentsProps {
  hoveredAgent: string | null;
  onHoverAgent: (id: string | null) => void;
  onSelectAgent: (id: string) => void;
}

export function WorldAgents({ hoveredAgent, onHoverAgent, onSelectAgent }: WorldAgentsProps) {
  const states = useAgentRuntimeStates();
  useAgentBehavior(states);

  // Single per-frame advance for all agents — no RAF + setState-per-frame.
  useFrame((_, dt) => advanceMotion(Math.min(dt, 0.1)));

  const hudFor = (id: string): AgentHudState =>
    states.find((s) => s.id === id)?.hudState ?? 'idle';

  return (
    <group>
      <AgentStateSources />
      {ROSTER.map((identity) => (
        <AgentCharacterV2
          key={identity.id}
          identity={identity}
          hudState={hudFor(identity.id)}
          hovered={hoveredAgent === identity.id}
          onPointerOver={() => onHoverAgent(identity.id)}
          onPointerOut={() => onHoverAgent(null)}
          onClick={() => onSelectAgent(identity.id)}
        />
      ))}
    </group>
  );
}
