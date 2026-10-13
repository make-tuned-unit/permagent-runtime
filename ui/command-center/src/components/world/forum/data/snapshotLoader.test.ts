import { describe, expect, it } from 'vitest';
import { BOUNDS, MAX_REQUESTS_PER_PLACE_VISIT, STALE_AFTER_MS } from './bounds';
import { WORLD_PLACE_IDS } from './identity';
import { createWorldSnapshotLoader, PLACE_REQUEST_PLAN } from './snapshotLoader';
import { sectionFreshness, visibleItems } from './types';
import { FIXTURE_NOW, FIXTURE_PROJECT, WORLD_FIXTURES, createFixtureClient, type FixtureData } from './fixtures/worldFixtures';
import { runFixture } from './fixtures/fixtureSnapshots';

const fixture = (id: string) => WORLD_FIXTURES.find((f) => f.id === id)!;

function setup(data: FixtureData | (() => FixtureData), start = FIXTURE_NOW) {
  let clock = start;
  const calls: string[] = [];
  const client = createFixtureClient(typeof data === 'function' ? data : () => data, calls);
  const loader = createWorldSnapshotLoader(client, { now: () => clock });
  return { loader, calls, advance: (ms: number) => { clock += ms; }, now: () => clock };
}

describe('request-count discipline', () => {
  it(`no place visit exceeds ${MAX_REQUESTS_PER_PLACE_VISIT} requests, with or without a project, in any fixture`, async () => {
    const measured: Record<string, number> = {};
    for (const f of WORLD_FIXTURES) {
      for (const projectId of [null, FIXTURE_PROJECT]) {
        for (const placeId of WORLD_PLACE_IDS) {
          // A fresh loader per visit: the worst case, nothing cached.
          const { loader, calls } = setup(f.phases[0]);
          const snap = await loader.loadPlace(placeId, { projectId });
          expect(snap.requestCount).toBe(calls.length);
          expect(calls.length).toBeLessThanOrEqual(MAX_REQUESTS_PER_PLACE_VISIT);
          expect(calls.length).toBeLessThanOrEqual(PLACE_REQUEST_PLAN[placeId].length);
          const k = `${placeId}${projectId ? '+project' : ''}`;
          measured[k] = Math.max(measured[k] ?? 0, calls.length);
        }
      }
    }
    // The measurement recorded in the W1 receipt.
    expect(measured).toEqual({
      commons: 3, 'commons+project': 3,
      gallery: 2, 'gallery+project': 3,
      maker: 3, 'maker+project': 3,
      council_garden: 2, 'council_garden+project': 3,
      observatory_garden: 0, 'observatory_garden+project': 1,
      conservatory: 3, 'conservatory+project': 3,
    });
  });

  it('concurrent visits share in-flight requests; a fresh revisit makes none', async () => {
    const { loader, calls } = setup(fixture('active_request').phases[0]);
    const [a, b] = await Promise.all([loader.loadPlace('commons', { projectId: null }), loader.loadPlace('commons', { projectId: null })]);
    expect(calls).toHaveLength(3);
    expect(a.requestCount + b.requestCount).toBe(3);
    const again = await loader.loadPlace('commons', { projectId: null });
    expect(again.requestCount).toBe(0);
    expect(calls).toHaveLength(3);
    // Decisions fetched at the Arrival are reused by the Maker visit.
    const maker = await loader.loadPlace('maker', { projectId: null });
    expect(maker.requestCount).toBe(2);
    expect(loader.totalRequests).toBe(5);
  });

  it('refetches after the staleness threshold or a World event, not before', async () => {
    const { loader, calls, advance } = setup(fixture('active_request').phases[0]);
    await loader.loadPlace('commons', { projectId: null });
    advance(STALE_AFTER_MS.goals - 1);
    expect((await loader.loadPlace('commons', { projectId: null })).requestCount).toBe(0);
    expect(loader.invalidateForWorldEvent({ type: 'decision_created' })).toEqual(['/api/decisions']);
    expect((await loader.loadPlace('commons', { projectId: null })).requestCount).toBe(1);
    advance(2);
    // Status and goals passed their threshold; decisions were refetched 2 ms ago.
    expect((await loader.loadPlace('commons', { projectId: null })).requestCount).toBe(2);
    expect(calls.length).toBe(6);
    expect(loader.invalidateForWorldEvent({ type: 'something_else', replayed: true })).toEqual([]);
  });
});

describe('evidence states are values, not nulls', () => {
  it('empty: a fresh account renders honest empty copy, not errors', async () => {
    const { snapshots } = await runFixture(fixture('fresh_account'));
    const arrival = snapshots.commons.sections;
    expect(arrival.active_goals?.state).toBe('empty');
    expect(arrival.decisions?.state).toBe('empty');
    expect(arrival.requests?.state).toBe('empty');
    const lib = snapshots.gallery.sections.librarian_activity!;
    expect(lib.state).toBe('empty');
    if (lib.state === 'empty') expect(lib.copy).toMatch(/does not prove/);
    expect(snapshots.gallery.sections.notes?.state).toBe('setup_required');
    const grower = snapshots.conservatory.sections.grower_state!;
    expect(grower.state).toBe('ready');
    if (grower.state === 'ready') {
      expect(grower.items[0].truth.configured.value).toBe('off');
      expect(grower.items[0].truth.ready.value).toBe('not_ready');
    }
  });

  it('truncated: more than the bound is shown as truncated, never as complete', async () => {
    const memories = Array.from({ length: BOUNDS.memories + 4 }, (_, i) => ({ id: `fixture-m${i}`, key: null, text: 'x'.repeat(BOUNDS.memoryTextChars + 50), description: null, timestamp: new Date(FIXTURE_NOW).toISOString(), source: null }));
    const { loader } = setup({ ...fixture('agents_no_work').phases[0], brainMemories: { memories, total: memories.length, has_more: true } });
    const s = (await loader.loadPlace('gallery', { projectId: null })).sections.memories!;
    expect(s.state).toBe('ready');
    if (s.state === 'ready') {
      expect(s.items).toHaveLength(BOUNDS.memories);
      expect(s.truncated).toBe(true);
      expect(s.items[0].textClipped).toBe(true);
      expect(s.items[0].text.length).toBeLessThanOrEqual(BOUNDS.memoryTextChars + 1);
      expect(s.items[0].provenance).toEqual({ kind: 'none' });
    }
  });

  it('spend names its scope and is a lower bound when goals were truncated', async () => {
    const base = fixture('completed_goal').phases[0];
    const w = JSON.parse(JSON.stringify(base.agentWork!.git_steward));
    w.goals.truncated = true;
    const { loader } = setup({ ...base, agentWork: { ...base.agentWork, git_steward: w } });
    const spend = (await loader.loadPlace('maker', { projectId: null })).sections.steward_spend!;
    expect(spend.state).toBe('ready');
    if (spend.state === 'ready') {
      expect(spend.items[0].lowerBound).toBe(true);
      expect(spend.items[0].scope).toMatch(/not a lifetime total/);
    }
  });

  it('a section the daemon reports unavailable is an error, not an empty list', async () => {
    const base = fixture('agents_no_work').phases[0];
    const w = JSON.parse(JSON.stringify(base.agentWork!.librarian));
    w.activity = { attribution: 'librarian', status: 'unavailable', reason: 'journal locked' };
    const { loader } = setup({ ...base, agentWork: { ...base.agentWork, librarian: w } });
    const s = (await loader.loadPlace('gallery', { projectId: null })).sections.librarian_activity!;
    expect(s.state).toBe('error');
    if (s.state === 'error') expect(s.failure).toMatchObject({ cause: 'section_unavailable', message: 'journal locked' });
  });

  it('stale + error: a lost connection keeps last good data, marked stale, then recovers', async () => {
    const f = fixture('disconnect_replay');
    let phase = 0;
    const { loader, advance, now } = setup(() => f.phases[phase]);
    const live = await loader.loadPlace('commons', { projectId: null });
    expect(live.sections.active_goals?.state).toBe('ready');
    expect(sectionFreshness(live.sections.active_goals!, now())).toBe('fresh');
    advance(STALE_AFTER_MS.goals + 1);
    // Old data is visibly stale before anything is refetched.
    expect(sectionFreshness(live.sections.active_goals!, now())).toBe('stale');

    phase = 1;
    const down = await loader.loadPlace('commons', { projectId: null });
    const goals = down.sections.active_goals!;
    expect(goals.state).toBe('error');
    if (goals.state === 'error') {
      expect(goals.failure.cause).toBe('disconnected');
      expect(goals.lastGood?.items).toHaveLength(1);
    }
    // A source that last answered "nothing" keeps that as its last known answer.
    const decisions = down.sections.decisions!;
    expect(decisions.state === 'error' && decisions.lastGood).toEqual({ items: [], truncated: false, fetchedAt: new Date(FIXTURE_NOW).toISOString() });
    const v = visibleItems(goals, now());
    expect(v.stale).toBe(true);
    expect(v.items).toHaveLength(1);

    phase = 2;
    advance(10_000);
    const back = await loader.loadPlace('commons', { projectId: null });
    expect(back.sections.active_goals?.state).toBe('ready');
    expect(back.requestCount).toBe(3);
  });

  it('a failed task is a record, distinct from a lost connection', async () => {
    const { snapshots } = await runFixture(fixture('failed_cancelled'));
    const req = snapshots.commons.sections.requests!;
    expect(req.state).toBe('ready');
    if (req.state === 'ready') expect(req.items.map((r) => r.outcome)).toEqual(['failed', 'cancelled']);
    const goals = snapshots.maker.sections.steward_goals!;
    if (goals.state === 'ready') expect(goals.items.map((g) => g.phase)).toEqual(['failed', 'cancelled']);
  });

  it('forecast refusal, usable forecast, Council dissent', async () => {
    const { snapshots } = await runFixture(fixture('forecasts'));
    const fc = snapshots.council_garden.sections.forecasts!;
    expect(fc.state).toBe('ready');
    if (fc.state === 'ready') {
      expect(fc.items.map((r) => r.outcome.kind)).toEqual(['forecast', 'refusal']);
      expect(fc.items[1].outcome).toMatchObject({ reason: 'insufficient_history', points: 9, needed: 28 });
    }
    const council = snapshots.council_garden.sections.council!;
    if (council.state === 'ready') expect(council.items[0].dissentCount).toBe(1);
    // No series bound is a setup state, not a flat trend.
    const { snapshots: none } = await runFixture({ ...fixture('fresh_account'), projectId: FIXTURE_PROJECT });
    expect(none.council_garden.sections.forecasts?.state).toBe('setup_required');
  });

  it('growth keeps helped and inconclusive verdicts', async () => {
    const { snapshots } = await runFixture(fixture('growth'));
    const g = snapshots.conservatory.sections.growth_results!;
    if (g.state === 'ready') expect(g.items.map((r) => r.verdict)).toEqual(['helped', 'inconclusive']);
    else throw new Error(`expected ready, got ${g.state}`);
  });

  it('attribution: work records name the attributed agent and their source', async () => {
    const { snapshots } = await runFixture(fixture('active_request'));
    const act = snapshots.maker.sections.steward_activity!;
    expect(act.state).toBe('ready');
    if (act.state === 'ready') {
      expect(act.items[0].actor).toBe('git_steward');
      expect(act.items[0].source).toMatchObject({ contract: 'C2', endpoint: '/api/agents/git_steward/work', scope: 'activity attributed to git_steward' });
      expect(act.items[0].link).toEqual({ to: 'goal', projectId: FIXTURE_PROJECT, goalId: 'fixture-goal-1' });
    }
    const orch = snapshots.commons.sections.orchestrator!;
    if (orch.state === 'ready') expect(orch.items[0].sessions[0].link).toEqual({ to: 'conversation', sessionId: 'fixture-session-1' });
  });

  it('memory provenance: a source tag where recorded, an explicit none where not', async () => {
    const { snapshots } = await runFixture(fixture('memory_provenance'));
    const m = snapshots.gallery.sections.memories!;
    if (m.state !== 'ready') throw new Error('expected ready');
    expect(m.items.map((x) => x.provenance)).toEqual([{ kind: 'source_tag', tag: 'fixture-meeting-notes.md' }, { kind: 'none' }]);
    expect(m.items[0].link).toMatchObject({ to: 'brain', memoryId: 'fixture-mem-1', memoryKey: 'note:fixture-1' });
  });
});

describe('unsupported agents', () => {
  it('Reader, Picker and Polybot make zero requests and resolve unsupported', async () => {
    const { loader, calls } = setup(fixture('active_request').phases[0]);
    for (const id of ['reader', 'picker', 'polybot', 'not-an-agent']) {
      const ev = await loader.loadAgent(id);
      expect(ev.identity.status).toBe('unsupported');
      expect(ev.requestCount).toBe(0);
      for (const s of [ev.state, ev.activity, ev.goals, ev.spend]) expect(s.state).toBe('unsupported');
    }
    expect(calls).toEqual([]);
  });

  it('a worker profile reads its own detail and work only; the orchestrator reads status only', async () => {
    const { loader, calls } = setup(fixture('active_request').phases[0]);
    const steward = await loader.loadAgent('steward');
    expect(calls.sort()).toEqual(['agentDetail:git_steward', 'agentWork:git_steward']);
    expect(steward.requestCount).toBe(2);
    expect(steward.state.state).toBe('ready');
    calls.length = 0;
    const henry = await loader.loadAgent('henry');
    expect(calls).toEqual(['henryStatus']);
    expect(henry.activity.state).toBe('unsupported');
  });
});

describe('replay and supersession', () => {
  it('loading the same fixture twice yields identical snapshots', async () => {
    for (const f of WORLD_FIXTURES) {
      const a = await runFixture(f);
      const b = await runFixture(f);
      expect(b.snapshots).toEqual(a.snapshots);
      expect(b.calls).toEqual(a.calls);
    }
  });

  it('a cached revisit yields a deep-equal snapshot', async () => {
    const { loader } = setup(fixture('decision_review').phases[0]);
    const a = await loader.loadPlace('maker', { projectId: null });
    const b = await loader.loadPlace('maker', { projectId: null });
    expect({ ...b, requestCount: 0 }).toEqual({ ...a, requestCount: 0 });
  });

  it('latest visit wins: an older visit resolves as not current', async () => {
    const { loader } = setup(fixture('active_request').phases[0]);
    const first = loader.loadPlace('commons', { projectId: null });
    const second = loader.loadPlace('gallery', { projectId: null });
    expect((await first).current).toBe(false);
    expect((await second).current).toBe(true);
  });

  it('cancel aborts in-flight requests without caching the abort as a failure', async () => {
    const signals: AbortSignal[] = [];
    const base = createFixtureClient(() => fixture('active_request').phases[0]);
    let henryCalls = 0;
    const client = {
      ...base,
      henryStatus: (signal: AbortSignal) => {
        henryCalls += 1;
        if (henryCalls > 1) return base.henryStatus(signal);
        return new Promise<never>((_, reject) => {
          signals.push(signal);
          signal.addEventListener('abort', () => reject(Object.assign(new Error('aborted'), { name: 'AbortError' })));
        });
      },
    };
    const loader = createWorldSnapshotLoader(client, { now: () => FIXTURE_NOW });
    const pending = loader.loadPlace('commons', { projectId: null });
    await Promise.resolve();
    loader.cancel();
    const snap = await pending;
    expect(signals[0].aborted).toBe(true);
    expect(snap.current).toBe(false);
    // The abort was not cached as a failure: the next visit asks again and succeeds.
    const retry = await loader.loadPlace('commons', { projectId: null });
    expect(henryCalls).toBe(2);
    expect(retry.sections.orchestrator?.state).toBe('ready');
    expect(retry.current).toBe(true);
  });

  it('the loader exposes no write, send or answer method', () => {
    const { loader } = setup({});
    expect(Object.keys(loader).sort()).toEqual(['cancel', 'invalidate', 'invalidateForWorldEvent', 'loadAgent', 'loadPlace', 'totalRequests']);
  });
});
