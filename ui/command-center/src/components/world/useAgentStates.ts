// TODO: Replace with WebSocket subscription to permagent-daemon
// at ws://localhost:3001/ws/agents. Daemon will emit agent state
// updates derived from session activity + orchestrator status.
// Until then, this hook simulates agent states locally.

import { useState, useCallback, useEffect, useRef } from 'react';
import type { AgentState } from './types';
import { STATIONS } from './constants';

const WANDER_INTERVAL_MIN = 15000;
const WANDER_INTERVAL_MAX = 30000;
const WALK_SPEED = 3; // units/sec

const INITIAL_AGENTS: AgentState[] = [
  {
    id: 'henry',
    name: 'Henry',
    role: 'orchestrator',
    position: { x: 0, y: 0, z: 0 },
    activity: 'idle',
    currentStation: null,
    togaTrimColor: '#00D9FF',
    isHenry: true,
  },
  {
    id: 'aria',
    name: 'Aria',
    role: 'agent',
    position: { x: 4, y: 0, z: 2 },
    activity: 'idle',
    currentStation: null,
    togaTrimColor: '#FFB347',
    isHenry: false,
  },
  {
    id: 'felix',
    name: 'Felix',
    role: 'agent',
    position: { x: -3, y: 0, z: -4 },
    activity: 'idle',
    currentStation: null,
    togaTrimColor: '#FF6B9D',
    isHenry: false,
  },
  {
    id: 'nova',
    name: 'Nova',
    role: 'agent',
    position: { x: -5, y: 0, z: 3 },
    activity: 'idle',
    currentStation: null,
    togaTrimColor: '#A78BFA',
    isHenry: false,
  },
  {
    id: 'librarian',
    name: 'The Librarian',
    role: 'agent',
    position: { x: 14, y: 5.15, z: 0 },
    activity: 'idle',
    currentStation: null,
    togaTrimColor: '#8B7E6F',
    isHenry: false,
  },
];

function randomInterval(): number {
  return WANDER_INTERVAL_MIN + Math.random() * (WANDER_INTERVAL_MAX - WANDER_INTERVAL_MIN);
}

// Mezzanine waypoints for the Librarian — stays on the ring walkway
const MEZZ_MID_R = 14; // center of the walkway ring (12.5 to 15.5)
const MEZZ_Y = 5.15;
const MEZZ_WAYPOINTS = Array.from({ length: 12 }, (_, i) => {
  const angle = (i / 12) * Math.PI * 2;
  return { id: `mezz-${i}`, x: Math.cos(angle) * MEZZ_MID_R, z: Math.sin(angle) * MEZZ_MID_R };
});

function pickRandomStation(currentStation: string | null, isLibrarian: boolean): string {
  if (isLibrarian) {
    const available = MEZZ_WAYPOINTS.filter((w) => w.id !== currentStation);
    return available[Math.floor(Math.random() * available.length)].id;
  }
  const available = STATIONS.filter((s) => s.id !== currentStation);
  const idx = Math.floor(Math.random() * (available.length + 1));
  if (idx === available.length) return 'center';
  return available[idx].id;
}

function getStationPosition(stationId: string): { x: number; y: number; z: number } {
  if (stationId === 'center') return { x: 0, y: 0, z: 0 };
  const mezzWp = MEZZ_WAYPOINTS.find((w) => w.id === stationId);
  if (mezzWp) return { x: mezzWp.x, y: MEZZ_Y, z: mezzWp.z };
  const station = STATIONS.find((s) => s.id === stationId);
  if (!station) return { x: 0, y: 0, z: 0 };
  return { x: station.position[0], y: 0, z: station.position[2] };
}

export function useAgentStates(): {
  agents: AgentState[];
  setAgentTarget: (id: string, station: string) => void;
  moveAgent: (id: string, dx: number, dz: number) => void;
} {
  const [agents, setAgents] = useState<AgentState[]>(INITIAL_AGENTS);
  const animRef = useRef<number>(0);
  const targetsRef = useRef<Map<string, { x: number; y: number; z: number; stationId: string }>>(new Map());
  const timersRef = useRef<Map<string, ReturnType<typeof setTimeout>>>(new Map());

  const setAgentTarget = useCallback((id: string, station: string) => {
    const pos = getStationPosition(station);
    targetsRef.current.set(id, { ...pos, stationId: station });
    setAgents((prev) =>
      prev.map((a) => (a.id === id ? { ...a, activity: 'walking' as const, currentStation: station } : a))
    );
  }, []);

  // Direct movement: nudge an agent by dx/dz (for keyboard control in third-person)
  const moveAgent = useCallback((id: string, dx: number, dz: number) => {
    // Cancel any wander target so keyboard takes priority
    targetsRef.current.delete(id);
    setAgents((prev) =>
      prev.map((a) => {
        if (a.id !== id) return a;
        return {
          ...a,
          activity: 'walking' as const,
          position: { x: a.position.x + dx, y: a.position.y, z: a.position.z + dz },
        };
      })
    );
  }, []);

  // Wander behavior: each agent picks a new station every 15-30s
  useEffect(() => {
    const scheduleWander = (agentId: string) => {
      const timer = setTimeout(() => {
        setAgents((prev) => {
          const agent = prev.find((a) => a.id === agentId);
          if (agent && agent.activity === 'idle') {
            const station = pickRandomStation(agent.currentStation, agentId === 'librarian');
            setAgentTarget(agentId, station);
          }
          return prev;
        });
        scheduleWander(agentId);
      }, randomInterval());
      timersRef.current.set(agentId, timer);
    };

    INITIAL_AGENTS.forEach((a) => scheduleWander(a.id));
    return () => {
      timersRef.current.forEach((timer) => clearTimeout(timer));
      timersRef.current.clear();
    };
  }, [setAgentTarget]);

  // Animation loop: move agents toward targets
  useEffect(() => {
    let lastTime = performance.now();

    const animate = (time: number) => {
      const dt = (time - lastTime) / 1000;
      lastTime = time;

      setAgents((prev) => {
        let changed = false;
        const next = prev.map((agent) => {
          const target = targetsRef.current.get(agent.id);
          if (!target || agent.activity !== 'walking') return agent;

          const dx = target.x - agent.position.x;
          const dz = target.z - agent.position.z;
          const dist = Math.sqrt(dx * dx + dz * dz);

          if (dist < 0.3) {
            changed = true;
            targetsRef.current.delete(agent.id);
            return { ...agent, activity: 'idle' as const, position: { x: target.x, y: target.y, z: target.z } };
          }

          const step = Math.min(WALK_SPEED * dt, dist);
          const nx = agent.position.x + (dx / dist) * step;
          const nz = agent.position.z + (dz / dist) * step;
          changed = true;
          return { ...agent, position: { x: nx, y: target.y, z: nz } };
        });
        return changed ? next : prev;
      });

      animRef.current = requestAnimationFrame(animate);
    };

    animRef.current = requestAnimationFrame(animate);
    return () => cancelAnimationFrame(animRef.current);
  }, []);

  return { agents, setAgentTarget, moveAgent };
}
