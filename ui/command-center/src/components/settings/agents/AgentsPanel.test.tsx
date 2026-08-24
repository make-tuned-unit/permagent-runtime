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
  api: { readConfig: vi.fn(), upsertConfig: vi.fn(), setExtensionEnabled: vi.fn() },
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
import type { RosterResponse, WorkReview } from '../../../lib/agentsApi';

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

function mockRosterAndDetail() {
  apiFetchMock.mockImplementation(async (endpoint: string) => {
    if (endpoint === '/api/agents/roster') return ROSTER as never;
    const work = endpoint.match(/^\/api\/agents\/([^/?]+)\/work/);
    if (work) return EMPTY_WORK as never;
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

  /**
   * REGRESSION. A capability row used to offer only a "Manage in Tools" link,
   * and the Tools pane lists extensions read-only — so nothing in the whole app
   * could switch on a capability shipping `default_enabled: false`. The
   * Financier was exactly that: declared on the roster, described, and
   * unreachable. The row must now WRITE, through the one route that owns the
   * `extensions.<key>.enabled` bit.
   */
  it('switches a capability on through the shared extension route', async () => {
    const capability = { ...ROSTER.capabilities[0], enabled: false };
    apiFetchMock.mockImplementation(async (endpoint: string) => {
      if (endpoint === '/api/agents/roster') {
        return { ...ROSTER, capabilities: [capability] } as never;
      }
      throw new Error(`unexpected fetch ${endpoint}`);
    });
    vi.mocked(api.setExtensionEnabled).mockResolvedValue('Enabled extension grow' as never);
    await mount();

    const row = container.querySelector('[data-testid="capability-row-grow"]') as HTMLElement;
    expect(row.textContent).toContain('disabled');
    const toggle = row.querySelector('button') as HTMLButtonElement;
    await act(async () => { toggle.click(); });
    // The capability KEY is what the route is addressed by — the same key the
    // daemon derived the row from, never a display name.
    expect(api.setExtensionEnabled).toHaveBeenCalledWith('grow', true);
  });

  it('never reports a failed capability write as a success', async () => {
    const capability = { ...ROSTER.capabilities[0], enabled: false };
    apiFetchMock.mockImplementation(async (endpoint: string) => {
      if (endpoint === '/api/agents/roster') {
        return { ...ROSTER, capabilities: [capability] } as never;
      }
      throw new Error(`unexpected fetch ${endpoint}`);
    });
    vi.mocked(api.setExtensionEnabled).mockRejectedValue(new Error('config is read-only'));
    await mount();

    const row = container.querySelector('[data-testid="capability-row-grow"]') as HTMLElement;
    const toggle = row.querySelector('button') as HTMLButtonElement;
    await act(async () => { toggle.click(); });
    expect(row.textContent).toContain('config is read-only');
    // The optimistic guess is dropped, so the row shows what the daemon says.
    expect(row.textContent).toContain('disabled');
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
});
