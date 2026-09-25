// Live signal for a Projects-list card: open tasks, goals in flight, and the
// most recent card activity.
//
// QA walk 2026-09-24 (P3): every card in the Projects grid showed only name,
// description and tags — the same three static fields whether a project had
// six goals in flight or had not been touched in a month. There was no way to
// tell which projects needed attention without opening each one.
//
// Reuses the board's own existing endpoints (`GET .../cards`, `GET
// .../columns` — both already served for the Kanban lens) rather than adding
// a route: a card carries no status of its own, only a `columnId`, and a
// column's `stateBinding` is what says whether that column is a settled state
// (`complete` / `cancelled`) or an open one. No new backend route.
import { useEffect, useRef, useState } from 'react';
import { apiFetch } from '../../lib/api';
import type { BoardColumn, Card } from './types';

export interface ProjectSignal {
  openTasks: number;
  goalsInFlight: number;
  /** ISO timestamp of the most recently touched card, or null if the
   *  project has no cards yet. */
  lastActivityAt: string | null;
}

/** Columns bound to one of these are settled — not "open" or "in flight". */
const TERMINAL_STATES = new Set(['complete', 'cancelled']);

const EMPTY_SIGNAL: ProjectSignal = { openTasks: 0, goalsInFlight: 0, lastActivityAt: null };

function summarize(cards: Card[], columns: BoardColumn[]): ProjectSignal {
  const bindingById = new Map(columns.map(c => [c.id, c.stateBinding ?? null]));
  let openTasks = 0;
  let goalsInFlight = 0;
  let lastActivityAt: string | null = null;
  for (const card of cards) {
    if (card.archivedAt) continue;
    const binding = bindingById.get(card.columnId) ?? null;
    const isOpen = !binding || !TERMINAL_STATES.has(binding);
    if (card.cardType === 'goal') {
      if (isOpen) goalsInFlight++;
    } else if (isOpen) {
      openTasks++;
    }
    if (!lastActivityAt || Date.parse(card.updatedAt) > Date.parse(lastActivityAt)) {
      lastActivityAt = card.updatedAt;
    }
  }
  return { openTasks, goalsInFlight, lastActivityAt };
}

/**
 * Fetches each project's cards + columns in parallel and folds them into a
 * per-project signal. Keyed by the joined id list, so it re-fetches only when
 * the SET of visible projects actually changes (not on every re-render, and
 * not lost on a reorder within the same set). A project whose fetch fails
 * reads as "nothing to show" rather than breaking the whole grid.
 */
export function useProjectSignals(projectIds: string[]): Record<string, ProjectSignal> {
  const [signals, setSignals] = useState<Record<string, ProjectSignal>>({});
  const key = projectIds.join(',');
  const generation = useRef(0);

  useEffect(() => {
    if (projectIds.length === 0) {
      setSignals({});
      return;
    }
    const gen = ++generation.current;
    let cancelled = false;
    (async () => {
      const entries = await Promise.all(projectIds.map(async (id): Promise<readonly [string, ProjectSignal]> => {
        try {
          const [cards, columns] = await Promise.all([
            apiFetch<Card[]>(`/api/projects/${encodeURIComponent(id)}/cards`),
            apiFetch<BoardColumn[]>(`/api/projects/${encodeURIComponent(id)}/columns`),
          ]);
          return [id, summarize(cards, columns)] as const;
        } catch {
          return [id, EMPTY_SIGNAL] as const;
        }
      }));
      if (cancelled || gen !== generation.current) return;
      setSignals(Object.fromEntries(entries));
    })();
    return () => { cancelled = true; };
  }, [key]); // eslint-disable-line react-hooks/exhaustive-deps

  return signals;
}
