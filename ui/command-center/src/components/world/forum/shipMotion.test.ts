import { describe, expect, it } from 'vitest';
import { LAGOON_RADIUS, SHIP_COUNT, shipCourse, WATERLINE_Y } from './shipMotion';

describe('ships sailing the lagoon', () => {
  it('keeps every ship on the water, inside the lagoon', () => {
    for (let index = 0; index < SHIP_COUNT; index += 1) {
      for (let seconds = 0; seconds < 400; seconds += 7) {
        const pose = shipCourse(seconds, index);
        const distance = Math.hypot(pose.x, pose.z);
        expect(distance).toBeLessThan(LAGOON_RADIUS);
        // A hull that drifts off the waterline reads as a flying boat.
        expect(Math.abs(pose.y - WATERLINE_Y)).toBeLessThan(0.2);
      }
    }
  });

  it('points each ship where it is actually going', () => {
    for (let index = 0; index < SHIP_COUNT; index += 1) {
      const now = shipCourse(60, index);
      const next = shipCourse(60.5, index);
      const travelled = Math.atan2(next.x - now.x, next.z - now.z);
      // The model's bow is +Y, so yaw is measured the same way as the step.
      const difference = Math.abs(Math.atan2(Math.sin(travelled - now.yaw), Math.cos(travelled - now.yaw)));
      expect(difference).toBeLessThan(0.25);
    }
  });

  it('is continuous — no teleporting between frames', () => {
    for (let index = 0; index < SHIP_COUNT; index += 1) {
      let previous = shipCourse(0, index);
      for (let seconds = 0.1; seconds < 120; seconds += 0.1) {
        const pose = shipCourse(seconds, index);
        expect(Math.hypot(pose.x - previous.x, pose.z - previous.z)).toBeLessThan(1.5);
        previous = pose;
      }
    }
  });

  it('gives each ship its own course, so the fleet never moves as one', () => {
    const first = shipCourse(30, 0);
    for (let index = 1; index < SHIP_COUNT; index += 1) {
      const other = shipCourse(30, index);
      expect(Math.hypot(other.x - first.x, other.z - first.z)).toBeGreaterThan(4);
    }
  });

  it('reports a bounded speed, so the wake can fade with it', () => {
    for (let seconds = 0; seconds < 200; seconds += 3) {
      const pose = shipCourse(seconds, 2);
      expect(pose.speed).toBeGreaterThanOrEqual(0);
      expect(pose.speed).toBeLessThanOrEqual(1);
    }
  });

  it('is deterministic — the same second always gives the same pose', () => {
    expect(shipCourse(83.25, 3)).toEqual(shipCourse(83.25, 3));
  });
});
