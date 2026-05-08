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
];

function randomInterval(): number {
  return WANDER_INTERVAL_MIN + Math.random() * (WANDER_INTERVAL_MAX - WANDER_INTERVAL_MIN);
}

function pickRandomStation(currentStation: string | null): string {
  const available = STATIONS.filter((s) => s.id !== currentStation);
  const idx = Math.floor(Math.random() * (available.length + 1));
  if (idx === available.length) return 'center';
  return available[idx].id;
}

function getStationPosition(stationId: string): { x: number; y: number; z: number } {
  if (stationId === 'center') return { x: 0, y: 0, z: 0 };
  const station = STATIONS.find((s) => s.id === stationId);
  if (!station) return { x: 0, y: 0, z: 0 };
  return { x: station.position[0], y: 0, z: station.position[2] };
}

export function useAgentStates(): {
  agents: AgentState[];
  setAgentTarget: (id: string, station: string) => void;
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

  // Wander behavior: each agent picks a new station every 15-30s
  useEffect(() => {
    const scheduleWander = (agentId: string) => {
      const timer = setTimeout(() => {
        setAgents((prev) => {
          const agent = prev.find((a) => a.id === agentId);
          if (agent && agent.activity === 'idle') {
            const station = pickRandomStation(agent.currentStation);
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
            return { ...agent, activity: 'idle' as const, position: { x: target.x, y: 0, z: target.z } };
          }

          const step = Math.min(WALK_SPEED * dt, dist);
          const nx = agent.position.x + (dx / dist) * step;
          const nz = agent.position.z + (dz / dist) * step;
          changed = true;
          return { ...agent, position: { x: nx, y: 0, z: nz } };
        });
        return changed ? next : prev;
      });

      animRef.current = requestAnimationFrame(animate);
    };

    animRef.current = requestAnimationFrame(animate);
    return () => cancelAnimationFrame(animRef.current);
  }, []);

  return { agents, setAgentTarget };
}
