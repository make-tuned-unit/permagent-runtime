/**
 * Stalled-Goal Pattern — Operator Command Center's synthesis card (§2.2, §3
 * `stalled_pattern`). The daemon clusters on a shared two-word PHRASE in the
 * blocker notes (not a single word — a keyword is not an insight) and hands
 * back a ready sentence, the goals by name, and a suggested next step. This
 * card renders exactly that; it does no clustering of its own.
 */
import { radius, space, font, textSize } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { SectionTitle, EmptyNote, CardScrollBody } from '../atoms';
import { useCommandCenter } from '../../../lib/store';
import { useDashboardInsights } from './useDashboardInsights';

export function StalledPatternCard() {
  const { colors } = useTheme();
  const { data, loading, error } = useDashboardInsights();
  const openGoalDetail = useCommandCenter(s => s.openGoalDetail);
  const clusters = data?.stalledPatterns ?? [];
  const projectNames = new Map((data?.projectHealthGrid ?? []).map(p => [p.projectId, p.projectName]));

  return (
    <div style={{
      height: '100%', boxSizing: 'border-box',
      borderRadius: radius.lg,
      background: colors.surface,
      border: `1px solid ${colors.border}`,
      boxShadow: [colors.elevationRaised, colors.cardHighlight].filter(Boolean).join(', '),
      padding: space.xxl,
      display: 'flex', flexDirection: 'column',
      overflow: 'hidden',
    }}>
      <SectionTitle title="Shared blockers" />
      {error ? (
        <EmptyNote hint="Couldn't reach the daemon">Couldn't look for patterns</EmptyNote>
      ) : loading && !data ? (
        <EmptyNote>Loading…</EmptyNote>
      ) : clusters.length === 0 ? (
        <EmptyNote hint="Shown when 2+ stalled goals share a phrase in their blocker note">
          Not enough concurrent activity yet to find cross-project patterns
        </EmptyNote>
      ) : (
        <CardScrollBody testId="stalled-pattern-body">
          {clusters.map(cluster => (
            <div key={cluster.phrase} data-card-row="" style={{ padding: `${space.md}px 0`, borderTop: `1px solid ${colors.border}` }}>
              <div style={{ fontFamily: font.body, fontSize: textSize.small, fontWeight: 600, color: colors.text, lineHeight: 1.4 }}>
                {cluster.sentence}
              </div>
              {cluster.nextStep && (
                <div style={{ fontFamily: font.body, fontSize: textSize.caption, color: colors.cyan, marginTop: space.xxs }}>
                  {cluster.nextStep}
                </div>
              )}
              <div style={{ display: 'flex', flexDirection: 'column', gap: space.xxs, marginTop: space.sm, maxWidth: 640 }}>
                {cluster.goals.map(g => (
                  <button
                    key={g.goalId}
                    type="button"
                    onClick={() => openGoalDetail(g.projectId, g.goalId)}
                    style={{
                      display: 'flex', alignItems: 'center', gap: space.sm, width: '100%',
                      background: 'transparent', border: 'none', padding: `${space.xxs}px 0`,
                      cursor: 'pointer', textAlign: 'left',
                    }}
                  >
                    <span style={{
                      fontFamily: font.body, fontSize: textSize.caption, color: colors.text,
                      overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', minWidth: 0, flex: 1,
                    }}>{g.title}</span>
                    <span style={{ fontFamily: font.body, fontSize: textSize.micro, color: colors.textDim, flexShrink: 0 }}>
                      {projectNames.get(g.projectId) ?? g.projectId}
                    </span>
                  </button>
                ))}
              </div>
            </div>
          ))}
        </CardScrollBody>
      )}
    </div>
  );
}
