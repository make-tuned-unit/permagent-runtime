import { describe, expect, it } from 'vitest';
import { LAGOON_RADIUS, SHIP_COUNT, WATERLINE_Y, courseIsAfloat, fitCourse, shipCourse } from './shipMotion';

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

describe('ships only sail where the world says there is water', () => {
  // The island as the builder reports it: a plaza and terraces out to 62m, a
  // lagoon from 62m to 118m, open sea beyond. This is the shape that broke the
  // first version, which sized courses from a single radius and sailed most of
  // the fleet across the plaza.
  const realisticWater = (x: number, z: number) => {
    const r = Math.hypot(x, z);
    return r > 62 && r < 118;
  };

  it('fits every ship onto the lagoon', () => {
    for (let index = 0; index < SHIP_COUNT; index += 1) {
      const course = fitCourse(index, realisticWater);
      expect(course, `ship ${index} found no water`).not.toBeNull();
      expect(courseIsAfloat(course!, realisticWater)).toBe(true);
    }
  });

  it('never puts a fitted course over the plaza', () => {
    for (let index = 0; index < SHIP_COUNT; index += 1) {
      const course = fitCourse(index, realisticWater)!;
      for (let seconds = 0; seconds < 600; seconds += 3) {
        const pose = shipCourse(seconds, index, course);
        expect(Math.hypot(pose.x, pose.z)).toBeGreaterThan(62);
      }
    }
  });

  it('rejects a course that crosses land, rather than sailing it', () => {
    // A lagoon with a jetty across it: any loop enclosing the centre must cross
    // the jetty, so there is no course to fit and the honest answer is none.
    const jetty = (x: number, z: number) => realisticWater(x, z) && Math.abs(z) > 4;
    expect(fitCourse(0, jetty)).toBeNull();
  });

  it('reports no course at all when there is no water', () => {
    expect(fitCourse(2, () => false)).toBeNull();
  });

  it('is deterministic, so the fleet is identical on every load', () => {
    expect(fitCourse(3, realisticWater)).toEqual(fitCourse(3, realisticWater));
  });
});
