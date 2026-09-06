/**
 * Multi-client liveness (#629) — wiring tests for the /events → surface-
 * invalidation seam.
 *
 * Drives {@link routeGlobalFrame} (the ONE global /events router) exactly as
 * ws.onmessage does — parsed frame in — and asserts each `*_changed` domain
 * event genuinely refreshes its surface's data seam:
 *
 *   project_changed   → projectsRev bump (ProjectsView + Documents/Memories/
 *                       Notes panels refetch on it)
 *   person_changed    → peopleRev bump (PeoplePanel refetches on it)
 *   session_changed   → loadSessions() re-reads /api/sessions into the store
 *   workspace_changed → refreshWorkspaces() refetches layouts, PRESERVING this
 *                       client's active workspace
 *   identity_changed  → refreshIdentity() re-reads /api/agent/identity
 *   config_changed    → configRev bump (every Settings pane's read effect
 *                       depends on it)
 *   finance_changed   → financeRev bump (FinanceView's load effect depends
 *                       on it)
 *
 * Also asserts the honesty gates: replayed frames never fire a refetch (the
 * daemon replays ≤1000 frames on every reconnect), and a same-kind burst
 * coalesces into ONE refetch (debounce of real events — not a poll).
 *
 * `./api` is mocked (the store-test pattern) so nothing touches the network.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';

const { getSessions, getWorkspaces, getIdentity, getActiveHarnessRuns } = vi.hoisted(() => ({
  getSessions: vi.fn(),
  getWorkspaces: vi.fn(),
  getIdentity: vi.fn(),
  getActiveHarnessRuns: vi.fn(),
}));

vi.mock('./api', () => ({
  api: { getSessions, getWorkspaces, getIdentity, getActiveHarnessRuns },
  apiFetch: vi.fn(),
  extractText: vi.fn(() => ''),
  extractThinking: vi.fn(() => ''),
  fileToBase64: vi.fn(),
  readerIngest: vi.fn(),
  getApiBaseUrl: vi.fn(() => 'http://localhost:1234'),
  eventsWsUrl: vi.fn(async () => 'ws://localhost:1234/events'),
}));

// Imported AFTER the mock is registered (vi.mock is hoisted above imports).
import { routeGlobalFrame } from '../hooks/useAppNavigate';
import { applyLivenessFrame, _resetLivenessSync, LIVENESS_DEBOUNCE_MS } from './livenessSync';
import { useCommandCenter } from './store';
import { _resetTraceDedupe } from './traceEvents';

/** Epoch this "connection" opened; frames stamped after it count as live. */
const EPOCH = Date.parse('2026-07-18T10:00:00Z');
const LIVE_TS = '2026-07-18T10:00:05Z';
const STALE_TS = '2026-07-18T09:00:00Z';

let frameSeq = 0;
function frame(
  type: string,
  payload: Record<string, unknown>,
  opts: { timestamp?: string; replayed?: boolean } = {},
): unknown {
  return {
    id: `evt-${++frameSeq}`,
    type,
    timestamp: opts.timestamp ?? LIVE_TS,
    ...(opts.replayed ? { replayed: true } : {}),
    payload,
  };
}

/** Route a frame through the production global router (stub nav handlers). */
function route(f: unknown) {
  routeGlobalFrame(f, EPOCH, {
    navigate: () => {},
    launch: () => {},
    theme: 'dark' as never,
  });
}

/** Flush the trailing debounce + any refetch promises it kicked off. */
async function flush() {
  await vi.advanceTimersByTimeAsync(LIVENESS_DEBOUNCE_MS + 10);
}

beforeEach(() => {
  vi.useFakeTimers();
  _resetLivenessSync();
  _resetTraceDedupe();
  getSessions.mockReset().mockResolvedValue([]);
  getWorkspaces.mockReset().mockResolvedValue([]);
  getIdentity.mockReset().mockResolvedValue({ first_name: 'Henry' });
  getActiveHarnessRuns.mockReset().mockResolvedValue([]);
  useCommandCenter.setState({
    events: [],
    projectsRev: 0,
    peopleRev: 0,
    identityRev: 0,
    configRev: 0,
    financeRev: 0,
    agentName: 'Agent',
    sessions: [],
    workspaces: [],
    activeWorkspaceId: null,
    personDetail: null,
    codingSpend: null,
    codingSpendLastKnown: null,
    codingHarnessHydration: 'initial',
    codingHarnessRunId: null,
    codingHarnessRevision: 0,
  });
});

afterEach(() => {
  _resetLivenessSync();
  vi.useRealTimers();
});

describe('project_changed → projects surfaces refetch', () => {
  it('bumps projectsRev (ProjectsView + Documents/Memories/Notes panels depend on it)', async () => {
    route(frame('project_changed', { project_id: 'p1', change: 'updated' }));
    await flush();
    expect(useCommandCenter.getState().projectsRev).toBe(1);
  });

  it('coalesces a burst (multi-file upload) into ONE refetch signal', async () => {
    route(frame('project_changed', { project_id: 'p1', change: 'documents' }));
    route(frame('project_changed', { project_id: 'p1', change: 'documents' }));
    route(frame('project_changed', { project_id: 'p1', change: 'documents' }));
    await flush();
    expect(useCommandCenter.getState().projectsRev).toBe(1);
  });
});

describe('config_changed → open Settings panes refetch', () => {
  // FAILS BEFORE: `config_changed` was not in REFRESH_BY_TYPE (the frame did
  // not exist), so a key written by the agent — or on a second device — left
  // every Settings pane rendering the value it read at mount.
  it('bumps configRev so the Settings read effects re-run', async () => {
    route(frame('config_changed', { keys: ['GOOSE_PROVIDER'], change: 'set', secret: false }));
    await flush();
    expect(useCommandCenter.getState().configRev).toBe(1);
  });

  it('treats a secret write exactly like any other — the name is all it needs', async () => {
    route(frame('config_changed', { keys: ['OPENAI_API_KEY'], change: 'set', secret: true }));
    await flush();
    expect(useCommandCenter.getState().configRev).toBe(1);
  });

  it('coalesces a multi-key write (set_params) into ONE refetch signal', async () => {
    route(frame('config_changed', { keys: ['GOOSE_PROVIDER'], change: 'set', secret: false }));
    route(frame('config_changed', { keys: ['GOOSE_MODEL'], change: 'set', secret: false }));
    await flush();
    expect(useCommandCenter.getState().configRev).toBe(1);
  });

  it('ignores a replayed frame — a reconnect burst must not restage Settings', async () => {
    route(frame('config_changed', { keys: ['GOOSE_MODEL'], change: 'set' }, { replayed: true }));
    route(frame('config_changed', { keys: ['GOOSE_MODEL'], change: 'set' }, { timestamp: STALE_TS }));
    await flush();
    expect(useCommandCenter.getState().configRev).toBe(0);
  });
});

describe('finance_changed → Finance view refetch', () => {
  // FAILS BEFORE: no such frame and no listener; the Finance view's only way to
  // see an agent's trade record was its 60s poll.
  it('bumps financeRev so FinanceView re-reads /api/finance', async () => {
    route(frame('finance_changed', { kind: 'position', change: 'created' }));
    await flush();
    expect(useCommandCenter.getState().financeRev).toBe(1);
  });
});

describe('person_changed → People panel refetch', () => {
  it('bumps peopleRev — the cross-client version of the local modal bump', async () => {
    route(frame('person_changed', { project_id: 'p1', entity_uuid: 'e1', change: 'associated' }));
    await flush();
    expect(useCommandCenter.getState().peopleRev).toBe(1);
  });
});

describe('person_merged → people refetch + open-detail reconciliation', () => {
  it('bumps peopleRev so the directory/graph refetch', async () => {
    route(frame('person_merged', { survivor_uuid: 's1', duplicate_uuid: 'd1', merge_id: 'm1' }));
    await flush();
    expect(useCommandCenter.getState().peopleRev).toBe(1);
  });

  it('closes the open person detail when it is the duplicate that got absorbed', async () => {
    useCommandCenter.setState({
      personDetail: { projectId: null, person: { entity_uuid: 'd1', display_name: 'Dup' } as never, association: null },
    });
    route(frame('person_merged', { survivor_uuid: 's1', duplicate_uuid: 'd1', merge_id: 'm1' }));
    await flush();
    expect(useCommandCenter.getState().personDetail).toBeNull();
    expect(useCommandCenter.getState().peopleRev).toBe(1);
  });

  it('leaves an unrelated open person detail alone', async () => {
    useCommandCenter.setState({
      personDetail: { projectId: null, person: { entity_uuid: 'other', display_name: 'Other' } as never, association: null },
    });
    route(frame('person_merged', { survivor_uuid: 's1', duplicate_uuid: 'd1', merge_id: 'm1' }));
    await flush();
    expect(useCommandCenter.getState().personDetail?.person.entity_uuid).toBe('other');
  });
});

describe('session_changed → sessions list re-reads', () => {
  it('drives loadSessions so a phone-created session lands in the desktop list', async () => {
    getSessions.mockResolvedValue([
      { id: 's-new', name: 'From phone', created_at: 'c', updated_at: 'u', message_count: 1 },
    ]);
    route(frame('session_changed', { session_id: 's-new', change: 'created' }));
    await flush();
    expect(getSessions).toHaveBeenCalledTimes(1);
    expect(useCommandCenter.getState().sessions.map(s => s.id)).toEqual(['s-new']);
  });
});

describe('workspace_changed → layouts refetch, active workspace preserved', () => {
  it('refetches the workspace list and updates layouts', async () => {
    useCommandCenter.setState({
      workspaces: [
        { id: 'w1', name: 'Old', icon: 'i', sortOrder: 0, layoutJson: { type: 'panel', tool: 'chat' } as never, isDefault: true },
      ],
      activeWorkspaceId: 'w1',
    });
    getWorkspaces.mockResolvedValue([
      { id: 'w1', name: 'Renamed', icon: 'i', sortOrder: 0, layoutJson: { type: 'panel', tool: 'world' }, isDefault: true },
      { id: 'w2', name: 'New', icon: 'j', sortOrder: 1, layoutJson: { type: 'panel', tool: 'build' }, isDefault: false },
    ]);
    route(frame('workspace_changed', { workspace_id: 'w1', change: 'layout' }));
    await flush();
    const s = useCommandCenter.getState();
    expect(getWorkspaces).toHaveBeenCalledTimes(1);
    expect(s.workspaces.map(w => w.name)).toEqual(['Renamed', 'New']);
    // A remote layout edit must never yank which workspace THIS client views.
    expect(s.activeWorkspaceId).toBe('w1');
  });

  it('falls back to the first workspace only if the active one disappeared', async () => {
    useCommandCenter.setState({
      workspaces: [
        { id: 'w-gone', name: 'Gone', icon: 'i', sortOrder: 0, layoutJson: { type: 'panel', tool: 'chat' } as never, isDefault: false },
      ],
      activeWorkspaceId: 'w-gone',
    });
    getWorkspaces.mockResolvedValue([
      { id: 'w2', name: 'Survivor', icon: 'j', sortOrder: 0, layoutJson: { type: 'panel', tool: 'build' }, isDefault: true },
    ]);
    route(frame('workspace_changed', { workspace_id: 'w-gone', change: 'layout' }));
    await flush();
    expect(useCommandCenter.getState().activeWorkspaceId).toBe('w2');
  });
});

describe('identity_changed → identity consumers re-read', () => {
  it('re-reads /api/agent/identity, updates agentName, bumps identityRev', async () => {
    getIdentity.mockResolvedValue({ first_name: 'Aria' });
    route(frame('identity_changed', { display_name: 'Aria Prime' }));
    await flush();
    const s = useCommandCenter.getState();
    expect(getIdentity).toHaveBeenCalledTimes(1);
    expect(s.agentName).toBe('Aria');
    expect(s.identityRev).toBe(1);
  });

  it('keeps the current name when the re-read fails (no blanking a live UI)', async () => {
    useCommandCenter.setState({ agentName: 'Henry' });
    getIdentity.mockRejectedValue(new Error('down'));
    route(frame('identity_changed', { display_name: 'X' }));
    await flush();
    const s = useCommandCenter.getState();
    expect(s.agentName).toBe('Henry');
    expect(s.identityRev).toBe(0);
  });
});

describe('session_spend_changed → codingSpend applies immediately (not debounced)', () => {
  const rawPayload = {
    session_id: 'harness-1',
    turn_usd: 0.0032,
    session_usd: 0.0332,
    today_usd: 0.5332,
    total_tokens: 12800,
    provider: 'zai',
    model: 'glm-5.3',
    working_dir: '/tmp/proj',
    estimated: true,
    final_turn: false,
  };

  it('camelCases a raw daemon frame into store.codingSpend synchronously, with no debounce wait', () => {
    // Exactly as the daemon serializes it: snake_case payload keys, applied
    // via applyLivenessFrame directly (this lane is not driven through
    // routeGlobalFrame's nav-only stub in this file's `route` helper).
    applyLivenessFrame(frame('session_spend_changed', rawPayload), EPOCH);

    // No `await flush()` — the APPLY_BY_TYPE lane is synchronous by design
    // (see livenessSync.ts module doc), so the store must already reflect it.
    expect(useCommandCenter.getState().codingSpend).toEqual({
      sessionId: 'harness-1',
      turnUsd: 0.0032,
      sessionUsd: 0.0332,
      todayUsd: 0.5332,
      totalTokens: 12800,
      provider: 'zai',
      model: 'glm-5.3',
      workingDir: '/tmp/proj',
      estimated: true,
      finalTurn: false,
    });
  });

  it('ignores a replayed spend frame — a reconnect burst must not stomp a live total', () => {
    applyLivenessFrame(frame('session_spend_changed', rawPayload, { replayed: true }), EPOCH);
    expect(useCommandCenter.getState().codingSpend).toBeNull();
  });

  function canonicalBudget(rootSessionId: string, asOf: string) {
    const scope = {
      cap: { softUsd: 1, gateUsd: 2, hardUsd: 3, source: 'current_budget_config' },
      settledUsd: 0.25, heldUsd: 0, unknownUsd: 0,
      effectiveUsedUsd: 0.25, remainingUsd: 2.75,
      band: 'ok', completeness: 'complete', error: null,
    };
    const evidence = {
      billingClass: 'paid_api', provider: 'fixture', model: 'fixture-model',
      callId: 'call-1', isEstimated: false,
      observedAt: asOf, source: 'cost_ledger',
    };
    return {
      taskId: 'task-1', rootSessionId, task: scope, session: scope,
      taskBilling: evidence, sessionBilling: evidence,
      provenance: {
        version: 'budget-projection.v1', asOf, completeness: 'complete',
        sources: ['sessions', 'cost_ledger'], error: null,
      },
    };
  }

  it('accepts a canonical projection but rejects bad identity/version instead of overwriting', () => {
    const budget = canonicalBudget('harness-1', LIVE_TS);
    applyLivenessFrame(frame('session_spend_changed', {
      ...rawPayload, session_id: 'harness-1', session_usd: 0.25, budget,
    }), EPOCH);
    expect(useCommandCenter.getState().codingSpend?.budget?.rootSessionId).toBe('harness-1');

    applyLivenessFrame(frame('session_spend_changed', {
      ...rawPayload, session_id: 'harness-1', session_usd: 0,
      budget: canonicalBudget('different-root', LIVE_TS),
    }), EPOCH);
    expect(useCommandCenter.getState().codingSpend?.sessionUsd).toBe(0.25);
    expect(useCommandCenter.getState().codingSpend?.budgetStatus).toBe('unavailable');
    expect(useCommandCenter.getState().codingSpendLastKnown?.sessionUsd).toBe(0.25);
    expect(useCommandCenter.getState().codingHarnessHydration).toBe('unavailable');

    const badVersion = canonicalBudget('harness-1', LIVE_TS);
    badVersion.provenance.version = 'budget-projection.v0';
    applyLivenessFrame(frame('session_spend_changed', {
      ...rawPayload, session_id: 'harness-1', budget: badVersion,
    }), EPOCH);
    expect(useCommandCenter.getState().codingSpend?.sessionUsd).toBe(0.25);
  });

  it('does not let a stale canonical frame or late non-terminal frame regress a terminal total', () => {
    const current = canonicalBudget('harness-1', '2026-07-18T10:00:05Z');
    applyLivenessFrame(frame('session_spend_changed', {
      ...rawPayload, session_id: 'harness-1', session_usd: 0.5, budget: current, final_turn: true,
    }), EPOCH);
    const stale = canonicalBudget('harness-1', '2026-07-18T10:00:04Z');
    applyLivenessFrame(frame('session_spend_changed', {
      ...rawPayload, session_id: 'harness-1', session_usd: 0.1, budget: stale, final_turn: false,
    }), EPOCH);
    expect(useCommandCenter.getState().codingSpend?.sessionUsd).toBe(0.5);
    expect(useCommandCenter.getState().codingSpend?.finalTurn).toBe(true);
  });

  it('keeps malformed session B unavailable under B identity while A remains last-known only', () => {
    applyLivenessFrame(frame('session_spend_changed', {
      ...rawPayload, session_id: 'session-a',
      budget: canonicalBudget('session-a', '2026-07-18T10:00:10Z'), session_usd: 1,
    }), EPOCH);
    applyLivenessFrame(frame('session_spend_changed', {
      ...rawPayload, session_id: 'session-b', session_usd: 99,
      budget: { malformed: true },
    }), EPOCH);
    const state = useCommandCenter.getState();
    expect(state.codingSpend?.sessionId).toBe('session-b');
    expect(state.codingSpend?.sessionUsd).toBeNull();
    expect(state.codingSpend?.budget).toBeUndefined();
    expect(state.codingSpend?.budgetStatus).toBe('unavailable');
    expect(state.codingSpendLastKnown?.sessionId).toBe('session-a');

    // A legitimate new B session may have an older provider timestamp than A;
    // cross-session ordering must not discard its recovery frame.
    applyLivenessFrame(frame('session_spend_changed', {
      ...rawPayload, session_id: 'session-b', session_usd: 2,
      budget: canonicalBudget('session-b', '2026-07-18T09:00:00Z'),
    }), EPOCH);
    expect(useCommandCenter.getState().codingSpend?.sessionId).toBe('session-b');
    expect(useCommandCenter.getState().codingSpend?.budget?.rootSessionId).toBe('session-b');
    expect(useCommandCenter.getState().codingSpend?.sessionUsd).toBe(2);
  });

  it('hydrates the canonical active harness once and preserves it when a read races a live frame', async () => {
    const budget = canonicalBudget('harness-1', LIVE_TS);
    getActiveHarnessRuns.mockResolvedValueOnce([{
      runId: 'run-1', sessionId: 'harness-1', project: '/tmp/proj',
      status: 'running', updatedAt: LIVE_TS, tokens: 12800,
      provider: 'fixture', model: 'fixture-model', budget,
    }]);
    await useCommandCenter.getState().hydrateCodingHarness();
    expect(useCommandCenter.getState().codingHarnessRunId).toBe('run-1');
    expect(useCommandCenter.getState().codingSpend?.budget?.session.effectiveUsedUsd).toBe(0.25);

    let release!: (runs: unknown[]) => void;
    getActiveHarnessRuns.mockReturnValueOnce(new Promise(resolve => { release = resolve; }));
    const pending = useCommandCenter.getState().hydrateCodingHarness();
    applyLivenessFrame(frame('session_spend_changed', {
      ...rawPayload, session_id: 'harness-1', session_usd: 0.5,
      budget: canonicalBudget('harness-1', '2026-07-18T10:00:06Z'),
    }), EPOCH);
    release([{
      runId: 'old-run', sessionId: 'harness-1', project: '/tmp/proj',
      status: 'running', updatedAt: '2026-07-18T10:00:01Z', tokens: 1,
      provider: 'fixture', model: 'fixture-model', budget: canonicalBudget('harness-1', '2026-07-18T10:00:01Z'),
    }]);
    await pending;
    expect(useCommandCenter.getState().codingSpend?.sessionUsd).toBe(0.5);
    expect(useCommandCenter.getState().codingHarnessRunId).toBe('run-1');
  });

  it('marks an initial 503 unavailable without inventing a harness/session id', async () => {
    getActiveHarnessRuns.mockRejectedValueOnce(new Error('503'));
    await useCommandCenter.getState().hydrateCodingHarness();
    expect(useCommandCenter.getState().codingHarnessHydration).toBe('unavailable');
    expect(useCommandCenter.getState().codingSpend).toBeNull();
    expect(useCommandCenter.getState().codingHarnessRunId).toBeNull();
  });

  it('uses request generation ordering for overlapping old success/empty/error responses', async () => {
    let releaseOld!: (value: unknown) => void;
    let rejectOld!: (reason?: unknown) => void;
    getActiveHarnessRuns
      .mockReturnValueOnce(new Promise(resolve => { releaseOld = resolve; }))
      .mockReturnValueOnce(new Promise((_, reject) => { rejectOld = reject; }));
    const oldSuccess = useCommandCenter.getState().hydrateCodingHarness();
    const latestError = useCommandCenter.getState().hydrateCodingHarness();
    releaseOld([{
      runId: 'old', sessionId: 'old-session', project: '/tmp', status: 'running',
      updatedAt: LIVE_TS, tokens: 1, provider: null, model: null,
      budget: canonicalBudget('old-session', LIVE_TS),
    }]);
    rejectOld(new Error('latest 503'));
    await Promise.all([oldSuccess, latestError]);
    expect(useCommandCenter.getState().codingHarnessHydration).toBe('unavailable');
    expect(useCommandCenter.getState().codingSpend).toBeNull();
    expect(useCommandCenter.getState().codingHarnessRunId).toBeNull();

    getActiveHarnessRuns
      .mockReturnValueOnce(new Promise(resolve => { releaseOld = resolve; }))
      .mockResolvedValueOnce([]);
    const oldSuccess2 = useCommandCenter.getState().hydrateCodingHarness();
    const latestEmpty = useCommandCenter.getState().hydrateCodingHarness();
    releaseOld([{
      runId: 'old-2', sessionId: 'old-session-2', project: '/tmp', status: 'running',
      updatedAt: LIVE_TS, tokens: 1, provider: null, model: null,
      budget: canonicalBudget('old-session-2', LIVE_TS),
    }]);
    await Promise.all([oldSuccess2, latestEmpty]);
    expect(useCommandCenter.getState().codingHarnessHydration).toBe('none');
    expect(useCommandCenter.getState().codingSpend).toBeNull();
  });

  it('keeps terminal evidence when active TTL hydration returns an empty list', async () => {
    getActiveHarnessRuns.mockResolvedValueOnce([{
      runId: 'terminal-run', sessionId: 'harness-1', project: '/tmp/proj',
      status: 'succeeded', updatedAt: LIVE_TS, tokens: 12800,
      provider: 'fixture', model: 'fixture-model', budget: canonicalBudget('harness-1', LIVE_TS),
    }]);
    await useCommandCenter.getState().hydrateCodingHarness();
    expect(useCommandCenter.getState().codingSpend?.finalTurn).toBe(true);
    getActiveHarnessRuns.mockResolvedValueOnce([]);
    await useCommandCenter.getState().hydrateCodingHarness();
    expect(useCommandCenter.getState().codingHarnessHydration).toBe('none');
    expect(useCommandCenter.getState().codingSpend?.finalTurn).toBe(true);
    expect(useCommandCenter.getState().codingSpend?.sessionId).toBe('harness-1');
  });

  it('does not label malformed hydrated B with valid A cost, then recovers valid B', async () => {
    getActiveHarnessRuns.mockResolvedValueOnce([{
      runId: 'run-a', sessionId: 'session-a', project: '/tmp/a',
      status: 'running', updatedAt: LIVE_TS, tokens: 10,
      provider: 'fixture', model: 'fixture-model', budget: canonicalBudget('session-a', LIVE_TS),
    }]);
    await useCommandCenter.getState().hydrateCodingHarness();
    getActiveHarnessRuns.mockResolvedValueOnce([{
      runId: 'run-b', sessionId: 'session-b', project: '/tmp/b',
      status: 'running', updatedAt: '2026-07-18T10:00:06Z', tokens: 20,
      provider: 'fixture', model: 'fixture-model', budget: { malformed: true },
    }]);
    await useCommandCenter.getState().hydrateCodingHarness();
    let state = useCommandCenter.getState();
    expect(state.codingSpend?.sessionId).toBe('session-b');
    expect(state.codingSpend?.sessionUsd).toBeNull();
    expect(state.codingSpend?.budget).toBeUndefined();
    expect(state.codingSpendLastKnown?.sessionId).toBe('session-a');
    expect(state.codingHarnessRunId).toBe('run-b');

    getActiveHarnessRuns.mockResolvedValueOnce([{
      runId: 'run-b', sessionId: 'session-b', project: '/tmp/b',
      status: 'running', updatedAt: '2026-07-18T10:00:07Z', tokens: 21,
      provider: 'fixture', model: 'fixture-model',
      budget: canonicalBudget('session-b', '2026-07-18T09:00:00Z'),
    }]);
    await useCommandCenter.getState().hydrateCodingHarness();
    state = useCommandCenter.getState();
    expect(state.codingSpend?.sessionId).toBe('session-b');
    expect(state.codingSpend?.budget?.rootSessionId).toBe('session-b');
    expect(state.codingSpend?.sessionUsd).toBe(0.25);
    expect(state.codingHarnessHydration).toBe('active');
  });
});

describe('replay honesty — reconnect bursts never refetch', () => {
  it('drops frames carrying the server-side replayed marker', async () => {
    applyLivenessFrame(
      frame('project_changed', { project_id: 'p1', change: 'updated' }, { replayed: true }),
      EPOCH,
    );
    await flush();
    expect(useCommandCenter.getState().projectsRev).toBe(0);
  });

  it('drops unmarked frames stamped before this connection opened', async () => {
    applyLivenessFrame(
      frame('session_changed', { session_id: 's1', change: 'created' }, { timestamp: STALE_TS }),
      EPOCH,
    );
    await flush();
    expect(getSessions).not.toHaveBeenCalled();
  });

  it('ignores non-liveness frame types entirely', async () => {
    applyLivenessFrame(frame('goal_state_changed', { goal_id: 'g1' }), EPOCH);
    applyLivenessFrame(frame('memory_added', { memory_id: 'm1' }), EPOCH);
    await flush();
    const s = useCommandCenter.getState();
    expect(s.projectsRev).toBe(0);
    expect(s.peopleRev).toBe(0);
    expect(getSessions).not.toHaveBeenCalled();
    expect(getWorkspaces).not.toHaveBeenCalled();
    expect(getIdentity).not.toHaveBeenCalled();
  });
});
