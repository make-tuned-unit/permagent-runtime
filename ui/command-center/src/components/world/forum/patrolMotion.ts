/** Resolve ordinary waypoints in the current frame; only authored stops return null. */
export function patrolTarget(
  patrol: { route: number[][]; next: number; direction: number; pause: number },
  position: { x: number; z: number },
): { dx: number; dz: number; distance: number } | null {
  for (let skipped = 0; skipped < patrol.route.length; skipped++) {
    const target = patrol.route[patrol.next];
    const dx = target[0] - position.x, dz = target[2] - position.z;
    const distance = Math.hypot(dx, dz);
    if (distance >= .09) return { dx, dz, distance };
    patrol.next = (patrol.next + patrol.direction + patrol.route.length) % patrol.route.length;
    if (patrol.next % 12 === 0) {
      patrol.pause = 3 + (patrol.next % 5) * 1.2;
      return null;
    }
  }
  return null;
}
