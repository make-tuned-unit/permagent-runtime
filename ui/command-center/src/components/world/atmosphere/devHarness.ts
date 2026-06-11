// DEV-ONLY harness — never active in production builds.
// Lets evidence recordings drive shared/agentStatus with test 'daemon' states
// to demonstrate activity-reactive ambience (idle room vs busy room) before
// W3's real daemon bindings land. The sim-state clamp in shared/agentStatus
// is intentionally NOT bypassed: this feeds source='daemon' exactly like the
// real Henry/Librarian bindings will.

import { setAgentSource, removeAgent } from '../shared/agentStatus';
import type { AgentHudState } from '../shared/palette';

declare global {
  interface Window {
    __worldDev?: {
      setAgentState: (id: string, state: AgentHudState, name?: string) => void;
      removeAgent: (id: string) => void;
    };
  }
}

export function installDevHarness(): void {
  if (!import.meta.env.DEV || typeof window === 'undefined') return;
  window.__worldDev = {
    setAgentState: (id: string, state: AgentHudState, name?: string) =>
      setAgentSource(id, name ?? id, state, 'daemon'),
    removeAgent,
  };
}
