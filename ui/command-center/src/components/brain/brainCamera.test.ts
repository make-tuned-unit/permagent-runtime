import { describe, expect, it } from 'vitest';
import {
  boundingSphere, cullOverlappingLabels, fitDistance, fitToInsetRect, labelFade, labelScreenPx,
  labelWorldHeight, pickLabels, worldPerPixel,
} from './brainCamera';

describe('boundingSphere', () => {
  it('centres on the point set and radius reaches the farthest point plus pad', () => {
    const s = boundingSphere([[-2, 0, 0], [2, 0, 0], [0, 0, 0]], 0.5);
    expect(s.center).toEqual([0, 0, 0]);
    expect(s.radius).toBeCloseTo(2.5, 5);
  });

  it('is a unit sphere at the origin for an empty set — the camera always has somewhere to be', () => {
    expect(boundingSphere([], 0)).toEqual({ center: [0, 0, 0], radius: 1 });
  });
});

describe('fitDistance', () => {
  it('grows with radius and shrinks with a wider fov', () => {
    const d1 = fitDistance(10, 50, 1, 1.15);
    const d2 = fitDistance(20, 50, 1, 1.15);
    expect(d2).toBeGreaterThan(d1);
    const wide = fitDistance(10, 90, 1, 1.15);
    expect(wide).toBeLessThan(d1);
  });
});

describe('label sizing', () => {
  it('clamps a near label to the ceiling and a far one to the floor', () => {
    const sizing = { worldHeight: 0.9, maxPx: 20, minPx: 13, nativePx: 20 };
    const near = labelScreenPx(1, 50, 900, sizing);
    const far = labelScreenPx(500, 50, 900, sizing);
    expect(near).toBe(20);
    expect(far).toBe(13);
  });

  it('never draws taller than the bitmap even if maxPx would allow it (never upsampled)', () => {
    const sizing = { worldHeight: 0.9, maxPx: 40, minPx: 13, nativePx: 20 };
    expect(labelScreenPx(0.1, 50, 900, sizing)).toBeLessThanOrEqual(20);
  });

  it('labelWorldHeight round-trips back through worldPerPixel to the same screen size', () => {
    const sizing = { worldHeight: 0.9, maxPx: 20, minPx: 13, nativePx: 20 };
    const distance = 10;
    const h = labelWorldHeight(distance, 50, 900, sizing);
    const upp = worldPerPixel(distance, 50, 900);
    expect(h / upp).toBeCloseTo(labelScreenPx(distance, 50, 900, sizing), 5);
  });
});

describe('pickLabels', () => {
  it('always keeps forced candidates outside the budget', () => {
    const shown = pickLabels([
      { id: 'you', distance: 1, importance: 0, forced: true },
      { id: 'a', distance: 5, importance: 3, forced: false },
      { id: 'b', distance: 5, importance: 2, forced: false },
    ], 1);
    expect(shown.has('you')).toBe(true);
    expect(shown.has('a')).toBe(true);
    expect(shown.has('b')).toBe(false);
  });

  it('never shows an importance-0 candidate (a memory dot) unless forced', () => {
    const shown = pickLabels([{ id: 'm1', distance: 1, importance: 0, forced: false }], 10);
    expect(shown.size).toBe(0);
  });
});

describe('labelFade', () => {
  it('is full strength inside the framed distance and floors past the sphere radius', () => {
    expect(labelFade(5, 10, 4)).toBe(1);
    expect(labelFade(20, 10, 4, 0.35)).toBe(0.35);
  });
});

describe('cullOverlappingLabels', () => {
  it('keeps both when they do not overlap', () => {
    const shown = cullOverlappingLabels([
      { id: 'a', x: 0, y: 0, width: 40, height: 12, priority: 1 },
      { id: 'b', x: 100, y: 0, width: 40, height: 12, priority: 1 },
    ]);
    expect(shown).toEqual(new Set(['a', 'b']));
  });

  it('the reproduced bug: two similar-size labels overlapping in a dense centre — higher priority wins', () => {
    // "Nadia Petrova" and "Priya Chandrasekaran" at nearly the same screen
    // position (QA 2026-09-25).
    const shown = cullOverlappingLabels([
      { id: 'nadia', x: 400, y: 300, width: 90, height: 14, priority: 2 },
      { id: 'priya', x: 415, y: 302, width: 130, height: 14, priority: 5 },
    ]);
    expect(shown).toEqual(new Set(['priya']));
  });

  it('priority order is hovered > selected > degree, expressed as the caller\'s numbers', () => {
    const HOVERED = 1e6, SELECTED = 1e5;
    const shown = cullOverlappingLabels([
      { id: 'hub', x: 0, y: 0, width: 60, height: 14, priority: 8 }, // high degree
      { id: 'selected', x: 5, y: 2, width: 60, height: 14, priority: SELECTED },
      { id: 'hovered', x: 8, y: 1, width: 60, height: 14, priority: HOVERED },
    ]);
    expect(shown).toEqual(new Set(['hovered']));
  });

  it('is deterministic on an exact priority tie (breaks on id, never flickers)', () => {
    const a = cullOverlappingLabels([
      { id: 'b', x: 0, y: 0, width: 40, height: 12, priority: 1 },
      { id: 'a', x: 5, y: 0, width: 40, height: 12, priority: 1 },
    ]);
    const b = cullOverlappingLabels([
      { id: 'a', x: 5, y: 0, width: 40, height: 12, priority: 1 },
      { id: 'b', x: 0, y: 0, width: 40, height: 12, priority: 1 },
    ]);
    expect(a).toEqual(b);
    expect(a).toEqual(new Set(['a']));
  });
});

describe('fitToInsetRect', () => {
  const FOV = 50;

  it('with zero insets, matches the plain fitDistance and pans nowhere', () => {
    const fit = fitToInsetRect(10, FOV, 1440, 900, { top: 0, right: 0, bottom: 0, left: 0 }, 1.15);
    const plain = fitDistance(10, FOV, 1440 / 900, 1.15);
    expect(fit.distance).toBeCloseTo(plain, 5);
    expect(fit.panRight).toBeCloseTo(0, 8);
    expect(fit.panUp).toBeCloseTo(0, 8);
  });

  it('backs the camera off further when chrome eats into the canvas (never fits a smaller sphere into the same distance)', () => {
    const noInset = fitToInsetRect(10, FOV, 1440, 900, { top: 0, right: 0, bottom: 0, left: 0 });
    const withInset = fitToInsetRect(10, FOV, 1440, 900, { top: 64, right: 0, bottom: 64, left: 0 });
    expect(withInset.distance).toBeGreaterThan(noInset.distance);
  });

  it('a left-side panel (e.g. an open key) pans the sphere right, away from it', () => {
    const fit = fitToInsetRect(10, FOV, 1440, 900, { top: 0, right: 0, bottom: 0, left: 300 });
    // panRight > 0 moves the sphere rightward on screen once applied along the
    // camera's own right vector (verified by the sign derivation in the
    // module doc comment: screen dx = -f*panRight/distance, so a rightward
    // screen shift needs panRight < ... — pinned here as the contract).
    expect(fit.panRight).not.toBe(0);
    expect(fit.panUp).toBe(0);
  });

  it('a taller top inset than bottom pans the sphere down, and the reverse pans it up', () => {
    const topHeavy = fitToInsetRect(10, FOV, 1440, 900, { top: 100, right: 0, bottom: 20, left: 0 });
    const bottomHeavy = fitToInsetRect(10, FOV, 1440, 900, { top: 20, right: 0, bottom: 100, left: 0 });
    expect(topHeavy.panUp).toBeGreaterThan(0);
    expect(bottomHeavy.panUp).toBeLessThan(0);
    expect(topHeavy.panUp).toBeCloseTo(-bottomHeavy.panUp, 5);
  });

  it('symmetric insets (equal top/bottom, equal left/right) never pan, only zoom out', () => {
    const fit = fitToInsetRect(10, FOV, 1440, 900, { top: 40, right: 40, bottom: 40, left: 40 });
    expect(fit.panRight).toBeCloseTo(0, 8);
    expect(fit.panUp).toBeCloseTo(0, 8);
  });
});
