/**
 * Stalled-Goal Pattern — Operator Command Center's synthesis card (§2.2, §3
 * `stalled_pattern`). The daemon's deterministic clustering already refuses
 * to report a "pattern" from fewer than 2 goals sharing a keyword (P12) —
 * this card's only job is to hide itself gracefully when that comes back
 * empty, never to force one into view.
 */
import { radius, space, font, textSize } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { SectionTitle, EmptyNote, CardScrollBody } from '../atoms';
import { useCommandCenter } from '../../../lib/store';
import { useDashboardInsights } from './useDashboardInsights';
import { Button } from '../../common/Button';
import type { CSSProperties } from 'react';

export function StalledPatternCard() {
  const { colors } = useTheme();
  const { data, loading, error } = useDashboardInsights();
  const openGoalDetail = useCommandCenter(s => s.openGoalDetail);
  const clusters = data?.stalledPatterns ?? [];

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
        <EmptyNote hint="Shown when 2+ stalled goals share a keyword in their blocker note">
          Not enough concurrent activity yet to find cross-project patterns
        </EmptyNote>
      ) : (
        <CardScrollBody testId="stalled-pattern-body">
          {clusters.map(cluster => {
            const projectCount = new Set(cluster.goals.map(g => g.projectId)).size;
            return (
              <div key={cluster.keyword} data-card-row="" style={{ padding: `${space.md}px 0`, borderTop: `1px solid ${colors.border}` }}>
                <div style={{ fontFamily: font.body, fontSize: textSize.small, fontWeight: 600, color: colors.text }}>
                  {cluster.goals.length} goals across {projectCount} project{projectCount === 1 ? '' : 's'} share "{cluster.keyword}"
                </div>
                <div style={{ display: 'flex', flexWrap: 'wrap', gap: space.sm, marginTop: space.sm }}>
                  {cluster.goals.map(g => (
                    <Button
                      key={g.goalId}
                      colors={colors}
                      type="button"
                      onClick={() => openGoalDetail(g.projectId, g.goalId)}
                      style={{
                        '--pa-btn-bg': colors.cyanSoft,
                        '--pa-btn-fg': colors.cyan,
                        '--pa-btn-border': 'transparent',
                        '--pa-btn-bg-hover': colors.cyanSoft,
                        '--pa-btn-fg-hover': colors.cyan,
                        '--pa-btn-pad': `${space.xxs}px ${space.sm}px`,
                        '--pa-btn-radius': `${radius.sm}px`,
                        '--pa-btn-weight': 600,
                        fontFamily: font.body, fontSize: textSize.micro,
                      } as CSSProperties}
                    >
                      Goal →
                    </Button>
                  ))}
                </div>
              </div>
            );
          })}
        </CardScrollBody>
      )}
    </div>
  );
}
