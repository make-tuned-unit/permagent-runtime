// #105 — the ring-locked Librarian must never leave the mezzanine vertically.
// Its Y is pinned to the mezzanine floor (ringLockY) every frame, so no stray
// waypoint height, third-person nudge, or drift can lift it toward the ceiling.
// A free-roaming agent (ringLockY null) is unaffected and keeps surface-follow.

import { describe, expect, it } from 'vitest';
import { ensureMotion, getMotion, advanceMotion, nudgeAgent, setPath } from './motion';

const MEZZ_Y = 10.15; // roster.MEZZ_Y — the mezzanine floor height
const RING_R = 15.2; // roster.MEZZ_RADIUS — the walk ring

describe('ring-locked vertical clamp (#105)', () => {
  it('captures ringLockY from home.y for ring-locked agents', () => {
    ensureMotion('lib-capture', { x: RING_R, y: MEZZ_Y, z: 0 }, RING_R);
    const m = getMotion('lib-capture')!;
    expect(m.ringLockY).toBe(MEZZ_Y);
  });

  it('leaves ringLockY null for free-roaming agents', () => {
    ensureMotion('free-capture', { x: 0, y: 0, z: 0 }, null);
    expect(getMotion('free-capture')!.ringLockY).toBeNull();
  });

  it('re-pins Y to the mezzanine floor after a drift, on the next frame', () => {
    ensureMotion('lib-drift', { x: RING_R, y: MEZZ_Y, z: 0 }, RING_R);
    const m = getMotion('lib-drift')!;
    m.y = 45; // simulate an off-path drift up toward the ceiling
    advanceMotion(0.016);
    expect(m.y).toBe(MEZZ_Y);
  });

  it('never rises when a waypoint carries a stray height', () => {
    ensureMotion('lib-badwp', { x: RING_R, y: MEZZ_Y, z: 0 }, RING_R);
    const m = getMotion('lib-badwp')!;
    // A waypoint far around the ring but with a bogus ceiling-high y.
    setPath('lib-badwp', [{ x: -RING_R, y: 30, z: 0 }]);
    for (let i = 0; i < 60; i++) advanceMotion(0.05);
    expect(m.y).toBe(MEZZ_Y);
  });

  it('clamps Y even under a third-person nudge', () => {
    ensureMotion('lib-nudge', { x: RING_R, y: MEZZ_Y, z: 0 }, RING_R);
    const m = getMotion('lib-nudge')!;
    m.y = 22; // pretend a prior frame left it high
    nudgeAgent('lib-nudge', 0.5, 0); // arrow-key drive
    expect(m.y).toBe(MEZZ_Y);
    // XZ also stays on the ring.
    expect(Math.hypot(m.x, m.z)).toBeCloseTo(RING_R, 5);
  });

  it('does not pin a free-roaming agent to any height', () => {
    // On the ground (y=0, inside the rotunda edge) surface-follow keeps it at 0,
    // proving the ring-lock clamp doesn't apply to non-locked agents.
    ensureMotion('free-ground', { x: 2, y: 0, z: 0 }, null);
    const m = getMotion('free-ground')!;
    advanceMotion(0.016);
    expect(m.ringLockY).toBeNull();
    expect(m.y).toBe(0);
  });
});
