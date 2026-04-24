import { create } from 'zustand';
import type { PermagentEventType } from './store';

export interface PermagentEvent {
  id: string;
  type: PermagentEventType;
  timestamp: string;
  payload: Record<string, unknown>;
}

export interface EventFilter {
  types?: PermagentEventType[];
  after?: string; // ISO 8601
  before?: string; // ISO 8601
}

const MAX_BUFFER = 1000;
const PRUNE_AGE_MS = 24 * 60 * 60 * 1000; // 24 hours

interface EventBusState {
  events: PermagentEvent[];
  addEvent: (event: PermagentEvent) => void;
  getEvents: (filter?: EventFilter) => PermagentEvent[];
  clearEvents: () => void;
  pruneOld: () => void;
}

function matchesFilter(event: PermagentEvent, filter: EventFilter): boolean {
  if (filter.types && filter.types.length > 0 && !filter.types.includes(event.type)) {
    return false;
  }
  if (filter.after && event.timestamp < filter.after) {
    return false;
  }
  if (filter.before && event.timestamp > filter.before) {
    return false;
  }
  return true;
}

export const useEventBus = create<EventBusState>((set, get) => ({
  events: [],

  addEvent: (event) => {
    set(s => ({
      events: [event, ...s.events].slice(0, MAX_BUFFER),
    }));
  },

  getEvents: (filter) => {
    const { events } = get();
    if (!filter) return events;
    return events.filter(e => matchesFilter(e, filter));
  },

  clearEvents: () => set({ events: [] }),

  pruneOld: () => {
    const cutoff = new Date(Date.now() - PRUNE_AGE_MS).toISOString();
    set(s => ({
      events: s.events.filter(e => e.timestamp >= cutoff),
    }));
  },
}));

// Auto-prune every 5 minutes
let _pruneInterval: ReturnType<typeof setInterval> | null = null;

export function startEventPruning() {
  if (_pruneInterval) return;
  _pruneInterval = setInterval(() => {
    useEventBus.getState().pruneOld();
  }, 5 * 60 * 1000);
}

export function stopEventPruning() {
  if (_pruneInterval) {
    clearInterval(_pruneInterval);
    _pruneInterval = null;
  }
}
