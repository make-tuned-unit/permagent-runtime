// What the project's "Recent activity" feed says about each task.
//
// The feed is derived from the board's CURRENT cards — the daemon keeps no
// per-card change history, so there is no "status from → to" to show. What a
// card does carry is enough to say more than "Task updated" on every line
// (QA walk 2026-09-24: a long list of identical "Task updated" rows):
//   - created and never touched since      → "Added to <column>"
//   - archived                             → "Archived from <column>"
//   - otherwise                            → "Updated · now in <column>"
// plus the due date when the card has one. Consecutive entries that say the
// same thing about the same column (an agent moving six tasks to Done in one
// pass) fold into one line that names them.

import type { BoardColumn, Card } from './types';

export type FeedKind = 'memory' | 'note' | 'task';

export interface FeedItem {
  id: string;
  kind: FeedKind;
  title: string;
  detail: string;
  timestamp: string;
  /** Tasks only: what happened, without the task's name, for grouping. */
  action?: string;
  /** Tasks folded into this line (a group), newest first. */
  tasks?: string[];
}

/** A card touched within this long of its creation counts as "added". */
const ADDED_WINDOW_MS = 60_000;

function shortDate(iso: string): string | null {
  const t = Date.parse(`${iso}T00:00:00`);
  if (Number.isNaN(t)) return null;
  return new Date(t).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
}

export function describeTask(card: Card, columns: Map<string, BoardColumn>): FeedItem {
  const column = columns.get(card.columnId)?.name;
  const where = column ?? 'the board';
  const created = Date.parse(card.createdAt);
  const updated = Date.parse(card.updatedAt);
  let action: string;
  let timestamp = card.updatedAt;
  if (card.archivedAt) {
    action = `Archived from ${where}`;
    timestamp = card.archivedAt;
  } else if (Number.isFinite(created) && Number.isFinite(updated) && updated - created < ADDED_WINDOW_MS) {
    action = `Added to ${where}`;
    timestamp = card.createdAt;
  } else {
    action = column ? `Updated · now in ${column}` : 'Updated';
  }
  const due = typeof card.metadataJson?.dueDate === 'string' ? shortDate(card.metadataJson.dueDate as string) : null;
  const noun = card.cardType === 'goal' ? 'Goal' : 'Task';
  return {
    id: `task-${card.id}`,
    kind: 'task',
    title: card.title,
    detail: `${noun} · ${action}${due ? ` · due ${due}` : ''}`,
    timestamp,
    action,
  };
}

/**
 * Fold runs of consecutive task lines with the same action into one line.
 * `items` must already be newest-first. A run of one is left as it was.
 */
export function groupTaskRuns(items: FeedItem[]): FeedItem[] {
  const out: FeedItem[] = [];
  for (const item of items) {
    const prev = out[out.length - 1];
    if (item.kind === 'task' && prev?.kind === 'task' && prev.action && prev.action === item.action) {
      const tasks = prev.tasks ?? [prev.title];
      tasks.push(item.title);
      out[out.length - 1] = {
        ...prev,
        id: `${prev.id}+`,
        tasks,
        title: `${tasks.length} tasks · ${prev.action}`,
        detail: tasks.length <= 3
          ? tasks.join(' · ')
          : `${tasks.slice(0, 3).join(' · ')} · +${tasks.length - 3} more`,
      };
      continue;
    }
    out.push(item);
  }
  return out;
}
