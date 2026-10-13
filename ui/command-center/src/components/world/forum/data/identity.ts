/**
 * World W1 — identity mapping: World agents and places ↔ daemon identities.
 *
 * The World roster (components/world/agents/roster.ts) and the agents API are
 * separate id namespaces. This module is the ONE place a World id becomes a
 * daemon id, and it never guesses: an avatar with no daemon source resolves to
 * `unsupported` with a stated reason, so no caller can blindly call another
 * agent's `/work` or silently substitute its records (Astra ruling §1,
 * "Identity mapping is mandatory").
 *
 * Daemon bindings (paths in this checkout):
 * - Workers: `GET /api/agents/{id}` and `/api/agents/{id}/work`
 *   (crates/goose-server/src/routes/agents_surface.rs:1089-1090). `/work`
 *   answers 404 for any id that is neither a configured worker nor in
 *   WORKER_DESCRIPTORS (agents_surface.rs:742-749;
 *   crates/goose/src/agents/self_knowledge/mod.rs:146-172).
 * - Orchestrator: `GET /api/henry/status` (routes/henry_status.rs:433). It has
 *   no agents-API entry.
 * - The existing World→agents map lives in lib/worldAgentIds.ts:11-20 and is
 *   reused here; identity.test.ts pins the two against each other.
 */
import { agentIdForWorldAgent } from '../../../../lib/worldAgentIds';
import type { AgentDetail, WorkReview } from '../../../../lib/agentsApi';

/** The six current places (habitatLayout.ts:69-75 HABITAT_PLACES ids). */
export const WORLD_PLACE_IDS = ['commons', 'gallery', 'maker', 'council_garden', 'observatory_garden', 'conservatory'] as const;
export type WorldPlaceId = (typeof WORLD_PLACE_IDS)[number];

export function isWorldPlaceId(id: string): id is WorldPlaceId {
  return (WORLD_PLACE_IDS as readonly string[]).includes(id);
}

/** Host agent per place, as authored in habitatLayout.ts:69-75. */
export const PLACE_HOST: Readonly<Record<WorldPlaceId, string>> = {
  commons: 'henry',
  gallery: 'librarian',
  maker: 'steward',
  council_garden: 'forecaster',
  observatory_garden: 'forecaster',
  conservatory: 'growth_measurement',
};

/** Every World roster id (components/world/agents/roster.ts:50-268). */
export const WORLD_AGENT_IDS = [
  'henry', 'librarian', 'reader', 'watcher', 'steward', 'strix',
  'financier', 'forecaster', 'council', 'polybot', 'picker', 'growth_measurement',
] as const;

export type WorldAgentIdentity =
  | { status: 'orchestrator'; worldId: string; statusEndpoint: '/api/henry/status'; binding: string }
  | { status: 'worker'; worldId: string; agentId: string; detailEndpoint: string; workEndpoint: string; binding: string }
  | { status: 'unsupported'; worldId: string; reason: string };

/** Where each worker id is declared on the daemon (for the receipt and tests). */
const WORKER_BINDING: Readonly<Record<string, string>> = {
  librarian: 'crates/goose/src/agents/platform_extensions/librarian.rs:30 id "librarian"',
  watcher: 'crates/goose/src/echo.rs:18 id "watcher"',
  git_steward: 'crates/goose/src/steward/mod.rs:508 id "git_steward"',
  strix: 'crates/goose/src/strix.rs:42 STRIX_FEATURE_ID "strix"',
  financier: 'crates/goose/src/agents/platform_extensions/finance.rs:1118 id "financier"',
  forecaster: 'crates/goose/src/agents/platform_extensions/forecaster.rs:57 AGENT_ID "forecaster"',
  council: 'crates/goose/src/council/mod.rs:30 AGENT_ID "council"',
  growth_measurement: 'crates/goose/src/growth/mod.rs:52 id "growth_measurement"',
};

/** Explicit reasons for avatars with no daemon agent. */
const UNSUPPORTED_REASON: Readonly<Record<string, string>> = {
  reader:
    'The Reader is a surface, not a background worker. Its descriptor (crates/goose/src/reader/mod.rs:47) is not in WORKER_DESCRIPTORS, so the agents API has no work record for it.',
  polybot: 'Polybot has no agents-API entry on this daemon, so no configuration or attributed work can be shown.',
  picker: 'Picker has no agents-API entry on this daemon (its scanner is an external service), so no configuration or attributed work can be shown.',
};

/** Resolve a World roster id to its daemon identity. Never a guess. */
export function resolveWorldAgent(worldId: string): WorldAgentIdentity {
  if (worldId === 'henry') {
    return { status: 'orchestrator', worldId, statusEndpoint: '/api/henry/status', binding: 'crates/goose-server/src/routes/henry_status.rs:433' };
  }
  const agentId = agentIdForWorldAgent(worldId);
  if (agentId && WORKER_BINDING[agentId]) {
    return {
      status: 'worker',
      worldId,
      agentId,
      detailEndpoint: `/api/agents/${encodeURIComponent(agentId)}`,
      workEndpoint: `/api/agents/${encodeURIComponent(agentId)}/work`,
      binding: WORKER_BINDING[agentId],
    };
  }
  return {
    status: 'unsupported',
    worldId,
    reason: UNSUPPORTED_REASON[worldId] ?? `"${worldId}" is not a World agent this client can map to a daemon identity.`,
  };
}

/** The daemon agent id whose `/work` may be called for this World id, or null. */
export function workAgentIdFor(worldId: string): string | null {
  const identity = resolveWorldAgent(worldId);
  return identity.status === 'worker' ? identity.agentId : null;
}

// ── State truth: configured vs reachable vs ready vs actively used ──────────
// Astra UX ruling rank 2: "Distinguish configured, reachable, ready, and
// actively used." Each fact carries the evidence sentence it rests on.

export interface TruthFact<V extends string> {
  value: V;
  evidence: string;
}

export interface AgentStateTruth {
  configured: TruthFact<'on' | 'off' | 'no_switch' | 'unknown'>;
  reachable: TruthFact<'reachable' | 'unreachable' | 'probe_failed' | 'not_probed' | 'unknown'>;
  ready: TruthFact<'ready' | 'not_ready' | 'unknown'>;
  activelyUsed: TruthFact<'active_now' | 'attributed_in_window' | 'none_attributed_in_window' | 'unknown'>;
}

const UNKNOWN_USE: AgentStateTruth['activelyUsed'] = { value: 'unknown', evidence: 'Attributed work was not read.' };

function usageFromWork(work: WorkReview | null | undefined): AgentStateTruth['activelyUsed'] {
  if (!work) return UNKNOWN_USE;
  const a = work.activity;
  if (a.status === 'unavailable') return { value: 'unknown', evidence: `Attributed activity unavailable: ${a.reason}` };
  if (a.items.length === 0) {
    return {
      value: 'none_attributed_in_window',
      evidence: `Nothing attributed to "${a.attribution}" in the retrieved window. This does not prove the agent is inactive.`,
    };
  }
  return {
    value: 'attributed_in_window',
    evidence: `${a.items.length}${a.truncated ? '+' : ''} activity record(s) attributed to "${a.attribution}"; latest ${a.items[0]?.ts ?? 'unknown'}.`,
  };
}

/** Truth for a worker / dispatch persona from GET /api/agents/{id} (+ /work). */
export function truthFromAgent(detail: AgentDetail, work?: WorkReview | null): AgentStateTruth {
  const gate = detail.gate;
  const configured: AgentStateTruth['configured'] = gate == null
    ? { value: 'no_switch', evidence: 'This agent has no on/off switch; it is not proof it is running.' }
    : gate.enabled
      ? { value: 'on', evidence: `Switch ${gate.config_key} is on. A configured role is not proof of current execution.` }
      : { value: 'off', evidence: `Switch ${gate.config_key} is off.` };

  let reachable: AgentStateTruth['reachable'];
  if (detail.kind === 'worker') {
    const live = detail.live_state;
    reachable = live.status === 'ok'
      ? { value: 'reachable', evidence: `Live state reported: ${live.value}.` }
      : live.status === 'not_queryable'
        ? { value: 'not_probed', evidence: 'This worker exposes no live state to query.' }
        : { value: 'unreachable', evidence: `Live state unavailable: ${live.reason}` };
  } else {
    const av = detail.availability;
    reachable = av.status === 'available'
      ? { value: 'reachable', evidence: 'Availability probe passed.' }
      : av.status === 'unavailable'
        ? { value: 'unreachable', evidence: `Unavailable: ${av.reason}` }
        : { value: 'probe_failed', evidence: `Availability probe failed: ${av.reason}` };
  }

  let ready: AgentStateTruth['ready'];
  if (configured.value === 'off') ready = { value: 'not_ready', evidence: 'Switched off.' };
  else if (reachable.value === 'unreachable' || reachable.value === 'probe_failed') ready = { value: 'not_ready', evidence: reachable.evidence };
  else if (reachable.value === 'reachable') ready = { value: 'ready', evidence: 'Configured and reachable.' };
  else ready = { value: 'unknown', evidence: 'Readiness cannot be confirmed without a live probe.' };

  return { configured, reachable, ready, activelyUsed: usageFromWork(work) };
}

/** Minimal shape of /api/henry/status this module reads. */
export interface HenryStatusLike {
  current_state: string;
  current_tool: string | null;
  tasks_in_flight: number;
}

/**
 * Truth for the orchestrator from /api/henry/status. The status route answers
 * even with no model configured, so `configured` and `ready` stay unknown
 * here: a reachable daemon is not a working model (UX ruling rank 2).
 */
export function truthFromHenryStatus(status: HenryStatusLike | null): AgentStateTruth {
  if (!status) {
    return {
      configured: { value: 'unknown', evidence: 'Orchestrator status was not read.' },
      reachable: { value: 'unknown', evidence: 'Orchestrator status was not read.' },
      ready: { value: 'unknown', evidence: 'Orchestrator status was not read.' },
      activelyUsed: UNKNOWN_USE,
    };
  }
  const active = status.tasks_in_flight > 0 || !!status.current_tool;
  return {
    configured: { value: 'unknown', evidence: 'The status route does not report model configuration.' },
    reachable: { value: 'reachable', evidence: `Status route answered: ${status.current_state}.` },
    ready: { value: 'unknown', evidence: 'A reachable daemon is not proof a model can answer.' },
    activelyUsed: active
      ? { value: 'active_now', evidence: status.current_tool ? `Using ${status.current_tool} now.` : `${status.tasks_in_flight} task(s) in flight.` }
      : { value: 'none_attributed_in_window', evidence: 'No task in flight at the time of reading.' },
  };
}
