import { memo, useEffect, useState } from 'react';
import { font, radius } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { SectionTitle, EmptyNote, StatCompact } from '../atoms';
import { apiFetch } from '../../../lib/api';
import { navigateToTool, useCommandCenter } from '../../../lib/store';
import { GrowthSparkline } from '../../grow/GrowthSparkline';
import type { GrowthResultsData } from '../../grow/growthResults';
import { lastCumulativeNet } from '../../grow/growthTrend';

/**
 * Home-dashboard view of growth actions taken — fleet trend, then each project.
 *
 * Grow's Actions lens is for deciding what to do; Tracking is one card's
 * windows. This card answers "is the work paying off?" with a 12-week
 * cumulative helped-minus-hindered line across every active project, and
 * the same line per project underneath.
 */
export const GrowthResultsCard = memo(function GrowthResultsCard() {
  const { colors } = useTheme();
  const setOpenGrowLens = useCommandCenter((s) => s.setOpenGrowLens);
  const growProject = useCommandCenter((s) => s.growProject);
  const [data, setData] = useState<GrowthResultsData | null>(null);
  const [error, setError] = useState(false);

  useEffect(() => {
    let alive = true;
    apiFetch<GrowthResultsData>('/api/growth-results')
      .then((d) => {
        if (!alive) return;
        setData(d);
        setError(false);
      })
      .catch(() => {
        if (!alive) return;
        setError(true);
      });
    return () => { alive = false; };
  }, []);

  const openFleet = () => {
    setOpenGrowLens('results');
    navigateToTool('grow');
  };

  const openProject = (projectId: string) => {
    setOpenGrowLens('results');
    growProject(projectId);
  };

  const fleet = data?.fleet;
  const measured = fleet
    ? fleet.helped + fleet.hindered + fleet.noEffect + fleet.inconclusive
    : 0;
  const trend = fleet?.trend ?? [];
  const byProject = fleet?.byProject ?? [];
  const hasTrend = trend.some((p) => p.helped + p.hindered + p.noEffect > 0);

  return (
    <div
      style={{
        height: '100%', boxSizing: 'border-box',
        borderRadius: radius.lg,
        background: colors.surface,
        border: `1px solid ${colors.border}`,
        boxShadow: [colors.cardShadow, colors.cardHighlight].filter(Boolean).join(', '),
        padding: 16,
        display: 'flex', flexDirection: 'column',
        overflow: 'hidden',
      }}
    >
      <div onClick={openFleet} title="Open growth results" style={{ cursor: 'pointer' }}>
        <SectionTitle title="Growth" right={measured > 0 ? `${measured} measured` : undefined} />
      </div>

      {error ? (
        <EmptyNote hint="Open Grow → Results to try again">Couldn’t load growth results</EmptyNote>
      ) : !fleet ? (
        <EmptyNote>Loading…</EmptyNote>
      ) : measured === 0 && !hasTrend ? (
        <EmptyNote hint="Send an action to a coding agent, then verify it — 7/14/28-day windows land here">
          No growth actions measured yet
        </EmptyNote>
      ) : (
        <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
          <div style={{
            display: 'grid', gridTemplateColumns: 'repeat(4, minmax(0, 1fr))',
            gap: '10px 12px',
          }}>
            <StatCompact label="Helped" value={fleet.helped} cyan={fleet.helped > 0} />
            <StatCompact label="Hindered" value={fleet.hindered} />
            <StatCompact label="No change" value={fleet.noEffect} />
            <StatCompact label="Projects" value={fleet.projects} />
          </div>

          {trend.length > 0 && (
            <div onClick={openFleet} title="All projects, last 12 weeks" style={{ cursor: 'pointer', marginTop: 12 }}>
              <div style={{
                fontFamily: font.mono, fontSize: 9, letterSpacing: '0.08em',
                textTransform: 'uppercase', color: colors.textDim, marginBottom: 4,
              }}>All projects</div>
              <GrowthSparkline points={trend} colors={colors} height={52} />
            </div>
          )}

          {byProject.length > 0 && (
            <div style={{ flex: 1, overflowY: 'auto', marginTop: 10, minHeight: 0 }}>
              <div style={{
                fontFamily: font.mono, fontSize: 9, letterSpacing: '0.08em',
                textTransform: 'uppercase', color: colors.textDim, marginBottom: 4,
              }}>By project</div>
              {byProject.map((row) => {
                const net = lastCumulativeNet(row.points);
                return (
                  <button
                    key={row.projectId}
                    type="button"
                    onClick={() => openProject(row.projectId)}
                    title={`Open ${row.projectName} results`}
                    style={{
                      display: 'flex', alignItems: 'center', gap: 10,
                      width: '100%', padding: '5px 0',
                      border: 'none', borderTop: `1px solid ${colors.border}`,
                      background: 'transparent', cursor: 'pointer', textAlign: 'left',
                    }}
                  >
                    <span style={{
                      fontFamily: font.body, fontSize: 12, color: colors.text,
                      overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                      minWidth: 0, flex: 1,
                    }}>{row.projectName}</span>
                    <span style={{
                      fontFamily: font.mono, fontSize: 10, color: colors.textDim, flexShrink: 0,
                    }}>
                      {row.helped} helped
                      {row.hindered > 0 ? ` · ${row.hindered} hindered` : ''}
                      {net !== 0 ? ` · ${net > 0 ? '+' : ''}${net}` : ''}
                    </span>
                    <div style={{ width: 88, flexShrink: 0 }}>
                      <GrowthSparkline
                        points={row.points}
                        colors={colors}
                        height={22}
                        showAxis={false}
                        stroke={net < 0 ? colors.danger : colors.cyan}
                      />
                    </div>
                  </button>
                );
              })}
            </div>
          )}
        </div>
      )}
    </div>
  );
});
