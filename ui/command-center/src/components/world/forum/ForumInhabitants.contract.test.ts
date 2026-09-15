import { beforeEach, expect, it, vi } from 'vitest';
import { Vector3 } from 'three';

vi.mock('./walkCollision', () => ({
  attemptWalk: (feet: Vector3, dx: number, dz: number) => { feet.x += dx; feet.z += dz; return true; },
  getForumColliders: () => [{}],
}));

import { ensureMotion, getMotion, setPath } from '../agents/motion';
import { advanceForumInhabitants, ensureForumMotion, type ForumPerson } from './ForumInhabitants';

function person(id: string): ForumPerson {
  const motion = ensureForumMotion(id, { x: 0, y: 0, z: 0 });
  Object.assign(motion, { x: 0, y: 0, z: 0, heading: 0, walking: false, strideDistance: 0, engaged: 'none' });
  return { id, route: [[0, 0, 0], [4, 0, 0]], motion, next: 1, direction: 1, pause: 0, blocked: 0, feet: new Vector3(), speed: 1 };
}

beforeEach(() => { vi.clearAllMocks(); });

it('keeps shared worker motion intact while forum ambience advances its own record', () => {
  const live = ensureMotion('forum-live-worker', { x: 20, y: 0, z: 20 });
  setPath('forum-live-worker', [{ x: 25, y: 0, z: 20 }]);
  live.engaged = 'tending';
  const before = { x: live.x, z: live.z, walking: live.walking, engaged: live.engaged, queue: [...live.queue] };
  const ambient = person('forum-live-worker');

  advanceForumInhabitants([ambient], .25, null, false, [], true);

  expect(ambient.motion.x).toBeGreaterThan(0);
  expect(live.x).toBe(before.x);
  expect(live.z).toBe(before.z);
  expect(live.walking).toBe(before.walking);
  expect(live.engaged).toBe(before.engaged);
  expect(live.queue).toEqual(before.queue);
});

it('pauses the ambient record while the real worker reports working', () => {
  const ambient = person('forum-working-worker');
  const before = { x: ambient.motion.x, z: ambient.motion.z, stride: ambient.motion.strideDistance };

  advanceForumInhabitants([ambient], .25, null, false, [{ id: ambient.id, name: 'Worker', hudState: 'working', source: 'daemon' }], true);

  expect(ambient.motion.x).toBe(before.x);
  expect(ambient.motion.z).toBe(before.z);
  expect(ambient.motion.strideDistance).toBe(before.stride);
  expect(ambient.motion.walking).toBe(false);
});

it('does not leave a forum-owned record in the shared runtime store', () => {
  const ambient = person('forum-isolated-worker');
  advanceForumInhabitants([ambient], .25, null, false, [], true);
  expect(getMotion(ambient.id)).toBeUndefined();
});
