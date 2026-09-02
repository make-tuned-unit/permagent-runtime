import { memo, useState, type CSSProperties } from 'react';
import { radius, textSize } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { Button } from '../../common/Button';
import { SectionTitle, EmptyNote } from '../atoms';
import { useCommandCenter } from '../../../lib/store';
import {
  relativeDueLabel,
  type DueBucket,
  type DueGroup,
  type DueTodo,
  type UseDueTodos,
} from '../../../lib/useDueTodos';

/**
 * Every dated to-do from every kanban board, soonest first (#todo-dashboard).
 *
 * The point of this card is triage across projects: which of the things you
 * wrote on four different boards actually needs doing next. Three deliberate
 * choices follow from that:
 *
 *  - **Undated to-dos are absent.** Including them would make this "every card
 *    on every board", which is the thing the boards already are.
 *  - **Overdue sorts first and is coloured**, because a missed date is the one
 *    state that needs to survive a glance.
 *  - **Dismiss and reschedule live on the row**, so the list can be brought
 *    back under control without opening four boards. Dismiss hides the to-do
 *    here and does not touch the card; rescheduling un-dismisses it.
 */

interface Props {
  todos: UseDueTodos;
}

export const TodosCard = memo(function TodosCard({ todos }: Props) {
  const { colors } = useTheme();
  const { groups, today, loading, error, refresh } = todos;
  const total = todos.todos.length;

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
      <SectionTitle title="To-dos" right={total > 0 ? `${total} due` : undefined} />

      {error ? (
        // An empty list and a failed fetch look identical if you only render
        // "nothing due" — and here the difference is "you're clear" versus
        // "you have no idea what's due". Say which one it is.
        <Centered>
          <div style={{ fontSize: textSize.small, color: colors.textMuted, marginBottom: 6 }}>
            Couldn't load your to-dos
          </div>
          <div style={{ fontSize: textSize.micro, color: colors.textDim, marginBottom: 10 }}>{error}</div>
          <Button
            colors={colors}
            type="button"
            onClick={refresh}
            style={{
              '--pa-btn-bg': 'transparent',
              '--pa-btn-fg': colors.textMuted,
              '--pa-btn-border': colors.border,
              '--pa-btn-bg-hover': colors.surfaceHi,
              '--pa-btn-fg-hover': colors.text,
              '--pa-btn-border-hover': colors.borderHi,
              '--pa-btn-bg-active': colors.surface,
              '--pa-btn-pad': '4px 10px',
              '--pa-btn-radius': `${radius.sm}px`,
              '--pa-btn-weight': 400,
              fontSize: textSize.micro,
            } as CSSProperties}
          >Try again</Button>
        </Centered>
      ) : loading && total === 0 ? (
        <EmptyNote>Loading…</EmptyNote>
      ) : total === 0 ? (
        <EmptyNote hint="Give a card on any board a due date and it shows up here">
          Nothing due
        </EmptyNote>
      ) : (
        <div style={{ flex: 1, overflowY: 'auto', marginTop: 4, marginRight: -8, paddingRight: 8 }}>
          {groups.map(group => (
            <Group key={group.bucket} group={group} today={today} todos={todos} />
          ))}
        </div>
      )}
    </div>
  );
});

function Centered({ children }: { children: React.ReactNode }) {
  return (
    <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
      <div style={{ textAlign: 'center' }}>{children}</div>
    </div>
  );
}

function Group({ group, today, todos }: { group: DueGroup; today: string; todos: UseDueTodos }) {
  const { colors } = useTheme();
  const overdue = group.bucket === 'overdue';
  return (
    <div style={{ marginBottom: 14 }}>
      <div style={{
        fontSize: 10, textTransform: 'uppercase', letterSpacing: 0.6,
        color: overdue ? colors.danger : colors.textDim,
        fontWeight: 600, marginBottom: 6, position: 'sticky', top: 0,
        background: colors.surface, paddingBottom: 2,
      }}>
        {group.label} · {group.todos.length}
      </div>
      {group.todos.map(todo => (
        <TodoRow key={todo.id} todo={todo} bucket={group.bucket} today={today} todos={todos} />
      ))}
    </div>
  );
}

function TodoRow({
  todo, bucket, today, todos,
}: { todo: DueTodo; bucket: DueBucket; today: string; todos: UseDueTodos }) {
  const { colors } = useTheme();
  const [hovered, setHovered] = useState(false);
  const [editing, setEditing] = useState(false);
  const [busy, setBusy] = useState(false);
  const openCardOnBoard = useCommandCenter(s => s.openCardOnBoard);
  const overdue = bucket === 'overdue';

  const open = () => openCardOnBoard(todo.projectId, todo.id);

  const reschedule = async (value: string) => {
    setBusy(true);
    try { await todos.setDueDate(todo, value || null); }
    finally { setBusy(false); setEditing(false); }
  };

  return (
    <div
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      style={{
        display: 'flex', alignItems: 'center', gap: 10,
        padding: '7px 8px', marginBottom: 2,
        borderRadius: radius.sm,
        background: hovered ? colors.borderHi : 'transparent',
        opacity: busy ? 0.5 : 1,
        transition: 'background 120ms ease, opacity 120ms ease',
      }}
    >
      {/* Title + provenance. Clicking opens the card on its own board.
          Not on the Button primitive: this is a two-line block whose title and
          provenance lines truncate against the width the button's own `flex: 1`
          gives them. The primitive wraps its children in one span, which has no
          such width to truncate against — the row would overflow instead of
          ellipsing. Left as a raw button. */}
      <button
        onClick={open}
        title={`Open on the ${todo.projectName} board`}
        style={{
          flex: 1, minWidth: 0, textAlign: 'left', cursor: 'pointer',
          background: 'transparent', border: 'none', padding: 0, color: 'inherit', font: 'inherit',
        }}
      >
        <div style={{
          fontSize: 12.5, color: colors.text,
          whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
        }}>{todo.title}</div>
        <div style={{
          fontSize: 10.5, color: colors.textDim, marginTop: 2,
          whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
        }}>
          {todo.projectName} · {todo.columnName}
        </div>
      </button>

      {editing ? (
        <input
          type="date"
          autoFocus
          defaultValue={todo.dueDate}
          disabled={busy}
          onBlur={e => void reschedule(e.target.value)}
          onKeyDown={e => {
            if (e.key === 'Enter') void reschedule((e.target as HTMLInputElement).value);
            if (e.key === 'Escape') setEditing(false);
          }}
          style={{
            fontSize: textSize.micro, padding: '2px 4px', colorScheme: 'inherit',
            borderRadius: radius.sm, border: `1px solid ${colors.border}`,
            background: colors.surface, color: colors.text,
          }}
        />
      ) : (
        <Button
          colors={colors}
          variant="bare"
          type="button"
          onClick={() => setEditing(true)}
          title="Change the due date"
          style={{
            '--pa-btn-bg': 'transparent',
            '--pa-btn-fg': overdue ? colors.danger : colors.textMuted,
            '--pa-btn-border': 'transparent',
            '--pa-btn-bg-hover': colors.surfaceHi,
            '--pa-btn-fg-hover': overdue ? colors.danger : colors.text,
            '--pa-btn-bg-active': colors.surface,
            '--pa-btn-pad': '2px 4px',
            '--pa-btn-radius': `${radius.sm}px`,
            '--pa-btn-weight': overdue ? 600 : 400,
            fontSize: 10.5, whiteSpace: 'nowrap',
            // Driven by the ROW's hover, not this button's — keep it inline.
            textDecoration: hovered ? 'underline' : 'none',
          } as CSSProperties}
        >{relativeDueLabel(todo.dueDate, today)}</Button>
      )}

      <Button
        colors={colors}
        variant="bare"
        type="button"
        onClick={() => todos.dismiss(todo)}
        disabled={busy}
        aria-label={`Dismiss ${todo.title}`}
        title="Hide from this list — the card stays on its board"
        style={{
          '--pa-btn-bg': 'transparent',
          '--pa-btn-fg': colors.textDim,
          '--pa-btn-border': 'transparent',
          '--pa-btn-bg-hover': colors.surfaceHi,
          '--pa-btn-fg-hover': colors.text,
          '--pa-btn-bg-active': colors.surface,
          '--pa-btn-pad': '0',
          '--pa-btn-radius': `${radius.sm}px`,
          '--pa-btn-weight': 400,
          width: 20, height: 20, flexShrink: 0, lineHeight: '18px',
          fontSize: textSize.small,
          // Revealed by the ROW's hover, not this button's — keep it inline.
          visibility: hovered ? 'visible' : 'hidden',
        } as CSSProperties}
      >×</Button>
    </div>
  );
}
