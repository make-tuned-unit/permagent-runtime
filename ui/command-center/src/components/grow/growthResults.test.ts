import { describe, expect, it } from 'vitest';
import { ACTION_TITLE_CLAMP, LEGACY_TITLE_CAP, displayActionTitle, formatDeltaPct } from './growthResults';
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

describe('displayActionTitle (QA 2026-09-24: titles cut mid-word)', () => {
  it('trims a legacy 120-char cut back to a whole word and adds an ellipsis', () => {
    const legacy = "Rewrite top entry page (Celebrating India's Independence Day, 23 views, 22 entries) to eliminate single-event funnel and";
    expect(Array.from(legacy)).toHaveLength(LEGACY_TITLE_CAP);
    expect(displayActionTitle(legacy)).toBe(
      "Rewrite top entry page (Celebrating India's Independence Day, 23 views, 22 entries) to eliminate single-event funnel…",
    );
    const perplexit = 'Create and optimize structured answer content on a blog or FAQ page to capture answer-engine traffic (ChatGPT, Perplexit';
    expect(Array.from(perplexit)).toHaveLength(LEGACY_TITLE_CAP);
    expect(displayActionTitle(perplexit)).toBe(
      'Create and optimize structured answer content on a blog or FAQ page to capture answer-engine traffic (ChatGPT…',
    );
  });

  it('shows shorter or longer titles as they are', () => {
    expect(displayActionTitle('Fix event_save_failure')).toBe('Fix event_save_failure');
    const long = 'Create and optimize structured answer content on a blog or FAQ page to capture answer-engine traffic (ChatGPT, Perplexity)';
    expect(displayActionTitle(long)).toBe(long);
  });

  it('clamps at three lines with an ellipsis rather than hard-cutting', () => {
    expect(ACTION_TITLE_CLAMP.WebkitLineClamp).toBe(3);
    expect(ACTION_TITLE_CLAMP.overflow).toBe('hidden');
  });
});
