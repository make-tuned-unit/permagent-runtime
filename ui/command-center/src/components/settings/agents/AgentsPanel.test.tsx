/**
 * @vitest-environment jsdom
 *
 * Settings → Agents honesty pins: workers never offer dispatch, unenforced
 * grants stay disabled with a note, secret values never enter the DOM, and
 * unavailable / empty attribution stay distinct from idle.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const SEED_SECRET_VALUE = 'super-secret-value-NEVER-RENDER-me-9f3a';

vi.mock('../../../lib/api', () => ({
  apiFetch: vi.fn(),
  getApiBaseUrl: vi.fn(() => 'http://localhost:1234'),
}));

vi.mock('../../../lib/store', () => ({
  useCommandCenter: Object.assign(
    (selector: (s: Record<string, unknown>) => unknown) =>
      selector({
        pendingAgentFocus: null,
        clearPendingAgentFocus: vi.fn(),
        focusWorldAgent: vi.fn(() => true),
      }),
    {
      getState: () => ({
        pendingAgentFocus: null,
        clearPendingAgentFocus: vi.fn(),
        focusWorldAgent: vi.fn(() => true),
        openAgentSettings: vi.fn(),
      }),
    },
  ),
}));

import { AgentsPanel } from './AgentsPanel';
import { apiFetch } from '../../../lib/api';
import type { RosterResponse, WorkReview } from '../../../lib/agentsApi';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const apiFetchMock = vi.mocked(apiFetch);

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
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

/**
 * React tracks the DOM value setter, so assigning `input.value` directly is
 * swallowed and onChange never fires. Go through the native setter.
 */
function typeInto(input: HTMLInputElement, text: string) {
  const setter = Object.getOwnPropertyDescriptor(
    window.HTMLInputElement.prototype,
    'value',
  )?.set;
  setter?.call(input, text);
  input.dispatchEvent(new Event('input', { bubbles: true }));
}

async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });
}

function mockRosterAndDetail() {
  apiFetchMock.mockImplementation(async (endpoint: string) => {
    if (endpoint === '/api/agents/roster') return ROSTER as never;
    if (endpoint.startsWith('/api/agents/scheduler/work')) return EMPTY_WORK as never;
    if (endpoint.startsWith('/api/agents/claude_code/work')) return EMPTY_WORK as never;
    if (endpoint === '/api/agents/scheduler') {
      return { kind: 'worker', ...ROSTER.workers[0] } as never;
    }
    if (endpoint === '/api/agents/claude_code') {
      return { kind: 'dispatch_persona', ...ROSTER.dispatch_roster[0] } as never;
    }
    throw new Error(`unexpected fetch ${endpoint}`);
  });
}

describe('AgentsPanel', () => {
  it('renders workers with no dispatch / run now control', async () => {
    mockRosterAndDetail();
    await act(async () => {
      root.render(<AgentsPanel goto={vi.fn()} />);
    });
    await flush();

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

  it('shows not-enforced note and a disabled grants editor for external CLI', async () => {
    mockRosterAndDetail();
    await act(async () => {
      root.render(<AgentsPanel goto={vi.fn()} />);
    });
    await flush();

    const personaBtn = container.querySelector('[data-testid="persona-row-claude_code"]') as HTMLButtonElement;
    expect(personaBtn).toBeTruthy();
    await act(async () => { personaBtn.click(); });
    await flush();

    expect(container.textContent ?? '').toMatch(/cannot restrict|not enforced/i);
    const editor = container.querySelector('[data-testid="grants-editor"]');
    expect(editor).toBeTruthy();
    expect(editor?.getAttribute('aria-disabled')).toBe('true');
  });

  it('never renders a secret value or reveal control', async () => {
    mockRosterAndDetail();
    await act(async () => {
      root.render(<AgentsPanel goto={vi.fn()} />);
    });
    await flush();

    const personaBtn = container.querySelector('[data-testid="persona-row-claude_code"]') as HTMLButtonElement;
    await act(async () => { personaBtn.click(); });
    await flush();

    // Seed a distinctive value into the write field, then assert the *API*
    // presence path never echoed it — and no reveal control exists.
    const valueInput = container.querySelector('input[aria-label="Secret value"]') as HTMLInputElement;
    expect(valueInput).toBeTruthy();
    expect(valueInput.type).toBe('password');
    await act(async () => {
      typeInto(valueInput, SEED_SECRET_VALUE);
    });

    const html = container.innerHTML;
    // Presence is shown; the seeded write value must not appear as revealed text
    // outside the password input, and there is no reveal control.
    expect(container.textContent ?? '').toContain('present');
    expect(container.textContent ?? '').not.toMatch(/reveal/i);
    expect(html).not.toContain(`>${SEED_SECRET_VALUE}<`);
    expect(container.textContent ?? '').not.toContain(SEED_SECRET_VALUE);
  });

  it('clears the value field after a successful write and re-reads presence', async () => {
    mockRosterAndDetail();
    await act(async () => {
      root.render(<AgentsPanel goto={vi.fn()} />);
    });
    await flush();
    await act(async () => {
      (container.querySelector('[data-testid="persona-row-claude_code"]') as HTMLButtonElement).click();
    });
    await flush();

    const nameInput = container.querySelector('input[placeholder="Secret name"]') as HTMLInputElement;
    const valueInput = container.querySelector('input[aria-label="Secret value"]') as HTMLInputElement;
    await act(async () => {
      typeInto(nameInput, 'api_token');
      typeInto(valueInput, SEED_SECRET_VALUE);
    });
    expect(valueInput.value).toBe(SEED_SECRET_VALUE);

    const before = apiFetchMock.mock.calls.length;
    const setBtn = [...container.querySelectorAll('button')]
      .find(b => (b.textContent ?? '').trim() === 'Set secret') as HTMLButtonElement;
    apiFetchMock.mockImplementation(async (endpoint: string, options?: RequestInit) => {
      if (endpoint === '/api/agents/claude_code/secrets') {
        // The write must never come back carrying the value.
        return { name: 'api_token', presence: 'present' } as never;
      }
      if (endpoint === '/api/agents/claude_code') {
        return { kind: 'dispatch_persona', ...ROSTER.dispatch_roster[0] } as never;
      }
      if (endpoint.startsWith('/api/agents/claude_code/work')) return EMPTY_WORK as never;
      if (endpoint === '/api/agents/roster') return ROSTER as never;
      void options;
      throw new Error(`unexpected fetch ${endpoint}`);
    });
    await act(async () => { setBtn.click(); });
    await flush();

    // Presence is re-read from the API rather than assumed.
    const after = apiFetchMock.mock.calls.slice(before).map(c => c[0]);
    expect(after).toContain('/api/agents/claude_code/secrets');
    expect(after).toContain('/api/agents/claude_code');

    const clearedValue = container.querySelector('input[aria-label="Secret value"]') as HTMLInputElement;
    expect(clearedValue.value).toBe('');
    expect(container.innerHTML).not.toContain(SEED_SECRET_VALUE);
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
    await act(async () => {
      root.render(<AgentsPanel goto={vi.fn()} />);
    });
    await flush();

    const row = container.querySelector('[data-testid="capability-row-grow"]');
    expect(row?.textContent ?? '').toContain('GROW_KEY: absent');
    const hints = [...(row?.querySelectorAll('[data-testid="required-secret-hint"]') ?? [])].map(
      el => el.textContent,
    );
    expect(hints).toEqual(['GROW_KEY: Runs the GTM sweep. — unavailable without it']);
  });

  it('does not claim a pending-engine persona runs a CLI it cannot restrict', async () => {
    const pendingPersona = {
      ...ROSTER.dispatch_roster[0],
      key: 'unwired',
      display_name: 'Unwired',
      engine: 'pending' as const,
    };
    apiFetchMock.mockImplementation(async (endpoint: string) => {
      if (endpoint === '/api/agents/roster') {
        return { ...ROSTER, dispatch_roster: [pendingPersona] } as never;
      }
      if (endpoint === '/api/agents/unwired') {
        return { kind: 'dispatch_persona', ...pendingPersona } as never;
      }
      if (endpoint.startsWith('/api/agents/unwired/work')) return EMPTY_WORK as never;
      throw new Error(`unexpected fetch ${endpoint}`);
    });
    await act(async () => {
      root.render(<AgentsPanel goto={vi.fn()} />);
    });
    await flush();
    await act(async () => {
      (container.querySelector('[data-testid="persona-row-unwired"]') as HTMLButtonElement).click();
    });
    await flush();

    const note = container.querySelector('[data-testid="grants-not-enforced"]');
    expect(note?.textContent ?? '').toMatch(/no runnable engine/i);
    expect((note?.textContent ?? '').toLowerCase()).not.toContain('cli process');
  });

  it('does not render unavailable live_state as idle', async () => {
    mockRosterAndDetail();
    await act(async () => {
      root.render(<AgentsPanel goto={vi.fn()} />);
    });
    await flush();

    const row = container.querySelector('[data-testid="worker-row-scheduler"]');
    expect(row?.textContent ?? '').toMatch(/could not be read/i);
    expect(row?.textContent ?? '').toContain('pool locked');
    expect((row?.textContent ?? '').toLowerCase()).not.toMatch(/\bidle\b/);
  });

  it('renders attribution wording for empty activity', async () => {
    mockRosterAndDetail();
    await act(async () => {
      root.render(<AgentsPanel goto={vi.fn()} />);
    });
    await flush();

    const workerBtn = container.querySelector('[data-testid="worker-row-scheduler"]') as HTMLButtonElement;
    await act(async () => { workerBtn.click(); });
    await flush();

    const empty = container.querySelector('[data-testid="empty-activity"]');
    expect(empty?.textContent ?? '').toMatch(/attributed/i);
    expect((empty?.textContent ?? '').toLowerCase()).toContain('not proof the agent did nothing');
    expect((empty?.textContent ?? '').toLowerCase()).not.toContain('no work yet');
    expect((empty?.textContent ?? '').toLowerCase()).not.toMatch(/\bthis agent did nothing\b/);
  });
});
