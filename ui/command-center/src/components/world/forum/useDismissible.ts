import { useCallback, useMemo, useState } from 'react';

/**
 * Which of the world's cards the user has put away, remembered between visits.
 *
 * The user: "All of the cards on the world need to be dismissable." The world is
 * the thing worth looking at, and every panel on top of it is the player's
 * choice to keep. Dismissal is therefore permanent until they ask for it back,
 * not a per-session tidy-up — a card you have to close on every load is a card
 * that is closing itself against your wishes.
 *
 * Storage failures are not errors here. A private window still gets working
 * dismiss buttons; it just forgets them, which is exactly what a private window
 * is for.
 */
const STORAGE_KEY = 'permagent.forum.dismissed';

export type CardId = 'brand' | 'place' | 'feed' | 'passport' | 'hints';

/** Cards that can be put away, in the order the restore menu lists them. */
export const DISMISSIBLE: { id: CardId; label: string }[] = [
  { id: 'brand', label: 'World title' },
  { id: 'place', label: 'Place card' },
  { id: 'feed', label: 'Job feed' },
  { id: 'passport', label: 'Sovereign passport' },
  { id: 'hints', label: 'Control hints' },
];

function read(): Set<CardId> {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return new Set();
    const parsed = JSON.parse(raw) as unknown;
    if (!Array.isArray(parsed)) return new Set();
    const known = new Set(DISMISSIBLE.map((card) => card.id));
    return new Set(parsed.filter((id): id is CardId => typeof id === 'string' && known.has(id as CardId)));
  } catch {
    return new Set();
  }
}

export function useDismissible() {
  const [dismissed, setDismissed] = useState<Set<CardId>>(read);

  const persist = useCallback((next: Set<CardId>) => {
    setDismissed(next);
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify([...next]));
    } catch {
      /* a private window keeps the behaviour and forgets the preference */
    }
  }, []);

  const dismiss = useCallback(
    (id: CardId) => {
      const next = new Set(dismissed);
      next.add(id);
      persist(next);
    },
    [dismissed, persist],
  );

  const restore = useCallback(
    (id: CardId) => {
      const next = new Set(dismissed);
      next.delete(id);
      persist(next);
    },
    [dismissed, persist],
  );

  const restoreAll = useCallback(() => persist(new Set()), [persist]);

  return useMemo(
    () => ({
      isDismissed: (id: CardId) => dismissed.has(id),
      hiddenCount: dismissed.size,
      dismiss,
      restore,
      restoreAll,
    }),
    [dismissed, dismiss, restore, restoreAll],
  );
}
