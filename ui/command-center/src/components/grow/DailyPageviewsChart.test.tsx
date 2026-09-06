// @vitest-environment jsdom

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { DailyPageviewsChart } from './DailyPageviewsChart';
import type { DailyAnalyticsPoint } from './analyticsFormat';
import { textSize } from '../../styles/tokens';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const colors = {
  border: '#334155',
  text: '#f8fafc',
  textDim: '#94a3b8',
  cyan: '#22d3ee',
  warning: '#fbbf24',
} as never;

const days: DailyAnalyticsPoint[] = [
  { day: '2026-09-01', pageviews: 2, visitors: 1 },
  { day: '2026-09-02', pageviews: 0, visitors: 0 },
  { day: '2026-09-03', pageviews: 8, visitors: 4 },
];

describe('DailyPageviewsChart', () => {
  let root: Root;
  let container: HTMLDivElement;

  beforeEach(() => {
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
  });

  it('renders one bar per UTC day and an explanatory trendline', async () => {
    await act(async () => root.render(<DailyPageviewsChart days={days} colors={colors} />));
    expect(container.querySelectorAll('[data-testid="daily-pageviews-bar"]')).toHaveLength(3);
    expect(container.querySelector('[data-testid="daily-pageviews-trendline"]')).toBeTruthy();
    expect(container.textContent).toContain('Pageviews by day');
    expect(container.textContent).toContain('Trendline');
    expect(container.querySelector('svg')?.getAttribute('role')).toBe('img');
  });

  it('renders an honest empty state rather than a blank chart', async () => {
    await act(async () => root.render(<DailyPageviewsChart days={[]} colors={colors} />));
    expect(container.textContent).toContain('No daily pageview data is available');
    expect(container.querySelector('[data-testid="daily-pageviews-trendline"]')).toBeNull();
  });

  it('uses the existing readable type ramp for SVG ticks and HTML annotations', async () => {
    await act(async () => root.render(<DailyPageviewsChart days={days} colors={colors} />));
    for (const tick of container.querySelectorAll('svg text')) {
      expect(tick.getAttribute('font-size')).toBe(String(textSize.micro));
    }
    const annotation = Array.from(container.querySelectorAll('div')).find(element =>
      element.textContent === 'Trendline shows the overall direction; hover a bar for the exact day.');
    expect(annotation?.style.fontSize).toBe(`${textSize.micro}px`);
  });
});
