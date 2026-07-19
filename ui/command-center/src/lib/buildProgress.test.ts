import { describe, it, expect } from 'vitest';
import { progressRailStep } from './buildProgress';

// Regression for the Build progress-rail fix (2026-07 wiring audit): the rail
// used to be hardcoded to step 3 for any running task. It now tracks the
// daemon's real per-task progress (dashboard in_flight[].progress, 0..0.95).
describe('progressRailStep', () => {
  it('is dark (0) when nothing is in flight', () => {
    expect(progressRailStep(null)).toBe(0);
    expect(progressRailStep(undefined)).toBe(0);
    expect(progressRailStep(0)).toBe(0);
  });

  it('lights at least the first segment for any started task', () => {
    expect(progressRailStep(0.01)).toBe(1);
    expect(progressRailStep(0.2)).toBe(1);
  });

  it('maps progress across the five segments', () => {
    expect(progressRailStep(0.3)).toBe(2);
    expect(progressRailStep(0.5)).toBe(3);
    expect(progressRailStep(0.7)).toBe(4);
    // the daemon caps in-flight progress at 0.95 — still segment 5
    expect(progressRailStep(0.95)).toBe(5);
  });

  it('never exceeds 5 even at full progress', () => {
    expect(progressRailStep(1)).toBe(5);
    expect(progressRailStep(2)).toBe(5);
  });

  it('is not the old constant — early and late tasks differ', () => {
    expect(progressRailStep(0.1)).not.toBe(progressRailStep(0.9));
  });
});
