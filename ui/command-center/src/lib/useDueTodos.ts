/**
 * Dated to-dos across every kanban board, for the dashboard's due list.
 *
 * The daemon (`GET /api/cards/due`) owns the *scope* question — which cards
 * count as unfinished to-dos — and returns them already sorted by due date.
 * This module owns only the *relative* question: how does each date compare to
 * today. That split matters because "today" is a client-side notion (the user's
 * local calendar day), and a card sitting in a browser overnight must not keep
 * calling yesterday "today".
 */

import { useCallback, useEffect, useMemo, useState } from 'react';
import { apiFetch } from './api';

export interface DueTodo {
  id: string;
  title: string;
  projectId: string;
  projectName: string;
  columnId: string;
  columnName: string;
  /** ISO-8601 calendar date, `YYYY-MM-DD`. */
  dueDate: string;
  assignedTo: string | null;
  updatedAt: string;
}

export type DueBucket = 'overdue' | 'today' | 'tomorrow' | 'week' | 'later';

export const BUCKET_LABEL: Record<DueBucket, string> = {
  overdue: 'Overdue',
  today: 'Today',
  tomorrow: 'Tomorrow',
  week: 'This week',
  later: 'Later',
};

/** Bucket order is display order — overdue first, as agreed. */
export const BUCKET_ORDER: DueBucket[] = ['overdue', 'today', 'tomorrow', 'week', 'later'];

/**
 * Local calendar day as `YYYY-MM-DD`.
 *
 * Deliberately NOT `toISOString().slice(0, 10)`, which converts to UTC first
 * and so reports tomorrow's date for anyone east of Greenwich in the evening —
 * the exact population that would see a to-do due today filed under "overdue".
 */
export function localIsoDate(now: Date): string {
  const year = now.getFullYear();
  const month = `${now.getMonth() + 1}`.padStart(2, '0');
  const day = `${now.getDate()}`.padStart(2, '0');
  return `${year}-${month}-${day}`;
}

/** Whole days from `today` to `date`; negative means the date has passed. */
export function daysUntil(dueDate: string, today: string): number {
  // Parse as UTC midnight on both sides. Both values are plain calendar dates
  // with no zone, so comparing them in a single fixed zone avoids DST making a
  // difference of exactly one day come out as 0.96 and floor to the day before.
  const due = Date.parse(`${dueDate}T00:00:00Z`);
  const ref = Date.parse(`${today}T00:00:00Z`);
  if (Number.isNaN(due) || Number.isNaN(ref)) return 0;
  return Math.round((due - ref) / 86_400_000);
}

export function bucketFor(dueDate: string, today: string): DueBucket {
  const days = daysUntil(dueDate, today);
  if (days < 0) return 'overdue';
  if (days === 0) return 'today';
  if (days === 1) return 'tomorrow';
  if (days <= 7) return 'week';
  return 'later';
}

/** A human phrase for how overdue / how soon a to-do is. */
export function relativeDueLabel(dueDate: string, today: string): string {
  const days = daysUntil(dueDate, today);
  if (days === 0) return 'due today';
  if (days === 1) return 'due tomorrow';
  if (days === -1) return '1 day overdue';
  if (days < 0) return `${Math.abs(days)} days overdue`;
  return `in ${days} days`;
}

export interface DueGroup {
  bucket: DueBucket;
  label: string;
  todos: DueTodo[];
}

/**
 * Group the daemon's date-sorted list into display buckets, preserving order
 * and dropping empty buckets.
 */
export function groupByBucket(todos: DueTodo[], today: string): DueGroup[] {
  const byBucket = new Map<DueBucket, DueTodo[]>();
  for (const todo of todos) {
    const bucket = bucketFor(todo.dueDate, today);
    const list = byBucket.get(bucket);
    if (list) list.push(todo);
    else byBucket.set(bucket, [todo]);
  }
  return BUCKET_ORDER.filter(b => byBucket.has(b)).map(bucket => ({
    bucket,
    label: BUCKET_LABEL[bucket],
    todos: byBucket.get(bucket)!,
  }));
}

const POLL_MS = 60_000;

export interface UseDueTodos {
  todos: DueTodo[];
  groups: DueGroup[];
  /** Local calendar day the grouping is relative to. */
  today: string;
  loading: boolean;
  /** Set when the list could not be loaded — the card says so rather than
   *  rendering an empty state that would read as "nothing is due". */
  error: string | null;
  refresh: () => void;
  setDueDate: (todo: DueTodo, dueDate: string | null) => Promise<void>;
  dismiss: (todo: DueTodo) => Promise<void>;
}

export function useDueTodos(): UseDueTodos {
  const [todos, setTodos] = useState<DueTodo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [today, setToday] = useState(() => localIsoDate(new Date()));

  const load = useCallback(async () => {
    try {
      const list = await apiFetch<DueTodo[]>('/api/cards/due');
      setTodos(Array.isArray(list) ? list : []);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Could not load your to-dos');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    const tick = () => {
      if (cancelled) return;
      // Re-derive "today" on every tick so a window left open past midnight
      // re-buckets instead of insisting yesterday is still today.
      setToday(localIsoDate(new Date()));
      void load();
    };
    tick();
    const timer = setInterval(tick, POLL_MS);
    return () => { cancelled = true; clearInterval(timer); };
  }, [load]);

  const setDueDate = useCallback(async (todo: DueTodo, dueDate: string | null) => {
    // Optimistic: drop or re-date the row immediately, then reconcile.
    setTodos(prev => (dueDate === null
      ? prev.filter(t => t.id !== todo.id)
      : prev.map(t => (t.id === todo.id ? { ...t, dueDate } : t))
        .sort((a, b) => a.dueDate.localeCompare(b.dueDate))));
    try {
      await apiFetch(`/api/projects/${todo.projectId}/cards/${todo.id}/due-date`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ dueDate }),
      });
    } finally {
      await load();
    }
  }, [load]);

  const dismiss = useCallback(async (todo: DueTodo) => {
    setTodos(prev => prev.filter(t => t.id !== todo.id));
    try {
      await apiFetch(`/api/projects/${todo.projectId}/cards/${todo.id}/dismiss-due`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ dismissed: true }),
      });
    } finally {
      await load();
    }
  }, [load]);

  const groups = useMemo(() => groupByBucket(todos, today), [todos, today]);

  return { todos, groups, today, loading, error, refresh: load, setDueDate, dismiss };
}
