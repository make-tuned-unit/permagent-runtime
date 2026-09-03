import { memo, useState } from 'react';
import { duration, ease, font, radius, space, textSize } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { useOrchestratorName } from '../../world/shared/useOrchestratorName';
import { Mobius } from '../../mobius/Mobius';
import { SectionTitle, EmptyNote } from '../atoms';
import { useCommandCenter } from '../../../lib/store';
import type { ActiveGoal } from '../../../lib/useLiveGoals';

import { Tooltip } from '../../common/Tooltip';
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
      boxShadow: [colors.elevationRaised, colors.cardHighlight].filter(Boolean).join(', '),
      padding: space.xxl,
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
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(280px, 1fr))', gap: space.xxl }}>
            {goals.map(goal => <GoalCard key={goal.id} goal={goal} />)}
          </div>
        </div>
      )}
    </div>
  );
});

const GoalCard = memo(function GoalCard({ goal }: { goal: ActiveGoal }) {
  const { colors, reduceMotion } = useTheme();
  const openGoalDetail = useCommandCenter(s => s.openGoalDetail);
  const mobiusState = goal.state === 'review' ? 'idle' : 'thinking';
  // Clickable tile with no hover/press feedback before this (D10): the whole
  // card is a button and looked exactly like one that wasn't.
  const [hover, setHover] = useState(false);
  const [pressed, setPressed] = useState(false);
  return (
    <Tooltip content="View goal detail">
      <div
        onClick={() => openGoalDetail(goal.project_id, goal.id)}
        onMouseEnter={() => setHover(true)}
        onMouseLeave={() => { setHover(false); setPressed(false); }}
        onPointerDown={() => setPressed(true)}
        onPointerUp={() => setPressed(false)}
        style={{
          padding: space.xxxl, borderRadius: radius.md, cursor: 'pointer',
          background: pressed ? colors.fillActive : hover ? colors.fillHover : colors.surface,
          border: `1px solid ${colors.border}`,
          boxShadow: [colors.elevationRaised, colors.cardHighlight].filter(Boolean).join(', '),
          transform: !reduceMotion && pressed ? 'scale(0.98)' : 'scale(1)',
          transition: reduceMotion
            ? 'none'
            : `background ${duration.fast}ms ${ease.out}, transform ${duration.fast}ms ${ease.out}`,
        }}
      >
        <div style={{ display: 'flex', alignItems: 'center', gap: space.xl }}>
          <Mobius size={36} state={mobiusState} logoMode />
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{
              fontFamily: font.body, fontSize: textSize.small, fontWeight: 600, color: colors.text,
              overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
            }}>{goal.title}</div>
            <div style={{ fontFamily: font.mono, fontSize: textSize.micro, color: colors.cyan }}>
              {goal.hold_note ? 'Held' : (STATE_LABEL[goal.state] ?? goal.state)}
              {goal.assigned_to ? ` · ${goal.assigned_to}` : ''}
            </div>
            {(goal.hold_note || goal.routing_note) && (
              <div style={{
                fontFamily: font.body, fontSize: textSize.micro, color: colors.textMuted,
                marginTop: 6, lineHeight: 1.4,
              }}>
                {goal.hold_note || goal.routing_note}
              </div>
            )}
          </div>
        </div>
      </div>
    </Tooltip>
  );
}, (a, b) =>
  a.goal.id === b.goal.id
  && a.goal.state === b.goal.state
  && a.goal.title === b.goal.title
  && a.goal.assigned_to === b.goal.assigned_to,
);
