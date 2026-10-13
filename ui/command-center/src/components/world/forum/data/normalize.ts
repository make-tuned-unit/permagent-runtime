/**
 * World W1 — pure normalizers: daemon wire → bounded, attributed Sections.
 *
 * Pure and deterministic: the same wire payload and the same `fetchedAt`
 * always produce a deep-equal result (the replay gate). No clock reads, no
 * randomness, no network.
 */
import type { WorkReview, AgentDetail, ListSection } from '../../../../lib/agentsApi';
import type { CouncilReportsResponse, Skill } from '../../../../lib/api';
import type { ProjectMemory, ProjectNote } from '../../../projects/types';
import type { GrowthResultsData, GrowthResultRow } from '../../../grow/growthResults';
import { BOUNDS, STALE_AFTER_MS, clipText, type StaleFamily } from './bounds';
import type {
  ActiveGoalsWire, BrainMemoriesWire, DecisionsWire, HenryStatusWire, MarketWire,
} from './evidenceClient';
import { truthFromAgent, truthFromHenryStatus } from './identity';
import type {
  ActivityRecord, AgentStateRecord, ContractId, CouncilRecord, DecisionRecord, ForecastRecord,
  GoalPhase, GoalRecord, GrowthRecord, GrowthVerdict, MemoryRecord, NoteRecord, OrchestratorRecord,
  Provenance, RequestOutcome, RequestRecord, Section, SectionFailure, SectionMeta, SkillRecord, SpendRecord,
} from './types';

export interface NormalizeContext {
  fetchedAt: string;
}

function meta(contract: ContractId, endpoint: string, family: StaleFamily, ctx: NormalizeContext): SectionMeta {
  return { contract, endpoint, fetchedAt: ctx.fetchedAt, staleAfterMs: STALE_AFTER_MS[family] };
}

/** Bound a list into a ready / empty section. `moreExists` = source said so. */
function listSection<T>(m: SectionMeta, all: readonly T[], bound: number, emptyCopy: string, moreExists = false): Section<T> {
  if (all.length === 0) return { state: 'empty', copy: emptyCopy, ...m };
  return { state: 'ready', items: all.slice(0, bound), truncated: moreExists || all.length > bound, bound, ...m };
}

/** An error section that keeps the last good data (rendered stale). */
export function errorSection<T>(m: SectionMeta, failure: SectionFailure, previous: Section<T> | undefined): Section<T> {
  let lastGood: { items: readonly T[]; truncated: boolean; fetchedAt: string } | null = null;
  if (previous?.state === 'ready') lastGood = { items: previous.items, truncated: previous.truncated, fetchedAt: previous.fetchedAt };
  // A source that last answered "nothing" is still a last known answer.
  else if (previous?.state === 'empty') lastGood = { items: [], truncated: false, fetchedAt: previous.fetchedAt };
  else if (previous?.state === 'error') lastGood = previous.lastGood;
  return { state: 'error', failure, lastGood, ...m };
}

const title = (t: string) => clipText(t, BOUNDS.titleChars).text;

export function goalPhase(state: string): GoalPhase {
  switch (state) {
    case 'triage': return 'triage';
    case 'ready': return 'ready';
    case 'in_progress': return 'executing';
    case 'review': return 'review';
    case 'complete': case 'completed': case 'done': return 'complete';
    case 'failed': return 'failed';
    case 'cancelled': case 'canceled': case 'abandoned': return 'cancelled';
    default: return 'unknown';
  }
}

export function requestOutcome(status: string): RequestOutcome {
  const s = status.toLowerCase();
  if (s === 'running' || s === 'in_progress' || s === 'pending' || s === 'started') return 'running';
  if (s === 'completed' || s === 'complete' || s === 'success' || s === 'succeeded' || s === 'done') return 'completed';
  if (s === 'failed' || s === 'error' || s === 'errored') return 'failed';
  if (s === 'cancelled' || s === 'canceled' || s === 'aborted') return 'cancelled';
  return 'other';
}

export function growthVerdict(verdict: string | null): GrowthVerdict {
  switch (verdict) {
    case 'helped': case 'hindered': case 'no_effect': case 'inconclusive': case 'confounded': return verdict;
    case null: return 'not_judged';
    // An unknown verdict string is not evidence of anything: keep it honest.
    default: return 'inconclusive';
  }
}

// ── C1 / C3: orchestrator ───────────────────────────────────────────────────

export function normalizeOrchestrator(w: HenryStatusWire, ctx: NormalizeContext): Section<OrchestratorRecord> {
  const m = meta('C3', '/api/henry/status', 'orchestrator', ctx);
  const sessions = w.active_sessions.slice(0, BOUNDS.sessions).map((s) => ({
    id: s.id, name: title(s.name || 'Untitled conversation'), startedAt: s.started_at,
    link: { to: 'conversation' as const, sessionId: s.id },
  }));
  const record: OrchestratorRecord = {
    kind: 'orchestrator',
    name: w.identity?.name || 'Orchestrator',
    currentState: w.current_state,
    currentTool: w.current_tool,
    tasksInFlight: w.tasks_in_flight,
    sessions,
    sessionsTruncated: w.active_sessions.length > BOUNDS.sessions,
    truth: truthFromHenryStatus(w),
    source: { contract: 'C3', endpoint: m.endpoint, id: 'orchestrator', scope: 'the configured orchestrator' },
  };
  return { state: 'ready', items: [record], truncated: false, bound: 1, ...m };
}

export function normalizeRequests(w: HenryStatusWire, ctx: NormalizeContext): Section<RequestRecord> {
  const m = meta('C3', '/api/henry/status', 'orchestrator', ctx);
  const items = w.recent_tasks.map((t): RequestRecord => ({
    kind: 'request', id: t.id, description: title(t.description), status: t.status,
    outcome: requestOutcome(t.status), toolUsed: t.tool_used, completedAt: t.completed_at,
    source: { contract: 'C3', endpoint: m.endpoint, id: t.id, scope: 'orchestrator recent tasks' },
  }));
  return listSection(m, items, BOUNDS.requests, 'The orchestrator reports no recent tasks.');
}

// ── C5: goals and decisions ────────────────────────────────────────────────

export function normalizeActiveGoals(w: ActiveGoalsWire, ctx: NormalizeContext): Section<GoalRecord> {
  const m = meta('C5', '/api/goals/active', 'goals', ctx);
  const items = (w.goals ?? []).map((g): GoalRecord => ({
    kind: 'goal', id: g.id, title: title(g.title), projectId: g.project_id, state: g.state,
    phase: goalPhase(g.state), updatedAt: g.updated_at, assignedTo: g.assigned_to ?? null, reviewDecisions: null,
    link: { to: 'goal', projectId: g.project_id, goalId: g.id },
    source: { contract: 'C5', endpoint: m.endpoint, id: g.id, scope: 'all active goals (ready or executing)' },
  }));
  return listSection(m, items, BOUNDS.goals, 'No goal is ready or executing. Goals awaiting review are in the Decision Inbox.');
}

export function normalizeDecisions(w: DecisionsWire, ctx: NormalizeContext): Section<DecisionRecord> {
  const m = meta('C5', '/api/decisions', 'decisions', ctx);
  const open = (w.items ?? []).filter((d) => d.status === 'open');
  const items = open.map((d): DecisionRecord => ({
    kind: 'decision', id: d.id, headline: title(d.headline), decisionKind: d.kind, goalId: d.goal_id,
    goalTitle: d.goal_title ?? null, projectId: d.project_id, tier: d.tier, createdAt: d.created_at,
    link: { to: 'decision_inbox', decisionId: d.id },
    source: { contract: 'C5', endpoint: m.endpoint, id: d.id, scope: 'open decisions (ranked by the daemon)' },
  }));
  const moreExists = (w.summary?.total_pending ?? open.length) > items.length;
  return listSection(m, items, BOUNDS.decisions, 'Nothing is waiting for your review.', moreExists);
}

// ── C4: memories and notes ─────────────────────────────────────────────────

function keyOrigin(key: string | null): Provenance | null {
  if (!key) return null;
  const colon = key.indexOf(':');
  return colon > 0 ? { kind: 'key_origin', origin: key.slice(0, colon) } : null;
}

export function normalizeBrainMemories(w: BrainMemoriesWire, ctx: NormalizeContext): Section<MemoryRecord> {
  const m = meta('C4', '/api/brain/memories', 'memories', ctx);
  const items = (w.memories ?? []).map((mem): MemoryRecord => {
    const clipped = clipText(mem.text, BOUNDS.memoryTextChars);
    const provenance: Provenance = mem.source ? { kind: 'source_tag', tag: mem.source } : keyOrigin(mem.key) ?? { kind: 'none' };
    return {
      kind: 'memory', id: mem.id, key: mem.key, text: clipped.text, textClipped: clipped.clipped,
      description: mem.description, timestamp: mem.timestamp, provenance, projectId: null,
      link: { to: 'brain', memoryId: mem.id, memoryKey: mem.key, preview: { text: clipped.text, timestamp: mem.timestamp } },
      source: { contract: 'C4', endpoint: m.endpoint, id: mem.id, scope: 'all Brain memories, newest first' },
    };
  });
  return listSection(m, items, BOUNDS.memories, 'Nothing has been remembered yet. Memories appear after conversations, notes or documents are saved.', !!w.has_more);
}

export function normalizeProjectMemories(projectId: string, w: ProjectMemory[], ctx: NormalizeContext): Section<MemoryRecord> {
  const endpoint = `/api/projects/${encodeURIComponent(projectId)}/memories`;
  const m = meta('C4', endpoint, 'memories', ctx);
  const items = (w ?? []).map((mem): MemoryRecord => {
    const clipped = clipText(mem.content, BOUNDS.memoryTextChars);
    return {
      kind: 'memory', id: mem.id, key: mem.key, text: clipped.text, textClipped: clipped.clipped,
      description: mem.description, timestamp: mem.associated_at || mem.created_at,
      provenance: keyOrigin(mem.key) ?? { kind: 'none' }, projectId,
      link: { to: 'brain', memoryId: mem.id, memoryKey: mem.key, preview: { text: clipped.text, timestamp: mem.created_at } },
      source: { contract: 'C4', endpoint, id: mem.id, scope: `memories associated with project ${projectId}` },
    };
  });
  return listSection(m, items, BOUNDS.memories, 'No memory is associated with this project yet.');
}

export function normalizeNotes(projectId: string, w: ProjectNote[], ctx: NormalizeContext, emptyCopy = 'No notes on this project yet.'): Section<NoteRecord> {
  const endpoint = `/api/projects/${encodeURIComponent(projectId)}/notes`;
  const m = meta('C4', endpoint, 'notes', ctx);
  const items = (w ?? []).map((n): NoteRecord => {
    const clipped = clipText(n.body, BOUNDS.noteBodyChars);
    return {
      kind: 'note', id: n.id, projectId, title: n.title ? title(n.title) : null, body: clipped.text, bodyClipped: clipped.clipped,
      memoryKey: n.memory_key, createdAt: n.created_at,
      link: { to: 'project_note', projectId, noteId: n.id },
      source: { contract: 'C4', endpoint, id: n.id, scope: `notes on project ${projectId}` },
    };
  });
  return listSection(m, items, BOUNDS.notes, emptyCopy);
}

// ── C1 / C2: agents ────────────────────────────────────────────────────────

function sectionFromList<W, T>(
  m: SectionMeta, list: ListSection<W> | { status: 'unavailable'; reason: string } | { status: 'ok'; items: W[]; truncated: boolean },
  map: (w: W) => T, bound: number, emptyCopy: string, previous?: Section<T>,
): Section<T> {
  if (list.status === 'unavailable') {
    return errorSection(m, { cause: 'section_unavailable', message: list.reason, status: null }, previous);
  }
  return listSection(m, list.items.map(map), bound, emptyCopy, list.truncated);
}

export function normalizeWorkActivity(agentId: string, w: WorkReview, ctx: NormalizeContext, previous?: Section<ActivityRecord>): Section<ActivityRecord> {
  const endpoint = `/api/agents/${encodeURIComponent(agentId)}/work`;
  const m = meta('C2', endpoint, 'work', ctx);
  return sectionFromList(m, w.activity, (j): ActivityRecord => ({
    kind: 'activity', id: j.id, ts: j.ts, actor: j.actor, activityKind: j.kind, title: title(j.title), detail: j.detail,
    link: j.ref_kind === 'goal' && j.ref_id && j.goal_project_id ? { to: 'goal', projectId: j.goal_project_id, goalId: j.ref_id } : null,
    source: { contract: 'C2', endpoint, id: j.id, scope: `activity attributed to ${w.activity.attribution}` },
  }), BOUNDS.activity, `Nothing attributed to ${agentId} in this window. That does not prove it was inactive.`, previous);
}

export function normalizeWorkGoals(agentId: string, w: WorkReview, ctx: NormalizeContext, previous?: Section<GoalRecord>): Section<GoalRecord> {
  const endpoint = `/api/agents/${encodeURIComponent(agentId)}/work`;
  const m = meta('C2', endpoint, 'work', ctx);
  return sectionFromList(m, w.goals, (g): GoalRecord => ({
    kind: 'goal', id: g.id, title: title(g.title), projectId: g.project_id, state: g.state, phase: goalPhase(g.state),
    updatedAt: g.updated_at, assignedTo: agentId,
    reviewDecisions: g.review_decisions.status === 'ok'
      ? { status: 'ok', count: g.review_decisions.items.length, truncated: g.review_decisions.truncated }
      : { status: 'unavailable', reason: g.review_decisions.reason },
    link: { to: 'goal', projectId: g.project_id, goalId: g.id },
    source: { contract: 'C2', endpoint, id: g.id, scope: `goals owned by worker ${agentId}` },
  }), BOUNDS.goals, `No goals are owned by ${agentId}.`, previous);
}

export function normalizeWorkSpend(agentId: string, w: WorkReview, ctx: NormalizeContext, previous?: Section<SpendRecord>): Section<SpendRecord> {
  const endpoint = `/api/agents/${encodeURIComponent(agentId)}/work`;
  const m = meta('C2', endpoint, 'work', ctx);
  const goalsTruncated = w.goals.status === 'ok' ? w.goals.truncated : true;
  const goalCount = w.goals.status === 'ok' ? w.goals.items.length : 0;
  return sectionFromList(m, w.spend, (s): SpendRecord => ({
    kind: 'spend', costUsd: s.cost_usd, callCount: s.call_count, estimatedCallCount: s.estimated_call_count,
    scope: goalsTruncated
      ? `Spend attributed through the ${goalCount} most recent goals owned by ${agentId} — at least this much; not a lifetime total.`
      : `Spend attributed through the ${goalCount} goal(s) owned by ${agentId}. Session cost is a separate scope; do not add them.`,
    lowerBound: goalsTruncated, note: s.note,
    source: { contract: 'C2', endpoint, id: `${agentId}:spend`, scope: `cost_ledger rows for goals owned by ${agentId}` },
  }), 1, `No spend attributed to ${agentId}.`, previous);
}

export function normalizeAgentState(worldAgentId: string, detail: AgentDetail, work: WorkReview | null, ctx: NormalizeContext): Section<AgentStateRecord> {
  const endpoint = `/api/agents/${encodeURIComponent(detail.kind === 'worker' ? detail.id : detail.key)}`;
  const m = meta('C1', endpoint, 'agent', ctx);
  const agentId = detail.kind === 'worker' ? detail.id : detail.key;
  const record: AgentStateRecord = {
    kind: 'agent_state', worldAgentId, agentId, displayName: detail.display_name,
    truth: truthFromAgent(detail, work), link: { to: 'agent_settings', agentId },
    source: { contract: 'C1', endpoint, id: agentId, scope: 'agent configuration and capability' },
  };
  return { state: 'ready', items: [record], truncated: false, bound: 1, ...m };
}

// ── C6: skills ─────────────────────────────────────────────────────────────

export function normalizeSkills(w: Skill[], ctx: NormalizeContext): Section<SkillRecord> {
  const m = meta('C6', '/permagent/skills', 'skills', ctx);
  const items = (w ?? []).map((s): SkillRecord => ({
    kind: 'skill', id: s.id, name: title(s.name), description: clipText(s.description ?? '', BOUNDS.titleChars).text,
    status: s.status ?? null, usageCount: typeof s.usageCount === 'number' ? s.usageCount : null,
    lastRun: s.last_run ?? null, toolUsed: s.toolUsed ?? null, link: { to: 'skills' },
    source: { contract: 'C6', endpoint: m.endpoint, id: s.id, scope: 'saved skills (saved is not the same as exercised)' },
  }));
  return listSection(m, items, BOUNDS.skills, 'No skills are saved yet. A proposal is not a saved skill.');
}

// ── C7: forecasts and Council ──────────────────────────────────────────────

export function normalizeMarket(projectId: string, w: MarketWire, ctx: NormalizeContext): Section<ForecastRecord> {
  const endpoint = `/api/projects/${encodeURIComponent(projectId)}/market`;
  const m = meta('C7', endpoint, 'market', ctx);
  if (w.noSeriesBound || (w.rows ?? []).length === 0) {
    // "No series bound is not a flat trend" (ruling C7).
    return { state: 'setup_required', reason: 'No market series is bound to this project yet. That is not a flat trend.', action: 'Open the project to approve a series.' };
  }
  const items = w.rows.filter((r) => r.status !== 'dismissed').map((r): ForecastRecord => {
    const hist = r.history ?? [];
    const kept = hist.slice(-BOUNDS.forecastHistoryPoints);
    return {
      kind: 'forecast', seriesId: r.seriesId, label: title(r.label), sourceLabel: r.sourceLabel, cadence: r.cadence, verdict: r.verdict,
      outcome: r.forecast
        ? { kind: 'forecast', method: r.forecast.method, methodLabel: r.forecast.methodLabel, horizon: r.forecast.point.length, maseVsBaseline: r.forecast.maseVsBaseline, folds: r.forecast.folds, foldWins: r.forecast.foldWins }
        : { kind: 'refusal', reason: r.refusal?.reason ?? r.verdict, detail: r.refusal?.detail ?? null, points: r.refusal?.points ?? r.points ?? null, needed: r.refusal?.needed ?? null, lastCollectedAt: r.refusal?.lastCollectedAt ?? null },
      history: kept, historyClipped: kept.length < hist.length, points: r.points, lastError: r.lastError, projectId,
      link: { to: 'project', projectId },
      source: { contract: 'C7', endpoint, id: r.seriesId, scope: `market series for project ${projectId}, generated ${w.generatedAt}` },
    };
  });
  return listSection(m, items, BOUNDS.forecasts, 'Every series on this project is dismissed.');
}

export function normalizeCouncil(w: CouncilReportsResponse, ctx: NormalizeContext): Section<CouncilRecord> {
  const m = meta('C7', '/api/council/reports', 'council', ctx);
  const items = (w.sessions ?? []).map(({ session, report }): CouncilRecord => {
    const consensus = report?.consensus ?? [];
    return {
      kind: 'council_report', id: report?.id ?? session.id, sessionId: session.id, status: session.status,
      generatedAt: report?.generated_at ?? session.finished_at,
      headline: clipText(report?.headline ?? (session.status === 'failed' ? 'This Council session failed before a report was written.' : 'No report was written for this session.'), BOUNDS.headlineChars).text,
      consensus: consensus.slice(0, BOUNDS.councilConsensus).map((c) => clipText(String(c), BOUNDS.titleChars).text),
      consensusClipped: consensus.length > BOUNDS.councilConsensus,
      dissentCount: Array.isArray(report?.dissent) ? report!.dissent.length : 0,
      chair: report?.chair_model ?? session.chair_model ?? null,
      link: { to: 'council', sessionId: session.id },
      source: { contract: 'C7', endpoint: m.endpoint, id: session.id, scope: 'finished Council sessions, newest first' },
    };
  });
  return listSection(m, items, BOUNDS.councilReports, 'No Council session has finished yet.');
}

// ── C8: growth ─────────────────────────────────────────────────────────────

function growthRecord(r: GrowthResultRow, endpoint: string): GrowthRecord {
  return {
    kind: 'growth_result', actionId: r.actionId, projectId: r.projectId, projectName: r.projectName, title: title(r.title),
    category: r.category, status: r.status, verdict: growthVerdict(r.verdict), deltaPct: r.deltaPct, windowDays: r.windowDays,
    judgedAt: r.judgedAt, targetMetric: r.targetMetric, link: { to: 'grow', projectId: r.projectId },
    source: { contract: 'C8', endpoint, id: r.actionId, scope: 'growth_action_outcomes (measured, never model-authored)' },
  };
}

export function normalizeGrowth(projectId: string | null, w: GrowthResultsData, ctx: NormalizeContext): Section<GrowthRecord> {
  const endpoint = projectId ? `/api/growth-results?projectId=${encodeURIComponent(projectId)}` : '/api/growth-results';
  const m = meta('C8', endpoint, 'growth', ctx);
  const rows = projectId && w.project ? w.project.actions : w.fleet?.recent ?? [];
  if (projectId && !w.project) {
    return { state: 'setup_required', reason: 'This project is not set up for growth measurement. A lack of analytics is a setup state, not zero growth.', action: 'Open Grow to connect analytics.' };
  }
  // The daemon keeps only the 12 most recent fleet rows (growth_actions.rs:2865).
  const moreExists = !projectId && (w.fleet?.helped ?? 0) + (w.fleet?.hindered ?? 0) + (w.fleet?.noEffect ?? 0) + (w.fleet?.inconclusive ?? 0) > rows.length;
  return listSection(m, rows.map((r) => growthRecord(r, endpoint)), BOUNDS.growthResults, 'No intervention has been measured yet. Implemented is not the same as helped.', moreExists);
}
