import { radius } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { Stat } from '../atoms';
import type { DashboardStats } from '../useDashboard';

interface Props {
  stats: DashboardStats;
}

export function StatsCard({ stats }: Props) {
  const { colors } = useTheme();
  return (
    <div style={{
      padding: 24, borderRadius: radius.lg,
      background: colors.surface,
      border: `1px solid ${colors.border}`,
      boxShadow: [colors.cardShadow, colors.cardHighlight].filter(Boolean).join(', '),
      height: '100%', boxSizing: 'border-box',
      overflow: 'hidden',
      display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 20,
      alignContent: 'center',
    }}>
      <Stat label="Sessions today" value={stats.sessions_today} />
      <Stat label="Total sessions" value={stats.sessions_total} />
      <Stat label="Memory nodes" value={stats.memory_count} />
      <Stat label="New today" value={stats.memory_delta_today} delta={stats.memory_delta_today > 0 ? `+${stats.memory_delta_today}` : undefined} cyan />
    </div>
  );
}
