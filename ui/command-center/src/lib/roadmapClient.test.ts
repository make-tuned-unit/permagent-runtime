/**
 * roadmapClient (#251/#252) — proves each wrapper hits the real mounted
 * endpoint with the right method/body and returns the parsed payload.
 */
import { describe, expect, it, vi, beforeEach } from 'vitest';

vi.mock('./api', () => ({ apiFetch: vi.fn() }));

import { apiFetch } from './api';
import {
  insertRoadmapGoal,
  removeRoadmapGoal,
  setGoalDependencies,
} from './roadmapClient';

const apiFetchMock = vi.mocked(apiFetch);

beforeEach(() => {
  apiFetchMock.mockReset();
});

describe('roadmapClient', () => {
  it('insertRoadmapGoal POSTs the goal to the roadmap endpoint', async () => {
    const card = { id: 'g1', title: 'New goal' };
    apiFetchMock.mockResolvedValueOnce(card);

    const out = await insertRoadmapGoal('proj-1', {
      title: 'New goal',
      dependsOn: ['dep-1'],
    });

    expect(out).toBe(card);
    expect(apiFetchMock).toHaveBeenCalledWith('/api/projects/proj-1/roadmap/goals', {
      method: 'POST',
      body: JSON.stringify({ title: 'New goal', dependsOn: ['dep-1'] }),
    });
  });

  it('setGoalDependencies PUTs the dependsOn set', async () => {
    apiFetchMock.mockResolvedValueOnce({ id: 'g1' });

    await setGoalDependencies('proj-1', 'g1', ['a', 'b']);

    expect(apiFetchMock).toHaveBeenCalledWith(
      '/api/projects/proj-1/roadmap/goals/g1/dependencies',
      { method: 'PUT', body: JSON.stringify({ dependsOn: ['a', 'b'] }) },
    );
  });

  it('removeRoadmapGoal POSTs to the remove endpoint', async () => {
    const res = { removed: true, cancelled: true, rewiredDependents: 2 };
    apiFetchMock.mockResolvedValueOnce(res);

    const out = await removeRoadmapGoal('proj-1', 'g1');

    expect(out).toEqual(res);
    expect(apiFetchMock).toHaveBeenCalledWith(
      '/api/projects/proj-1/roadmap/goals/g1/remove',
      { method: 'POST' },
    );
  });

  it('URL-encodes ids', async () => {
    apiFetchMock.mockResolvedValueOnce({});
    await removeRoadmapGoal('p/1', 'g 2');
    expect(apiFetchMock).toHaveBeenCalledWith(
      '/api/projects/p%2F1/roadmap/goals/g%202/remove',
      { method: 'POST' },
    );
  });

  it('propagates daemon validation errors verbatim', async () => {
    apiFetchMock.mockRejectedValueOnce(new Error('Dependency cycle detected among goals'));
    await expect(setGoalDependencies('p', 'g', ['x'])).rejects.toThrow(
      'Dependency cycle detected among goals',
    );
  });
});
