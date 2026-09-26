import { memo, useEffect, useState } from 'react';
import { duration, ease, font, radius, space, textSize } from '../../../styles/tokens';
import type { ThemeColors } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { SectionTitle, EmptyNote, StatCompact, CardScrollBody } from '../atoms';
import { apiFetch } from '../../../lib/api';
import { navigateToTool, useCommandCenter } from '../../../lib/store';
import { GrowthSparkline } from '../../grow/GrowthSparkline';
import type { GrowthResultsData } from '../../grow/growthResults';
import { lastCumulativeNet } from '../../grow/growthTrend';

import { Tooltip } from '../../common/Tooltip';
/**
 * Home-dashboard view of growth actions taken — fleet trend, then each project.
 *
 * Grow's Actions lens is for deciding what to do; Tracking is one card's
 * windows. This card answers "is the work paying off?" with a 12-week
 * cumulative helped-minus-hindered line across every active project, and
 * the same line per project underneath.
 */
export const GrowthResultsCard = memo(function GrowthResultsCard() {
  const { colors, reduceMotion } = useTheme();
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
  const summary = fleet ? growthSummary(fleet) : null;

  return (
    <div
      style={{
        height: '100%', boxSizing: 'border-box',
        borderRadius: radius.lg,
        background: colors.surface,
        border: `1px solid ${colors.border}`,
        boxShadow: [colors.elevationRaised, colors.cardHighlight].filter(Boolean).join(', '),
        padding: space.xxl,
        display: 'flex', flexDirection: 'column',
        overflow: 'hidden',
      }}
    >
      <Tooltip content="Open growth results">
        <div onClick={openFleet} style={{ cursor: 'pointer' }}>
          <SectionTitle title="Growth" right={summary?.right} />
        </div>
      </Tooltip>

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
            // Capped (R3, coordinator 2560px round 2): an unbounded 4-column
            // `1fr` grid stretched to the card's full width spread Helped /
            // Hindered / No change / Inconclusive ~2,000px apart on a wide
            // card — four numbers that belong to one summary read as four
            // unrelated ones. Same 720px ceiling as the by-project row below
            // (GrowthProjectRow), so every label/value cluster in this card
            // stays a readable width no matter how wide the card is drawn.
            display: 'grid', gridTemplateColumns: 'repeat(4, minmax(0, 1fr))',
            gap: `${space.lg}px ${space.xl}px`, maxWidth: 720,
          }}>
            <StatCompact label="Helped" value={fleet.helped} cyan={fleet.helped > 0} />
            <StatCompact label="Hindered" value={fleet.hindered} />
            <StatCompact label="No change" value={fleet.noEffect} />
            <StatCompact label="Inconclusive" value={fleet.inconclusive} />
          </div>

          {summary?.allInconclusive && (
            // Every measured window came back inconclusive. Without this the
            // card read "13 measured" over Helped 0 / Hindered 0 / No change 0
            // and flat lines — which looks like the numbers failed to load.
            <div data-testid="growth-inconclusive-note" style={{ marginTop: space.lg }}>
              <EmptyNote hint="Inconclusive means the change stayed inside the project's normal week-to-week swing — the common result at low traffic. Later windows may still decide it.">
                No verdict yet: all {measured} measured window{measured === 1 ? ' was' : 's were'} inconclusive
              </EmptyNote>
            </div>
          )}

          {/* A 12-week line with no decisive verdict in it is a flat zero: it
              says nothing the note above does not, and reads as broken. */}
          {hasTrend && trend.length > 0 && (
            <Tooltip content="All projects, last 12 weeks">
              <div onClick={openFleet} style={{ cursor: 'pointer', marginTop: space.xl }}>
                <div style={{
                  fontFamily: font.mono, fontSize: 9, letterSpacing: '0.08em',
                  textTransform: 'uppercase', color: colors.textDim, marginBottom: space.xs,
                }}>All projects</div>
                <GrowthSparkline points={trend} colors={colors} height={52} />
              </div>
            </Tooltip>
          )}

          {byProject.length > 0 && (
            <CardScrollBody testId="growth-by-project" style={{ marginTop: space.lg }}>
              <div style={{
                fontFamily: font.mono, fontSize: 9, letterSpacing: '0.08em',
                textTransform: 'uppercase', color: colors.textDim, marginBottom: space.xs,
              }}>By project</div>
              {byProject.map((row) => {
                const net = lastCumulativeNet(row.points);
                const decisive = row.helped + row.hindered + row.noEffect > 0;
                return (
                  // Not on the Button primitive: the whole row IS the button —
                  // a truncating project name that must flex, a stats line, and
                  // a fixed-width sparkline, all laid out by the button itself.
                  // The primitive wraps its children in a single span, which
                  // would collapse that three-part row. Left as a raw button —
                  // which is why it needs its own hover/press feedback (D10);
                  // this row had none before.
                  <GrowthProjectRow
                      key={row.projectId}
                      onClick={() => openProject(row.projectId)}
                      title={`Open ${row.projectName} results`}
                      reduceMotion={reduceMotion}
                      colors={colors}
                    >
                      <span style={{
                        fontFamily: font.body, fontSize: textSize.caption, color: colors.text,
                        overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                        minWidth: 0, flex: 1,
                      }}>{row.projectName}</span>
                      <span style={{
                        fontFamily: font.mono, fontSize: 10, color: colors.textDim, flexShrink: 0,
                      }}>
                        {decisive ? (
                          <>
                            {row.helped} helped
                            {row.hindered > 0 ? ` · ${row.hindered} hindered` : ''}
                            {net !== 0 ? ` · ${net > 0 ? '+' : ''}${net}` : ''}
                          </>
                        ) : 'inconclusive so far'}
                      </span>
                      <div style={{ width: 88, flexShrink: 0, visibility: decisive ? 'visible' : 'hidden' }}>
                        <GrowthSparkline
                          points={row.points}
                          colors={colors}
                          height={22}
                          showAxis={false}
                          stroke={net < 0 ? colors.danger : colors.cyan}
   />
                      </div>
                    </GrowthProjectRow>
                );
              })}
            </CardScrollBody>
          )}
        </div>
      )}
    </div>
  );
});

/**
 * The per-project row's own hover/press feedback (D10) — a plain `<button>`
 * had none. `fillHover`/`fillActive` layer over the card's own `colors.surface`
 * (the row sits directly on it), so a hover reads as the same surface lit up
 * a shade, not a second material.
 */
function GrowthProjectRow({
  onClick, title, reduceMotion, colors, children,
}: {
  onClick: () => void;
  title: string;
  reduceMotion: boolean;
  colors: ThemeColors;
  children: React.ReactNode;
}) {
  const [hover, setHover] = useState(false);
  const [pressed, setPressed] = useState(false);
  return (
    <Tooltip content={title}>
      <button
        type="button"
        data-card-row=""
        onClick={onClick}
        onMouseEnter={() => setHover(true)}
        onMouseLeave={() => { setHover(false); setPressed(false); }}
        onPointerDown={() => setPressed(true)}
        onPointerUp={() => setPressed(false)}
        style={{
          display: 'flex', alignItems: 'center',
          width: '100%', padding: `${space.sm}px 0`,
          border: 'none', borderTop: `1px solid ${colors.border}`,
          background: pressed ? colors.fillActive : hover ? colors.fillHover : 'transparent',
          cursor: 'pointer', textAlign: 'left', borderRadius: radius.xs,
          transition: reduceMotion ? 'none' : `background ${duration.fast}ms ${ease.out}`,
        }}
      >
        {/* dagK coordinator, 2560px pass: this row used to be the flex
            container itself, so a card stretched to the full 12-column grid
            (fillHoles) put the project name at the left edge and its stats
            ~1,800px away at the right edge — readable as two unrelated lines,
            not one row. Capped at a readable width (R3), the name/stats/
            sparkline cluster together regardless of how wide the card is
            drawn; the row's hit target and hover fill still span the card. */}
        <span style={{ display: 'flex', alignItems: 'center', gap: space.lg, width: '100%', maxWidth: 720, minWidth: 0 }}>
          {children}
        </span>
      </button>
    </Tooltip>
  );
}

/**
 * The card's headline numbers, said without contradiction.
 *
 * "Measured" counts every judged window — helped, hindered, no change AND
 * inconclusive (`tally_verdicts` in growth_actions.rs folds `inconclusive` and
 * `confounded` together). The stat row used to show only the first three, so a
 * fleet whose 13 windows were all inconclusive read "13 measured" beside three
 * zeros. The fourth number is now on the card, and the all-inconclusive case
 * is named.
 */
export function growthSummary(fleet: {
  helped: number; hindered: number; noEffect: number; inconclusive: number; projects: number;
}): { measured: number; decisive: number; allInconclusive: boolean; right: string | undefined } {
  const decisive = fleet.helped + fleet.hindered + fleet.noEffect;
  const measured = decisive + fleet.inconclusive;
  if (measured === 0) return { measured, decisive, allInconclusive: false, right: undefined };
  const projects = fleet.projects > 0 ? ` · ${fleet.projects} project${fleet.projects === 1 ? '' : 's'}` : '';
  return {
    measured,
    decisive,
    allInconclusive: decisive === 0,
    right: `${measured} measured${projects}`,
  };
}
