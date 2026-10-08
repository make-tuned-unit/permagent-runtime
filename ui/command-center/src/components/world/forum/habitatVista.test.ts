import { expect, it } from 'vitest';
import { Vector3 } from 'three';
import { existsSync } from 'node:fs';
import { resolve } from 'node:path';
import {
  HABITAT_DAY_PANORAMA_URLS,
  HABITAT_HORIZON_CARDS,
  HABITAT_SUN_DIR,
  HABITAT_PANORAMA_ROTATION_Y,
  HABITAT_PANORAMA_SUN_U,
  HABITAT_VISTA_ASSETS,
  horizonCardOutsideWalkBounds,
} from './habitatVista';

it('points day panorama loading at the Codex FAL drop paths with a smaller fallback', () => {
  expect(HABITAT_VISTA_ASSETS.panoramaPrimary).toBe('world/vista/golden-hour-panorama-4096.jpg');
  expect(HABITAT_VISTA_ASSETS.panoramaFallback).toBe('world/vista/golden-hour-panorama-2048.jpg');
  expect(HABITAT_DAY_PANORAMA_URLS[0]).toContain('golden-hour-panorama-4096.jpg');
  expect(HABITAT_DAY_PANORAMA_URLS[1]).toContain('golden-hour-panorama-2048.jpg');
});

it('keeps every horizon card outside walkable bounds and solid footprints', () => {
  expect(HABITAT_HORIZON_CARDS.length).toBeGreaterThan(0);
  for (const card of HABITAT_HORIZON_CARDS) {
    expect(horizonCardOutsideWalkBounds(card), card.id).toBe(true);
    expect(Math.hypot(card.position[0], card.position[2])).toBeGreaterThan(20);
  }
});

it('uses a low warm sun bearing for golden-hour lighting alignment', () => {
  expect(HABITAT_SUN_DIR.length()).toBeCloseTo(1, 6);
  const azimuth = HABITAT_PANORAMA_SUN_U * Math.PI * 2;
  const elevation = (0.5 - 0.470) * Math.PI; // measured source sun centre
  const visibleSun = new Vector3(-Math.cos(azimuth) * Math.cos(elevation), Math.sin(elevation), Math.sin(azimuth) * Math.cos(elevation));
  visibleSun.applyAxisAngle(new Vector3(0, 1, 0), HABITAT_PANORAMA_ROTATION_Y);
  expect(visibleSun.dot(HABITAT_SUN_DIR)).toBeGreaterThan(0.99999);
  expect(HABITAT_SUN_DIR.x).toBeLessThan(-0.8);
});

it('names every card texture under world/vista/cards/', () => {
  for (const card of HABITAT_HORIZON_CARDS) {
    const path = HABITAT_VISTA_ASSETS.cards[card.kind];
    expect(path.startsWith('world/vista/cards/')).toBe(true);
    expect(path.endsWith('.png')).toBe(true);
    expect(existsSync(resolve(process.cwd(), 'public', path)), path).toBe(true);
  }
});
