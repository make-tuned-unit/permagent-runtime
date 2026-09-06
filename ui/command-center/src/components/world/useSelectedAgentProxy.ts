import { useMemo } from 'react';
import { useFrame } from '@react-three/fiber';
import type { AgentState } from './types';
import { ROSTER, getAgentPosition } from './agents';

// The camera must receive the selected identity on the selection render.
// Assigning only a ref from useFrame does not notify React: the child can
// otherwise keep receiving null until an unrelated hover causes a render.
export function useSelectedAgentProxy(selectedAgentId: string | null): AgentState | null {
  const proxy = useMemo<AgentState | null>(() => {
    if (!selectedAgentId) return null;
    const identity = ROSTER.find((entry) => entry.id === selectedAgentId);
    const position = getAgentPosition(selectedAgentId);
    if (!identity || !position) return null;
    return {
      id: identity.id,
      name: identity.name,
      role: identity.role,
      position: { x: position.x, y: position.y, z: position.z },
      activity: 'idle',
      currentStation: null,
      togaTrimColor: identity.trimColor,
      isHenry: identity.isHenry,
    };
  }, [selectedAgentId]);

  useFrame(() => {
    if (!proxy) return;
    const position = getAgentPosition(proxy.id);
    if (!position) return;
    proxy.position.x = position.x;
    proxy.position.y = position.y;
    proxy.position.z = position.z;
  });
  return proxy;
}
