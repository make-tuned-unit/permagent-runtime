import { radius, space } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { SectionTitle, StatCompact } from '../atoms';
import type { DashboardStats } from '../useDashboard';

interface Props {
  stats: DashboardStats;
}

/**
 * Four counters. Previously four 32px numbers on `alignContent: center`, which
 * floated them in the middle of the card with dead bands above and below — the
 * card was mostly padding. Now titled, top-aligned and compact, so the same
 * four facts occupy roughly a third of the height.
 */
export function StatsCard({ stats }: Props) {
  const { colors } = useTheme();
  return (
    <div style={{
      padding: space.xxl, borderRadius: radius.lg,
      background: colors.surface,
      border: `1px solid ${colors.border}`,
      boxShadow: [colors.elevationRaised, colors.cardHighlight].filter(Boolean).join(', '),
      height: '100%', boxSizing: 'border-box',
      overflow: 'hidden',
      display: 'flex', flexDirection: 'column',
    }}>
      <SectionTitle title="Activity" />
      <div style={{
        display: 'grid', gridTemplateColumns: '1fr 1fr',
        gap: `${space.xl}px ${space.xxl}px`, alignContent: 'start',
      }}>
        <StatCompact label="Sessions today" value={stats.sessions_today} />
        <StatCompact label="Total sessions" value={stats.sessions_total} />
        <StatCompact label="Memory nodes" value={stats.memory_count} />
        <StatCompact
          label="New today"
          value={stats.memory_delta_today}
          delta={stats.memory_delta_today > 0 ? `+${stats.memory_delta_today}` : undefined}
          cyan
        />
      </div>
    </div>
  );
}
