/**
 * World W1 — the single bounded evidence adapter for place visits.
 *
 * One loader instance serves the whole World (Astra: "Implement one bounded
 * World evidence adapter rather than separate polling loops for every prop").
 *
 * Request discipline:
 * - A place visit runs a fixed PLAN of at most MAX_REQUESTS_PER_PLACE_VISIT
 *   (3) daemon reads. The per-place plans are listed in PLACE_REQUEST_PLAN
 *   and proven by snapshotLoader.test.ts.
 * - Requests are de-duplicated by endpoint key: a request already in flight is
 *   shared (two concurrent visits → one request), and a fresh result is reused
 *   until its staleness threshold (bounds.ts STALE_AFTER_MS) passes or a World
 *   event invalidates it. A failure is reused for FAILURE_RETRY_MS.
 * - Latest visit wins: every visit gets a token; a superseded visit resolves
 *   with `current: false` and the renderer ignores it. `cancel()` aborts all
 *   in-flight requests (World hidden / left).
 * - No polling here. Invalidation comes from the shared World event stream
 *   (`invalidateForWorldEvent`); a replayed event only marks data stale for the
 *   next visit — it never replays a completion or proves continuing work.
 *
 * The loader holds a READ-ONLY client (evidenceClient.ts). It cannot send a
 * message, answer a decision or write a note.
 */
import { BOUNDS, FAILURE_RETRY_MS, MAX_REQUESTS_PER_PLACE_VISIT, STALE_AFTER_MS, type StaleFamily } from './bounds';
import { classifyFailure, isAbort, type WorldEvidenceClient } from './evidenceClient';
import { PLACE_HOST, resolveWorldAgent, workAgentIdFor, type WorldAgentIdentity, type WorldPlaceId } from './identity';
import * as N from './normalize';
import type {
  ActivityRecord, AgentStateRecord, ContractId, GoalRecord, OrchestratorRecord, PlaceSections, Section, SectionFailure, SpendRecord,
} from './types';

export interface PlaceVisitContext {
  /** The project the user chose for this place, or null (never guessed). */
  projectId: string | null;
}

export interface PlaceSnapshot {
  placeId: WorldPlaceId;
  context: PlaceVisitContext;
  sections: PlaceSections;
  /** Network requests this visit started (shared / cached requests count 0). */
  requestCount: number;
  /** False when a newer visit superseded this one. Render only current snapshots. */
  current: boolean;
}

type Outcome<W> =
  | { ok: true; value: W; fetchedAt: string; at: number }
  | { ok: false; failure: SectionFailure; fetchedAt: string; at: number; aborted?: boolean };

interface Entry {
  family: StaleFamily;
  inflight: Promise<Outcome<unknown>> | null;
  controller: AbortController | null;
  outcome: Outcome<unknown> | null;
  lastOk: { value: unknown; fetchedAt: string } | null;
  invalidated: boolean;
}

interface Spec<W> {
  key: string;
  family: StaleFamily;
  run: (client: WorldEvidenceClient, signal: AbortSignal) => Promise<W>;
}

/** What a place plan resolves for each spec. */
interface Resolved<W> {
  outcome: Outcome<W>;
  lastOk: { value: W; fetchedAt: string } | null;
}

/** Documented request plan per place (the endpoints one visit may read). */
export const PLACE_REQUEST_PLAN: Readonly<Record<WorldPlaceId, readonly string[]>> = {
  commons: ['/api/henry/status', '/api/goals/active', '/api/decisions'],
  gallery: ['/api/brain/memories?limit=8 | /api/projects/{id}/memories', '/api/agents/librarian/work?limit=10', '/api/projects/{id}/notes (project chosen only)'],
  maker: ['/api/agents/git_steward/work?limit=10', '/api/decisions', '/permagent/skills'],
  council_garden: ['/api/agents/forecaster/work?limit=10', '/api/projects/{id}/market (project chosen only)', '/api/council/reports?limit=3'],
  observatory_garden: ['/api/projects/{id}/notes (project chosen only)'],
  conservatory: ['/api/growth-results[?projectId=]', '/api/agents/growth_measurement', '/api/agents/growth_measurement/work?limit=10'],
};

const S = {
  henry: (): Spec<Awaited<ReturnType<WorldEvidenceClient['henryStatus']>>> => ({ key: '/api/henry/status', family: 'orchestrator', run: (c, s) => c.henryStatus(s) }),
  goals: (): Spec<Awaited<ReturnType<WorldEvidenceClient['activeGoals']>>> => ({ key: '/api/goals/active', family: 'goals', run: (c, s) => c.activeGoals(s) }),
  decisions: (): Spec<Awaited<ReturnType<WorldEvidenceClient['decisions']>>> => ({ key: '/api/decisions', family: 'decisions', run: (c, s) => c.decisions(s) }),
  brain: (): Spec<Awaited<ReturnType<WorldEvidenceClient['brainMemories']>>> => ({ key: `/api/brain/memories?limit=${BOUNDS.memories}`, family: 'memories', run: (c, s) => c.brainMemories(BOUNDS.memories, s) }),
  projectMemories: (id: string): Spec<Awaited<ReturnType<WorldEvidenceClient['projectMemories']>>> => ({ key: `/api/projects/${encodeURIComponent(id)}/memories`, family: 'memories', run: (c, s) => c.projectMemories(id, s) }),
  notes: (id: string): Spec<Awaited<ReturnType<WorldEvidenceClient['projectNotes']>>> => ({ key: `/api/projects/${encodeURIComponent(id)}/notes`, family: 'notes', run: (c, s) => c.projectNotes(id, s) }),
  detail: (agentId: string): Spec<Awaited<ReturnType<WorldEvidenceClient['agentDetail']>>> => ({ key: `/api/agents/${encodeURIComponent(agentId)}`, family: 'agent', run: (c, s) => c.agentDetail(agentId, s) }),
  work: (agentId: string): Spec<Awaited<ReturnType<WorldEvidenceClient['agentWork']>>> => ({ key: `/api/agents/${encodeURIComponent(agentId)}/work?limit=${BOUNDS.workLimit}`, family: 'work', run: (c, s) => c.agentWork(agentId, BOUNDS.workLimit, s) }),
  skills: (): Spec<Awaited<ReturnType<WorldEvidenceClient['skills']>>> => ({ key: '/permagent/skills', family: 'skills', run: (c, s) => c.skills(s) }),
  market: (id: string): Spec<Awaited<ReturnType<WorldEvidenceClient['projectMarket']>>> => ({ key: `/api/projects/${encodeURIComponent(id)}/market`, family: 'market', run: (c, s) => c.projectMarket(id, s) }),
  council: (): Spec<Awaited<ReturnType<WorldEvidenceClient['councilReports']>>> => ({ key: `/api/council/reports?limit=${BOUNDS.councilReports}`, family: 'council', run: (c, s) => c.councilReports(BOUNDS.councilReports, s) }),
  growth: (id: string | null): Spec<Awaited<ReturnType<WorldEvidenceClient['growthResults']>>> => ({ key: id ? `/api/growth-results?projectId=${encodeURIComponent(id)}` : '/api/growth-results', family: 'growth', run: (c, s) => c.growthResults(id, s) }),
};

/**
 * Turn a resolved request into a section: the normalizer on success; on
 * failure an error section that keeps the last good data (stale).
 */
function toSection<W, T>(
  r: Resolved<W>, contract: ContractId, endpoint: string, family: StaleFamily,
  normalize: (w: W, ctx: N.NormalizeContext) => Section<T>,
): Section<T> {
  if (r.outcome.ok) return normalize(r.outcome.value, { fetchedAt: r.outcome.fetchedAt });
  const previous = r.lastOk ? normalize(r.lastOk.value, { fetchedAt: r.lastOk.fetchedAt }) : undefined;
  return N.errorSection<T>({ contract, endpoint, fetchedAt: r.outcome.fetchedAt, staleAfterMs: STALE_AFTER_MS[family] }, r.outcome.failure, previous);
}

const unsupported = (identity: WorldAgentIdentity): Section<never> => ({
  state: 'unsupported',
  reason: identity.status === 'unsupported' ? identity.reason : `${identity.worldId} has no attributed-work source.`,
});

const needsProject = (what: string): Section<never> => ({
  state: 'setup_required',
  reason: `Choose a project to see its ${what}.`,
  action: 'Choose a project',
});

export interface LoaderOptions {
  /** Injectable clock (ms since epoch). Tests and fixtures pin it. */
  now?: () => number;
}

export interface AgentEvidence {
  identity: WorldAgentIdentity;
  state: Section<AgentStateRecord> | Section<OrchestratorRecord>;
  activity: Section<ActivityRecord>;
  goals: Section<GoalRecord>;
  spend: Section<SpendRecord>;
  requestCount: number;
}

export interface WorldSnapshotLoader {
  loadPlace(placeId: WorldPlaceId, context: PlaceVisitContext): Promise<PlaceSnapshot>;
  /** The shared agent profile: identity, state truth and attributed work. ≤2 requests. */
  loadAgent(worldAgentId: string): Promise<AgentEvidence>;
  /** Mark cached data stale so the next visit refetches (keeps last good data). */
  invalidate(keyPrefixes: readonly string[]): void;
  /** Apply the invalidation a World event implies. Returns the prefixes hit. */
  invalidateForWorldEvent(event: { type: string; replayed?: boolean }): readonly string[];
  /** Abort in-flight requests and supersede pending visits. */
  cancel(): void;
  /** Total network requests started by this loader (for measurement). */
  readonly totalRequests: number;
}

/** Which cached endpoints a World event makes stale. */
export function invalidationsForEvent(type: string): readonly string[] {
  switch (type) {
    case 'goal_state_changed': case 'task_completed': case 'task_failed':
      return ['/api/goals/active', '/api/agents/', '/api/decisions', '/api/henry/status'];
    case 'decision_created': case 'decision_resolved':
      return ['/api/decisions'];
    case 'memory_added': case 'librarian_describe_completed':
      return ['/api/brain/memories', '/api/projects/'];
    case 'agent_state_changed':
      return ['/api/henry/status', '/api/agents/'];
    case 'schedule_changed':
      return ['/api/agents/'];
    default:
      return [];
  }
}

export function createWorldSnapshotLoader(client: WorldEvidenceClient, options: LoaderOptions = {}): WorldSnapshotLoader {
  const now = options.now ?? (() => Date.now());
  const entries = new Map<string, Entry>();
  let latestVisit = 0;
  let totalRequests = 0;

  /** Run (or reuse) one request. `started` is true only for a new network call. */
  async function request<W>(spec: Spec<W>): Promise<{ resolved: Resolved<W>; started: boolean }> {
    let entry = entries.get(spec.key);
    if (!entry) {
      entry = { family: spec.family, inflight: null, controller: null, outcome: null, lastOk: null, invalidated: false };
      entries.set(spec.key, entry);
    }
    const asResolved = (o: Outcome<unknown>): Resolved<W> => ({
      outcome: o as Outcome<W>,
      lastOk: entry!.lastOk as { value: W; fetchedAt: string } | null,
    });
    if (entry.inflight) return { resolved: asResolved(await entry.inflight), started: false };
    const t = now();
    const o = entry.outcome;
    if (o && !entry.invalidated) {
      const ttl = o.ok ? STALE_AFTER_MS[spec.family] : FAILURE_RETRY_MS;
      if (t - o.at < ttl) return { resolved: asResolved(o), started: false };
    }
    const controller = new AbortController();
    entry.controller = controller;
    totalRequests += 1;
    const current = entry;
    const promise: Promise<Outcome<unknown>> = spec.run(client, controller.signal).then(
      (value): Outcome<unknown> => {
        const at = now();
        return { ok: true, value, fetchedAt: new Date(at).toISOString(), at };
      },
      (error): Outcome<unknown> => {
        const at = now();
        return { ok: false, failure: classifyFailure(error), fetchedAt: new Date(at).toISOString(), at, aborted: isAbort(error) || controller.signal.aborted };
      },
    ).then((outcome) => {
      current.inflight = null;
      current.controller = null;
      if (outcome.ok) {
        current.outcome = outcome;
        current.lastOk = { value: outcome.value, fetchedAt: outcome.fetchedAt };
        current.invalidated = false;
      } else if (!outcome.aborted) {
        current.outcome = outcome;
        current.invalidated = false;
      }
      return outcome;
    });
    entry.inflight = promise;
    return { resolved: asResolved(await promise), started: true };
  }

  async function runAll<T extends readonly Spec<unknown>[]>(specs: T): Promise<{ resolved: { [K in keyof T]: T[K] extends Spec<infer W> ? Resolved<W> : never }; started: number }> {
    if (specs.length > MAX_REQUESTS_PER_PLACE_VISIT) throw new Error(`World place plan exceeds ${MAX_REQUESTS_PER_PLACE_VISIT} requests`);
    const results = await Promise.all(specs.map((s) => request(s)));
    return {
      resolved: results.map((r) => r.resolved) as never,
      started: results.filter((r) => r.started).length,
    };
  }

  function workSections(agentId: string, r: Resolved<Awaited<ReturnType<WorldEvidenceClient['agentWork']>>>) {
    const endpoint = `/api/agents/${encodeURIComponent(agentId)}/work`;
    return {
      activity: toSection(r, 'C2', endpoint, 'work', (w, c) => N.normalizeWorkActivity(agentId, w, c)),
      goals: toSection(r, 'C2', endpoint, 'work', (w, c) => N.normalizeWorkGoals(agentId, w, c)),
      spend: toSection(r, 'C2', endpoint, 'work', (w, c) => N.normalizeWorkSpend(agentId, w, c)),
    };
  }

  async function buildPlace(placeId: WorldPlaceId, ctx: PlaceVisitContext): Promise<{ sections: PlaceSections; started: number }> {
    const pid = ctx.projectId;
    const host = PLACE_HOST[placeId];
    const hostWorkId = workAgentIdFor(host);
    switch (placeId) {
      case 'commons': {
        const { resolved: [h, g, d], started } = await runAll([S.henry(), S.goals(), S.decisions()] as const);
        return {
          started,
          sections: {
            orchestrator: toSection(h, 'C3', '/api/henry/status', 'orchestrator', N.normalizeOrchestrator),
            requests: toSection(h, 'C3', '/api/henry/status', 'orchestrator', N.normalizeRequests),
            active_goals: toSection(g, 'C5', '/api/goals/active', 'goals', N.normalizeActiveGoals),
            decisions: toSection(d, 'C5', '/api/decisions', 'decisions', N.normalizeDecisions),
          },
        };
      }
      case 'gallery': {
        const memSpec = pid ? S.projectMemories(pid) : S.brain();
        const specs = [memSpec, ...(hostWorkId ? [S.work(hostWorkId)] : []), ...(pid ? [S.notes(pid)] : [])] as Spec<unknown>[];
        const { resolved, started } = await runAll(specs);
        const [mem, ...rest] = resolved as Resolved<unknown>[];
        const work = hostWorkId ? rest.shift() : undefined;
        const notes = pid ? rest.shift() : undefined;
        return {
          started,
          sections: {
            memories: pid
              ? toSection(mem as Resolved<never>, 'C4', memSpec.key, 'memories', (w, c) => N.normalizeProjectMemories(pid, w, c))
              : toSection(mem as Resolved<never>, 'C4', memSpec.key, 'memories', N.normalizeBrainMemories),
            librarian_activity: hostWorkId && work ? workSections(hostWorkId, work as never).activity : unsupported(resolveWorldAgent(host)),
            notes: pid && notes ? toSection(notes as Resolved<never>, 'C4', S.notes(pid).key, 'notes', (w, c) => N.normalizeNotes(pid, w, c)) : needsProject('notes'),
          },
        };
      }
      case 'maker': {
        const specs = [...(hostWorkId ? [S.work(hostWorkId)] : []), S.decisions(), S.skills()] as Spec<unknown>[];
        const { resolved, started } = await runAll(specs);
        const list = [...(resolved as Resolved<unknown>[])];
        const work = hostWorkId ? list.shift() : undefined;
        const [d, sk] = list;
        const ws = hostWorkId && work ? workSections(hostWorkId, work as never) : null;
        const un = unsupported(resolveWorldAgent(host));
        return {
          started,
          sections: {
            steward_activity: ws ? ws.activity : un,
            steward_goals: ws ? ws.goals : un,
            steward_spend: ws ? ws.spend : un,
            decisions: toSection(d as Resolved<never>, 'C5', '/api/decisions', 'decisions', N.normalizeDecisions),
            skills: toSection(sk as Resolved<never>, 'C6', '/permagent/skills', 'skills', N.normalizeSkills),
          },
        };
      }
      case 'council_garden': {
        const specs = [...(hostWorkId ? [S.work(hostWorkId)] : []), ...(pid ? [S.market(pid)] : []), S.council()] as Spec<unknown>[];
        const { resolved, started } = await runAll(specs);
        const list = [...(resolved as Resolved<unknown>[])];
        const work = hostWorkId ? list.shift() : undefined;
        const market = pid ? list.shift() : undefined;
        const council = list.shift()!;
        return {
          started,
          sections: {
            forecaster_activity: hostWorkId && work ? workSections(hostWorkId, work as never).activity : unsupported(resolveWorldAgent(host)),
            forecasts: pid && market ? toSection(market as Resolved<never>, 'C7', S.market(pid).key, 'market', (w, c) => N.normalizeMarket(pid, w, c)) : needsProject('forecasts'),
            council: toSection(council as Resolved<never>, 'C7', '/api/council/reports', 'council', N.normalizeCouncil),
          },
        };
      }
      case 'observatory_garden': {
        const sky = {
          state: 'ready' as const,
          items: [{
            kind: 'sky_provenance' as const,
            path: '/world/astronomy/gaia-edr3-provenance.json',
            statement: 'Survey backdrop derived from Gaia EDR3 (CC BY-SA 3.0 IGO). Orientation is artistic and the planets are illustrative: this is not live or calibrated sky data.',
          }],
          truncated: false, bound: 1, contract: 'C7' as const, endpoint: 'bundled:/world/astronomy/gaia-edr3-provenance.json',
          fetchedAt: new Date(0).toISOString(), staleAfterMs: Number.MAX_SAFE_INTEGER,
        };
        if (!pid) return { started: 0, sections: { hypotheses: needsProject('saved hypotheses'), sky } };
        const { resolved: [n], started } = await runAll([S.notes(pid)] as const);
        return {
          started,
          sections: {
            hypotheses: toSection(n, 'C4', S.notes(pid).key, 'notes', (w, c) => N.normalizeNotes(pid, w, c, 'No hypothesis saved on this project yet. Save one from the Observatory.')),
            sky,
          },
        };
      }
      case 'conservatory': {
        const specs = [S.growth(pid), ...(hostWorkId ? [S.detail(hostWorkId), S.work(hostWorkId)] : [])] as Spec<unknown>[];
        const { resolved, started } = await runAll(specs);
        const [gr, det, work] = resolved as Resolved<unknown>[];
        const un = unsupported(resolveWorldAgent(host));
        let growerState: Section<AgentStateRecord> = un;
        if (hostWorkId && det) {
          const workValue = work && (work as Resolved<never>).outcome.ok ? ((work as Resolved<never>).outcome as { value: never }).value : null;
          growerState = toSection(det as Resolved<never>, 'C1', S.detail(hostWorkId).key, 'agent', (w, c) => N.normalizeAgentState(host, w, workValue, c));
        }
        return {
          started,
          sections: {
            growth_results: toSection(gr as Resolved<never>, 'C8', S.growth(pid).key, 'growth', (w, c) => N.normalizeGrowth(pid, w, c)),
            grower_state: growerState,
            grower_activity: hostWorkId && work ? workSections(hostWorkId, work as never).activity : un,
          },
        };
      }
    }
  }

  return {
    get totalRequests() { return totalRequests; },

    async loadPlace(placeId, context) {
      const token = ++latestVisit;
      const ctx: PlaceVisitContext = { projectId: context.projectId || null };
      const { sections, started } = await buildPlace(placeId, ctx);
      return { placeId, context: ctx, sections, requestCount: started, current: token === latestVisit };
    },

    async loadAgent(worldAgentId) {
      const identity = resolveWorldAgent(worldAgentId);
      if (identity.status === 'unsupported') {
        const u = unsupported(identity);
        return { identity, state: u, activity: u, goals: u, spend: u, requestCount: 0 };
      }
      if (identity.status === 'orchestrator') {
        const { resolved: [h], started } = await runAll([S.henry()] as const);
        const none: Section<never> = { state: 'unsupported', reason: 'The orchestrator has no agents-API work record; its current work is under the orchestrator status.' };
        return { identity, state: toSection(h, 'C3', '/api/henry/status', 'orchestrator', N.normalizeOrchestrator), activity: none, goals: none, spend: none, requestCount: started };
      }
      const { resolved: [det, work], started } = await runAll([S.detail(identity.agentId), S.work(identity.agentId)] as const);
      const workValue = work.outcome.ok ? work.outcome.value : null;
      const ws = workSections(identity.agentId, work);
      return {
        identity,
        state: toSection(det, 'C1', identity.detailEndpoint, 'agent', (w, c) => N.normalizeAgentState(worldAgentId, w, workValue, c)),
        activity: ws.activity,
        goals: ws.goals,
        spend: ws.spend,
        requestCount: started,
      };
    },

    invalidate(prefixes) {
      for (const [key, entry] of entries) {
        if (prefixes.some((p) => key.startsWith(p))) entry.invalidated = true;
      }
    },

    invalidateForWorldEvent(event) {
      const prefixes = invalidationsForEvent(event.type);
      this.invalidate(prefixes);
      return prefixes;
    },

    cancel() {
      latestVisit += 1;
      for (const entry of entries.values()) entry.controller?.abort();
    },
  };
}
