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

export interface BackgroundWorker {
  id: string;
  display_name: string;
  what_it_does: string;
  why_it_matters: string;
  state_source: 'queryable' | 'static';
  live_state: LiveState;
  dispatchable: boolean;
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
}

export type RequiredSecrets =
  | { status: 'declared'; items: { name: string; present: boolean }[]; truncated: boolean }
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

export interface WorkReview {
  activity: ActivitySection;
  goals: ListSection<GoalItem>;
  spend: ListSection<SpendItem>;
  scheduled_jobs: ListSection<ScheduledJobItem>;
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
