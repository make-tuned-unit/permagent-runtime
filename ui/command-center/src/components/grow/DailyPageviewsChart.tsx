import { useEffect, useRef, useState } from 'react';
import type { ThemeColors } from '../../styles/tokens';
import { font, radius, space, textSize } from '../../styles/tokens';
import type { DailyAnalyticsPoint } from './analyticsFormat';
import { linearTrendline } from './analyticsFormat';

/**
 * The drawing is laid out in CSS pixels, not in an arbitrary viewBox that is
 * then stretched to fit. The chart used to declare a fixed 720x190 viewBox,
 * render it at `width="100%"` (~1950px on a wide window) and 160px tall with
 * `preserveAspectRatio="none"`: that scales x by ~2.7 and y by ~0.84, so every
 * glyph inside the SVG came out wide, squat and soft — the "stretched and low
 * res" the axis labels showed. Measuring the container and drawing at that
 * width keeps the mapping 1:1, so text renders at its true size and the bars
 * keep their proportions at any window width.
 */
const FALLBACK_W = 720;
const VIEW_H = 172;
const PLOT_LEFT = 44;
const PLOT_RIGHT = 14;
const PLOT_TOP = 14;
const PLOT_BOTTOM = 34;

/** The element's content width in CSS pixels, tracked across resizes. */
function useMeasuredWidth<T extends HTMLElement>(fallback: number) {
  const ref = useRef<T | null>(null);
  const [width, setWidth] = useState(fallback);
  useEffect(() => {
    const node = ref.current;
    if (!node) return;
    // jsdom and older browsers have no ResizeObserver; the fallback width keeps
    // the chart renderable (and the tests deterministic) without it.
    if (typeof ResizeObserver === 'undefined') {
      const measured = node.clientWidth;
      if (measured > 0) setWidth(measured);
      return;
    }
    const observer = new ResizeObserver((entries) => {
      const measured = entries[0]?.contentRect.width ?? 0;
      // Round to whole pixels: a fractional viewBox width reintroduces the
      // sub-pixel blur this change exists to remove.
      if (measured > 0) setWidth(Math.round(measured));
    });
    observer.observe(node);
    return () => observer.disconnect();
  }, []);
  return { ref, width };
}

function shortDay(day: string): string {
  const [, month, date] = day.split('-');
  return month && date ? `${month}/${date}` : day;
}

/** Responsive, accessible first-party pageview history. */
export function DailyPageviewsChart({
  days,
  colors,
}: {
  days: readonly DailyAnalyticsPoint[];
  colors: ThemeColors;
}) {
  const { ref: frameRef, width: viewW } = useMeasuredWidth<HTMLDivElement>(FALLBACK_W);
  if (days.length === 0) {
    return (
      <section
        aria-label="Pageviews by day"
        data-testid="daily-pageviews-chart"
        style={{
          border: `1px solid ${colors.border}`,
          borderRadius: radius.md,
          padding: `${space.xl}px ${space.xxl}px`,
          color: colors.textDim,
          fontSize: textSize.micro,
        }}
      >
        No daily pageview data is available for this period.
      </section>
    );
  }

  const values = days.map((day) => Math.max(0, day.pageviews));
  const max = Math.max(1, ...values);
  const plotW = Math.max(120, viewW - PLOT_LEFT - PLOT_RIGHT);
  const plotH = VIEW_H - PLOT_TOP - PLOT_BOTTOM;
  const step = plotW / days.length;
  const barWidth = Math.max(2, step * 0.72);
  const yFor = (value: number) => PLOT_TOP + plotH - (Math.max(0, Math.min(max, value)) / max) * plotH;
  const trend = linearTrendline(values);
  const labelEvery = Math.max(1, Math.ceil(days.length / 6));

  return (
    <section
      aria-label="Pageviews by day"
      data-testid="daily-pageviews-chart"
      style={{
        border: `1px solid ${colors.border}`,
        borderRadius: radius.md,
        padding: `${space.lg}px ${space.xl}px ${space.md}px`,
        minWidth: 0,
      }}
    >
      <div style={{ display: 'flex', alignItems: 'baseline', justifyContent: 'space-between', gap: space.md, flexWrap: 'wrap', marginBottom: space.xxs }}>
        <div style={{ fontFamily: font.mono, fontSize: textSize.micro, color: colors.textDim, letterSpacing: '0.08em', textTransform: 'uppercase' }}>
          Pageviews by day
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: space.lg, flexWrap: 'wrap', fontSize: textSize.micro, color: colors.textDim }}>
          <span style={{ display: 'inline-flex', alignItems: 'center', gap: space.xs }}>
            {/* A 10x3 legend dash: `pill` clamps to the half-height, which is
                the fully-rounded cap this wants. A scale step would be bigger
                than the swatch is tall. */}
            <span aria-hidden="true" style={{ width: 10, height: 3, borderRadius: radius.pill, background: colors.cyan, display: 'inline-block' }} />
            Daily pageviews
          </span>
          <span style={{ display: 'inline-flex', alignItems: 'center', gap: space.xs }}>
            <span aria-hidden="true" style={{ width: 10, borderTop: `2px dashed ${colors.warning}`, display: 'inline-block' }} />
            Trendline
          </span>
        </div>
      </div>
      <div ref={frameRef} style={{ width: '100%', minWidth: 0 }}>
      <svg
        viewBox={`0 0 ${viewW} ${VIEW_H}`}
        width={viewW}
        height={VIEW_H}
        role="img"
        aria-label={`Daily pageviews over ${days.length} days, ${values.reduce((sum, value) => sum + value, 0).toLocaleString()} total`}
        style={{ display: 'block', maxWidth: '100%' }}
      >
        {[0, 0.5, 1].map((fraction) => {
          const y = PLOT_TOP + plotH * fraction;
          const value = Math.round(max * (1 - fraction));
          return (
            <g key={fraction}>
              <line x1={PLOT_LEFT} x2={viewW - PLOT_RIGHT} y1={y} y2={y} stroke={colors.border} strokeWidth={1} vectorEffect="non-scaling-stroke" />
              <text x={PLOT_LEFT - 6} y={y + 3} textAnchor="end" fontSize={textSize.micro} fill={colors.textDim}>{value}</text>
            </g>
          );
        })}
        {days.map((day, index) => {
          const x = PLOT_LEFT + index * step + (step - barWidth) / 2;
          const y = yFor(values[index]);
          const height = Math.max(0, PLOT_TOP + plotH - y);
          return (
            <g key={day.day} data-testid="daily-pageviews-bar">
              <rect x={x} y={y} width={barWidth} height={height} rx={1} fill={colors.cyan} opacity={values[index] > 0 ? 0.9 : 0.22}>
                <title>{`${day.day}: ${values[index].toLocaleString()} pageviews · ${day.visitors.toLocaleString()} devices`}</title>
              </rect>
              {(index % labelEvery === 0 || index === days.length - 1) && (
                <text x={x + barWidth / 2} y={VIEW_H - 10} textAnchor="middle" fontSize={textSize.micro} fill={colors.textDim}>{shortDay(day.day)}</text>
              )}
            </g>
          );
        })}
        {trend && (
          <line
            data-testid="daily-pageviews-trendline"
            x1={PLOT_LEFT + step / 2}
            x2={PLOT_LEFT + plotW - step / 2}
            y1={yFor(trend.start)}
            y2={yFor(trend.end)}
            stroke={colors.warning}
            strokeWidth={2}
            strokeDasharray="5 4"
            vectorEffect="non-scaling-stroke"
          />
        )}
      </svg>
      </div>
      <div style={{ color: colors.textDim, fontSize: textSize.micro, lineHeight: 1.4 }}>
        Trendline shows the overall direction; hover a bar for the exact day.
      </div>
    </section>
  );
}
