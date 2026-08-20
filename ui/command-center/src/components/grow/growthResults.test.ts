import { describe, expect, it } from 'vitest';
import { formatDeltaPct } from './growthResults';
import {
  lastCumulativeNet,
  sparklinePolyline,
  sparklinePoints,
  sparklineZeroY,
} from './growthTrend';

describe('formatDeltaPct', () => {
  it('renders a signed percent from a 0-1 fraction', () => {
    expect(formatDeltaPct(0.12)).toBe('+12%');
    expect(formatDeltaPct(-0.08)).toBe('-8%');
  });

  it('returns null when there is no delta', () => {
    expect(formatDeltaPct(null)).toBeNull();
    expect(formatDeltaPct(undefined)).toBeNull();
  });
});

describe('sparklinePolyline', () => {
  it('moves right and down as cumulative net rises then falls', () => {
    const pts = sparklinePoints([0, 1, 0], 100, 40, 0);
    expect(pts).toHaveLength(3);
    expect(pts[0].x).toBeLessThan(pts[1].x);
    expect(pts[1].x).toBeLessThan(pts[2].x);
    // Higher net sits higher on the chart (smaller y).
    expect(pts[1].y).toBeLessThan(pts[0].y);
    expect(pts[2].y).toBeGreaterThan(pts[1].y);
  });

  it('keeps a zero baseline when the series goes negative', () => {
    const y = sparklineZeroY([-1, 0, 1], 40, 0);
    expect(y).toBeCloseTo(20, 5);
  });

  it('emits an SVG point list', () => {
    expect(sparklinePolyline([0, 2], 10, 10, 0)).toMatch(/^\d/);
  });

  it('reads the last cumulative net', () => {
    expect(lastCumulativeNet([{ cumulativeNet: 1 }, { cumulativeNet: 3 }])).toBe(3);
    expect(lastCumulativeNet([])).toBe(0);
  });
});
