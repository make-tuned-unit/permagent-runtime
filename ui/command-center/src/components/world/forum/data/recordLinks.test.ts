import { describe, expect, it, vi } from 'vitest';
import {
  createStoreNavigation, openRecordLink, parseRecordLinkKey, recordLinkKey, recordLinkLabel,
  type StoreNavigationSlice, type WorldNavigation, type WorldRecordLink,
} from './recordLinks';

const LINKS: WorldRecordLink[] = [
  { to: 'brain', memoryId: 'm/1', memoryKey: 'note:a b' },
  { to: 'brain', memoryId: null, memoryKey: 'code:x' },
  { to: 'goal', projectId: 'p1', goalId: 'g:1' },
  { to: 'decision_inbox', decisionId: 'd1' },
  { to: 'decision_inbox', decisionId: null },
  { to: 'conversation', sessionId: 's1' },
  { to: 'history', tab: 'spend' },
  { to: 'grow', projectId: 'p1' },
  { to: 'grow', projectId: null },
  { to: 'finance' },
  { to: 'project', projectId: 'p1' },
  { to: 'project_note', projectId: 'p1', noteId: 'n1' },
  { to: 'skills' },
  { to: 'agent_settings', agentId: 'git_steward' },
  { to: 'build' },
  { to: 'council', sessionId: 'c1' },
];

function fakeNav(reachable = true): WorldNavigation & { log: string[] } {
  const log: string[] = [];
  const f = (name: string) => (...args: unknown[]) => { log.push(`${name}(${args.map((a) => JSON.stringify(a)).join(',')})`); return reachable; };
  return {
    log,
    focusBrainMemory: f('focusBrainMemory'), openGoalDetail: f('openGoalDetail'), openDecisionInbox: f('openDecisionInbox'),
    openConversation: f('openConversation'), openHistory: f('openHistory'), openGrow: f('openGrow'), openTool: f('openTool'),
    openProject: f('openProject'), focusProjectNote: f('focusProjectNote'), openSkills: f('openSkills'), openAgentSettings: f('openAgentSettings'), openHome: f('openHome'),
  };
}

describe('record links', () => {
  it('keys round-trip for every target, ignoring previews', () => {
    for (const link of LINKS) expect(parseRecordLinkKey(recordLinkKey(link))).toEqual(link);
    const withPreview: WorldRecordLink = { to: 'brain', memoryId: 'm1', memoryKey: null, preview: { text: 'x', timestamp: null } };
    expect(recordLinkKey(withPreview)).toBe(recordLinkKey({ to: 'brain', memoryId: 'm1', memoryKey: null }));
  });

  it('rejects malformed keys rather than guessing', () => {
    for (const bad of ['', 'brain', 'brain:~/~', 'goal:p1', 'history:everything', 'nope:x', 'conversation:~', 'finance:x']) {
      expect(parseRecordLinkKey(bad)).toBeNull();
    }
  });

  it('dispatches each link to exactly one navigation seam', () => {
    for (const link of LINKS) {
      const nav = fakeNav();
      expect(openRecordLink(link, nav)).toEqual({ ok: true });
      expect(nav.log).toHaveLength(1);
      expect(recordLinkLabel(link).length).toBeGreaterThan(3);
    }
    const nav = fakeNav();
    openRecordLink({ to: 'goal', projectId: 'p', goalId: 'g' }, nav);
    expect(nav.log[0]).toBe('openGoalDetail("p","g")');
  });

  it('reports an unreachable destination instead of pretending', () => {
    const r = openRecordLink({ to: 'decision_inbox', decisionId: null }, fakeNav(false));
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.reason).toMatch(/Home/);
    expect(openRecordLink({ to: 'brain', memoryId: null, memoryKey: null }, fakeNav()).ok).toBe(false);
  });

  it('store adapter uses the existing actions and checks reachability first', () => {
    const slice: StoreNavigationSlice = {
      focusBrainMemory: vi.fn(), openGoalDetail: vi.fn(), openDecisionInbox: vi.fn(() => true), switchToSession: vi.fn(async () => {}),
      setActivePanel: vi.fn(), setPendingSettingsSection: vi.fn(), growProject: vi.fn(), focusProjectNote: vi.fn(),
      setPendingProjectNavigation: vi.fn(), openAgentSettings: vi.fn(),
    };
    const navigateToTool = vi.fn(() => false);
    const nav = createStoreNavigation(() => slice, navigateToTool);
    expect(openRecordLink({ to: 'brain', memoryId: 'm', memoryKey: null }, nav).ok).toBe(false);
    expect(slice.focusBrainMemory).not.toHaveBeenCalled();
    navigateToTool.mockReturnValue(true);
    expect(openRecordLink({ to: 'brain', memoryId: 'm', memoryKey: null }, nav).ok).toBe(true);
    expect(slice.focusBrainMemory).toHaveBeenCalledWith({ id: 'm' });
    openRecordLink({ to: 'history', tab: 'activity' }, nav);
    expect(slice.setPendingSettingsSection).toHaveBeenCalledWith('activity');
    expect(slice.setActivePanel).toHaveBeenCalledWith('history');
    openRecordLink({ to: 'conversation', sessionId: 's1' }, nav);
    expect(slice.switchToSession).toHaveBeenCalledWith('s1');
  });
});
