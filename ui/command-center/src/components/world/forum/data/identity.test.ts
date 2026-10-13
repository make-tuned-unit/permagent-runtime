import { describe, expect, it } from 'vitest';
import { agentIdForWorldAgent } from '../../../../lib/worldAgentIds';
import { HABITAT_PLACES } from '../habitatLayout';
import { ROSTER } from '../../agents';
import {
  PLACE_HOST, WORLD_AGENT_IDS, WORLD_PLACE_IDS, resolveWorldAgent, truthFromAgent, truthFromHenryStatus, workAgentIdFor,
} from './identity';
import type { AgentDetail, WorkReview } from '../../../../lib/agentsApi';

describe('World identity mapping', () => {
  it('names exactly the six current places, with the hosts habitatLayout authors', () => {
    expect(HABITAT_PLACES.map((p) => p.id)).toEqual([...WORLD_PLACE_IDS]);
    for (const p of HABITAT_PLACES) expect(PLACE_HOST[p.id as keyof typeof PLACE_HOST]).toBe(p.agent);
  });

  it('covers every World roster id', () => {
    expect(ROSTER.map((a) => a.id).sort()).toEqual([...WORLD_AGENT_IDS].sort());
  });

  it('maps the Steward to git_steward and agrees with lib/worldAgentIds for every worker', () => {
    expect(resolveWorldAgent('steward')).toMatchObject({ status: 'worker', agentId: 'git_steward', workEndpoint: '/api/agents/git_steward/work' });
    for (const id of WORLD_AGENT_IDS) {
      const r = resolveWorldAgent(id);
      if (r.status === 'worker') expect(r.agentId).toBe(agentIdForWorldAgent(id));
    }
  });

  it('resolves the orchestrator to henry status, never to an agents-API id', () => {
    const r = resolveWorldAgent('henry');
    expect(r).toMatchObject({ status: 'orchestrator', statusEndpoint: '/api/henry/status' });
    expect(workAgentIdFor('henry')).toBeNull();
  });

  it('resolves Reader, Picker, Polybot and unknown ids to explicit unsupported states', () => {
    for (const id of ['reader', 'picker', 'polybot', 'nobody', '']) {
      const r = resolveWorldAgent(id);
      expect(r.status).toBe('unsupported');
      if (r.status === 'unsupported') expect(r.reason.length).toBeGreaterThan(20);
      expect(workAgentIdFor(id)).toBeNull();
    }
  });

  it('every place host resolves to something supported', () => {
    for (const p of WORLD_PLACE_IDS) expect(resolveWorldAgent(PLACE_HOST[p]).status).not.toBe('unsupported');
  });
});

const worker = (over: Partial<Extract<AgentDetail, { kind: 'worker' }>> = {}): AgentDetail => ({
  kind: 'worker', id: 'librarian', display_name: 'Librarian', what_it_does: '', why_it_matters: '',
  state_source: 'queryable', live_state: { status: 'ok', value: 'idle' }, dispatchable: false,
  gate: { config_key: 'librarian_enabled', enabled: true }, ...over,
});
const work = (items: number, status: 'ok' | 'unavailable' = 'ok'): WorkReview => ({
  activity: status === 'ok'
    ? { attribution: 'librarian', status, items: Array.from({ length: items }, (_, i) => ({ id: `a${i}`, ts: '2026-10-12T11:00:00Z', kind: 'k', actor: 'librarian', title: 't', detail: null, ref_kind: null, ref_id: null, goal_project_id: null })), truncated: false }
    : { attribution: 'librarian', status, reason: 'db locked' },
  goals: { status: 'ok', items: [], truncated: false },
  spend: { status: 'ok', items: [], truncated: false },
  scheduled_jobs: { status: 'ok', items: [], truncated: false },
});

describe('state truth: configured vs reachable vs ready vs actively used', () => {
  it('keeps the four facts separate', () => {
    const t = truthFromAgent(worker(), work(0));
    expect(t.configured.value).toBe('on');
    expect(t.reachable.value).toBe('reachable');
    expect(t.ready.value).toBe('ready');
    // Empty attribution is not inactivity.
    expect(t.activelyUsed.value).toBe('none_attributed_in_window');
    expect(t.activelyUsed.evidence).toMatch(/does not prove/);
  });

  it('a configured switch alone is not readiness', () => {
    const t = truthFromAgent(worker({ live_state: { status: 'not_queryable' } }), null);
    expect(t.configured.value).toBe('on');
    expect(t.reachable.value).toBe('not_probed');
    expect(t.ready.value).toBe('unknown');
    expect(t.activelyUsed.value).toBe('unknown');
  });

  it('off, unreachable and unavailable work stay distinct', () => {
    expect(truthFromAgent(worker({ gate: { config_key: 'k', enabled: false } })).ready.value).toBe('not_ready');
    expect(truthFromAgent(worker({ live_state: { status: 'unavailable', reason: 'x' } })).reachable.value).toBe('unreachable');
    expect(truthFromAgent(worker({ gate: null })).configured.value).toBe('no_switch');
    expect(truthFromAgent(worker(), work(0, 'unavailable')).activelyUsed.value).toBe('unknown');
    expect(truthFromAgent(worker(), work(2)).activelyUsed.value).toBe('attributed_in_window');
  });

  it('a reachable orchestrator is not proof a model can answer', () => {
    const t = truthFromHenryStatus({ current_state: 'idle', current_tool: null, tasks_in_flight: 0 });
    expect(t.reachable.value).toBe('reachable');
    expect(t.ready.value).toBe('unknown');
    expect(t.configured.value).toBe('unknown');
    expect(truthFromHenryStatus({ current_state: 'working', current_tool: 'shell', tasks_in_flight: 1 }).activelyUsed.value).toBe('active_now');
    expect(truthFromHenryStatus(null).reachable.value).toBe('unknown');
  });
});
