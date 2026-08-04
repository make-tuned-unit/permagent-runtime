import { useCallback } from 'react';
import { font, radius } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { useCommandCenter } from '../../../lib/store';
import { SectionTitle, StatusIcon, EmptyNote } from '../atoms';
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
  const switchToSession = useCommandCenter(s => s.switchToSession);
  const openChatDock = useCommandCenter(s => s.openChatDock);

  // A recent item's id IS a session id (dashboard.rs builds it from sessions).
  // Open it in the chat dock so the past conversation is immediately visible.
  const openSession = useCallback((id: string) => {
    switchToSession(id).catch(err => console.error('[recent] open session failed:', err));
    openChatDock();
  }, [switchToSession, openChatDock]);

  return (
    <div style={{
      height: '100%', boxSizing: 'border-box',
      borderRadius: radius.lg,
      background: colors.surface,
      border: `1px solid ${colors.border}`,
      boxShadow: [colors.cardShadow, colors.cardHighlight].filter(Boolean).join(', '),
      padding: 16,
      display: 'flex', flexDirection: 'column',
      overflow: 'hidden',
    }}>
      <SectionTitle title="Recent" right={items.length > 0 ? 'last 24h' : undefined} />
      {items.length === 0 ? (
        <EmptyNote hint="Sessions you run will appear here">No recent activity</EmptyNote>
      ) : (
        <div style={{ flex: 1, overflow: 'auto', marginRight: -8, paddingRight: 8 }}>
          {items.map((item, i) => (
            <ActivityItem key={item.id} item={item} isLast={i === items.length - 1} onOpen={openSession} />
          ))}
        </div>
      )}
    </div>
  );
}

function ActivityItem({ item, isLast, onOpen }: { item: RecentSession; isLast: boolean; onOpen: (id: string) => void }) {
  const { colors } = useTheme();
  const statusColor: Record<string, string> = {
    completed: colors.success,
    paused: colors.danger,
    awaiting_input: colors.cyan,
  };
  return (
    <div
      onClick={() => onOpen(item.id)}
      role="button"
      tabIndex={0}
      aria-label={`Open conversation: ${item.title}`}
      onKeyDown={e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); onOpen(item.id); } }}
      onMouseEnter={e => { e.currentTarget.style.background = colors.border; }}
      onMouseLeave={e => { e.currentTarget.style.background = 'transparent'; }}
      onFocus={e => { e.currentTarget.style.background = colors.border; e.currentTarget.style.boxShadow = `0 0 0 2px ${colors.cyanGlow}`; }}
      onBlur={e => { e.currentTarget.style.background = 'transparent'; e.currentTarget.style.boxShadow = 'none'; }}
      style={{
      display: 'flex', alignItems: 'center', gap: 10, padding: '7px 8px',
      borderBottom: isLast ? 'none' : `1px solid ${colors.border}`,
      cursor: 'pointer', borderRadius: radius.sm, outline: 'none',
      background: 'transparent', transition: 'background 100ms, box-shadow 100ms',
    }}>
      <StatusIcon state={item.state} />
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{
          fontFamily: font.body, fontSize: 12.5, fontWeight: 500, color: colors.text,
          overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
        }}>{item.title}</div>
        <div style={{ fontSize: 10.5, color: colors.textDim, marginTop: 1 }}>
          {timeAgo(item.ended_at)}
        </div>
      </div>
      <span style={{
        fontFamily: font.body, fontSize: 9.5, fontWeight: 600,
        letterSpacing: '0.06em', textTransform: 'uppercase', flexShrink: 0,
        color: statusColor[item.state] || colors.textMuted,
      }}>{item.state.replace('_', ' ')}</span>
    </div>
  );
}
