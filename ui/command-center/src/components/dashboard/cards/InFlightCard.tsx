import { font, radius, ease } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { Mobius, type MobiusState } from '../../mobius/Mobius';
import { SectionTitle } from '../atoms';
import type { InFlightSession } from '../useDashboard';

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
  tasks: InFlightSession[];
}

export function InFlightCard({ tasks }: Props) {
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
      <SectionTitle title="In flight" right={tasks.length > 0 ? `${tasks.length} active` : undefined} />
      {tasks.length === 0 ? (
        <div style={{
          flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center',
        }}>
          <div style={{ textAlign: 'center' }}>
            <div style={{ fontSize: 13, color: colors.textMuted, marginBottom: 4 }}>No active sessions</div>
            <div style={{ fontSize: 11, color: colors.textDim }}>Run something from Automate to see it here</div>
          </div>
        </div>
      ) : (
        <div style={{ flex: 1, overflow: 'auto' }}>
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(280px, 1fr))', gap: 14 }}>
            {tasks.map(task => <TaskCard key={task.id} task={task} />)}
          </div>
        </div>
      )}
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
          background: colors.ribbonGradient,
          boxShadow: `0 0 8px ${colors.cyanGlow}`,
          transition: `width 300ms ${ease.out}`,
        }} />
      </div>
    </div>
  );
}
