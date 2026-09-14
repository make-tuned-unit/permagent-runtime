import { expect, it } from 'vitest';
import { patrolTarget } from './patrolMotion';
it('crosses an ordinary waypoint without inserting an idle frame', () => {
  const p = { route: [[0,0,0],[1,0,0],[2,0,0]], next: 1, direction: 1, pause: 0 };
  expect(patrolTarget(p,{x:.98,z:0})).toEqual({dx:1.02,dz:0,distance:1.02});
  expect(p.next).toBe(2);
  expect(p.pause).toBe(0);
});
it('stops at an authored rest point and skips duplicate ordinary waypoints', () => {
  const route = Array.from({length:14},(_,i)=>[i,0,0]);
  const p = { route, next: 11, direction: 1, pause: 0 };
  expect(patrolTarget(p,{x:11,z:0})).toBeNull();
  expect(p.pause).toBeGreaterThan(0);
  const q = {route:[[0,0,0],[1,0,0],[1,0,0],[2,0,0]],next:1,direction:1,pause:0};
  expect(patrolTarget(q,{x:1,z:0})?.distance).toBe(1);
  expect(q.next).toBe(3);
});
