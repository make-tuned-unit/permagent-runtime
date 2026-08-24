/**
 * @vitest-environment jsdom
 *
 * Settings → Agents honesty pins: workers never offer dispatch, a gated agent is
 * LISTED with its own switch rather than hidden, controls that enforce nothing
 * are not offered at all, secret values never enter the DOM, and unavailable /
 * empty attribution stay distinct from idle.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const SEED_SECRET_VALUE = 'super-secret-value-NEVER-RENDER-me-9f3a';

vi.mock('../../../lib/api', () => ({
  apiFetch: vi.fn(),
  getApiBaseUrl: vi.fn(() => 'http://localhost:1234'),
  api: { readConfig: vi.fn(), upsertConfig: vi.fn() },
}));

/**
 * The deep-link focus has to be settable per test (the unknown-agent note is
 * only reachable through it), so the mocked store reads a hoisted box rather
 * than a frozen literal.
 */
const store = vi.hoisted(() => ({ pendingAgentFocus: null as string | null }));

vi.mock('../../../lib/store', () => {
  const state = () => ({
    pendingAgentFocus: store.pendingAgentFocus,
    clearPendingAgentFocus: () => { store.pendingAgentFocus = null; },
    focusWorldAgent: () => true,
    openAgentSettings: () => {},
  });
  return {
    useCommandCenter: Object.assign(
      (selector: (s: Record<string, unknown>) => unknown) => selector(state()),
      { getState: state },
    ),
  };
});

import { AgentsPanel } from './AgentsPanel';
import { api, apiFetch } from '../../../lib/api';
import { AGENT_TRIM } from '../../world/shared/palette';
import { getThemedColors } from '../../../styles/tokens';
import { CAPABILITY_NOT_REPORTED } from '../../../lib/agentsApi';
import type {
  AgentRun,
  BriefingItem,
  RosterResponse,
  WorkReview,
} from '../../../lib/agentsApi';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const apiFetchMock = vi.mocked(apiFetch);
const upsertConfig = vi.mocked(api.upsertConfig);

/**
 * Typed against the wire mirror, so a daemon-side rename of `gate.config_key`
 * breaks the build here rather than silently rendering no switch. The panel
 * still reads the field through the validating `readAgentGate`, because a daemon
 * older than this app serialises no gate at all.
 */
const ROSTER: RosterResponse = {
  workers: [
    {
      id: 'scheduler',
      display_name: 'Scheduler',
      what_it_does: 'Runs scheduled jobs',
      why_it_matters: 'Automations fire',
      state_source: 'queryable',
      live_state: { status: 'unavailable', reason: 'pool locked' },
      dispatchable: false,
      gate: null,
      ask: { status: 'unavailable', reason: 'the scheduler exposes no conversational surface' },
      run_now: { status: 'unavailable', reason: 'the scheduler ticks on its own timer and cannot be stepped by hand' },
    },
    {
      id: 'strix',
      display_name: 'The Guard',
      what_it_does: 'Sweeps one project per pass for security flaws',
      why_it_matters: 'Exposed secrets get found before someone else finds them',
      state_source: 'queryable',
      live_state: { status: 'ok', value: 'off (strix_enabled=false)' },
      dispatchable: false,
      gate: { config_key: 'strix_enabled', enabled: false },
      ask: { status: 'available' },
      run_now: { status: 'available' },
    },
  ],
  dispatch_roster: [
    {
      key: 'claude_code',
      display_name: 'Claude Code',
      role: 'coder',
      cost_tier: 'subscription',
      engine: 'external_cli',
      workflow_role: null,
      availability: { status: 'available' },
      grants: { mode: 'inherit_global' },
      grants_enforced: false,
      secrets: {
        status: 'ok',
        items: [{ name: 'api_token', presence: 'present' }],
        truncated: false,
      },
      gate: null,
      ask: { status: 'available' },
      run_now: { status: 'unavailable', reason: 'this engine has no single pass to run — work reaches it through a goal' },
    },
    {
      key: 'steward',
      display_name: 'Steward',
      role: 'repo hygiene',
      cost_tier: 'local',
      engine: 'pending',
      workflow_role: null,
      availability: { status: 'available' },
      grants: { mode: 'explicit', extensions: ['grow'], truncated: false },
      grants_enforced: false,
      secrets: { status: 'ok', items: [], truncated: false },
      gate: { config_key: 'steward_scan_enabled', enabled: false },
      ask: { status: 'available' },
      run_now: { status: 'available' },
    },
    {
      key: 'planner',
      display_name: 'Planner',
      role: 'planning',
      cost_tier: 'metered',
      engine: 'internal_subagent',
      workflow_role: null,
      availability: { status: 'available' },
      grants: { mode: 'explicit', extensions: ['grow'], truncated: false },
      grants_enforced: true,
      secrets: { status: 'ok', items: [], truncated: false },
      gate: null,
      ask: { status: 'available' },
      run_now: { status: 'available' },
    },
  ],
  capabilities: [
    {
      key: 'grow',
      display_name: 'Grow',
      description: 'GTM',
      enabled: true,
      default_enabled: null,
      source: 'platform',
      required_secrets: { status: 'not_declared' },
    },
  ],
};

const EMPTY_WORK: WorkReview = {
  runs: { status: 'ok', items: [], truncated: false },
  briefings: { status: 'ok', items: [], truncated: false },
  activity: {
    attribution: 'actor_exact_match',
    status: 'ok',
    items: [],
    truncated: false,
  },
  goals: { status: 'ok', items: [], truncated: false },
  spend: { status: 'ok', items: [], truncated: false },
  scheduled_jobs: { status: 'ok', items: [], truncated: false },
};

/** Three passes that must read as three DIFFERENT things on screen. */
const RECORDED_RUNS: AgentRun[] = [
  {
    id: 'run-1', agent_id: 'strix', trigger: 'interval', outcome: 'ok',
    started_at: '2026-08-19T02:10:00Z', finished_at: '2026-08-19T02:10:31Z',
    examined: 7, produced: 'briefing brief-3', reason: null,
  },
  {
    // A pass that counted nothing and produced nothing, on purpose.
    id: 'run-2', agent_id: 'strix', trigger: 'interval', outcome: 'skipped',
    started_at: '2026-08-19T01:10:00Z', finished_at: '2026-08-19T01:10:01Z',
    examined: null, produced: null, reason: 'no project was due this pass',
  },
  {
    id: 'run-3', agent_id: 'strix', trigger: 'manual', outcome: 'failed',
    started_at: '2026-08-19T00:10:00Z', finished_at: '2026-08-19T00:10:04Z',
    examined: 2, produced: null, reason: 'the model provider refused the request',
  },
];

const MANUAL_RUN: AgentRun = {
  id: 'run-manual-1', agent_id: 'strix', trigger: 'manual', outcome: 'ok',
  started_at: '2026-08-19T09:00:00Z', finished_at: '2026-08-19T09:00:12Z',
  examined: 3, produced: 'briefing brief-9', reason: null,
};

const BRIEFINGS: BriefingItem[] = [
  {
    id: 'brief-3', from_agent: 'strix', kind: 'security', severity: 'action_required',
    summary: 'A live provider key is committed in goose-server',
    detail: 'crates/goose-server/.env line 4', ref_kind: 'file', ref_id: 'crates/goose-server/.env',
    created_at: '2026-08-19T02:10:30Z', acknowledged_at: null,
  },
  {
    id: 'brief-2', from_agent: 'strix', kind: 'security', severity: 'info',
    summary: 'Nothing new since the last sweep', detail: null,
    ref_kind: null, ref_id: null,
    created_at: '2026-08-18T02:10:30Z', acknowledged_at: '2026-08-18T08:00:00Z',
  },
];

function workWith(overrides: Partial<WorkReview>): WorkReview {
  return { ...EMPTY_WORK, ...overrides };
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  apiFetchMock.mockReset();
  upsertConfig.mockReset();
  upsertConfig.mockResolvedValue({});
  store.pendingAgentFocus = null;
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });
}

/** Detail for an id is the roster row it came from, tagged with its kind. */
function detailFor(id: string): unknown | null {
  const worker = (ROSTER.workers as unknown as { id: string }[]).find(w => w.id === id);
  if (worker) return { kind: 'worker', ...worker };
  const persona = (ROSTER.dispatch_roster as unknown as { key: string }[]).find(p => p.key === id);
  if (persona) return { kind: 'dispatch_persona', ...persona };
  return null;
}

function displayNameFor(id: string): string {
  const d = detailFor(id) as { display_name?: string } | null;
  return d?.display_name ?? id;
}

/**
 * The ask and run endpoints answer WITH the id they were called on, so a test
 * that opens one agent's page can prove the call went to THAT agent rather than
 * to whichever one happened to be first in the roster.
 *
 * `work` may be a getter, for the run-now path where the second read has to
 * return something the first did not.
 */
function mockRosterAndDetail(work: WorkReview | (() => WorkReview) = EMPTY_WORK) {
  apiFetchMock.mockImplementation(async (endpoint: string) => {
    if (endpoint === '/api/agents/roster') return ROSTER as never;
    const ask = endpoint.match(/^\/api\/agents\/([^/?]+)\/ask$/);
    if (ask) {
      return {
        answer: `${ask[1]} answered: two exposed keys in goose-server`,
        display_name: displayNameFor(ask[1]),
        persona_applied: true,
        // The real wire shape: a tagged union, not a string.
        tool_scope: { mode: 'explicit', granted: ['grow', 'memory'], applied: ['grow'] },
      } as never;
    }
    const run = endpoint.match(/^\/api\/agents\/([^/?]+)\/run$/);
    if (run) return { run: { ...MANUAL_RUN, agent_id: run[1] } } as never;
    const w = endpoint.match(/^\/api\/agents\/([^/?]+)\/work/);
    if (w) return (typeof work === 'function' ? work() : work) as never;
    const detail = endpoint.match(/^\/api\/agents\/([^/?]+)$/);
    if (detail) {
      const d = detailFor(detail[1]);
      if (d) return d as never;
    }
    throw new Error(`unexpected fetch ${endpoint}`);
  });
}

async function mount() {
  await act(async () => { root.render(<AgentsPanel goto={vi.fn()} />); });
  await flush();
}

async function open(testid: string) {
  const btn = container.querySelector(`[data-testid="${testid}"]`) as HTMLButtonElement;
  expect(btn).toBeTruthy();
  await act(async () => { btn.click(); });
  await flush();
}

/** Types into a controlled field the way React hears it. */
async function typeInto(el: HTMLTextAreaElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(
    HTMLTextAreaElement.prototype, 'value',
  )?.set as (v: string) => void;
  await act(async () => {
    setter.call(el, value);
    el.dispatchEvent(new Event('input', { bubbles: true }));
  });
  await flush();
}

function testid<T extends HTMLElement>(id: string): T | null {
  return container.querySelector(`[data-testid="${id}"]`) as T | null;
}

/** Endpoints hit, in order, for asserting WHICH agent a call went to. */
function calledEndpoints(): string[] {
  return apiFetchMock.mock.calls.map(c => String(c[0]));
}

/** The gate Toggle — atoms.Toggle is the only 36px-wide button on the page. */
function toggles(): HTMLButtonElement[] {
  return Array.from(container.querySelectorAll('button')).filter(
    b => (b as HTMLButtonElement).style.width === '36px',
  ) as HTMLButtonElement[];
}

function knob(t: HTMLButtonElement): string {
  return (t.firstElementChild as HTMLElement).style.transform;
}

describe('AgentsPanel', () => {
  it('renders workers with no dispatch / run now control', async () => {
    mockRosterAndDetail();
    await mount();

    const text = container.textContent ?? '';
    expect(text).toContain('Scheduler');
    expect(text).toMatch(/run themselves/i);
    // No dispatch / run-now control — section titles may say "Dispatch roster".
    // Run now exists, but on the agent's OWN page and only where the daemon
    // reports the capability — never as a row-level control on this list.
    const controls = [...container.querySelectorAll('button, a')].filter(el =>
      /^(dispatch|run now)$/i.test((el.textContent ?? '').trim()),
    );
    expect(controls).toHaveLength(0);
    const workerText = (container.querySelector('[data-testid="worker-row-scheduler"]')?.textContent ?? '').toLowerCase();
    expect(workerText).not.toMatch(/\brun now\b/);
  });

  // REGRESSION on the CHIP, not on the row. The hiding itself was daemon-side
  // (`agents_surface.rs` filtered `background_workers` on
  // `worker_descriptor_visible`), and it is pinned there by
  // `gated_worker_is_listed_while_its_flag_is_off`; this fixture supplies its own
  // roster, so the old component would have rendered `worker-row-strix` quite
  // happily. What fails against the old component is `gate-chip-strix`: there was
  // no way at all to see, from the list, that an agent was switched off.
  it('lists a gated worker with an off chip instead of hiding it', async () => {
    mockRosterAndDetail();
    await mount();

    expect(container.querySelector('[data-testid="worker-row-strix"]')).toBeTruthy();
    expect(container.querySelector('[data-testid="gate-chip-strix"]')?.textContent).toBe('off');
    // An ungated worker gets no chip: "no switch known" is not "switched off".
    expect(container.querySelector('[data-testid="gate-chip-scheduler"]')).toBeNull();
  });

  // REGRESSION. There was no enable control on an agent's own page at all — the
  // Guard's only toggle lived in the Models pane, two groups away, and the
  // product owner could not find it. `agent-gate` did not exist.
  it('shows a flag-gated agent its own switch, written through /config/upsert', async () => {
    mockRosterAndDetail();
    await mount();
    await open('persona-row-steward');

    const gate = container.querySelector('[data-testid="agent-gate"]');
    expect(gate).toBeTruthy();
    expect(gate?.textContent).toContain('steward_scan_enabled');
    const toggle = toggles()[0];
    expect(knob(toggle)).toBe('translateX(0)');

    const writesBefore = apiFetchMock.mock.calls.filter(c => (c[1] as RequestInit | undefined)?.method === 'POST');
    await act(async () => { toggle.click(); });
    await flush();

    expect(upsertConfig).toHaveBeenCalledTimes(1);
    expect(upsertConfig).toHaveBeenCalledWith('steward_scan_enabled', true);
    // The flag has NO agent-scoped write route; a second one would be a second
    // source of truth. Nothing was POSTed to /api/agents.
    const writesAfter = apiFetchMock.mock.calls.filter(c => (c[1] as RequestInit | undefined)?.method === 'POST');
    expect(writesAfter.length).toBe(writesBefore.length);
  });

  it('re-reads the agent after a flip, so its live state is not left stale', async () => {
    mockRosterAndDetail();
    await mount();
    await open('worker-row-strix');
    expect(container.textContent).toContain('off (strix_enabled=false)');

    // After the write lands the daemon answers with the flag on, and the page
    // must show THAT rather than the state it was opened with.
    apiFetchMock.mockImplementation(async (endpoint: string) => {
      if (endpoint === '/api/agents/roster') return ROSTER as never;
      if (endpoint.startsWith('/api/agents/strix/work')) return EMPTY_WORK as never;
      if (endpoint === '/api/agents/strix') {
        return {
          kind: 'worker',
          ...(ROSTER.workers[1] as unknown as Record<string, unknown>),
          live_state: { status: 'ok', value: 'on — sweeping every 24h' },
          gate: { config_key: 'strix_enabled', enabled: true },
        } as never;
      }
      throw new Error(`unexpected fetch ${endpoint}`);
    });

    const before = apiFetchMock.mock.calls.length;
    await act(async () => { toggles()[0].click(); });
    await flush();

    expect(upsertConfig).toHaveBeenCalledWith('strix_enabled', true);
    const after = apiFetchMock.mock.calls.slice(before).map(c => c[0]);
    expect(after).toContain('/api/agents/strix');
    expect(container.textContent).toContain('on — sweeping every 24h');
    expect(container.textContent).not.toContain('off (strix_enabled=false)');
  });

  // REGRESSION. The roster was fetched once, on mount, and nothing refetched it —
  // not the flip, not Back. So switching the Guard on from its own page and
  // returning to the list landed on a chip still reading "off", i.e. two of our
  // own surfaces disagreeing about a flag the user had just set, which is the
  // exact confusion the chip was added to end. Against the old wiring the final
  // expectation below reads 'off'.
  it('refreshes the list behind the page, so the chip is not stale on Back', async () => {
    let strixOn = false;
    apiFetchMock.mockImplementation(async (endpoint: string) => {
      if (endpoint === '/api/agents/roster') {
        return {
          ...ROSTER,
          workers: ROSTER.workers.map(w =>
            w.id === 'strix' ? { ...w, gate: { config_key: 'strix_enabled', enabled: strixOn } } : w,
          ),
        } as never;
      }
      if (endpoint.startsWith('/api/agents/strix/work')) return EMPTY_WORK as never;
      if (endpoint === '/api/agents/strix') {
        return {
          kind: 'worker',
          ...(ROSTER.workers[1] as unknown as Record<string, unknown>),
          gate: { config_key: 'strix_enabled', enabled: strixOn },
        } as never;
      }
      throw new Error(`unexpected fetch ${endpoint}`);
    });
    upsertConfig.mockImplementation(async (key: string, value: unknown) => {
      if (key === 'strix_enabled') strixOn = value === true;
      return {} as never;
    });

    await mount();
    expect(container.querySelector('[data-testid="gate-chip-strix"]')?.textContent).toBe('off');

    await open('worker-row-strix');
    await act(async () => { toggles()[0].click(); });
    await flush();

    const back = [...container.querySelectorAll('button')].find(
      b => (b.textContent ?? '').includes('Back to agents'),
    ) as HTMLButtonElement;
    expect(back).toBeTruthy();
    await act(async () => { back.click(); });
    await flush();

    expect(container.querySelector('[data-testid="gate-chip-strix"]')?.textContent).toBe('on');
  });

  // REGRESSION. The optimistic value used to be mirrored into state and re-seeded
  // by an effect keyed on `[gate.enabled]`. React skips an effect whose dep did
  // not change, so precisely when the re-read DISAGREES with the guess — the one
  // case the re-read exists for, and a real one: `Config::get_param` lets an env
  // var shadow the config file, so the upsert returns 200 and the flag stays off
  // — the toggle was left showing ON forever, with no error to explain it.
  it("shows the daemon's answer when the re-read disagrees with the flip", async () => {
    mockRosterAndDetail();
    await mount();
    await open('worker-row-strix');

    const toggle = toggles()[0];
    expect(knob(toggle)).toBe('translateX(0)');
    // The write succeeds; the daemon keeps answering "off" (an env var shadows
    // the key it just wrote).
    await act(async () => { toggle.click(); });
    await flush();

    expect(upsertConfig).toHaveBeenCalledWith('strix_enabled', true);
    expect(knob(toggles()[0])).toBe('translateX(0)');
  });

  it('reverts the switch and says why when the write fails', async () => {
    mockRosterAndDetail();
    upsertConfig.mockRejectedValue(new Error('daemon said no'));
    await mount();
    await open('worker-row-strix');

    const toggle = toggles()[0];
    expect(knob(toggle)).toBe('translateX(0)');
    await act(async () => { toggle.click(); });
    await flush();

    // A failed write is never shown as a success.
    expect(knob(toggles()[0])).toBe('translateX(0)');
    expect(container.textContent).toContain("Couldn't save: daemon said no");
  });

  it('offers no switch for an agent that has none', async () => {
    mockRosterAndDetail();
    await mount();
    await open('persona-row-claude_code');

    expect(container.querySelector('[data-testid="agent-gate"]')).toBeNull();
    expect(toggles()).toHaveLength(0);
  });

  // REGRESSION. The full chip / checkbox / Save-grants editor used to render for
  // every persona, merely greyed out, on engines where a saved grant enforces
  // nothing at all. `grants-editor` was present with aria-disabled="true"; now
  // it must not exist.
  it('offers no grants editor where the engine enforces nothing', async () => {
    mockRosterAndDetail();
    await mount();
    await open('persona-row-claude_code');

    expect(container.querySelector('[data-testid="grants-not-enforced"]')).toBeTruthy();
    expect(container.textContent).toMatch(/cannot restrict|not enforced/i);
    // The recorded grants are still shown, read-only — hiding the editor must
    // not hide what is on disk.
    expect(container.textContent).toContain('Inherits globally enabled capabilities');
    expect(container.querySelector('[data-testid="grants-editor"]')).toBeNull();
    const save = [...container.querySelectorAll('button')].find(b => /save grants/i.test(b.textContent ?? ''));
    expect(save).toBeUndefined();
  });

  it('keeps the grants editor where the engine does enforce them', async () => {
    mockRosterAndDetail();
    await mount();
    await open('persona-row-planner');

    expect(container.querySelector('[data-testid="grants-editor"]')).toBeTruthy();
    expect(container.querySelector('[data-testid="grants-not-enforced"]')).toBeNull();
    const save = [...container.querySelectorAll('button')]
      .find(b => /save grants/i.test(b.textContent ?? '')) as HTMLButtonElement;
    expect(save).toBeTruthy();

    apiFetchMock.mockImplementation(async (endpoint: string) => {
      if (endpoint === '/api/agents/planner') return detailFor('planner') as never;
      if (endpoint.startsWith('/api/agents/planner/work')) return EMPTY_WORK as never;
      if (endpoint === '/api/agents/roster') return ROSTER as never;
      if (endpoint === '/api/agents/planner/grants') {
        return (ROSTER.dispatch_roster[2] as unknown) as never;
      }
      throw new Error(`unexpected fetch ${endpoint}`);
    });
    await act(async () => { save.click(); });
    await flush();
    expect(apiFetchMock.mock.calls.map(c => c[0])).toContain('/api/agents/planner/grants');
  });

  // REGRESSION. The Secrets section used to offer a blank name/value pair with
  // no hint of what belonged in it — for an agent that declares no secrets, on a
  // runtime where nothing reads `agent_secret.*` at all. The password input and
  // the "Set secret" button below both existed.
  it('tells an agent with no stored secrets there is nothing to enter', async () => {
    mockRosterAndDetail();
    await mount();
    await open('persona-row-steward');

    const note = container.querySelector('[data-testid="no-agent-secrets"]');
    expect(note?.textContent).toContain('nothing in the runtime reads a per-agent secret');
    expect(container.querySelector('input[type="password"]')).toBeNull();
    const set = [...container.querySelectorAll('button')].find(b => /set secret/i.test(b.textContent ?? ''));
    expect(set).toBeUndefined();
  });

  it('still lists and still removes an already-stored secret', async () => {
    mockRosterAndDetail();
    await mount();
    await open('persona-row-claude_code');

    const row = container.querySelector('[data-testid="secret-row-api_token"]');
    expect(row?.textContent).toContain('present');
    expect(container.textContent).toContain('values never are');
    // No add form, but nothing here deletes a stored secret on its own either.
    expect(container.querySelector('input[type="password"]')).toBeNull();

    const removeBtn = [...(row?.querySelectorAll('button') ?? [])]
      .find(b => /remove/i.test(b.textContent ?? '')) as HTMLButtonElement;
    expect(removeBtn).toBeTruthy();

    const posted: unknown[] = [];
    apiFetchMock.mockImplementation(async (endpoint: string, options?: RequestInit) => {
      if (endpoint === '/api/agents/claude_code/secrets') {
        posted.push(JSON.parse(String(options?.body)));
        return { name: 'api_token', presence: 'absent' } as never;
      }
      if (endpoint === '/api/agents/claude_code') return detailFor('claude_code') as never;
      if (endpoint.startsWith('/api/agents/claude_code/work')) return EMPTY_WORK as never;
      if (endpoint === '/api/agents/roster') return ROSTER as never;
      throw new Error(`unexpected fetch ${endpoint}`);
    });
    await act(async () => { removeBtn.click(); });
    await flush();

    expect(posted).toEqual([{ name: 'api_token', value: null }]);
    // Presence is re-read from the API rather than assumed.
    expect(apiFetchMock.mock.calls.map(c => c[0])).toContain('/api/agents/claude_code');
  });

  it('renders an unreadable secret store as a failure, not as an absence', async () => {
    const broken = {
      ...(ROSTER.dispatch_roster[0] as unknown as Record<string, unknown>),
      secrets: { status: 'unavailable', reason: 'keychain locked' },
    };
    apiFetchMock.mockImplementation(async (endpoint: string) => {
      if (endpoint === '/api/agents/roster') {
        return { ...ROSTER, dispatch_roster: [broken] } as never;
      }
      if (endpoint === '/api/agents/claude_code') return { kind: 'dispatch_persona', ...broken } as never;
      if (endpoint.startsWith('/api/agents/claude_code/work')) return EMPTY_WORK as never;
      throw new Error(`unexpected fetch ${endpoint}`);
    });
    await mount();
    await open('persona-row-claude_code');

    expect(container.textContent).toContain('keychain locked');
    expect(container.querySelector('[data-testid="no-agent-secrets"]')).toBeNull();
  });

  it('never renders a secret value or reveal control', async () => {
    // Even if the daemon were to echo a value on the presence path, it must not
    // reach the DOM. There is no write field to type one into any more, so this
    // seeds the value from the wire instead.
    const leaky = {
      ...(ROSTER.dispatch_roster[0] as unknown as Record<string, unknown>),
      secrets: {
        status: 'ok',
        items: [{ name: 'api_token', presence: 'present', value: SEED_SECRET_VALUE }],
        truncated: false,
      },
    };
    apiFetchMock.mockImplementation(async (endpoint: string) => {
      if (endpoint === '/api/agents/roster') return { ...ROSTER, dispatch_roster: [leaky] } as never;
      if (endpoint === '/api/agents/claude_code') return { kind: 'dispatch_persona', ...leaky } as never;
      if (endpoint.startsWith('/api/agents/claude_code/work')) return EMPTY_WORK as never;
      throw new Error(`unexpected fetch ${endpoint}`);
    });
    await mount();
    await open('persona-row-claude_code');

    expect(container.textContent ?? '').toContain('present');
    expect(container.textContent ?? '').not.toMatch(/reveal/i);
    expect(container.innerHTML).not.toContain(SEED_SECRET_VALUE);
  });

  // The portrait is derived, not fetched: there are no per-character assets on
  // disk, so the trim has to be the world's own AGENT_TRIM or the same agent
  // would wear two faces. The expected colour is read from the palette here,
  // never written as a literal.
  it('draws the agent portrait in the roster row and again in the profile header', async () => {
    mockRosterAndDetail();
    await mount();

    const listed = container.querySelector('[data-testid="agent-portrait-strix"]');
    expect(listed).toBeTruthy();
    expect(listed?.getAttribute('data-variant')).toBe('strix');
    expect(listed?.getAttribute('data-trim-color')).toBe(AGENT_TRIM.strix);

    await open('worker-row-strix');
    const header = container.querySelector('[data-testid="agent-portrait-strix"]');
    expect(header).toBeTruthy();
    expect(header?.getAttribute('data-variant')).toBe('strix');
    expect(header?.getAttribute('data-trim-color')).toBe(AGENT_TRIM.strix);
  });

  it('gives an agent with no in-world character a portrait rather than a gap', async () => {
    mockRosterAndDetail();
    await mount();

    const blank = container.querySelector('[data-testid="agent-portrait-claude_code"]');
    expect(blank).toBeTruthy();
    expect(blank?.getAttribute('data-variant')).toBe('unknown');
    // No character means no identity colour to borrow — never another agent's.
    expect(blank?.getAttribute('data-trim-color')).toBe('');
  });

  // REGRESSION. The note used to explain an unknown id by saying a flag-gated
  // worker "is absent from the roster while its flag is off". That was true of
  // the old filter and is now false, and a false explanation is worse than none.
  it('no longer blames a flag for an agent the roster did not return', async () => {
    mockRosterAndDetail();
    store.pendingAgentFocus = 'nonesuch';
    await mount();

    const note = container.querySelector('[data-testid="unknown-agent"]');
    expect(note?.textContent).toContain('nonesuch');
    expect(note?.textContent ?? '').not.toMatch(/absent from the roster while its flag is off/);
    expect(note?.textContent ?? '').toMatch(/listed here even while their flag is off/);
  });

  it('surfaces the impact and unlocks hint a platform capability ships with its secrets', async () => {
    const capability = {
      ...ROSTER.capabilities[0],
      required_secrets: {
        status: 'declared' as const,
        truncated: false,
        items: [
          { name: 'GROW_KEY', present: false, impact: 'unavailable', unlocks: 'Runs the GTM sweep.' },
          { name: 'PLAIN_KEY', present: true },
        ],
      },
    };
    apiFetchMock.mockImplementation(async (endpoint: string) => {
      if (endpoint === '/api/agents/roster') {
        return { ...ROSTER, capabilities: [capability] } as never;
      }
      throw new Error(`unexpected fetch ${endpoint}`);
    });
    await mount();

    const row = container.querySelector('[data-testid="capability-row-grow"]');
    expect(row?.textContent ?? '').toContain('GROW_KEY: absent');
    const hints = [...(row?.querySelectorAll('[data-testid="required-secret-hint"]') ?? [])].map(
      el => el.textContent,
    );
    expect(hints).toEqual(['GROW_KEY: Runs the GTM sweep. — unavailable without it']);
  });

  it('does not claim a pending-engine persona runs a CLI it cannot restrict', async () => {
    mockRosterAndDetail();
    await mount();
    await open('persona-row-steward');

    const note = container.querySelector('[data-testid="grants-not-enforced"]');
    expect(note?.textContent ?? '').toMatch(/no runnable engine/i);
    expect((note?.textContent ?? '').toLowerCase()).not.toContain('cli process');
  });

  it('does not render unavailable live_state as idle', async () => {
    mockRosterAndDetail();
    await mount();

    const row = container.querySelector('[data-testid="worker-row-scheduler"]');
    expect(row?.textContent ?? '').toMatch(/could not be read/i);
    expect(row?.textContent ?? '').toContain('pool locked');
    expect((row?.textContent ?? '').toLowerCase()).not.toMatch(/\bidle\b/);
  });

  it('renders attribution wording for empty activity', async () => {
    mockRosterAndDetail();
    await mount();
    await open('worker-row-scheduler');

    const empty = container.querySelector('[data-testid="empty-activity"]');
    expect(empty?.textContent ?? '').toMatch(/attributed/i);
    expect((empty?.textContent ?? '').toLowerCase()).toContain('not proof the agent did nothing');
    expect((empty?.textContent ?? '').toLowerCase()).not.toContain('no work yet');
    expect((empty?.textContent ?? '').toLowerCase()).not.toMatch(/\bthis agent did nothing\b/);
  });

  // ── Liveness: ask it, see what it did, run it now ──────────────────────────
  // The problem these pin, in the product owner's words: "most of the agents
  // have no live query ability so I have no idea if they actually do the tasks
  // they are meant to do."

  it('asks the agent whose page is open, and says who answered', async () => {
    mockRosterAndDetail();
    await mount();
    await open('worker-row-strix');

    const box = testid<HTMLTextAreaElement>('ask-question')!;
    expect(box.disabled).toBe(false);
    // An empty question — and a whitespace one — must not send.
    expect(testid<HTMLButtonElement>('ask-send')!.disabled).toBe(true);
    await typeInto(box, '   ');
    expect(testid<HTMLButtonElement>('ask-send')!.disabled).toBe(true);

    await typeInto(box, 'did you sweep goose-server last night?');
    const send = testid<HTMLButtonElement>('ask-send')!;
    expect(send.disabled).toBe(false);
    await act(async () => { send.click(); });
    await flush();

    // The RIGHT agent: strix's page asks strix, not whoever is first.
    const askCalls = apiFetchMock.mock.calls.filter(c => String(c[0]).endsWith('/ask'));
    expect(askCalls.map(c => String(c[0]))).toEqual(['/api/agents/strix/ask']);
    expect(JSON.parse(String((askCalls[0][1] as RequestInit).body))).toEqual({
      question: 'did you sweep goose-server last night?',
    });

    expect(testid('ask-answer')?.textContent).toContain('strix answered');
    // Which agent answered, and under what tool scope — the difference between
    // this and a chat box that answers as nobody in particular.
    const attribution = testid('ask-answer-attribution')?.textContent ?? '';
    expect(attribution).toContain('The Guard');
    // The tool scope is reported as what ACTUALLY applied, and a declared grant
    // that narrowed to nothing is named rather than quietly dropped.
    expect(attribution).toContain('narrowed to its own grants: grow');
    expect(attribution).toContain('declared but not available: memory');
    expect(attribution).toContain('its own persona was applied');
  });

  // A turn that ran on the globally enabled set must not be described as having
  // carried the agent's own tools — that is the same set anything else here gets.
  it('does not call the global tool set the agent\'s own', async () => {
    mockRosterAndDetail();
    apiFetchMock.mockImplementation(async (endpoint: string) => {
      if (endpoint === '/api/agents/roster') return ROSTER as never;
      if (endpoint === '/api/agents/strix') return detailFor('strix') as never;
      if (endpoint.startsWith('/api/agents/strix/work')) return EMPTY_WORK as never;
      if (endpoint === '/api/agents/strix/ask') {
        return {
          answer: 'nothing new since the last sweep',
          display_name: 'The Guard',
          persona_applied: false,
          tool_scope: { mode: 'inherit_global', extensions: ['grow'] },
        } as never;
      }
      throw new Error(`unexpected fetch ${endpoint}`);
    });
    await mount();
    await open('worker-row-strix');
    await typeInto(testid<HTMLTextAreaElement>('ask-question')!, 'anything new?');
    await act(async () => { testid<HTMLButtonElement>('ask-send')!.click(); });
    await flush();

    const attribution = testid('ask-answer-attribution')?.textContent ?? '';
    expect(attribution).toContain('the globally enabled tools (grow)');
    expect(attribution).toContain('not a set of its own');
    // persona_applied: false is stated, never quietly implied to be true.
    expect(attribution).toContain('answered WITHOUT its persona');
  });

  it('renders the ask box disabled, with the daemon reason, when asking is unavailable', async () => {
    mockRosterAndDetail();
    await mount();
    await open('worker-row-scheduler');

    // Disabled and VISIBLE. Hiding it would leave the user unable to tell that
    // asking is a thing this surface does at all.
    const note = testid('ask-unavailable');
    expect(note?.textContent).toContain('exposes no conversational surface');
    const box = testid<HTMLTextAreaElement>('ask-question');
    expect(box).toBeTruthy();
    expect(box!.disabled).toBe(true);

    const send = testid<HTMLButtonElement>('ask-send')!;
    expect(send.disabled).toBe(true);
    await act(async () => { send.click(); });
    await flush();
    expect(calledEndpoints().filter(e => e.endsWith('/ask'))).toHaveLength(0);
  });

  it('renders recorded runs as real rows, with the fields the daemon sent', async () => {
    mockRosterAndDetail(workWith({
      runs: { status: 'ok', items: RECORDED_RUNS, truncated: false },
    }));
    await mount();
    await open('worker-row-strix');

    const ok = testid('run-row-run-1')?.textContent ?? '';
    expect(ok).toContain('2026-08-19T02:10:00Z');
    expect(ok).toContain('interval');
    expect(ok).toContain('ok');
    expect(ok).toContain('examined 7');
    expect(ok).toContain('produced briefing brief-3');

    const failed = testid('run-row-run-3')?.textContent ?? '';
    expect(failed).toContain('manual');
    expect(failed).toContain('the model provider refused the request');

    // A populated list is neither of the two "nothing here" states.
    expect(testid('empty-runs')).toBeNull();
    expect(testid('runs-not-recorded')).toBeNull();
  });

  it('says no runs are recorded YET when the list is merely empty', async () => {
    mockRosterAndDetail(workWith({ runs: { status: 'ok', items: [], truncated: false } }));
    await mount();
    await open('worker-row-strix');

    const empty = testid('empty-runs');
    expect(empty?.textContent ?? '').toMatch(/no runs recorded yet/i);
    expect(empty?.textContent ?? '').toMatch(/after its next/i);
    expect(testid('runs-not-recorded')).toBeNull();
  });

  // THE distinction this task turns on: an agent whose code writes no run record
  // must not read as an agent that has run zero times.
  it('keeps "records no runs" distinct from "no runs yet" and from a read failure', async () => {
    mockRosterAndDetail(workWith({
      runs: { status: 'not_recorded', reason: 'this worker writes no run record' },
    }));
    await mount();
    await open('worker-row-strix');

    const note = testid<HTMLElement>('runs-not-recorded');
    expect(note?.textContent).toContain('this worker writes no run record');
    // Not the empty state — a different element entirely, not the same testid
    // with different words.
    expect(testid('empty-runs')).toBeNull();
    expect(note?.textContent ?? '').not.toMatch(/no runs recorded yet/i);
    // And not an error either: it is calm, not the danger colour.
    const probe = document.createElement('span');
    probe.style.color = getThemedColors().danger;
    expect(note!.style.color).not.toBe(probe.style.color);
    expect(container.textContent ?? '').not.toMatch(/runs could not be read/i);
  });

  it('does not colour a skipped run as a failure', async () => {
    mockRosterAndDetail(workWith({
      runs: { status: 'ok', items: RECORDED_RUNS, truncated: false },
    }));
    await mount();
    await open('worker-row-strix');

    const skipped = testid<HTMLElement>('run-outcome-run-2')!;
    const failed = testid<HTMLElement>('run-outcome-run-3')!;
    const probe = document.createElement('span');
    probe.style.color = getThemedColors().danger;

    expect(failed.getAttribute('data-tone')).toBe('error');
    expect(failed.style.color).toBe(probe.style.color);
    // Skipped is the agent working as designed — nothing was due.
    expect(skipped.getAttribute('data-tone')).not.toBe('error');
    expect(skipped.style.color).not.toBe(probe.style.color);
    expect(skipped.style.fontWeight).not.toBe('600');
  });

  it('renders examined: null as nothing at all, never as 0', async () => {
    mockRosterAndDetail(workWith({
      runs: { status: 'ok', items: RECORDED_RUNS, truncated: false },
    }));
    await mount();
    await open('worker-row-strix');

    const skippedRow = testid('run-row-run-2')?.textContent ?? '';
    expect(skippedRow).toContain('no project was due this pass');
    // "this pass does not count" is not "it examined nothing".
    expect(skippedRow).not.toMatch(/examined/i);
    expect(skippedRow).not.toMatch(/produced/i);
    // The pass that DOES count still shows its count.
    expect(testid('run-row-run-1')?.textContent ?? '').toContain('examined 7');
  });

  it('disables Run now with the daemon reason where a pass cannot be run', async () => {
    mockRosterAndDetail();
    await mount();
    await open('persona-row-claude_code');

    const btn = testid<HTMLButtonElement>('run-now')!;
    expect(btn.disabled).toBe(true);
    expect(testid('run-now-unavailable')?.textContent).toContain('no single pass to run');
    await act(async () => { btn.click(); });
    await flush();
    expect(calledEndpoints().filter(e => e.endsWith('/run'))).toHaveLength(0);
  });

  it('runs the open agent now, shows the run it recorded, and re-reads the list', async () => {
    let ran = false;
    mockRosterAndDetail(() => workWith({
      runs: { status: 'ok', items: ran ? [MANUAL_RUN] : [], truncated: false },
    }));
    await mount();
    await open('worker-row-strix');
    expect(testid('empty-runs')).toBeTruthy();

    const workReadsBefore = calledEndpoints().filter(e => e.startsWith('/api/agents/strix/work')).length;
    ran = true;
    await act(async () => { testid<HTMLButtonElement>('run-now')!.click(); });
    await flush();

    const runCalls = apiFetchMock.mock.calls.filter(c => String(c[0]).endsWith('/run'));
    expect(runCalls.map(c => String(c[0]))).toEqual(['/api/agents/strix/run']);
    expect((runCalls[0][1] as RequestInit).method).toBe('POST');

    // The returned run, shown immediately…
    const result = testid('run-now-result')?.textContent ?? '';
    expect(result).toContain('manual');
    expect(result).toContain('examined 3');
    expect(result).toContain('2026-08-19T09:00:00Z');
    // …and the list behind it re-read, so the new pass is in it rather than
    // leaving the page insisting nothing has ever run.
    const workReadsAfter = calledEndpoints().filter(e => e.startsWith('/api/agents/strix/work')).length;
    expect(workReadsAfter).toBeGreaterThan(workReadsBefore);
    expect(testid('empty-runs')).toBeNull();
    expect(container.querySelectorAll('[data-testid="run-row-run-manual-1"]').length).toBeGreaterThan(1);
  });

  it('says why a run failed rather than showing a run that did not happen', async () => {
    mockRosterAndDetail(workWith({ runs: { status: 'ok', items: [], truncated: false } }));
    await mount();
    await open('worker-row-strix');

    apiFetchMock.mockImplementation(async (endpoint: string) => {
      if (endpoint === '/api/agents/strix/run') throw new Error('the sweep lock is held');
      if (endpoint === '/api/agents/roster') return ROSTER as never;
      if (endpoint.startsWith('/api/agents/strix/work')) return EMPTY_WORK as never;
      if (endpoint === '/api/agents/strix') return detailFor('strix') as never;
      throw new Error(`unexpected fetch ${endpoint}`);
    });
    await act(async () => { testid<HTMLButtonElement>('run-now')!.click(); });
    await flush();

    expect(testid('run-now-error')?.textContent).toContain('the sweep lock is held');
    expect(testid('run-now-result')).toBeNull();
  });

  // REGRESSION GUARD on the older-daemon path. A field the daemon never
  // serialises reads as `undefined`, and a control rendered from `undefined`
  // would be an ENABLED button that fails the moment it is pressed.
  it('reads a daemon that reports neither capability as unavailable, not as available', async () => {
    const old = { ...(ROSTER.workers[1] as unknown as Record<string, unknown>) };
    delete old.ask;
    delete old.run_now;
    apiFetchMock.mockImplementation(async (endpoint: string) => {
      if (endpoint === '/api/agents/roster') return { ...ROSTER, workers: [old] } as never;
      if (endpoint === '/api/agents/strix') return { kind: 'worker', ...old } as never;
      if (endpoint.startsWith('/api/agents/strix/work')) return EMPTY_WORK as never;
      throw new Error(`unexpected fetch ${endpoint}`);
    });
    await mount();
    await open('worker-row-strix');

    expect(testid<HTMLTextAreaElement>('ask-question')!.disabled).toBe(true);
    expect(testid<HTMLButtonElement>('ask-send')!.disabled).toBe(true);
    expect(testid('ask-unavailable')?.textContent).toContain(CAPABILITY_NOT_REPORTED);
    expect(testid<HTMLButtonElement>('run-now')!.disabled).toBe(true);
    expect(testid('run-now-unavailable')?.textContent).toContain(CAPABILITY_NOT_REPORTED);
  });

  it('renders briefings with severity, summary and whether they were acknowledged', async () => {
    mockRosterAndDetail(workWith({
      briefings: { status: 'ok', items: BRIEFINGS, truncated: false },
    }));
    await mount();
    await open('worker-row-strix');

    const urgent = testid('briefing-row-brief-3')?.textContent ?? '';
    expect(urgent).toContain('action_required');
    expect(urgent).toContain('A live provider key is committed in goose-server');
    expect(urgent).toContain('2026-08-19T02:10:30Z');
    expect(urgent).toContain('not acknowledged');

    const quiet = testid('briefing-row-brief-2')?.textContent ?? '';
    expect(quiet).toContain('info');
    expect(quiet).toContain('acknowledged 2026-08-18T08:00:00Z');
    expect(testid('empty-briefings')).toBeNull();
  });

  it('keeps an unreadable briefing list distinct from an empty one', async () => {
    mockRosterAndDetail(workWith({
      briefings: { status: 'unavailable', reason: 'the briefing table is locked' },
    }));
    await mount();
    await open('worker-row-strix');

    expect(container.textContent).toContain('the briefing table is locked');
    expect(testid('empty-briefings')).toBeNull();
  });
});
