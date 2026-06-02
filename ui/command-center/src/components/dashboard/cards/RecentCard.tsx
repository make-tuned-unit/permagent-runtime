import { font, radius } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { SectionTitle, StatusIcon } from '../atoms';
import type { RecentSession } from '../useDashboard';

function timeAgo(iso: string): string {
  const ms = Date.now() - new Date(iso).getTime();
  const min = Math.floor(ms / 60000);
  if (min < 1) return 'just now';
  if (min < 60) return `${min}m ago`;
  const hrs = Math.floor(min / 60);
  if (hrs < 24) return `${hrs}h ago`;
  return `${Math.floor(hrs / 24)}d ago`;
}

interface Props {
  items: RecentSession[];
}

export function RecentCard({ items }: Props) {
  const { colors } = useTheme();
  return (
    <div style={{
      height: '100%', boxSizing: 'border-box',
      borderRadius: radius.lg,
      background: colors.surface,
      border: `1px solid ${colors.border}`,
      boxShadow: [colors.cardShadow, colors.cardHighlight].filter(Boolean).join(', '),
      padding: '18px 20px',
      display: 'flex', flexDirection: 'column',
      overflow: 'hidden',
    }}>
      <SectionTitle title="Recent" right={items.length > 0 ? 'last 24h' : undefined} />
      {items.length === 0 ? (
        <div style={{
          flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center',
        }}>
          <div style={{ textAlign: 'center' }}>
            <div style={{ fontSize: 13, color: colors.textMuted, marginBottom: 4 }}>No recent activity</div>
            <div style={{ fontSize: 11, color: colors.textDim }}>Sessions you run will appear here</div>
          </div>
        </div>
      ) : (
        <div style={{ flex: 1, overflow: 'auto' }}>
          {items.map((item, i) => (
            <ActivityItem key={item.id} item={item} isLast={i === items.length - 1} />
          ))}
        </div>
      )}
    </div>
  );
}

function ActivityItem({ item, isLast }: { item: RecentSession; isLast: boolean }) {
  const { colors } = useTheme();
  const statusColor: Record<string, string> = {
    completed: colors.success,
    paused: colors.danger,
    awaiting_input: colors.cyan,
  };
  return (
    <div style={{
      display: 'flex', alignItems: 'center', gap: 16, padding: '14px 18px',
      borderBottom: isLast ? 'none' : `1px solid ${colors.border}`,
    }}>
      <StatusIcon state={item.state} />
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{
          fontFamily: font.body, fontSize: 14, fontWeight: 500, color: colors.text,
          overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
        }}>{item.title}</div>
        <div style={{ fontSize: 12, color: colors.textMuted, marginTop: 2 }}>
          {timeAgo(item.ended_at)}
        </div>
      </div>
      <span style={{
        fontFamily: font.body, fontSize: 11, fontWeight: 600,
        letterSpacing: '0.06em', textTransform: 'uppercase',
        color: statusColor[item.state] || colors.textMuted,
      }}>{item.state.replace('_', ' ')}</span>
    </div>
  );
}
