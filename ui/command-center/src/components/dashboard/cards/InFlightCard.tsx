import { memo } from 'react';
import { font, radius } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { useOrchestratorName } from '../../world/shared/useOrchestratorName';
import { Mobius } from '../../mobius/Mobius';
import { SectionTitle, EmptyNote } from '../atoms';
import { useCommandCenter } from '../../../lib/store';
import type { ActiveGoal } from '../../../lib/useLiveGoals';

const STATE_LABEL: Record<string, string> = {
  ready: 'Ready',
  in_progress: 'In Progress',
  review: 'Review',
};

interface Props {
  goals: ActiveGoal[];
}

export const InFlightCard = memo(function InFlightCard({ goals }: Props) {
  const { colors } = useTheme();
  const persona = useOrchestratorName() ?? 'your agent';
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
      <SectionTitle title="In flight" right={goals.length > 0 ? `${goals.length} active` : undefined} />
      {goals.length === 0 ? (
        <EmptyNote hint={`Goals ${persona} is working on appear here`}>
          Nothing in flight
        </EmptyNote>
      ) : (
        <div style={{ flex: 1, overflow: 'auto' }}>
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(280px, 1fr))', gap: 14 }}>
            {goals.map(goal => <GoalCard key={goal.id} goal={goal} />)}
          </div>
        </div>
      )}
    </div>
  );
});

const GoalCard = memo(function GoalCard({ goal }: { goal: ActiveGoal }) {
  const { colors } = useTheme();
  const openGoalDetail = useCommandCenter(s => s.openGoalDetail);
  const mobiusState = goal.state === 'review' ? 'idle' : 'thinking';
  return (
    <div
      onClick={() => openGoalDetail(goal.project_id, goal.id)}
      title="View goal detail"
      style={{
        padding: 18, borderRadius: radius.md, cursor: 'pointer',
        background: colors.surface,
        border: `1px solid ${colors.border}`,
        boxShadow: [colors.cardShadow, colors.cardHighlight].filter(Boolean).join(', '),
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
        <Mobius size={36} state={mobiusState} logoMode />
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{
            fontFamily: font.body, fontSize: 13, fontWeight: 600, color: colors.text,
            overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
          }}>{goal.title}</div>
          <div style={{ fontFamily: font.mono, fontSize: 11, color: colors.cyan }}>
            {goal.hold_note ? 'Held' : (STATE_LABEL[goal.state] ?? goal.state)}
            {goal.assigned_to ? ` · ${goal.assigned_to}` : ''}
          </div>
          {(goal.hold_note || goal.routing_note) && (
            <div style={{
              fontFamily: font.body, fontSize: 11, color: colors.textMuted,
              marginTop: 6, lineHeight: 1.4,
            }}>
              {goal.hold_note || goal.routing_note}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}, (a, b) =>
  a.goal.id === b.goal.id
  && a.goal.state === b.goal.state
  && a.goal.title === b.goal.title
  && a.goal.assigned_to === b.goal.assigned_to,
);
