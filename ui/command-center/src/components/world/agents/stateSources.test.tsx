// @vitest-environment jsdom
/**
 * The World's Henry poll must not ask a daemon that cannot answer it.
 *
 * 2026-09-19: an unpaired `forum.html` tab in Chrome (served by Vite, proxied
 * to :3001) held no credential and still polled /api/henry/status every 2s —
 * 30 guaranteed 401s a minute, for hours, alongside the rest of the World's
 * pollers (804 x 401 in 11 minutes). The poll now asks `loadDaemonToken` first
 * and makes no request without one; it resumes by itself once a credential
 * appears.
 *
 * What this proves: with no credential the component issues zero Henry-status
 * and zero agent-roster requests across many poll ticks, still publishes the
 * honest 'error' reading for Henry, and starts the real poll on the next tick
 * after a token becomes available. What it does not prove: the World's OTHER
 * pollers (identity, skills, goals, the /events socket) — those are held back
 * in the api client itself, pinned by lib/apiUnpaired.test.ts.
 */
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const h = vi.hoisted(() => ({
  token: null as string | null,
  getHenryStatus: vi.fn(),
  getAgents: vi.fn(),
}));

vi.mock('../../../lib/api', () => ({
  loadDaemonToken: () => Promise.resolve(h.token),
  api: { getHenryStatus: h.getHenryStatus, getAgents: h.getAgents },
}));
vi.mock('../shared/worldEvents', () => ({ subscribeWorldEvents: () => () => {} }));

import { AgentStateSources } from './stateSources';
import { getAgentRuntimeStates } from '../shared/agentStatus';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

async function render(): Promise<{ unmount: () => void }> {
  const root = createRoot(document.createElement('div'));
  await act(async () => { root.render(<AgentStateSources />); await flush(); });
  return { unmount: () => act(() => root.unmount()) };
}

/** Let the awaited token read and any request settle. */
async function flush() {
  for (let i = 0; i < 5; i++) await Promise.resolve();
}

describe('AgentStateSources without a daemon credential', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    h.token = null;
    h.getHenryStatus.mockReset().mockResolvedValue({
      identity: { name: 'Henry', traits: [], tone: '' },
      current_state: 'idle',
    });
    h.getAgents.mockReset().mockResolvedValue({ agents: [] });
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it('sends no Henry-status or roster request while unpaired, and says Henry cannot be seen', async () => {
    const { unmount } = await render();
    // Ten poll intervals: before the fix this was eleven 401s.
    for (let i = 0; i < 10; i++) {
      await act(async () => { vi.advanceTimersByTime(2000); await flush(); });
    }
    expect(h.getHenryStatus).not.toHaveBeenCalled();
    expect(h.getAgents).not.toHaveBeenCalled();
    const henry = getAgentRuntimeStates().find(a => a.id === 'henry');
    expect(henry?.hudState).toBe('error');
    unmount();
  });

  it('does not re-publish the World state on every unpaired tick', async () => {
    // The unpaired poll re-reports 'error' every 2s. That must be a no-op for
    // subscribers after the first report, or the unpaired World re-renders 30
    // times a minute for nothing. The store replaces an agent's entry (and
    // publishes) only when its reading changes, so an unchanged Henry entry
    // means the poll woke no subscriber. (The whole snapshot is not compared:
    // the sim agents legitimately toggle on random timers.)
    const henryEntry = () => getAgentRuntimeStates().find(a => a.id === 'henry');
    const { unmount } = await render();
    await act(async () => { vi.advanceTimersByTime(2000); await flush(); });
    const settled = henryEntry();
    expect(settled?.hudState).toBe('error');
    for (let i = 0; i < 10; i++) {
      await act(async () => { vi.advanceTimersByTime(2000); await flush(); });
    }
    expect(henryEntry()).toBe(settled);
    unmount();
  });

  it('starts the real poll on its own once a credential appears', async () => {
    const { unmount } = await render();
    expect(h.getHenryStatus).not.toHaveBeenCalled();

    h.token = 'paired-now';
    await act(async () => { vi.advanceTimersByTime(2000); await flush(); });
    expect(h.getHenryStatus).toHaveBeenCalledTimes(1);
    const henry = getAgentRuntimeStates().find(a => a.id === 'henry');
    expect(henry?.hudState).toBe('available');
    unmount();
  });
});
