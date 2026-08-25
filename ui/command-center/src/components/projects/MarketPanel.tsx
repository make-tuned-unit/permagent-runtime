import { useCallback, useEffect, useRef, useState } from 'react';
import { apiFetch } from '../../lib/api';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { sparklinePolyline } from '../grow/growthTrend';
import { Panel } from './Panel';
import type { Project } from './types';

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
      return r.lastCollectedAt
        ? `Collector stale — last ran ${new Date(r.lastCollectedAt).toLocaleDateString()}`
        : 'Never collected';
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

  const load = useCallback(async () => {
    const generation = ++loadGeneration.current;
    setStatus('loading');
    try {
      const response = await apiFetch<MarketResponse>(
        `/api/projects/${encodeURIComponent(project.id)}/market`,
      );
      if (generation !== loadGeneration.current) return;
      if (!response || !Array.isArray(response.rows)) throw new Error('Invalid market response');
      setData(response);
      setStatus('ready');
    } catch {
      if (generation !== loadGeneration.current) return;
      setStatus('error');
    }
  }, [project.id]);

  useEffect(() => { load(); }, [load]);

  return (
    <Panel title="Market">
      {status === 'loading' && <div style={{ color: colors.textDim, fontSize: 11 }}>Loading market series…</div>}
      {status === 'error' && (
        <button type="button" onClick={load} style={{ border: 'none', background: 'none', color: colors.danger, cursor: 'pointer', padding: 0 }}>
          Couldn't load market series. Retry
        </button>
      )}
      {/* Nothing bound is not a flat market. Say which. */}
      {status === 'ready' && data?.noSeriesBound && (
        <div style={{ color: colors.textDim, fontSize: 11, lineHeight: 1.5 }}>
          No market series bound. Nothing here is a forecast of zero — there is simply nothing
          to forecast yet. Ask the Forecaster to bind a competitor's package, a category's
          Wikipedia article, or a Hacker News query.
        </div>
      )}
      {status === 'ready' && !data?.noSeriesBound && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
          {data?.rows.map(row => {
            const poly = row.history.length > 1 ? sparklinePolyline(row.history, VIEW_W, VIEW_H) : null;
            const forecastable = row.verdict === 'forecastable' && row.forecast;
            return (
              <div key={row.seriesId} style={{ borderLeft: `2px solid ${forecastable ? colors.cyan : colors.border}`, paddingLeft: 9 }}>
                <div style={{ display: 'flex', alignItems: 'baseline', justifyContent: 'space-between', gap: 8 }}>
                  <div style={{ color: colors.text, fontSize: 12, fontWeight: 600 }}>{row.subject}</div>
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
                    style={{ marginTop: 4 }}
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
                    <div style={{ color: colors.text, fontSize: 11, marginTop: 3 }}>
                      {row.direction ?? '—'}
                      {row.forecast.p10.length > 0 && (
                        <span style={{ color: colors.textMuted }}>
                          {' '}· 80% range {row.forecast.p10[row.forecast.p10.length - 1].toFixed(0)}–
                          {row.forecast.p90[row.forecast.p90.length - 1].toFixed(0)}
                        </span>
                      )}
                    </div>
                    {/* The method label is never optional and never hidden. */}
                    <div style={{ fontFamily: font.mono, fontSize: 9, color: colors.textDim, marginTop: 2 }}>
                      method: {row.forecast.methodLabel}
                      {row.forecast.maseVsBaseline != null && (
                        <> · MASE {row.forecast.maseVsBaseline.toFixed(2)}× baseline over {row.forecast.folds} folds</>
                      )}
                    </div>
                  </>
                ) : (
                  <div style={{ color: colors.textMuted, fontSize: 11, marginTop: 3 }}>
                    {refusalText(row)}
                  </div>
                )}

                <div style={{ display: 'flex', gap: 8, alignItems: 'center', marginTop: 3, fontFamily: font.mono, fontSize: 9, color: colors.textDim }}>
                  <span>{row.points} pts · {row.cadence}</span>
                  {row.snapshotOnly && <span title="This source cannot hand over history; it accumulates one point per sweep.">snapshot-only</span>}
                  {!row.officialSource && <span title="Not a supported API; used anyway, and said out loud.">unofficial source</span>}
                  {row.lastError && <span style={{ color: colors.danger }} title={row.lastError}>collector error</span>}
                </div>
              </div>
            );
          })}
        </div>
      )}
    </Panel>
  );
}
