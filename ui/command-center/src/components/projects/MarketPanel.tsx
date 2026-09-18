import { useCallback, useEffect, useRef, useState, type CSSProperties } from 'react';
import { apiFetch } from '../../lib/api';
import { font, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import { sparklinePolyline } from '../grow/growthTrend';
import { Panel } from './Panel';
import type { Project } from './types';

import { Tooltip } from '../common/Tooltip';
/**
 * The Market card.
 *
 * Sits directly beneath the Ecosystem panel because the two are one concept:
 * every series here hangs off a `project_intel` row that panel already shows.
 * One concept, one place — not a tab, not a Dashboard copy.
 *
 * The rule this component exists to keep: **the method label is always
 * visible.** A forecast without its method reads as better than it is, so the
 * label renders next to every number, and a series that cannot be forecast
 * renders the reason ("42 of 180 points") rather than an empty chart.
 */

type Verdict = 'forecastable' | 'not_bound' | 'collector_stale' | 'insufficient_history';

interface MarketForecast {
  point: number[];
  p10: number[];
  p90: number[];
  method: string;
  methodLabel: string;
  maseVsBaseline: number | null;
  folds: number;
  foldWins: number;
  selection: string;
}

interface MarketRefusal {
  reason: 'insufficient_history' | 'collector_stale' | 'not_bound' | 'no_method_beats_baseline';
  points?: number;
  needed?: number;
  lastCollectedAt?: string | null;
  detail?: string;
}

interface MarketRow {
  seriesId: string;
  sourceKind: string;
  sourceLabel: string;
  subject: string;
  subjectGroup: string | null;
  cadence: 'daily' | 'weekly';
  label: string;
  status: 'proposed' | 'active' | 'dismissed';
  points: number;
  spanDays: number;
  snapshotOnly: boolean;
  officialSource: boolean;
  lastError: string | null;
  verdict: Verdict;
  needed?: number;
  history: number[];
  forecast: MarketForecast | null;
  refusal: MarketRefusal | null;
  direction: string | null;
}

interface MarketResponse {
  rows: MarketRow[];
  noSeriesBound: boolean;
  generatedAt: string;
}

const VIEW_W = 240;
const VIEW_H = 34;

function refusalText(row: MarketRow): string {
  const r = row.refusal;
  if (!r) return 'No forecast.';
  switch (r.reason) {
    case 'insufficient_history':
      return `${r.points ?? row.points} of ${r.needed ?? row.needed ?? '?'} points — too short to forecast`;
    case 'collector_stale':
      // "Collector stale" was the daemon's word for it, rendered straight
      // through. A collector is the job that gathers this series; the sentence
      // now says that, and says what it means for the forecast.
      return r.lastCollectedAt
        ? `No recent data — the job that gathers this series last ran ${new Date(r.lastCollectedAt).toLocaleDateString()}`
        : 'No data yet — nothing has gathered this series';
    case 'not_bound':
      return 'Awaiting approval — nothing is collected yet';
    case 'no_method_beats_baseline':
      return r.detail ?? 'No method could be scored on this series';
    default:
      return 'No forecast.';
  }
}

export function MarketPanel({ project }: { project: Project }) {
  const { colors } = useTheme();
  const [data, setData] = useState<MarketResponse | null>(null);
  const [status, setStatus] = useState<'loading' | 'ready' | 'error'>('loading');
  const loadGeneration = useRef(0);

  // Resolves `false` when the load failed (or was superseded) so the retry
  // button's success tick can only fire over a load that actually landed.
  const load = useCallback(async () => {
    const generation = ++loadGeneration.current;
    setStatus('loading');
    try {
      const response = await apiFetch<MarketResponse>(
        `/api/projects/${encodeURIComponent(project.id)}/market`,
      );
      if (generation !== loadGeneration.current) return false;
      if (!response || !Array.isArray(response.rows)) throw new Error('Invalid market response');
      setData(response);
      setStatus('ready');
      return true;
    } catch {
      if (generation !== loadGeneration.current) return false;
      setStatus('error');
      return false;
    }
  }, [project.id]);

  useEffect(() => { load(); }, [load]);

  return (
    <Panel title="Market">
      {status === 'loading' && <div style={{ color: colors.textDim, fontSize: textSize.micro }}>Loading market series…</div>}
      {status === 'error' && (
        <Button
          colors={colors}
          variant="bare"
          type="button"
          className="hover:underline"
          onClick={load}
          style={{
            '--pa-btn-fg': colors.danger,
            '--pa-btn-bg-hover': 'transparent',
            '--pa-btn-pad': '0',
            '--pa-btn-weight': 'inherit',
            fontSize: 'inherit',
            lineHeight: 'inherit',
          } as CSSProperties}
        >
          Couldn't load market series. Retry
        </Button>
      )}
      {/* Nothing bound is not a flat market. Say which. */}
      {status === 'ready' && data?.noSeriesBound && (
        <div style={{ color: colors.textDim, fontSize: textSize.micro, lineHeight: 1.5 }}>
          No market series bound. Nothing here is a forecast of zero — there is simply nothing
          to forecast yet. Ask the Forecaster to bind a competitor's package, a category's
          Wikipedia article, or a Hacker News query.
        </div>
      )}
      {status === 'ready' && !data?.noSeriesBound && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: space.xl }}>
          {data?.rows.map(row => {
            const poly = row.history.length > 1 ? sparklinePolyline(row.history, VIEW_W, VIEW_H) : null;
            const forecastable = row.verdict === 'forecastable' && row.forecast;
            return (
              <div key={row.seriesId} style={{ borderLeft: `2px solid ${forecastable ? colors.cyan : colors.border}`, paddingLeft: space.md }}>
                <div style={{ display: 'flex', alignItems: 'baseline', justifyContent: 'space-between', gap: space.md }}>
                  <div style={{ color: colors.text, fontSize: textSize.caption, fontWeight: 600 }}>{row.subject}</div>
                  <div style={{ color: colors.textMuted, fontSize: 10 }}>{row.sourceLabel}</div>
                </div>

                {poly && (
                  <svg
                    viewBox={`0 0 ${VIEW_W} ${VIEW_H}`}
                    preserveAspectRatio="none"
                    width="100%"
                    height={VIEW_H}
                    role="img"
                    aria-label={`${row.subject} ${row.sourceLabel}, last ${row.history.length} points`}
                    style={{ marginTop: space.xs }}
                  >
                    <polyline
                      points={poly}
                      fill="none"
                      stroke={forecastable ? colors.cyan : colors.textDim}
                      strokeWidth={1.5}
                      strokeLinejoin="round"
                      strokeLinecap="round"
                      vectorEffect="non-scaling-stroke"
                    />
                  </svg>
                )}

                {forecastable && row.forecast ? (
                  <>
                    <div style={{ color: colors.text, fontSize: textSize.micro, marginTop: space.xxs }}>
                      {row.direction ?? '—'}
                      {row.forecast.p10.length > 0 && (
                        <span style={{ color: colors.textMuted }}>
                          {' '}· 80% range {row.forecast.p10[row.forecast.p10.length - 1].toFixed(0)}–
                          {row.forecast.p90[row.forecast.p90.length - 1].toFixed(0)}
                        </span>
                      )}
                    </div>
                    {/* The method label is never optional and never hidden. */}
                    <div style={{ fontFamily: font.mono, fontSize: 9, color: colors.textDim, marginTop: space.xxs }}>
                      method: {row.forecast.methodLabel}
                      {row.forecast.maseVsBaseline != null && (
                        <> · MASE {row.forecast.maseVsBaseline.toFixed(2)}× baseline over {row.forecast.folds} folds</>
                      )}
                    </div>
                  </>
                ) : (
                  <div style={{ color: colors.textMuted, fontSize: textSize.micro, marginTop: space.xxs }}>
                    {refusalText(row)}
                  </div>
                )}

                <div style={{ display: 'flex', gap: space.md, alignItems: 'center', marginTop: space.xxs, fontFamily: font.mono, fontSize: 9, color: colors.textDim }}>
                  <span>{row.points} pts · {row.cadence}</span>
                  {row.snapshotOnly && <Tooltip content="This source cannot hand over history; it accumulates one point per sweep."><span tabIndex={0} style={{ outline: 'none' }}><span>snapshot-only</span></span></Tooltip>}
                  {!row.officialSource && <Tooltip content="Not a supported API; used anyway, and said out loud."><span tabIndex={0} style={{ outline: 'none' }}><span>unofficial source</span></span></Tooltip>}
                  {row.lastError && <Tooltip content={row.lastError}><span tabIndex={0} style={{ outline: 'none' }}><span style={{ color: colors.danger }}>collector error</span></span></Tooltip>}
                </div>
              </div>
            );
          })}
        </div>
      )}
    </Panel>
  );
}
