// HenryToolSigil — the REAL in-flight tool, named, over Henry's head.
//
// /api/henry/status carries `current_tool` (#84: the daemon reads the active
// session's in-flight tool call). While Henry's HUD state is genuinely
// `working` AND a tool name is present, a small amber holo chip floats above
// him naming it — "⚙ web_search" means Henry is literally running web_search
// right now. No tool or not working ⇒ nothing renders. Poll failure clears
// the store (henryWork.ts), so a dead daemon can't leave a stale claim.
//
// Perf: the chip's group tracks Henry's live motion-store position in
// useFrame (zero alloc); React only re-renders on discrete tool/state
// changes via the two stores.

import { useRef } from 'react';
import { useFrame } from '@react-three/fiber';
import { Html } from '@react-three/drei';
import * as THREE from 'three';
import { ENV } from '../shared/palette';
import { useAgentRuntimeStates } from '../shared/agentStatus';
import { useHenryWork } from './henryWork';
import { getAgentPosition } from './motion';
import { radius, space } from '../../../styles/tokens';

export function HenryToolSigil() {
  const groupRef = useRef<THREE.Group>(null);
  const { tool } = useHenryWork();
  const states = useAgentRuntimeStates();
  const henry = states.find((s) => s.id === 'henry');
  const show = Boolean(tool) && henry?.hudState === 'working' && henry.source === 'daemon';

  useFrame(() => {
    const g = groupRef.current;
    if (!g) return;
    const pos = getAgentPosition('henry');
    if (pos) g.position.set(pos.x, pos.y + 3.15, pos.z);
  });

  if (!show) return null;

  return (
    <group ref={groupRef}>
      <Html center distanceFactor={13} style={{ pointerEvents: 'none' }}>
        <div
          style={{
            padding: `${space.xxs}px ${space.md}px`,
            borderRadius: radius.xs,
            background: 'rgba(10, 14, 26, 0.85)',
            border: `1px solid ${ENV.neonAmber}66`,
            boxShadow: `0 0 10px ${ENV.neonAmber}33`,
            color: ENV.neonAmber,
            fontFamily: 'JetBrains Mono, monospace',
            fontSize: 10,
            letterSpacing: '0.1em',
            whiteSpace: 'nowrap',
          }}
        >
          ⚙ {tool}
        </div>
      </Html>
    </group>
  );
}
