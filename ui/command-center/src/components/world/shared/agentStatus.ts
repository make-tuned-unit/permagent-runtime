// Agent runtime-state boundary — WORLD_VIEW_BIBLE.md §6 + §4 sim-state rule.
// API FROZEN after Phase 0. W3 feeds sources via setAgentSource(); all world visuals
// read via useAgentRuntimeStates(). The sim-state clamp lives HERE so the rule
// "sim agents never show working/error" reaches the pixels regardless of caller.

import { useSyncExternalStore } from 'react';
import type { AgentHudState } from './palette';

export type StateSource = 'daemon' | 'sim';

export interface AgentRuntimeState {
  id: string;
  name: string;
  hudState: AgentHudState;
  /** Honesty bit. Evidence recordings of working/error states must show 'daemon'. */
  source: StateSource;
}

/**
 * SIM-STATE RULE (bible §4, LAW): simulated agents may only display idle or
 * available. Amber working and red error are claims that real work is happening
 * or really failing — a sim never makes that claim. When daemon AgentStateChanged
 * events ship, sources flip to 'daemon' and the full range opens automatically.
 */
function clamp(state: AgentHudState, source: StateSource): AgentHudState {
  if (source === 'sim' && (state === 'working' || state === 'error')) return 'available';
  return state;
}

const states = new Map<string, AgentRuntimeState>();
let snapshot: AgentRuntimeState[] = [];
const listeners = new Set<() => void>();

function publish(): void {
  snapshot = [...states.values()];
  listeners.forEach((l) => l());
}

/** W3 only: report an agent's state from its source (Henry poll, Librarian events, sim). */
export function setAgentSource(
  id: string,
  name: string,
  hudState: AgentHudState,
  source: StateSource,
): void {
  const next: AgentRuntimeState = { id, name, hudState: clamp(hudState, source), source };
  const prev = states.get(id);
  if (
    prev &&
    prev.hudState === next.hudState &&
    prev.source === next.source &&
    prev.name === next.name
  )
    return;
  states.set(id, next);
  publish();
}

export function removeAgent(id: string): void {
  if (states.delete(id)) publish();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useAgentRuntimeStates(): AgentRuntimeState[] {
  return useSyncExternalStore(subscribe, () => snapshot);
}

/** Non-hook accessor for useFrame loops (no per-frame React work). */
export function getAgentRuntimeStates(): AgentRuntimeState[] {
  return snapshot;
}
