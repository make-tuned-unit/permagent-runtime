/**
 * TEST DATA — World W1 contract fixtures.
 *
 * Every value in this file is synthetic test data for the ten states in Astra's
 * 2026-10-12 ruling §5 "Candidate and fixtures". None of it describes a real
 * user, project or run, and nothing here may be used to populate a live
 * profile. Ids are prefixed `fixture-` so a leak would be recognisable.
 */
import type { AgentDetail, GoalItem, WorkReview, JournalItem } from '../../../../../lib/agentsApi';
import type { CouncilReportsResponse, Skill } from '../../../../../lib/api';
import type { ProjectMemory, ProjectNote } from '../../../../projects/types';
import type { GrowthResultsData, GrowthResultRow } from '../../../../grow/growthResults';
import type {
  ActiveGoalsWire, BrainMemoriesWire, DecisionsWire, HenryStatusWire, MarketWire, WorldEvidenceClient,
} from '../evidenceClient';

export const FIXTURE_MARKER = 'TEST DATA — synthetic World fixture, not a user profile' as const;

/** Pinned clock for every fixture: 2026-10-12 12:00:00 UTC. */
export const FIXTURE_NOW = Date.parse('2026-10-12T12:00:00.000Z');
const T = (minutesAgo: number) => new Date(FIXTURE_NOW - minutesAgo * 60_000).toISOString();

export const FIXTURE_PROJECT = 'fixture-project-garden';

export interface FixtureError { error: { status: number | null; message: string } }
type R<T> = T | FixtureError;

export interface FixtureData {
  henryStatus?: R<HenryStatusWire>;
  activeGoals?: R<ActiveGoalsWire>;
  decisions?: R<DecisionsWire>;
  brainMemories?: R<BrainMemoriesWire>;
  projectMemories?: Record<string, R<ProjectMemory[]>>;
  projectNotes?: Record<string, R<ProjectNote[]>>;
  agentDetail?: Record<string, R<AgentDetail>>;
  agentWork?: Record<string, R<WorkReview>>;
  skills?: R<Skill[]>;
  projectMarket?: Record<string, R<MarketWire>>;
  councilReports?: R<CouncilReportsResponse>;
  growthResults?: R<GrowthResultsData>;
}

export interface WorldFixture {
  id: string;
  /** Astra §5 item number. */
  item: number;
  title: string;
  description: string;
  marker: typeof FIXTURE_MARKER;
  projectId: string | null;
  /** One data set, or a sequence of phases (state 10). */
  phases: FixtureData[];
  /** Which phase the fixture panel renders (after loading every earlier one). */
  displayPhase: number;
}

// ── builders ───────────────────────────────────────────────────────────────

function henry(over: Partial<HenryStatusWire> = {}): HenryStatusWire {
  return {
    identity: { name: 'Orchestrator', traits: [], tone: 'calm' },
    current_state: 'idle', active_sessions: [], current_tool: null, tasks_in_flight: 0, recent_tasks: [],
    ...over,
  };
}

const WORKERS = ['librarian', 'git_steward', 'forecaster', 'growth_measurement'] as const;

function worker(id: string, enabled: boolean | null, live: 'ok' | 'not_queryable' | 'unavailable' = 'ok'): AgentDetail {
  return {
    kind: 'worker', id, display_name: id === 'git_steward' ? 'Git Steward' : id === 'growth_measurement' ? 'Grower' : id[0].toUpperCase() + id.slice(1),
    summary: `Fixture ${id}.`, what_it_does: `Fixture ${id}.`, why_it_matters: 'Fixture.',
    state_source: live === 'not_queryable' ? 'static' : 'queryable',
    live_state: live === 'ok' ? { status: 'ok', value: 'idle' } : live === 'not_queryable' ? { status: 'not_queryable' } : { status: 'unavailable', reason: 'fixture: probe timed out' },
    dispatchable: false,
    gate: enabled === null ? null : { config_key: `${id}_enabled`, enabled },
  };
}

function journal(id: string, actor: string, title: string, minutesAgo: number, ref?: { goal: string; project: string }): JournalItem {
  return {
    id, ts: T(minutesAgo), kind: ref ? 'goal_event' : 'note', actor, title, detail: null,
    ref_kind: ref ? 'goal' : null, ref_id: ref?.goal ?? null, goal_project_id: ref?.project ?? null,
  };
}

function work(agentId: string, parts: { activity?: JournalItem[]; goals?: GoalItem[]; spend?: number; calls?: number; truncatedGoals?: boolean } = {}): WorkReview {
  const goals = parts.goals ?? [];
  return {
    activity: { attribution: agentId, status: 'ok', items: parts.activity ?? [], truncated: false },
    goals: { status: 'ok', items: goals, truncated: parts.truncatedGoals ?? false },
    spend: {
      status: 'ok', truncated: false,
      items: [{ cost_usd: parts.spend ?? 0, call_count: parts.calls ?? 0, estimated_call_count: parts.calls ? 1 : 0, attribution: 'via_goal_id', note: goals.length ? null : 'this agent has no attributable goals' }],
    },
    scheduled_jobs: { status: 'ok', items: [], truncated: false },
  };
}

const noDecisions: DecisionsWire = { items: [], summary: { total_pending: 0 } };
const noCouncil: CouncilReportsResponse = { sessions: [] };
function growth(project: GrowthResultsData['project'] = null, recent: GrowthResultRow[] = []): GrowthResultsData {
  return {
    project,
    fleet: { projects: project ? 1 : 0, helped: recent.filter((r) => r.verdict === 'helped').length, hindered: 0, noEffect: 0, inconclusive: recent.filter((r) => r.verdict === 'inconclusive').length, categories: [], recent, trend: [], byProject: [] },
  };
}

function baseline(over: FixtureData = {}): FixtureData {
  const agentWork: Record<string, R<WorkReview>> = {};
  const agentDetail: Record<string, R<AgentDetail>> = {};
  for (const id of WORKERS) { agentWork[id] = work(id); agentDetail[id] = worker(id, true); }
  return {
    henryStatus: henry(),
    activeGoals: { count: 0, goals: [] },
    decisions: noDecisions,
    brainMemories: { memories: [], total: 0, has_more: false },
    projectMemories: { [FIXTURE_PROJECT]: [] },
    projectNotes: { [FIXTURE_PROJECT]: [] },
    agentDetail,
    agentWork,
    skills: [],
    projectMarket: { [FIXTURE_PROJECT]: { rows: [], noSeriesBound: true, generatedAt: T(0) } },
    councilReports: noCouncil,
    growthResults: growth(),
    ...over,
    ...(over.agentWork ? { agentWork: { ...agentWork, ...over.agentWork } } : {}),
    ...(over.agentDetail ? { agentDetail: { ...agentDetail, ...over.agentDetail } } : {}),
  };
}

const goal = (id: string, title: string, state: string, minutesAgo: number, reviews = 0) => ({
  id, title, project_id: FIXTURE_PROJECT, state, updated_at: T(minutesAgo),
  review_decisions: { status: 'ok' as const, items: Array.from({ length: reviews }, () => ({ answer: null, acted_by: null })), truncated: false },
});

const activeState: FixtureData = baseline({
  henryStatus: henry({
    current_state: 'working', current_tool: 'developer__shell', tasks_in_flight: 1,
    active_sessions: [{ id: 'fixture-session-1', name: 'Plan the garden launch page', started_at: T(12) }],
    recent_tasks: [{ id: 'fixture-task-1', description: 'Draft the launch page outline', status: 'running', tool_used: 'developer__shell', completed_at: null }],
  }),
  activeGoals: { count: 1, goals: [{ id: 'fixture-goal-1', title: 'Draft the launch page outline', project_id: FIXTURE_PROJECT, state: 'in_progress', assigned_to: 'git_steward', updated_at: T(4) }] },
  agentWork: {
    git_steward: work('git_steward', {
      activity: [journal('fixture-act-1', 'git_steward', 'Started: Draft the launch page outline', 4, { goal: 'fixture-goal-1', project: FIXTURE_PROJECT })],
      goals: [goal('fixture-goal-1', 'Draft the launch page outline', 'in_progress', 4)],
      spend: 0.08, calls: 3,
    }),
  },
});

const disconnected: FixtureError = { error: { status: null, message: 'Failed to fetch' } };
function allFail(): FixtureData {
  const d: FixtureData = {};
  for (const k of ['henryStatus', 'activeGoals', 'decisions', 'brainMemories', 'skills', 'councilReports', 'growthResults'] as const) (d as Record<string, unknown>)[k] = disconnected;
  d.projectMemories = { [FIXTURE_PROJECT]: disconnected };
  d.projectNotes = { [FIXTURE_PROJECT]: disconnected };
  d.projectMarket = { [FIXTURE_PROJECT]: disconnected };
  d.agentDetail = Object.fromEntries(WORKERS.map((id) => [id, disconnected]));
  d.agentWork = Object.fromEntries(WORKERS.map((id) => [id, disconnected]));
  return d;
}

export const WORLD_FIXTURES: readonly WorldFixture[] = [
  {
    id: 'fresh_account', item: 1, title: 'Fresh account', marker: FIXTURE_MARKER, projectId: null, displayPhase: 0,
    description: 'No model, memories or runs. Workers present but switched off; nothing attributed.',
    phases: [baseline({
      agentDetail: Object.fromEntries(WORKERS.map((id) => [id, worker(id, false, 'not_queryable')])),
    })],
  },
  {
    id: 'agents_no_work', item: 2, title: 'Configured agents, no attributed work', marker: FIXTURE_MARKER, projectId: null, displayPhase: 0,
    description: 'Agents are on and reachable; nothing is attributed in the window (which is not proof of inactivity).',
    phases: [baseline()],
  },
  {
    id: 'active_request', item: 3, title: 'One active request and tracked goal', marker: FIXTURE_MARKER, projectId: null, displayPhase: 0,
    description: 'The orchestrator is working in a real session; one goal is executing and attributed to the Steward.',
    phases: [activeState],
  },
  {
    id: 'decision_review', item: 4, title: 'A decision requiring review', marker: FIXTURE_MARKER, projectId: null, displayPhase: 0,
    description: 'A goal reached review and a tier-2 decision is open. The World can open the inbox; it never approves.',
    phases: [baseline({
      decisions: {
        items: [{ id: 'fixture-decision-1', kind: 'goal_review', goal_id: 'fixture-goal-2', project_id: FIXTURE_PROJECT, tier: 2, headline: 'Approve the launch page outline?', status: 'open', created_at: T(20), goal_title: 'Draft the launch page outline' }],
        summary: { total_pending: 1 },
      },
      agentWork: { git_steward: work('git_steward', { goals: [goal('fixture-goal-2', 'Draft the launch page outline', 'review', 20, 1)], spend: 0.21, calls: 5 }) },
    })],
  },
  {
    id: 'completed_goal', item: 5, title: 'Completed goal with evidence and cost', marker: FIXTURE_MARKER, projectId: null, displayPhase: 0,
    description: 'A goal completed with a recorded result; spend is attributed through the Steward’s goals.',
    phases: [baseline({
      agentWork: {
        git_steward: work('git_steward', {
          activity: [journal('fixture-act-5', 'git_steward', 'Completed: Fix the broken footer link', 30, { goal: 'fixture-goal-5', project: FIXTURE_PROJECT })],
          goals: [goal('fixture-goal-5', 'Fix the broken footer link', 'complete', 30, 1)],
          spend: 0.42, calls: 7,
        }),
      },
      skills: [{ id: 'fixture-skill-1', name: 'Check links', description: 'Fixture skill: verify every link on a page.', usageCount: 2, last_run: T(30), status: 'active', toolUsed: 'developer__shell' }],
    })],
  },
  {
    id: 'failed_cancelled', item: 6, title: 'A failed and a cancelled request', marker: FIXTURE_MARKER, projectId: null, displayPhase: 0,
    description: 'An agent task failed and another was cancelled. A failed task is not a lost connection.',
    phases: [baseline({
      henryStatus: henry({
        recent_tasks: [
          { id: 'fixture-task-6a', description: 'Publish the launch page', status: 'failed', tool_used: 'developer__shell', completed_at: T(45) },
          { id: 'fixture-task-6b', description: 'Rename the project folder', status: 'cancelled', tool_used: null, completed_at: T(50) },
        ],
      }),
      agentWork: { git_steward: work('git_steward', { goals: [goal('fixture-goal-6a', 'Publish the launch page', 'failed', 45), goal('fixture-goal-6b', 'Rename the project folder', 'cancelled', 50)], spend: 0.05, calls: 2 }) },
    })],
  },
  {
    id: 'memory_provenance', item: 7, title: 'A memory with provenance and one without', marker: FIXTURE_MARKER, projectId: null, displayPhase: 0,
    description: 'One memory carries a source tag; the other has none, and the World must say so.',
    phases: [baseline({
      brainMemories: {
        memories: [
          { id: 'fixture-mem-1', key: 'note:fixture-1', text: 'The launch page must load in under two seconds on a phone.', description: 'Launch performance requirement', timestamp: T(90), source: 'fixture-meeting-notes.md' },
          { id: 'fixture-mem-2', key: null, text: 'Prefers short headlines.', description: null, timestamp: T(200), source: null },
        ],
        total: 2, has_more: false,
      },
    })],
  },
  {
    id: 'forecasts', item: 8, title: 'A forecast refusal and a usable forecast', marker: FIXTURE_MARKER, projectId: FIXTURE_PROJECT, displayPhase: 0,
    description: 'One series is forecast with its method; another is refused for insufficient history. A Council report keeps its dissent.',
    phases: [baseline({
      projectMarket: {
        [FIXTURE_PROJECT]: {
          noSeriesBound: false, generatedAt: T(5),
          rows: [
            { seriesId: 'fixture-series-1', sourceLabel: 'Fixture source', label: 'Weekly visitors', cadence: 'weekly', status: 'active', points: 40, lastError: null, verdict: 'forecastable', history: Array.from({ length: 40 }, (_, i) => 100 + i * 2), forecast: { point: [182, 184, 186, 188], method: 'ets', methodLabel: 'Exponential smoothing', maseVsBaseline: 0.82, folds: 5, foldWins: 4 }, refusal: null },
            { seriesId: 'fixture-series-2', sourceLabel: 'Fixture source', label: 'Newsletter sign-ups', cadence: 'daily', status: 'active', points: 9, lastError: null, verdict: 'insufficient_history', history: [3, 4, 2, 5, 4, 6, 5, 4, 7], forecast: null, refusal: { reason: 'insufficient_history', points: 9, needed: 28 } },
          ],
        },
      },
      councilReports: {
        sessions: [{
          session: { id: 'fixture-council-1', started_at: T(300), finished_at: T(280), trigger: 'manual', extra_question: null, chair_provider: 'fixture', chair_model: 'fixture-chair', brief_json: '{}', status: 'finished' } as CouncilReportsResponse['sessions'][number]['session'],
          report: { id: 'fixture-report-1', session_id: 'fixture-council-1', generated_at: T(280), headline: 'Launch next week, but measure sign-ups first.', markdown: '', consensus: ['The page is ready.'], dissent: [{ member: 'fixture-b', position: 'Wait for more sign-up history.' }], actions: [], chair_provider: 'fixture', chair_model: 'fixture-chair' },
        }],
      },
    })],
  },
  {
    id: 'growth', item: 9, title: 'A measured growth result and an inconclusive one', marker: FIXTURE_MARKER, projectId: FIXTURE_PROJECT, displayPhase: 0,
    description: 'One intervention helped over a measured window; another was inconclusive.',
    phases: [baseline({
      growthResults: growth({
        projectId: FIXTURE_PROJECT, name: 'Fixture garden project', segmentLabel: 'fixture', implemented: 2, measuring: 0, judged: 2, helped: 1, hindered: 0, noEffect: 0, inconclusive: 1,
        actions: [
          { actionId: 'fixture-action-1', projectId: FIXTURE_PROJECT, projectName: 'Fixture garden project', title: 'Shorter signup form', category: 'conversion', status: 'judged', verdict: 'helped', deltaPct: 0.12, windowDays: 14, judgedAt: T(600), targetMetric: 'signups', targetDir: 'up' },
          { actionId: 'fixture-action-2', projectId: FIXTURE_PROJECT, projectName: 'Fixture garden project', title: 'New hero image', category: 'engagement', status: 'judged', verdict: 'inconclusive', deltaPct: null, windowDays: 14, judgedAt: T(620), targetMetric: 'time_on_page', targetDir: 'up' },
        ],
      }),
    })],
  },
  {
    id: 'disconnect_replay', item: 10, title: 'Disconnection, stale records, replay and reconnect', marker: FIXTURE_MARKER, projectId: null, displayPhase: 1,
    description: 'Phase 0 loads live data; phase 1 loses the daemon (last good data shown stale); phase 2 reconnects.',
    phases: [activeState, allFail(), activeState],
  },
];

/** Thrown by the fixture client to mimic apiFetch errors (status attached). */
function fail(e: FixtureError['error']): never {
  if (e.status === null) throw new TypeError(e.message);
  const err = new Error(e.message) as Error & { status: number };
  err.status = e.status;
  throw err;
}

function answer<T>(value: R<T> | undefined, what: string): T {
  if (value === undefined) fail({ status: 404, message: `fixture: no data for ${what}` });
  if (value && typeof value === 'object' && 'error' in (value as object) && !Array.isArray(value)) fail((value as FixtureError).error);
  // Hand out a copy so a consumer mutating it cannot change the fixture.
  return JSON.parse(JSON.stringify(value)) as T;
}

/**
 * A WorldEvidenceClient over fixture data. `calls` records every request so
 * tests can count them. `getData` is read per call (phase switching).
 */
export function createFixtureClient(getData: () => FixtureData, calls: string[] = []): WorldEvidenceClient & { calls: string[] } {
  const go = async <T,>(name: string, value: () => R<T> | undefined): Promise<T> => { calls.push(name); await Promise.resolve(); return answer(value(), name); };
  return {
    calls,
    henryStatus: () => go('henryStatus', () => getData().henryStatus),
    activeGoals: () => go('activeGoals', () => getData().activeGoals),
    decisions: () => go('decisions', () => getData().decisions),
    brainMemories: () => go('brainMemories', () => getData().brainMemories),
    projectMemories: (id) => go(`projectMemories:${id}`, () => getData().projectMemories?.[id]),
    projectNotes: (id) => go(`projectNotes:${id}`, () => getData().projectNotes?.[id]),
    agentDetail: (id) => go(`agentDetail:${id}`, () => getData().agentDetail?.[id]),
    agentWork: (id) => go(`agentWork:${id}`, () => getData().agentWork?.[id]),
    skills: () => go('skills', () => getData().skills),
    projectMarket: (id) => go(`projectMarket:${id}`, () => getData().projectMarket?.[id]),
    councilReports: () => go('councilReports', () => getData().councilReports),
    growthResults: () => go('growthResults', () => getData().growthResults),
  };
}
