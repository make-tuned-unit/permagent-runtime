/**
 * Settings → Agents API client.
 * Wire shapes mirror crates/goose-server/src/routes/agents_surface.rs — do not
 * invent fields or collapse tagged unions (unavailable must stay distinct from idle).
 */

import { apiFetch } from './api';

export type LiveState =
  | { status: 'ok'; value: string }
  | { status: 'not_queryable' }
  | { status: 'unavailable'; reason: string };

/**
 * The one boolean config key that switches an agent on. PRESENT AND NULL, never
 * omitted, for every worker row and every dispatch persona: a client must be
 * able to tell "this agent has no switch" from "the switch is off", and an
 * omitted field reads as `undefined`, which renders as a toggle claiming off.
 *
 * Declared here because this file is the wire mirror — but a daemon OLDER than
 * this app serialises no gate at all, so the panel still reads it through the
 * validating `readAgentGate` rather than trusting the type.
 */
export interface AgentGateWire {
  config_key: string;
  enabled: boolean;
}

/**
 * Whether this agent can be asked a question, or told to run a pass, RIGHT NOW.
 * Two states only, and the unavailable one carries the daemon's own reason —
 * a control the user cannot use must be able to say why, in the runtime's
 * words, rather than vanish or sit there looking hopeful.
 *
 * Named `AgentCapability` because `Capability` in this file is already the
 * platform-extension row; these are unrelated concepts on the same surface.
 */
export type AgentCapability =
  | { status: 'available' }
  | { status: 'unavailable'; reason: string };

/** A daemon older than this app serialises no `ask` / `run_now` at all. */
export const CAPABILITY_NOT_REPORTED =
  'this daemon does not report whether it can do this';

/** Present, but not in a shape this app can read — not the same as absent. */
export const CAPABILITY_UNREADABLE =
  'this daemon reported this capability in a shape this app cannot read';

/**
 * The sibling of `readAgentGate` (components/settings/agentsPanel.ts), and for
 * the same reason: the declared type above is a claim about a daemon we have
 * not spoken to yet, so it cannot be the check. A missing or malformed field
 * reads as UNAVAILABLE with a reason — never as available, because the whole
 * point of these two flags is that a control which cannot work says so instead
 * of failing when it is pressed.
 *
 * It lives here rather than beside `readAgentGate` because it validates a wire
 * field declared in this file, and the two must never drift apart.
 */
export function readAgentCapability(
  row: unknown,
  field: 'ask' | 'run_now',
): AgentCapability {
  if (typeof row !== 'object' || row === null) {
    return { status: 'unavailable', reason: CAPABILITY_NOT_REPORTED };
  }
  const cap = (row as Record<string, unknown>)[field];
  if (cap === undefined || cap === null) {
    return { status: 'unavailable', reason: CAPABILITY_NOT_REPORTED };
  }
  if (typeof cap !== 'object') {
    return { status: 'unavailable', reason: CAPABILITY_UNREADABLE };
  }
  const { status, reason } = cap as { status?: unknown; reason?: unknown };
  if (status === 'available') return { status: 'available' };
  if (status === 'unavailable' && typeof reason === 'string' && reason.length > 0) {
    return { status: 'unavailable', reason };
  }
  return { status: 'unavailable', reason: CAPABILITY_UNREADABLE };
}

export interface BackgroundWorker {
  id: string;
  display_name: string;
  what_it_does: string;
  why_it_matters: string;
  state_source: 'queryable' | 'static';
  live_state: LiveState;
  dispatchable: boolean;
  gate: AgentGateWire | null;
  /** ALWAYS serialised by a daemon that knows about it — read via
   *  `readAgentCapability`, never trusted from this type. */
  ask: AgentCapability;
  run_now: AgentCapability;
}

export type Availability =
  | { status: 'available' }
  | { status: 'unavailable'; reason: string }
  | { status: 'probe_failed'; reason: string };

export type Grants =
  | { mode: 'inherit_global' }
  | { mode: 'explicit'; extensions: string[]; truncated: boolean };

/** Externally tagged: string for unit variants, object for Unreadable(reason). */
export type SecretPresence =
  | 'present'
  | 'absent'
  | { unreadable: string };

export interface SecretItem {
  name: string;
  presence: SecretPresence;
}

export type Secrets =
  | { status: 'ok'; items: SecretItem[]; truncated: boolean }
  | { status: 'unavailable'; reason: string };

export interface DispatchPersona {
  key: string;
  display_name: string;
  role: string;
  cost_tier: string;
  engine: 'internal_subagent' | 'external_cli' | 'supervised_cli' | 'pending' | string;
  workflow_role: string | null;
  availability: Availability;
  grants: Grants;
  grants_enforced: boolean;
  secrets: Secrets;
  gate: AgentGateWire | null;
  /** Same contract as on a worker — see `readAgentCapability`. */
  ask: AgentCapability;
  run_now: AgentCapability;
}

/** One declared secret. `impact`/`unlocks` are serialised only for platform
 *  extensions (registry declarations); configured-transport entries omit
 *  both, mirroring `#[serde(skip_serializing_if = "Option::is_none")]`. */
export interface RequiredSecret {
  name: string;
  present: boolean;
  /** "degraded" = still works with gaps; "unavailable" = cannot do its job. */
  impact?: 'degraded' | 'unavailable' | string;
  /** Human sentence for what the secret unlocks. */
  unlocks?: string;
}

export type RequiredSecrets =
  | { status: 'declared'; items: RequiredSecret[]; truncated: boolean }
  | { status: 'not_declared' };

export interface Capability {
  key: string;
  display_name: string;
  description: string;
  enabled: boolean;
  /** null = source declares no default, not "off". */
  default_enabled: boolean | null;
  source: 'platform' | 'configured';
  required_secrets: RequiredSecrets;
}

export interface RosterResponse {
  workers: BackgroundWorker[];
  dispatch_roster: DispatchPersona[];
  capabilities: Capability[];
}

export type AgentDetail =
  | ({ kind: 'worker' } & BackgroundWorker)
  | ({ kind: 'dispatch_persona' } & DispatchPersona);

export type ListSection<T> =
  | { status: 'ok'; items: T[]; truncated: boolean }
  | { status: 'unavailable'; reason: string };

export interface JournalItem {
  id: string;
  ts: string;
  kind: string;
  actor: string;
  title: string;
  detail: string | null;
  ref_kind: string | null;
  ref_id: string | null;
  goal_project_id: string | null;
}

/** Activity flattens ListSection beside attribution. */
export type ActivitySection =
  | { attribution: string; status: 'ok'; items: JournalItem[]; truncated: boolean }
  | { attribution: string; status: 'unavailable'; reason: string };

export interface ReviewDecision {
  answer: string | null;
  acted_by: string | null;
}

export interface GoalItem {
  id: string;
  title: string;
  project_id: string;
  state: string;
  updated_at: string;
  review_decisions: ListSection<ReviewDecision>;
}

export interface SpendItem {
  cost_usd: number;
  call_count: number;
  estimated_call_count: number;
  attribution: 'via_goal_id' | string;
  note: string | null;
}

export interface ScheduledJobItem {
  id: string;
  cron: string;
  at: string | null;
  every: number | null;
  paused: boolean;
  run_count: number;
  last_run: string | null;
  last_status: string | null;
  last_error: string | null;
  consecutive_failures: number;
}

/** How a pass was started. `manual` is the one a person pressed. */
export type RunTrigger = 'interval' | 'manual';

/**
 * `skipped` is the agent WORKING AS DESIGNED — nothing was due, or a
 * precondition said not this pass. It is not a failure and must never be
 * rendered as one; `failed` is the only outcome that went wrong.
 */
export type RunOutcome = 'ok' | 'skipped' | 'failed';

export interface AgentRun {
  id: string;
  agent_id: string;
  trigger: RunTrigger;
  outcome: RunOutcome;
  started_at: string;
  finished_at: string;
  /** null = this pass does not count items at all. NOT zero — rendering it as
   *  0 would claim the agent looked at nothing, which is a different fact. */
  examined: number | null;
  /** null = produced nothing, which is the normal result of a healthy sweep. */
  produced: string | null;
  /** Why it skipped, or how it failed. null when neither applies. */
  reason: string | null;
}

/**
 * Three outcomes that must stay apart on screen:
 *   ok            — runs were recorded (the list may still be empty)
 *   not_recorded  — this agent's code records no runs AT ALL, so an empty list
 *                   would be a lie about the agent rather than about the data
 *   unavailable   — the record could not be READ
 */
export type RunsSection =
  | { status: 'ok'; items: AgentRun[]; truncated: boolean }
  | { status: 'not_recorded'; reason: string }
  | { status: 'unavailable'; reason: string };

export interface BriefingItem {
  id: string;
  from_agent: string;
  kind: string;
  severity: 'info' | 'attention' | 'action_required';
  summary: string;
  detail: string | null;
  ref_kind: string | null;
  ref_id: string | null;
  created_at: string;
  acknowledged_at: string | null;
}

export interface WorkReview {
  /** First in the type as it is first on screen: the direct answer to "did it
   *  actually do the thing". */
  runs: RunsSection;
  briefings: ListSection<BriefingItem>;
  activity: ActivitySection;
  goals: ListSection<GoalItem>;
  spend: ListSection<SpendItem>;
  scheduled_jobs: ListSection<ScheduledJobItem>;
}

/**
 * What the tools of an ask turn ACTUALLY were — named for what happened, not
 * for what was intended. Narrowing can only ever REMOVE, so `granted` and
 * `applied` differ exactly when a declared grant was globally disabled and
 * silently produced nothing.
 *
 * A tagged union, and it stays one: flattening it to a string would lose the
 * distinction between "carried everyone's tools" and "carried its own", which
 * is the fact that makes an ask answer attributable at all.
 */
export type AppliedToolScope =
  | { mode: 'inherit_global'; extensions: string[] }
  | { mode: 'explicit'; granted: string[]; applied: string[] };

/** What the agent answered, and under whose identity and tool scope. */
export interface AskAnswer {
  answer: string;
  display_name: string;
  /** false = answered WITHOUT this agent's persona applied. */
  persona_applied: boolean;
  tool_scope: AppliedToolScope;
}

export interface SecretWriteResponse {
  name: string;
  presence: 'present' | 'absent';
}

export function fetchRoster(): Promise<RosterResponse> {
  return apiFetch<RosterResponse>('/api/agents/roster');
}

export function fetchAgentDetail(id: string): Promise<AgentDetail> {
  return apiFetch<AgentDetail>(`/api/agents/${encodeURIComponent(id)}`);
}

export function fetchAgentWork(id: string, limit?: number): Promise<WorkReview> {
  const q = limit !== undefined ? `?limit=${encodeURIComponent(String(limit))}` : '';
  return apiFetch<WorkReview>(`/api/agents/${encodeURIComponent(id)}/work${q}`);
}

/**
 * Ask THIS agent a question, under its own persona and tool scope. Request /
 * response — the answer arrives whole, there is no token stream to watch.
 */
export function askAgent(id: string, question: string): Promise<AskAnswer> {
  return apiFetch<AskAnswer>(`/api/agents/${encodeURIComponent(id)}/ask`, {
    method: 'POST',
    body: JSON.stringify({ question }),
  });
}

/**
 * Run one pass NOW and return the run it recorded.
 *
 * Request / response: this resolves when the pass has FINISHED and the runtime
 * has written its row — there is no progress stream behind it, so nothing built
 * on this may be labelled "live" or "streaming".
 */
export function runAgentNow(id: string): Promise<{ run: AgentRun }> {
  return apiFetch<{ run: AgentRun }>(`/api/agents/${encodeURIComponent(id)}/run`, {
    method: 'POST',
  });
}

/** null = inherit global; [] = grant nothing; [k,…] = narrowed. */
export function saveGrants(id: string, extensions: string[] | null): Promise<DispatchPersona> {
  return apiFetch<DispatchPersona>(`/api/agents/${encodeURIComponent(id)}/grants`, {
    method: 'POST',
    body: JSON.stringify({ extensions }),
  });
}

/** value null deletes. Never log or echo the value after write. */
export function saveSecret(
  id: string,
  name: string,
  value: string | null,
): Promise<SecretWriteResponse> {
  return apiFetch<SecretWriteResponse>(`/api/agents/${encodeURIComponent(id)}/secrets`, {
    method: 'POST',
    body: JSON.stringify({ name, value }),
  });
}
