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
        focusWorldAgent: vi.fn(),
      }),
    {
      getState: () => ({
        pendingAgentFocus: null,
        clearPendingAgentFocus: vi.fn(),
        focusWorldAgent: vi.fn(),
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
      valueInput.value = SEED_SECRET_VALUE;
      valueInput.dispatchEvent(new Event('input', { bubbles: true }));
    });

    const html = container.innerHTML;
    // Presence is shown; the seeded write value must not appear as revealed text
    // outside the password input, and there is no reveal control.
    expect(container.textContent ?? '').toContain('present');
    expect(container.textContent ?? '').not.toMatch(/reveal/i);
    expect(html).not.toContain(`>${SEED_SECRET_VALUE}<`);
    // After a successful path the component clears the field; here we only
    // assert the roster-sourced presence never carried a value string.
    expect(container.textContent ?? '').not.toContain(SEED_SECRET_VALUE);
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
