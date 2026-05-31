import { font, ease, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Mobius, type MobiusState } from '../mobius/Mobius';
import { useDashboard, type InFlightSession, type RecentSession } from './useDashboard';
import { Stat, SectionTitle, StatusIcon } from './atoms';

function timeAgo(iso: string): string {
  const ms = Date.now() - new Date(iso).getTime();
  const min = Math.floor(ms / 60000);
  if (min < 1) return 'just now';
  if (min < 60) return `${min}m ago`;
  const hrs = Math.floor(min / 60);
  if (hrs < 24) return `${hrs}h ago`;
  return `${Math.floor(hrs / 24)}d ago`;
}

export function Dashboard() {
  const { gradient, showHeroMobius, colors } = useTheme();
  const { data, loading } = useDashboard();

  if (loading || !data) {
    return (
      <div style={{ width: '100%', height: '100%', background: colors.bg, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
        <Mobius size={120} state="thinking" />
      </div>
    );
  }

  const { agent, stats, in_flight, recent } = data;
  const mobiusState = (agent.state === 'thinking' ? 'thinking' : 'idle') as MobiusState;

  return (
    <div style={{ width: '100%', height: '100%', position: 'relative' }}>
      <div style={{
        width: '100%', height: '100%', overflowY: 'auto',
        background: gradient.workspace,
        padding: '28px 32px 40px',
      }}>
        {/* Hero + Stats row */}
        <div style={{ display: 'grid', gridTemplateColumns: '1.2fr 1fr', gap: 24, marginBottom: 24 }}>
          {/* Hero card */}
          <div style={{
            position: 'relative', overflow: 'hidden',
            padding: 24, borderRadius: radius.lg,
            background: gradient.card,
            border: `1px solid ${colors.border}`, minHeight: 220,
            display: 'flex', alignItems: 'center', gap: 24,
          }}>
            <div style={{ flex: 1 }}>
              <div style={{
                fontFamily: font.body, fontSize: 11, fontWeight: 600,
                letterSpacing: '0.14em', textTransform: 'uppercase',
                color: colors.cyan, marginBottom: 12,
              }}>
                Status — {agent.state}
              </div>
              <div style={{
                fontFamily: font.display, fontSize: 24, fontWeight: 600,
                letterSpacing: '-0.02em', lineHeight: 1.2, marginBottom: 10,
              }}>
                {agent.active_count > 0 ? (
                  <>{agent.name} is working on<br /><span style={{ color: colors.cyan }}>{agent.active_count} {agent.active_count === 1 ? 'thing' : 'things'}</span> for you</>
                ) : (
                  <>{agent.name} is<br />ready</>
                )}
              </div>
              <div style={{ fontSize: 14, color: colors.textMuted, lineHeight: 1.5, maxWidth: 360 }}>
                {agent.active_count > 0
                  ? `Working across ${agent.active_count} session${agent.active_count > 1 ? 's' : ''}`
                  : 'Ready when you are.'}
              </div>
            </div>
            {showHeroMobius && (
              <div style={{ flex: '0 0 auto' }}>
                <Mobius size={96} state={mobiusState} glow={1} />
              </div>
            )}
          </div>

          {/* Stats grid */}
          <div style={{
            padding: 24, borderRadius: radius.lg,
            background: colors.surface,
            border: `1px solid ${colors.border}`,
            boxShadow: [colors.cardShadow, colors.cardHighlight].filter(Boolean).join(', '),
            display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 20,
          }}>
            <Stat label="Sessions today" value={stats.sessions_today} />
            <Stat label="Total sessions" value={stats.sessions_total} />
            <Stat label="Memory nodes" value={stats.memory_count} />
            <Stat label="New today" value={stats.memory_delta_today} delta={stats.memory_delta_today > 0 ? `+${stats.memory_delta_today}` : undefined} cyan />
          </div>
        </div>

        {/* In flight */}
        {in_flight.length > 0 && (
          <div style={{ marginBottom: 24 }}>
            <SectionTitle title="In flight" right={`${in_flight.length} active`} />
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(280px, 1fr))', gap: 14 }}>
              {in_flight.map(task => <TaskCard key={task.id} task={task} />)}
            </div>
          </div>
        )}

        {/* Recent */}
        {recent.length > 0 && (
          <div>
            <SectionTitle title="Recent" right="last 24h" />
            <div style={{
              borderRadius: radius.lg, background: colors.surface,
              border: `1px solid ${colors.border}`, overflow: 'hidden',
              boxShadow: [colors.cardShadow, colors.cardHighlight].filter(Boolean).join(', '),
            }}>
              {recent.map((item, i) => (
                <ActivityItem key={item.id} item={item} isLast={i === recent.length - 1} />
              ))}
            </div>
          </div>
        )}
      </div>

    </div>
  );
}

function TaskCard({ task }: { task: InFlightSession }) {
  const { colors } = useTheme();
  const mobiusState = (task.state === 'speaking' ? 'speaking' : 'thinking') as MobiusState;
  return (
    <div style={{
      padding: 18, borderRadius: radius.md,
      background: colors.surface,
      border: `1px solid ${colors.border}`,
      boxShadow: [colors.cardShadow, colors.cardHighlight].filter(Boolean).join(', '),
    }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 12 }}>
        <Mobius size={36} state={mobiusState} logoMode />
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{
            fontFamily: font.body, fontSize: 13, fontWeight: 600, color: colors.text,
            overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
          }}>{task.title}</div>
          <div style={{ fontFamily: font.mono, fontSize: 11, color: colors.textDim }}>
            Started {timeAgo(task.started_at)}
          </div>
        </div>
      </div>
      <div style={{
        height: 4, borderRadius: 999, background: colors.border,
        overflow: 'hidden',
      }}>
        <div style={{
          height: '100%', borderRadius: 999,
          width: `${Math.max(2, task.progress * 100)}%`,
          background: 'linear-gradient(90deg, #00D5FF, #A855F7)',
          boxShadow: '0 0 8px rgba(0,213,255,0.5)',
          transition: `width 300ms ${ease.out}`,
        }} />
      </div>
    </div>
  );
}

function ActivityItem({ item, isLast }: { item: RecentSession; isLast: boolean }) {
  const { colors } = useTheme();
  const statusColor: Record<string, string> = {
    completed: '#5BD17F',
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
