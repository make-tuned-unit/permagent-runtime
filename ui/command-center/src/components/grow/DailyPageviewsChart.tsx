import type { ThemeColors } from '../../styles/tokens';
import { font, radius, textSize } from '../../styles/tokens';
import type { DailyAnalyticsPoint } from './analyticsFormat';
import { linearTrendline } from './analyticsFormat';

const VIEW_W = 720;
const VIEW_H = 190;
const PLOT_LEFT = 38;
const PLOT_RIGHT = 12;
const PLOT_TOP = 14;
const PLOT_BOTTOM = 34;

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
  if (days.length === 0) {
    return (
      <section
        aria-label="Pageviews by day"
        data-testid="daily-pageviews-chart"
        style={{
          border: `1px solid ${colors.border}`,
          borderRadius: radius.md,
          padding: '12px 14px',
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
  const plotW = VIEW_W - PLOT_LEFT - PLOT_RIGHT;
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
        padding: '10px 12px 8px',
        minWidth: 0,
      }}
    >
      <div style={{ display: 'flex', alignItems: 'baseline', justifyContent: 'space-between', gap: 8, flexWrap: 'wrap', marginBottom: 2 }}>
        <div style={{ fontFamily: font.mono, fontSize: textSize.micro, color: colors.textDim, letterSpacing: '0.08em', textTransform: 'uppercase' }}>
          Pageviews by day
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, flexWrap: 'wrap', fontSize: textSize.micro, color: colors.textDim }}>
          <span style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}>
            <span aria-hidden="true" style={{ width: 10, height: 3, borderRadius: 2, background: colors.cyan, display: 'inline-block' }} />
            Daily pageviews
          </span>
          <span style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}>
            <span aria-hidden="true" style={{ width: 10, borderTop: `2px dashed ${colors.warning}`, display: 'inline-block' }} />
            Trendline
          </span>
        </div>
      </div>
      <svg
        viewBox={`0 0 ${VIEW_W} ${VIEW_H}`}
        width="100%"
        height={160}
        role="img"
        aria-label={`Daily pageviews over ${days.length} days, ${values.reduce((sum, value) => sum + value, 0).toLocaleString()} total`}
        preserveAspectRatio="none"
      >
        {[0, 0.5, 1].map((fraction) => {
          const y = PLOT_TOP + plotH * fraction;
          const value = Math.round(max * (1 - fraction));
          return (
            <g key={fraction}>
              <line x1={PLOT_LEFT} x2={VIEW_W - PLOT_RIGHT} y1={y} y2={y} stroke={colors.border} strokeWidth={1} vectorEffect="non-scaling-stroke" />
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
                <text x={x + barWidth / 2} y={VIEW_H - 12} textAnchor="middle" fontSize={textSize.micro} fill={colors.textDim}>{shortDay(day.day)}</text>
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
      <div style={{ color: colors.textDim, fontSize: textSize.micro, lineHeight: 1.4 }}>
        Trendline shows the overall direction; hover a bar for the exact day.
      </div>
    </section>
  );
}
