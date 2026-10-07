import { expect, it } from 'vitest';
import { HABITAT_PLANT_FORMS } from './HabitatPlanting';
import { HABITAT_DECOR, HABITAT_SOLIDS } from './habitatLayout';

it('keeps the planted habitat visibly varied and the arrival tactile', () => {
  expect(HABITAT_PLANT_FORMS).toEqual(['canopy tree', 'broadleaf shrub', 'fern groundcover', 'climbing vine', 'flowering accent']);
  expect(HABITAT_DECOR.filter(placement => placement.asset === 'flowers').length).toBeGreaterThan(10);
  expect(HABITAT_SOLIDS.some(solid => solid.name === 'water garden')).toBe(true);
  expect(HABITAT_SOLIDS.some(solid => solid.name === 'commons table')).toBe(true);
});
