/**
 * Two store actions that resolved the same way whether they worked or not.
 *
 * `Button`'s contract is that a promise resolving to anything but `false` is a
 * success, and it ticks. `setDefaultProvider` and `saveProposal` both ended in
 * a bare `catch { console.error(...) }`, so both resolved `undefined` on a
 * rejected round trip — which is not `false`. The result: "Set as default"
 * ticked while the Default badge stayed where it was, and Automate's Save
 * ticked (until its call site opted the tick out by hand, to work around this)
 * while no skill had been created.
 *
 * `deleteSkill` established the convention this holds them to: report the
 * failure by resolving `false`, and let the caller put a sentence on screen.
 */

import { describe, expect, it, vi, beforeEach } from 'vitest';

const { setProvider, getProviders, createSkill, getSkills, getSkillProposals } = vi.hoisted(() => ({
  setProvider: vi.fn(),
  getProviders: vi.fn(),
  createSkill: vi.fn(),
  getSkills: vi.fn(),
  getSkillProposals: vi.fn(),
}));
vi.mock('./api', () => ({
  api: { setProvider, getProviders, createSkill, getSkills, getSkillProposals },
  apiFetch: vi.fn(),
  getApiBaseUrl: () => 'http://localhost:1234',
  loadDaemonToken: vi.fn(),
}));

import { useCommandCenter, type SkillProposal } from './store';

const proposal: SkillProposal = {
  description: 'Tidy the Downloads folder every Friday',
  tool_used: 'developer__shell',
  argument_shape_hash: 'abc123',
  occurrence_count: 4,
  source_task_ids: ['t1'],
  timestamp: '2026-08-31T12:00:00Z',
};

beforeEach(() => {
  for (const fn of [setProvider, getProviders, createSkill, getSkills, getSkillProposals]) fn.mockReset();
  getProviders.mockResolvedValue([]);
  getSkills.mockResolvedValue([]);
  getSkillProposals.mockResolvedValue([]);
  useCommandCenter.setState({ currentModel: null, providers: [], proposals: [], pendingSkillProposal: null });
});

describe('setDefaultProvider', () => {
  it('resolves true when the daemon took the change', async () => {
    setProvider.mockResolvedValue(undefined);
    await expect(useCommandCenter.getState().setDefaultProvider('openai', 'gpt-5')).resolves.toBe(true);
    expect(useCommandCenter.getState().currentModel).toBe('gpt-5');
  });

  it('resolves false when it did not, so nothing ticks', async () => {
    setProvider.mockRejectedValue(new Error('daemon unreachable'));
    await expect(useCommandCenter.getState().setDefaultProvider('openai', 'gpt-5')).resolves.toBe(false);
  });

  it('leaves the model it never switched to out of the store', async () => {
    setProvider.mockRejectedValue(new Error('daemon unreachable'));
    await useCommandCenter.getState().setDefaultProvider('openai', 'gpt-5');
    expect(useCommandCenter.getState().currentModel).toBeNull();
  });
});

describe('saveProposal', () => {
  it('resolves true when the skill was created', async () => {
    createSkill.mockResolvedValue({ id: 's1' });
    await expect(useCommandCenter.getState().saveProposal(proposal)).resolves.toBe(true);
  });

  it('resolves false when the create failed, so nothing ticks', async () => {
    createSkill.mockRejectedValue(new Error('daemon unreachable'));
    await expect(useCommandCenter.getState().saveProposal(proposal)).resolves.toBe(false);
  });

  it('keeps the banner up when the save it was offering did not happen', async () => {
    useCommandCenter.setState({ pendingSkillProposal: proposal });
    createSkill.mockRejectedValue(new Error('daemon unreachable'));
    await useCommandCenter.getState().saveProposal(proposal);
    expect(useCommandCenter.getState().pendingSkillProposal).toEqual(proposal);
  });
});
