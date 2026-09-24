import { memo, useState, type CSSProperties } from 'react';
import { duration, ease, radius, space, textSize } from '../../../styles/tokens';
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

import { Tooltip } from '../../common/Tooltip';
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
      boxShadow: [colors.elevationRaised, colors.cardHighlight].filter(Boolean).join(', '),
      padding: space.xxl,
      display: 'flex', flexDirection: 'column',
      overflow: 'hidden',
    }}>
      <SectionTitle title="To-dos" right={total > 0 ? `${total} due` : undefined} />

      {error ? (
        // An empty list and a failed fetch look identical if you only render
        // "nothing due" — and here the difference is "you're clear" versus
        // "you have no idea what's due". Say which one it is.
        <Centered>
          <div style={{ fontSize: textSize.small, color: colors.textMuted, marginBottom: space.sm }}>
            Couldn't load your to-dos
          </div>
          <div style={{ fontSize: textSize.micro, color: colors.textDim, marginBottom: space.lg }}>{error}</div>
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
        <div style={{ flex: 1, overflowY: 'auto', marginTop: space.xs, marginRight: -space.md, paddingRight: space.md }}>
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
        fontWeight: 600, marginBottom: space.sm, position: 'sticky', top: 0,
        background: colors.surface, paddingBottom: space.xs,
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
  const { colors, reduceMotion } = useTheme();
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
        display: 'flex', alignItems: 'flex-start', gap: space.lg,
        padding: `${space.md}px`, marginBottom: space.xxs,
        borderRadius: radius.sm,
        // Was `colors.borderHi` — a cyan-tinted hairline reused as a hover
        // fill. Neutral `fillHover` matches D10/D8: rows get a visible hover,
        // not a second tinted accent competing with the Decisions card's.
        background: hovered ? colors.fillHover : 'transparent',
        opacity: busy ? 0.5 : 1,
        transition: reduceMotion ? 'none' : `background ${duration.fast}ms ${ease.out}, opacity ${duration.fast}ms ${ease.out}`,
      }}
    >
      {/* Title, then provenance and due date on the second line.
          The due label used to sit on the title's row, nowrap, beside the
          dismiss button: at Home's real tile width (~245px) "16 days overdue"
          took most of the row and the title was cut to "Set …" (QA, real-size
          walk). It now shares the second line with the provenance, so the
          title owns the row's whole width and may wrap to two lines. The
          provenance truncates first; if even that leaves no room, the due
          label wraps under it rather than squeezing either. */}
      <div data-testid="todo-row-text" style={{ flex: 1, minWidth: 0 }}>
        {/* Not on the Button primitive: the primitive wraps its children in
            one span with no width to clamp against, so a long title would
            overflow instead of wrapping and clamping. Left as a raw button. */}
        <Tooltip content={`Open on the ${todo.projectName} board`}>
          <button
            data-testid="todo-row-title"
            onClick={open}
            style={{
              display: 'block', width: '100%', minWidth: 0,
              textAlign: 'left', cursor: 'pointer',
              background: 'transparent', border: 'none', padding: 0, color: 'inherit', font: 'inherit',
            }}
          >
            <span style={{
              display: '-webkit-box', WebkitLineClamp: 2, WebkitBoxOrient: 'vertical',
              overflow: 'hidden', overflowWrap: 'anywhere',
              fontSize: 12.5, lineHeight: 1.35, color: colors.text,
            }}>{todo.title}</span>
          </button>
        </Tooltip>
        <div
          data-testid="todo-row-meta"
          style={{
            display: 'flex', flexWrap: 'wrap', alignItems: 'center',
            columnGap: space.sm, rowGap: space.xxs, marginTop: space.xxs, minWidth: 0,
          }}
        >
          <span style={{
            flex: '0 1 auto', minWidth: 0,
            fontSize: 10.5, color: colors.textDim,
            whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
          }}>
            {todo.projectName} · {todo.columnName}
          </span>
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
                fontSize: textSize.micro, padding: `${space.xxs}px ${space.xs}px`, colorScheme: 'inherit',
                borderRadius: radius.sm, border: `1px solid ${colors.border}`,
                background: colors.surface, color: colors.text,
              }}
            />
          ) : (
            <Tooltip content="Change the due date">
              <Button
                colors={colors}
                variant="bare"
                type="button"
                data-testid="todo-row-due"
                onClick={() => setEditing(true)}
                style={{
                  '--pa-btn-bg': 'transparent',
                  '--pa-btn-fg': overdue ? colors.danger : colors.textMuted,
                  '--pa-btn-border': 'transparent',
                  '--pa-btn-bg-hover': colors.surfaceHi,
                  '--pa-btn-fg-hover': overdue ? colors.danger : colors.text,
                  '--pa-btn-bg-active': colors.surface,
                  // No side padding: the label lines up with the provenance
                  // text when it wraps under it.
                  '--pa-btn-pad': '0',
                  '--pa-btn-radius': `${radius.sm}px`,
                  '--pa-btn-weight': overdue ? 600 : 400,
                  fontSize: 10.5, whiteSpace: 'nowrap', flexShrink: 0,
                  // Driven by the ROW's hover, not this button's — keep it inline.
                  textDecoration: hovered ? 'underline' : 'none',
                } as CSSProperties}
              >{relativeDueLabel(todo.dueDate, today)}</Button>
            </Tooltip>
          )}
        </div>
      </div>

      <Tooltip content="Hide from this list — the card stays on its board">
        <span tabIndex={0} style={{ display: 'inline-flex', outline: 'none' }}>
          <Button
            colors={colors}
            variant="bare"
            type="button"
            onClick={() => todos.dismiss(todo)}
            disabled={busy}
            aria-label={`Dismiss ${todo.title}`}
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
        </span>
      </Tooltip>
    </div>
  );
}
