/**
 * @vitest-environment jsdom
 */
import { describe, expect, it } from 'vitest';
import { placeViewportTooltip } from './tooltipPlacement';

describe('tooltipPlacement (unit)', () => {
  it('clamps top/bottom midpoints inside the viewport', () => {
    const viewport = { width: 200, height: 200 };
    const nearLeft = { x: 0, y: 80, width: 20, height: 20 };
    const p = placeViewportTooltip(nearLeft, 'top', 8, viewport);
    expect(p.left).toBeGreaterThanOrEqual(8);
    expect(p.transform).toContain('translate(-50%');
  });
});
