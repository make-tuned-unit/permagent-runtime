import { font, space, textSize } from '../../styles/tokens';
import type { ThemeColors } from '../../styles/tokens';
import type { GrowthTrendPoint } from './growthResults';
import {
  formatWeekLabel,
  lastCumulativeNet,
  sparklinePolyline,
  sparklineZeroY,
} from './growthTrend';

const VIEW_W = 240;
const VIEW_H = 56;

export function GrowthSparkline({
  points,
  colors,
  height = VIEW_H,
  stroke,
  showAxis = true,
}: {
  points: GrowthTrendPoint[];
  colors: ThemeColors;
  height?: number;
  stroke?: string;
  showAxis?: boolean;
}) {
  const values = points.map((p) => p.cumulativeNet);
  const poly = sparklinePolyline(values, VIEW_W, VIEW_H);
  const zeroY = sparklineZeroY(values, VIEW_H);
  const first = points[0]?.week;
  const last = points[points.length - 1]?.week;
  const net = lastCumulativeNet(points);
  const netLabel = net > 0 ? `+${net}` : String(net);

  return (
    <div>
      <svg
        viewBox={`0 0 ${VIEW_W} ${VIEW_H}`}
        preserveAspectRatio="none"
        width="100%"
        height={height}
        role="img"
        aria-label={`Cumulative helped minus hindered, last ${points.length} weeks, now ${netLabel}`}
      >
        {zeroY != null && (
          <line
            x1={0} x2={VIEW_W} y1={zeroY} y2={zeroY}
            stroke={colors.border} strokeWidth={1} vectorEffect="non-scaling-stroke"
          />
        )}
        {poly && (
          <polyline
            points={poly}
            fill="none"
            stroke={stroke ?? colors.cyan}
            strokeWidth={1.75}
            strokeLinejoin="round"
            strokeLinecap="round"
            vectorEffect="non-scaling-stroke"
          />
        )}
      </svg>
      {showAxis && first && last && (
        <div style={{
          display: 'flex', justifyContent: 'space-between',
          fontFamily: font.mono, fontSize: textSize.micro, color: colors.textDim, marginTop: space.xs / 2,
        }}>
          <span>{formatWeekLabel(first)}</span>
          <span>net {netLabel}</span>
          <span>{formatWeekLabel(last)}</span>
        </div>
      )}
    </div>
  );
}
