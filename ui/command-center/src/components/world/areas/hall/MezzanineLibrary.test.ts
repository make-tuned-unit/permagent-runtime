import { describe, expect, it } from 'vitest';
import {
  buildMezzanineBalusterLayout,
  buildStaircaseLayout,
  stairPointAt,
  STAIR,
} from './MezzanineLibrary';

function angleOf([x, , z]: [number, number, number]): number {
  return Math.atan2(z, x);
}

function angularDistance(a: number, b: number): number {
  let d = a - b;
  while (d > Math.PI) d -= Math.PI * 2;
  while (d < -Math.PI) d += Math.PI * 2;
  return Math.abs(d);
}

describe('mezzanine static batching layouts', () => {
  it('keeps every stair step/nosing and both rail sides', () => {
    const layout = buildStaircaseLayout();
    expect(layout.steps).toHaveLength(30);
    expect(layout.nosings).toHaveLength(30);
    expect(layout.railPosts).toHaveLength(20);
    expect(layout.steps[0]?.position[1]).toBeCloseTo(0.05);
    expect(layout.steps[layout.steps.length - 1]?.position[1]).toBeCloseTo(STAIR.height + 0.05);
    expect(layout.nosings[0]?.position[1]).toBeCloseTo(0.105);
    expect(layout.steps[0]?.rotation?.[1]).toBeCloseTo(-(
      STAIR.gapCenter - STAIR.arcSpan
    ) + Math.PI / 2);
    expect(layout.nosings[0]?.position[0]).toBeCloseTo(
      layout.steps[0]!.position[0] + Math.cos(STAIR.gapCenter - STAIR.arcSpan) * 0.31,
    );
    expect(layout.nosings[0]?.position[2]).toBeCloseTo(
      layout.steps[0]!.position[2] + Math.sin(STAIR.gapCenter - STAIR.arcSpan) * 0.31,
    );
    expect(layout.railPosts.filter((_, i) => i % 2 === 0)).toHaveLength(10);
    expect(layout.railPosts.filter((_, i) => i % 2 === 1)).toHaveLength(10);
  });

  it('keeps the outer baluster stair opening clear', () => {
    const balusters = buildMezzanineBalusterLayout();
    expect(balusters).toHaveLength(47);
    expect(balusters.every((t) =>
      angularDistance(angleOf(t.position), STAIR.gapCenter) >= 0.12
    )).toBe(true);
  });

  it('does not move the stair anchor endpoints', () => {
    const ground = stairPointAt(0);
    const top = stairPointAt(1);
    expect(Math.hypot(ground.x, ground.z)).toBeCloseTo(STAIR.radius);
    expect(ground.y).toBeCloseTo(0);
    expect(Math.hypot(top.x, top.z)).toBeCloseTo(STAIR.radius);
    expect(top.y).toBeCloseTo(STAIR.height);
  });
});
