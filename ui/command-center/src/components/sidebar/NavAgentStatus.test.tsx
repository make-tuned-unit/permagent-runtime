/** @vitest-environment jsdom
 *
 * The agent's status on the Home H1. It used to live in the sidebar — a line
 * under Home when open, a loose green dot when collapsed — and moved to the
 * title at Jesse's request, so the two rail components and their hook are
 * gone. These cover the state mapping and the header that replaced them.
 */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../lib/api', () => ({ apiFetch: vi.fn(), api: { getIdentity: vi.fn() } }));
// The configured name comes from the shared, module-cached resolver the World
// uses. Mocked here so the tests pin WHICH source wins, not the network.
const orchestrator = vi.hoisted(() => ({ name: null as string | null }));
vi.mock('../world/shared/useOrchestratorName', () => ({ useOrchestratorName: () => orchestrator.name }));

import { apiFetch } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';
import type { DashboardData } from '../dashboard/useDashboard';
import { HeaderAgentStatus, resolveNavAgentState } from './NavAgentStatus';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
const apiFetchMock = vi.mocked(apiFetch);

function dashboard(state: 'idle' | 'thinking'): DashboardData {
  return {
    agent: { name: 'name-the-indicator-must-ignore', state, active_count: state === 'thinking' ? 1 : 0, summary: '' },
    stats: { sessions_today: 0, sessions_total: 0, memory_count: 0, memory_delta_today: 0 },
    in_flight: [],
    recent: [],
  };
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  apiFetchMock.mockReset();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  useCommandCenter.setState({ agentName: 'Agent' }); // the store's own default
  orchestrator.name = null;
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe('resolveNavAgentState', () => {
  it('is offline whenever unreachable, regardless of the last known daemon state', () => {
    expect(resolveNavAgentState('thinking', true)).toBe('offline');
    expect(resolveNavAgentState('idle', true)).toBe('offline');
    expect(resolveNavAgentState(undefined, true)).toBe('offline');
  });

  it('is thinking only when reachable and the daemon says thinking', () => {
    expect(resolveNavAgentState('thinking', false)).toBe('thinking');
  });

  it('defaults to online for idle, or an unknown/undefined state, while reachable', () => {
    expect(resolveNavAgentState('idle', false)).toBe('online');
    expect(resolveNavAgentState(undefined, false)).toBe('online');
  });
});

function render(el: JSX.Element) {
  act(() => { root.render(el); });
  return container.querySelector('[data-testid="header-agent-status"]') as HTMLElement;
}

describe('HeaderAgentStatus', () => {
  it('uses the name the user configured, the same one the World shows', () => {
    // QA: the header read "Agent online" while the World called the same
    // agent Hank. One resolver now serves both.
    orchestrator.name = 'Hank';
    const el = render(<HeaderAgentStatus data={dashboard('idle')} error={null} />);
    expect(el.textContent).toContain('Hank');
    expect(el.getAttribute('aria-label')).toBe('Hank is online');
  });

  it('never shows the dashboard payload name, which is the default persona', () => {
    orchestrator.name = 'Hank';
    const el = render(<HeaderAgentStatus data={dashboard('idle')} error={null} />);
    expect(el.textContent).not.toContain('name-the-indicator-must-ignore');
  });

  it('never shows the store default "Agent" as though it were a name', () => {
    const el = render(<HeaderAgentStatus data={dashboard('idle')} error={null} />);
    expect(el.textContent).not.toMatch(/\bAgent\b/);
    expect(el.textContent).toContain('Your agent');
  });

  it('takes state from the data it is handed, and does not fetch its own', () => {
    // It used to call useDashboard(), which starts a 15s poller per call, so
    // Home fetched /api/dashboard twice and header and body could disagree.
    orchestrator.name = 'Hank';
    const thinking = render(<HeaderAgentStatus data={dashboard('thinking')} error={null} />);
    expect(thinking.getAttribute('aria-label')).toBe('Hank is thinking');
    const offline = render(<HeaderAgentStatus data={dashboard('idle')} error={new Error('down')} />);
    expect(offline.getAttribute('aria-label')).toBe('Hank is offline');
    expect(apiFetchMock).not.toHaveBeenCalled();
  });
});
