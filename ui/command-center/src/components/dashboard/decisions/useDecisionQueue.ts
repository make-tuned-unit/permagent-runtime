import { useState } from 'react';
import type { Decision } from './types';

/** D1 presentation order only; never changes server ranking or answers. */
export function useDecisionQueue({ decisions, focusedId: initialId = null }: {
  decisions: Decision[];
  focusedId?: string | null;
}) {
  const [deferred, setDeferred] = useState<Set<string>>(() => new Set());
  // "Later" lasts until anything new arrives (Linear's snooze-until-activity):
  // a decision id this queue has never seen clears every deferral.
  const [seen, setSeen] = useState<Set<string>>(() => new Set(decisions.map(d => d.id)));
  const fresh = decisions.filter(d => !seen.has(d.id));
  if (fresh.length > 0) {
    setSeen(new Set([...seen, ...fresh.map(d => d.id)]));
    if (deferred.size > 0) setDeferred(new Set());
  }
  const ordered = [...decisions].sort((a, b) => {
    const tail = Number(deferred.has(a.id)) - Number(deferred.has(b.id));
    if (deferred.has(a.id) && deferred.has(b.id)) return [...deferred].indexOf(a.id) - [...deferred].indexOf(b.id);
    const time = (Date.parse(a.created_at) || 0) - (Date.parse(b.created_at) || 0);
    return tail || b.tier - a.tier || time || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
  });
  const ids = ordered.map(d => d.id);
  const [state, setState] = useState<{ ids: string[]; id: string | null }>(() => ({
    ids, id: initialId && ids.includes(initialId) ? initialId : ids[0] ?? null,
  }));
  let focusedId = state.id;
  if (state.ids.join('\0') !== ids.join('\0')) {
    if (focusedId && !ids.includes(focusedId)) {
      const index = state.ids.indexOf(focusedId);
      focusedId = state.ids.slice(index + 1).find(id => ids.includes(id))
        ?? state.ids.slice(0, index).reverse().find(id => ids.includes(id)) ?? null;
    } else if (!focusedId && state.ids.length === 0) {
      focusedId = ids[0] ?? null;
    }
    setState({ ids, id: focusedId });
  }
  const index = focusedId ? ids.indexOf(focusedId) : -1;
  const focus = (id: string) => { if (ids.includes(id)) setState({ ids, id }); };
  return {
    ordered, focusedId, position: index < 0 ? null : { index, total: ids.length }, deferred,
    focus,
    next: () => { if (index >= 0 && index + 1 < ids.length) focus(ids[index + 1]); },
    prev: () => { if (index > 0) focus(ids[index - 1]); },
    later: () => {
      if (!focusedId) return;
      const id = focusedId;
      setDeferred(previous => new Set([...previous].filter(candidate => candidate !== id).concat(id)));
      const nextId = ids[index + 1] ?? ids.find(candidate => candidate !== id);
      if (nextId) focus(nextId);
    },
  };
}
