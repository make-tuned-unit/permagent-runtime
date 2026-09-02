/** @vitest-environment jsdom
 *
 * The nav status indicator beside Home (2026-09-01 ruling) replaces the old
 * dashboard hero card. It reads the same two sources the hero card read —
 * `/api/dashboard`'s agent state/reachability (useDashboard, unchanged) and
 * the identity store's configured persona name (`agentName`) — so this covers
 * the mapping from those sources to the dot's three-way state, that the hook
 * actually reads the store rather than a literal, and that both rail-state
 * components render what they are given.
 */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../lib/api', () => ({ apiFetch: vi.fn() }));

import { apiFetch } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';
import type { DashboardData } from '../dashboard/useDashboard';
import {
  NavStatusBadge,
  NavStatusLine,
  resolveNavAgentState,
  useNavAgentStatus,
} from './NavAgentStatus';

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

let hookResult: ReturnType<typeof useNavAgentStatus> | undefined;
function HookHarness() {
  hookResult = useNavAgentStatus();
  return null;
}

describe('useNavAgentStatus', () => {
  it('reads the name from the identity store, not a literal — never "Henry"', async () => {
    useCommandCenter.setState({ agentName: 'Zephyr' });
    apiFetchMock.mockResolvedValue(dashboard('idle'));
    await act(async () => { root.render(<HookHarness />); });
    await act(async () => { await Promise.resolve(); });
    expect(hookResult!.name).toBe('Zephyr');
    expect(hookResult!.name).not.toBe('Henry');
    // The dashboard payload's OWN agent.name is deliberately ignored — the
    // identity store is the one persona-name source (see file header).
    expect(hookResult!.name).not.toBe('name-the-indicator-must-ignore');
  });

  it('maps a thinking daemon state to the thinking dot state', async () => {
    apiFetchMock.mockResolvedValue(dashboard('thinking'));
    await act(async () => { root.render(<HookHarness />); });
    await act(async () => { await Promise.resolve(); });
    expect(hookResult!.state).toBe('thinking');
    expect(hookResult!.word).toBe('thinking');
  });

  it('maps an idle daemon state to online', async () => {
    apiFetchMock.mockResolvedValue(dashboard('idle'));
    await act(async () => { root.render(<HookHarness />); });
    await act(async () => { await Promise.resolve(); });
    expect(hookResult!.state).toBe('online');
    expect(hookResult!.word).toBe('online');
  });

  it('maps a failed /api/dashboard poll to offline — the same reachability signal the Home freshness banner uses', async () => {
    apiFetchMock.mockRejectedValue(new Error('daemon down'));
    await act(async () => { root.render(<HookHarness />); });
    await act(async () => { await Promise.resolve(); });
    expect(hookResult!.state).toBe('offline');
    expect(hookResult!.word).toBe('offline');
  });
});

describe('NavStatusLine (open rail)', () => {
  it('renders the given name, not a hardcoded persona string', () => {
    act(() => { root.render(<NavStatusLine name="Zephyr" state="online" word="online" />); });
    const el = container.querySelector('[data-testid="nav-status-line"]') as HTMLElement;
    expect(el.textContent).toContain('Zephyr');
    expect(el.textContent).not.toContain('Henry');
    expect(el.getAttribute('aria-label')).toBe('Zephyr is online');
  });

  it('names the state word next to the dot', () => {
    act(() => { root.render(<NavStatusLine name="Zephyr" state="offline" word="offline" />); });
    const el = container.querySelector('[data-testid="nav-status-line"]') as HTMLElement;
    expect(el.textContent).toContain('offline');
  });
});

describe('NavStatusBadge (collapsed rail)', () => {
  it('is keyboard-focusable and announces "{name} is {word}" to a screen reader', () => {
    act(() => {
      root.render(
        <NavStatusBadge name="Zephyr" state="offline" word="offline" onHover={() => {}} onLeave={() => {}} />
      );
    });
    const el = container.querySelector('[data-testid="nav-status-badge"]') as HTMLElement;
    expect(el.getAttribute('aria-label')).toBe('Zephyr is offline');
    expect(el.getAttribute('tabindex')).toBe('0');
  });

  it('reuses the sidebar tooltip hover pattern, offering "{name} · {word}"', () => {
    const onHover = vi.fn();
    act(() => {
      root.render(
        <NavStatusBadge name="Zephyr" state="offline" word="offline" onHover={onHover} onLeave={() => {}} />
      );
    });
    const el = container.querySelector('[data-testid="nav-status-badge"]') as HTMLElement;
    act(() => { el.focus(); });
    expect(onHover).toHaveBeenCalledWith(el, 'Zephyr · offline');
  });
});
