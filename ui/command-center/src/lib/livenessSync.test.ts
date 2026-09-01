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

const { getSessions, getWorkspaces, getIdentity } = vi.hoisted(() => ({
  getSessions: vi.fn(),
  getWorkspaces: vi.fn(),
  getIdentity: vi.fn(),
}));

vi.mock('./api', () => ({
  api: { getSessions, getWorkspaces, getIdentity },
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
