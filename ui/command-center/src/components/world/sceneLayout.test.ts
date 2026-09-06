import { describe, expect, it } from 'vitest';
import { PLATFORM_RADIUS, WORLD_ORBIT_POSITION, WORLD_ORBIT_TARGET,
  WORLD_ORBIT_MAX_DISTANCE, WORLD_GRAIN_OPACITY } from './constants';

describe('grand Rotunda presentation', () => {
  it('starts outside the expanded colonnade and can zoom out past its establishing shot', () => {
    const distance = Math.hypot(...WORLD_ORBIT_POSITION.map((v,i) => v-WORLD_ORBIT_TARGET[i]));
    expect(Math.hypot(WORLD_ORBIT_POSITION[0],WORLD_ORBIT_POSITION[2])).toBeGreaterThan(PLATFORM_RADIUS);
    expect(distance).toBeLessThan(WORLD_ORBIT_MAX_DISTANCE);
    expect(WORLD_ORBIT_TARGET[1]).toBeGreaterThan(10);
  });
  it('keeps grain restrained enough for authored surface detail', () => {
    expect(WORLD_GRAIN_OPACITY).toBeGreaterThanOrEqual(0);
    expect(WORLD_GRAIN_OPACITY).toBeLessThan(.025);
  });
});
