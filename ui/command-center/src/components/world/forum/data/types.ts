/**
 * World W1 — typed evidence for the six places.
 *
 * Rules carried from Astra's 2026-10-12 rulings (product contract + state
 * truth), made into types so a renderer cannot skip them:
 * - Every record keeps its source (contract, endpoint, id, scope).
 * - Loading, error, empty, truncated, stale, unsupported and setup-required
 *   are VALUES, not nulls. An error keeps the last good data, marked stale;
 *   it is never turned into "nothing happened".
 * - A lost connection (`disconnected`) is a different value from an agent's
 *   failed task (a `RequestRecord` with outcome `failed`).
 * - Costs name their scope. Agent spend is "via the goals retrieved", not a
 *   lifetime total, and says so when those goals were truncated.
 */
import type { WorldRecordLink } from './recordLinks';
import type { AgentStateTruth } from './identity';

/** Astra's data contracts C1–C8 (ruling §1 "Existing data contracts to reuse"). */
export type ContractId = 'C1' | 'C2' | 'C3' | 'C4' | 'C5' | 'C6' | 'C7' | 'C8';

export interface SourceRef {
  contract: ContractId;
  /** Daemon endpoint the record came from, e.g. `/api/brain/memories`. */
  endpoint: string;
  /** The record's id in its home store. */
  id: string;
  /** What population this record belongs to, e.g. "attributed to git_steward". */
  scope: string;
}

export type FailureCause =
  /** The request never reached the daemon (daemon down, network). */
  | 'disconnected'
  /** The daemon refused the credential. */
  | 'unauthorized'
  /** The route or record does not exist on this daemon. */
  | 'not_found'
  /** The daemon answered with an error. */
  | 'server_error'
  /** The daemon answered, but reported this section unavailable. */
  | 'section_unavailable'
  /** The daemon answered with a shape this client cannot read. */
  | 'malformed';

export interface SectionFailure {
  cause: FailureCause;
  message: string;
  status: number | null;
}

export interface SectionMeta {
  contract: ContractId;
  endpoint: string;
  /** ISO time the data (or the failed attempt) was observed by this client. */
  fetchedAt: string;
  staleAfterMs: number;
}

export type Section<T> =
  | ({ state: 'ready'; items: readonly T[]; truncated: boolean; bound: number } & SectionMeta)
  /** The source answered and holds nothing in this scope. `copy` says what
   *  that does and does NOT mean (e.g. "nothing attributed in this window"). */
  | ({ state: 'empty'; copy: string } & SectionMeta)
  | ({
      state: 'error';
      failure: SectionFailure;
      /** The last successful answer for this source, shown marked stale.
       *  `items: []` means it last answered "nothing" — still a known answer. */
      lastGood: { items: readonly T[]; truncated: boolean; fetchedAt: string } | null;
    } & SectionMeta)
  /** This place/agent has no daemon source for the slot. Never a guess. */
  | { state: 'unsupported'; reason: string }
  /** A precondition is missing (no project chosen, no analytics, not bound).
   *  A setup state is not zero. */
  | { state: 'setup_required'; reason: string; action: string };

export type Freshness = 'fresh' | 'stale' | 'not_applicable';

/** Fresh/stale as observed at `now`. An error always reads stale. */
export function sectionFreshness(section: Section<unknown>, now: number): Freshness {
  if (section.state === 'unsupported' || section.state === 'setup_required') return 'not_applicable';
  if (section.state === 'error') return 'stale';
  const at = Date.parse(section.fetchedAt);
  if (!Number.isFinite(at)) return 'stale';
  return now - at > section.staleAfterMs ? 'stale' : 'fresh';
}

/** Items a renderer may show, with whether they are stale. */
export function visibleItems<T>(section: Section<T>, now: number): { items: readonly T[]; stale: boolean; truncated: boolean } {
  if (section.state === 'ready') return { items: section.items, stale: sectionFreshness(section, now) === 'stale', truncated: section.truncated };
  if (section.state === 'error' && section.lastGood) return { items: section.lastGood.items, stale: true, truncated: section.lastGood.truncated };
  return { items: [], stale: section.state === 'error', truncated: false };
}

// ── Records ─────────────────────────────────────────────────────────────────

export type Provenance =
  /** `source` on a Brain memory. A tag, not necessarily an openable document. */
  | { kind: 'source_tag'; tag: string }
  /** The memory key's prefix (`note:`, `code:`, `doc:`) names how it was written. */
  | { kind: 'key_origin'; origin: string }
  /** Nothing recorded. The UI must say so rather than invent one. */
  | { kind: 'none' };

export interface MemoryRecord {
  kind: 'memory';
  id: string;
  key: string | null;
  text: string;
  textClipped: boolean;
  description: string | null;
  timestamp: string;
  provenance: Provenance;
  projectId: string | null;
  link: WorldRecordLink;
  source: SourceRef;
}

/** Cards' lifecycle (cards.rs). `executing` = in_progress. */
export type GoalPhase = 'triage' | 'ready' | 'executing' | 'review' | 'complete' | 'failed' | 'cancelled' | 'unknown';

export interface GoalRecord {
  kind: 'goal';
  id: string;
  title: string;
  projectId: string;
  /** Raw daemon state, kept verbatim. */
  state: string;
  phase: GoalPhase;
  updatedAt: string;
  assignedTo: string | null;
  /** Review decisions on this goal, when the source returned them. */
  reviewDecisions: { status: 'ok'; count: number; truncated: boolean; /** Explicit acceptance: set only when the daemon names an accepted/approved verdict. */ verdict?: 'accepted' | 'approved' | null } | { status: 'unavailable'; reason: string } | null;
  link: WorldRecordLink;
  source: SourceRef;
}

export interface ActivityRecord {
  kind: 'activity';
  id: string;
  ts: string;
  actor: string;
  activityKind: string;
  title: string;
  detail: string | null;
  link: WorldRecordLink | null;
  source: SourceRef;
}

export interface DecisionRecord {
  kind: 'decision';
  id: string;
  headline: string;
  decisionKind: string;
  goalId: string | null;
  goalTitle: string | null;
  projectId: string | null;
  /** 2 = requires the user; 1 = policy may auto-answer. */
  tier: number;
  createdAt: string;
  /** Opening this NEVER answers it. */
  link: WorldRecordLink;
  source: SourceRef;
}

export interface SpendRecord {
  kind: 'spend';
  costUsd: number;
  callCount: number;
  estimatedCallCount: number;
  /** Plain sentence naming the scope. Always rendered beside the number. */
  scope: string;
  /** True when the goals behind this sum were themselves truncated. */
  lowerBound: boolean;
  note: string | null;
  source: SourceRef;
}

export type RequestOutcome = 'running' | 'completed' | 'failed' | 'cancelled' | 'other';

/** An orchestrator task (henry status recent_tasks). */
export interface RequestRecord {
  kind: 'request';
  id: string;
  description: string;
  status: string;
  outcome: RequestOutcome;
  toolUsed: string | null;
  completedAt: string | null;
  source: SourceRef;
}

export interface OrchestratorRecord {
  kind: 'orchestrator';
  name: string;
  /** Raw daemon string ("idle", "thinking", …). */
  currentState: string;
  currentTool: string | null;
  tasksInFlight: number;
  /** Real sessions only — the "Continue with…" offer. */
  sessions: Array<{ id: string; name: string; startedAt: string; link: WorldRecordLink }>;
  sessionsTruncated: boolean;
  truth: AgentStateTruth;
  source: SourceRef;
}

export type ForecastOutcome =
  | { kind: 'forecast'; method: string; methodLabel: string; horizon: number; maseVsBaseline: number | null; folds: number; foldWins: number }
  | { kind: 'refusal'; reason: string; detail: string | null; points: number | null; needed: number | null; lastCollectedAt: string | null };

export interface ForecastRecord {
  kind: 'forecast';
  seriesId: string;
  label: string;
  sourceLabel: string;
  cadence: string;
  verdict: string;
  outcome: ForecastOutcome;
  /** Most recent observations (observation, not projection). */
  history: number[];
  historyClipped: boolean;
  points: number;
  lastError: string | null;
  projectId: string;
  link: WorldRecordLink;
  source: SourceRef;
}

export interface CouncilRecord {
  kind: 'council_report';
  id: string;
  sessionId: string;
  status: string;
  generatedAt: string | null;
  headline: string;
  consensus: string[];
  consensusClipped: boolean;
  /** Dissent is kept as a count plus the raw entries' presence: disagreement
   *  must stay visible, never folded into consensus. */
  dissentCount: number;
  chair: string | null;
  link: WorldRecordLink;
  source: SourceRef;
}

/** All supported growth outcomes (growth_actions.rs:137) plus not-yet-judged. */
export type GrowthVerdict = 'helped' | 'hindered' | 'no_effect' | 'inconclusive' | 'confounded' | 'not_judged';

export interface GrowthRecord {
  kind: 'growth_result';
  actionId: string;
  projectId: string;
  projectName: string;
  title: string;
  category: string;
  /** Lifecycle status (implemented / measuring / judged …). Implemented ≠ helped. */
  status: string;
  verdict: GrowthVerdict;
  deltaPct: number | null;
  windowDays: number | null;
  judgedAt: string | null;
  targetMetric: string | null;
  link: WorldRecordLink;
  source: SourceRef;
}

export interface SkillRecord {
  kind: 'skill';
  id: string;
  name: string;
  description: string;
  status: string | null;
  usageCount: number | null;
  lastRun: string | null;
  toolUsed: string | null;
  link: WorldRecordLink;
  source: SourceRef;
}

export interface NoteRecord {
  kind: 'note';
  id: string;
  projectId: string;
  title: string | null;
  body: string;
  bodyClipped: boolean;
  memoryKey: string | null;
  createdAt: string;
  link: WorldRecordLink;
  source: SourceRef;
}

export interface AgentStateRecord {
  kind: 'agent_state';
  worldAgentId: string;
  agentId: string;
  displayName: string;
  truth: AgentStateTruth;
  link: WorldRecordLink;
  source: SourceRef;
}

/** Static local provenance (no request): the bundled Gaia sky backdrop. */
export interface SkyProvenanceRecord {
  kind: 'sky_provenance';
  path: string;
  statement: string;
}

/** Every evidence slot a place can show, and the record type it holds. */
export interface SlotItems {
  orchestrator: OrchestratorRecord;
  requests: RequestRecord;
  active_goals: GoalRecord;
  decisions: DecisionRecord;
  memories: MemoryRecord;
  librarian_activity: ActivityRecord;
  notes: NoteRecord;
  steward_activity: ActivityRecord;
  steward_goals: GoalRecord;
  steward_spend: SpendRecord;
  skills: SkillRecord;
  forecaster_activity: ActivityRecord;
  forecasts: ForecastRecord;
  council: CouncilRecord;
  hypotheses: NoteRecord;
  sky: SkyProvenanceRecord;
  growth_results: GrowthRecord;
  grower_state: AgentStateRecord;
  grower_activity: ActivityRecord;
}

export type EvidenceSlotId = keyof SlotItems;

export type PlaceSections = { [K in EvidenceSlotId]?: Section<SlotItems[K]> };
