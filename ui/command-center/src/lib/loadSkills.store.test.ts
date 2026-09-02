/**
 * `loadSkills` used to end `catch { set({ skills: [] }) }`.
 *
 * The Library's empty state is a genuinely good piece of writing — "No skills
 * saved yet. Skills are created when your agent detects repeated patterns." —
 * and it was what a user saw when the daemon was unreachable. The sentence is
 * an invitation when it is true and a lie when it is not, and nothing on the
 * screen distinguished the two.
 *
 * The codebase already carries this rule twice by name (#568 no-silent-catch,
 * and SkillExecutionHistory's own comment: "A backend failure is an ERROR, not
 * an empty history"). This holds the store to it.
 */

import { describe, expect, it, vi, beforeEach } from 'vitest';

const { getSkills } = vi.hoisted(() => ({ getSkills: vi.fn() }));
vi.mock('./api', () => ({
  api: { getSkills },
  apiFetch: vi.fn(),
  getApiBaseUrl: () => 'http://localhost:1234',
  loadDaemonToken: vi.fn(),
}));

import { useCommandCenter } from './store';

beforeEach(() => {
  getSkills.mockReset();
  useCommandCenter.setState({ skills: [], skillsLoading: false, skillsError: null });
});

describe('loadSkills', () => {
  it('reports what it loaded', async () => {
    getSkills.mockResolvedValue([{ id: 's1', name: 'Tidy Downloads', description: '' }]);
    await useCommandCenter.getState().loadSkills();
    const s = useCommandCenter.getState();
    expect(s.skills).toHaveLength(1);
    expect(s.skillsError).toBeNull();
    expect(s.skillsLoading).toBe(false);
  });

  it('never renders a failure as an empty library', async () => {
    getSkills.mockRejectedValue(new Error('daemon unreachable'));
    await useCommandCenter.getState().loadSkills();
    const s = useCommandCenter.getState();
    expect(s.skillsError).toContain('daemon unreachable');
    expect(s.skillsLoading).toBe(false);
  });

  it('keeps the last good list rather than blanking the screen on a failed refresh', async () => {
    getSkills.mockResolvedValue([{ id: 's1', name: 'Tidy Downloads', description: '' }]);
    await useCommandCenter.getState().loadSkills();
    getSkills.mockRejectedValue(new Error('daemon unreachable'));
    await useCommandCenter.getState().loadSkills();
    expect(useCommandCenter.getState().skills).toHaveLength(1);
    expect(useCommandCenter.getState().skillsError).toBeTruthy();
  });

  it('clears a stale error once the load lands', async () => {
    getSkills.mockRejectedValue(new Error('daemon unreachable'));
    await useCommandCenter.getState().loadSkills();
    getSkills.mockResolvedValue([]);
    await useCommandCenter.getState().loadSkills();
    expect(useCommandCenter.getState().skillsError).toBeNull();
  });
});
