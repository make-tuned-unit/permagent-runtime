/**
 * World W1 — the evidence client interface W1 owns.
 *
 * Read-only by construction: `WorldEvidenceClient` has no method that writes,
 * sends a message or answers anything, so the snapshot loader cannot cause a
 * side effect however it is driven. The one write World needs (saving a field
 * note to a project) is a separate `WorldNoteWriter`, called only by an
 * explicit Save action (drafts.ts `saveFieldNoteToProject`).
 *
 * Wire shapes mirror the daemon routes cited per method; where an existing
 * client type already mirrors a route, it is imported (type-only) rather than
 * redefined. No change to lib/api.ts or lib/store.ts is required: the default
 * implementation calls the exported `apiFetch` with the same paths existing
 * surfaces use.
 */
import type { AgentDetail, WorkReview } from '../../../../lib/agentsApi';
import type { CouncilReportsResponse, Skill } from '../../../../lib/api';
import type { ProjectMemory, ProjectNote } from '../../../projects/types';
import type { GrowthResultsData } from '../../../grow/growthResults';
import type { FailureCause, SectionFailure } from './types';

/** GET /api/henry/status (henry_status.rs:433; typed in api.ts getHenryStatus). */
export interface HenryStatusWire {
  identity: { name: string; traits: string[]; tone: string };
  current_state: string;
  active_sessions: { id: string; name: string; started_at: string }[];
  current_tool: string | null;
  tasks_in_flight: number;
  recent_tasks: { id: string; description: string; status: string; tool_used: string | null; completed_at: string | null }[];
}

/** GET /api/goals/active (cards.rs:1043; mirrored by lib/useLiveGoals.ts ActiveGoal). */
export interface ActiveGoalsWire {
  count?: number;
  goals: Array<{ id: string; title: string; project_id: string; state: string; assigned_to: string | null; updated_at: string }>;
}

/** GET /api/decisions (decisions.rs:53-59, 1130): { items, summary, attention_goals }. */
export interface DecisionsWire {
  items: Array<{
    id: string; kind: string; goal_id: string | null; project_id: string | null; tier: number;
    headline: string; status: string; created_at: string; goal_title?: string | null;
  }>;
  summary: { total_pending: number };
}

/** GET /api/brain/memories (brain.rs:1587; typed in api.ts getBrainMemories). */
export interface BrainMemoriesWire {
  memories: Array<{ id: string; key: string | null; text: string; description: string | null; timestamp: string; source?: string | null }>;
  total: number;
  has_more: boolean;
}

/** One row of GET /api/projects/{id}/market (projects.rs:2215; MarketPanel.tsx MarketRow). */
export interface MarketRowWire {
  seriesId: string;
  sourceLabel: string;
  label: string;
  cadence: string;
  status: string;
  points: number;
  lastError: string | null;
  verdict: string;
  history: number[];
  forecast: { point: number[]; method: string; methodLabel: string; maseVsBaseline: number | null; folds: number; foldWins: number } | null;
  refusal: { reason: string; points?: number; needed?: number; lastCollectedAt?: string | null; detail?: string } | null;
}

export interface MarketWire {
  rows: MarketRowWire[];
  noSeriesBound: boolean;
  generatedAt: string;
}

export interface WorldEvidenceClient {
  henryStatus(signal: AbortSignal): Promise<HenryStatusWire>;
  activeGoals(signal: AbortSignal): Promise<ActiveGoalsWire>;
  decisions(signal: AbortSignal): Promise<DecisionsWire>;
  brainMemories(limit: number, signal: AbortSignal): Promise<BrainMemoriesWire>;
  projectMemories(projectId: string, signal: AbortSignal): Promise<ProjectMemory[]>;
  projectNotes(projectId: string, signal: AbortSignal): Promise<ProjectNote[]>;
  agentDetail(agentId: string, signal: AbortSignal): Promise<AgentDetail>;
  agentWork(agentId: string, limit: number, signal: AbortSignal): Promise<WorkReview>;
  skills(signal: AbortSignal): Promise<Skill[]>;
  projectMarket(projectId: string, signal: AbortSignal): Promise<MarketWire>;
  councilReports(limit: number, signal: AbortSignal): Promise<CouncilReportsResponse>;
  growthResults(projectId: string | null, signal: AbortSignal): Promise<GrowthResultsData>;
}

/** The single write World performs, only from an explicit Save. */
export interface WorldNoteWriter {
  createProjectNote(projectId: string, note: { title?: string; body: string }): Promise<ProjectNote>;
}

type Fetcher = <T>(endpoint: string, init?: RequestInit) => Promise<T>;

const e = encodeURIComponent;

/**
 * Default client over `apiFetch` (lib/api.ts:864). Pass it in rather than
 * importing so tests and fixtures never touch the network module.
 */
export function createApiEvidenceClient(fetcher: Fetcher): WorldEvidenceClient {
  const get = <T,>(path: string, signal: AbortSignal) => fetcher<T>(path, { signal });
  return {
    henryStatus: (s) => get('/api/henry/status', s),
    activeGoals: (s) => get('/api/goals/active', s),
    decisions: (s) => get('/api/decisions', s),
    brainMemories: (limit, s) => get(`/api/brain/memories?limit=${e(String(limit))}`, s),
    projectMemories: (id, s) => get(`/api/projects/${e(id)}/memories`, s),
    projectNotes: (id, s) => get(`/api/projects/${e(id)}/notes`, s),
    agentDetail: (id, s) => get(`/api/agents/${e(id)}`, s),
    agentWork: (id, limit, s) => get(`/api/agents/${e(id)}/work?limit=${e(String(limit))}`, s),
    // Deliberately NOT api.getSkills(): that wrapper turns every failure into
    // [], which would render a daemon error as "no skills saved".
    skills: (s) => get('/permagent/skills', s),
    projectMarket: (id, s) => get(`/api/projects/${e(id)}/market`, s),
    councilReports: (limit, s) => get(`/api/council/reports?limit=${e(String(limit))}`, s),
    growthResults: (id, s) => get(id ? `/api/growth-results?projectId=${e(id)}` : '/api/growth-results', s),
  };
}

export function createApiNoteWriter(fetcher: Fetcher): WorldNoteWriter {
  return {
    createProjectNote: (projectId, note) =>
      fetcher<ProjectNote>(`/api/projects/${e(projectId)}/notes`, { method: 'POST', body: JSON.stringify(note) }),
  };
}

/**
 * Classify a thrown request error. apiFetch attaches `status` for HTTP errors
 * (api.ts:950-955); a thrown error without one never reached the daemon.
 */
export function classifyFailure(error: unknown): SectionFailure {
  const status = typeof (error as { status?: unknown })?.status === 'number' ? (error as { status: number }).status : null;
  const message = error instanceof Error ? error.message : String(error);
  let cause: FailureCause;
  if (status === null) cause = error instanceof SyntaxError ? 'malformed' : 'disconnected';
  else if (status === 401 || status === 403) cause = 'unauthorized';
  else if (status === 404) cause = 'not_found';
  else cause = 'server_error';
  return { cause, message, status };
}

export function isAbort(error: unknown): boolean {
  return (error as { name?: string })?.name === 'AbortError';
}
