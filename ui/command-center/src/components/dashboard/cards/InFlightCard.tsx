import { memo, useState } from 'react';
import { font, radius } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { Mobius } from '../../mobius/Mobius';
import { SectionTitle } from '../atoms';
import type { ActiveGoal } from '../../../lib/useLiveGoals';
import { GoalDetailModal } from '../../projects/GoalDetailModal';

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
  // Goal-detail modal (#503): clicking an in-flight goal opens the same modal
  // the Kanban board and Decision Inbox use — view, edit, and cancel from here.
  const [openGoal, setOpenGoal] = useState<{ projectId: string; goalId: string } | null>(null);
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
      <SectionTitle title="In flight" right={goals.length > 0 ? `${goals.length} active` : undefined} />
      {goals.length === 0 ? (
        <div style={{
          flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center',
        }}>
          <div style={{ textAlign: 'center' }}>
            <div style={{ fontSize: 13, color: colors.textMuted, marginBottom: 4 }}>No active goals</div>
            <div style={{ fontSize: 11, color: colors.textDim }}>Goals Henry is working on appear here</div>
          </div>
        </div>
      ) : (
        <div style={{ flex: 1, overflow: 'auto' }}>
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(280px, 1fr))', gap: 14 }}>
            {goals.map(goal => (
              <GoalCard
                key={goal.id}
                goal={goal}
                onOpen={() => setOpenGoal({ projectId: goal.project_id, goalId: goal.id })}
              />
            ))}
          </div>
        </div>
      )}

      {openGoal && (
        <GoalDetailModal
          projectId={openGoal.projectId}
          goalId={openGoal.goalId}
          onClose={() => setOpenGoal(null)}
        />
      )}
    </div>
  );
});

const GoalCard = memo(function GoalCard({ goal, onOpen }: { goal: ActiveGoal; onOpen: () => void }) {
  const { colors } = useTheme();
  const mobiusState = goal.state === 'review' ? 'idle' : 'thinking';
  return (
    <div
      onClick={onOpen}
      title="Open goal detail"
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
            {STATE_LABEL[goal.state] ?? goal.state}
            {goal.assigned_to ? ` · ${goal.assigned_to}` : ''}
          </div>
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
