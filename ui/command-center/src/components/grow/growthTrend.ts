/**
 * Sparkline geometry for the growth trend.
 *
 * The series is already padded to 12 weeks on the server. Plotting only
 * weeks that have a verdict would stretch two spikes into a solid block —
 * the same failure the analytics daily chart had with `byDay`.
 */

export function sparklinePoints(
  values: number[],
  width: number,
  height: number,
  pad = 2,
): { x: number; y: number }[] {
  if (values.length === 0) return [];
  const min = Math.min(0, ...values);
  const max = Math.max(0, ...values);
  const span = max - min || 1;
  const innerW = Math.max(1, width - pad * 2);
  const innerH = Math.max(1, height - pad * 2);
  const last = values.length - 1;
  return values.map((v, i) => ({
    x: pad + (last === 0 ? innerW / 2 : (i / last) * innerW),
    y: pad + innerH - ((v - min) / span) * innerH,
  }));
}

export function sparklinePolyline(
  values: number[],
  width: number,
  height: number,
  pad = 2,
): string {
  return sparklinePoints(values, width, height, pad)
    .map((p) => `${p.x.toFixed(1)},${p.y.toFixed(1)}`)
    .join(' ');
}

/** Y of the zero baseline, or null when 0 is not inside the range. */
export function sparklineZeroY(
  values: number[],
  height: number,
  pad = 2,
): number | null {
  if (values.length === 0) return null;
  const min = Math.min(0, ...values);
  const max = Math.max(0, ...values);
  if (min === max) return pad + Math.max(1, height - pad * 2);
  const span = max - min;
  const innerH = Math.max(1, height - pad * 2);
  return pad + innerH - ((0 - min) / span) * innerH;
}

export function formatWeekLabel(week: string): string {
  const d = new Date(`${week}T00:00:00Z`);
  if (Number.isNaN(d.getTime())) return week;
  return d.toLocaleDateString('en-US', { month: 'short', day: 'numeric', timeZone: 'UTC' });
}

export function lastCumulativeNet(points: { cumulativeNet: number }[]): number {
  return points.length ? points[points.length - 1].cumulativeNet : 0;
}
