import { create } from 'zustand';
import { hydrate, type IdentityState } from '../services/identity';

// ── localStorage persistence ─────────────────────────────────────────

const LS_KEY = 'permagent-identity-state';

function loadPersistedState(): { data: IdentityState | null; lastSuccessfulFetch: Date | null } {
  try {
    const raw = localStorage.getItem(LS_KEY);
    if (!raw) return { data: null, lastSuccessfulFetch: null };
    const parsed = JSON.parse(raw);
    return {
      data: parsed.data ?? null,
      lastSuccessfulFetch: parsed.lastSuccessfulFetch ? new Date(parsed.lastSuccessfulFetch) : null,
    };
  } catch {
    return { data: null, lastSuccessfulFetch: null };
  }
}

function persistState(data: IdentityState, lastSuccessfulFetch: Date): void {
  try {
    localStorage.setItem(LS_KEY, JSON.stringify({ data, lastSuccessfulFetch: lastSuccessfulFetch.toISOString() }));
  } catch {
    // storage full or unavailable
  }
}

// ── Store types ──────────────────────────────────────────────────────

type Connectivity = 'ok' | 'degraded' | 'offline';

interface IdentityStore {
  data: IdentityState | null;
  loading: boolean;
  error: string | null;
  lastSuccessfulFetch: Date | null;
  lastAttemptedFetch: Date | null;
  connectivity: Connectivity;
  consecutiveFailures: number;

  fetch: () => Promise<void>;
  refresh: () => Promise<void>;
  clear: () => void;

  _pollInterval: ReturnType<typeof setInterval> | null;
  /** Consumers currently holding the shared poll (ref-count). The interval is
   *  created when the count goes 0→1 and cleared only when it returns to 0,
   *  so a second consumer unmounting can neither orphan another consumer's
   *  polling nor duplicate the interval. */
  _pollRefs: number;
  startPolling: () => void;
  stopPolling: () => void;
}

// ── Initial state from localStorage ──────────────────────────────────

const persisted = loadPersistedState();

// ── Store ────────────────────────────────────────────────────────────

export const useIdentityStore = create<IdentityStore>((set, get) => ({
  data: persisted.data,
  loading: false,
  error: null,
  lastSuccessfulFetch: persisted.lastSuccessfulFetch,
  lastAttemptedFetch: null,
  connectivity: 'ok',
  consecutiveFailures: 0,

  fetch: async () => {
    const state = get();
    if (state.loading) return;
    set({ loading: true });

    try {
      const result = await hydrate(false);
      const now = new Date();
      const failures = result.chainReachable ? 0 : state.consecutiveFailures + 1;
      const connectivity: Connectivity = failures >= 3 ? 'offline' : failures > 0 ? 'degraded' : 'ok';

      set({
        data: result.data,
        loading: false,
        error: result.errors.length > 0 ? result.errors.join('; ') : null,
        lastAttemptedFetch: now,
        lastSuccessfulFetch: result.errors.length === 0 ? now : state.lastSuccessfulFetch,
        connectivity,
        consecutiveFailures: failures,
      });

      persistState(result.data, result.errors.length === 0 ? now : (state.lastSuccessfulFetch ?? now));
    } catch (err) {
      const failures = get().consecutiveFailures + 1;
      set({
        loading: false,
        error: err instanceof Error ? err.message : 'Unknown error',
        lastAttemptedFetch: new Date(),
        connectivity: failures >= 3 ? 'offline' : 'degraded',
        consecutiveFailures: failures,
      });
    }
  },

  refresh: async () => {
    set({ loading: true });

    try {
      const result = await hydrate(true);
      const now = new Date();
      const failures = result.chainReachable ? 0 : get().consecutiveFailures + 1;
      const connectivity: Connectivity = failures >= 3 ? 'offline' : failures > 0 ? 'degraded' : 'ok';

      set({
        data: result.data,
        loading: false,
        error: result.errors.length > 0 ? result.errors.join('; ') : null,
        lastAttemptedFetch: now,
        lastSuccessfulFetch: result.errors.length === 0 ? now : get().lastSuccessfulFetch,
        connectivity,
        consecutiveFailures: failures,
      });

      persistState(result.data, result.errors.length === 0 ? now : (get().lastSuccessfulFetch ?? now));
    } catch (err) {
      const failures = get().consecutiveFailures + 1;
      set({
        loading: false,
        error: err instanceof Error ? err.message : 'Unknown error',
        lastAttemptedFetch: new Date(),
        connectivity: failures >= 3 ? 'offline' : 'degraded',
        consecutiveFailures: failures,
      });
    }
  },

  clear: () => {
    try { localStorage.removeItem(LS_KEY); } catch { /* */ }
    set({
      data: null,
      loading: false,
      error: null,
      lastSuccessfulFetch: null,
      lastAttemptedFetch: null,
      connectivity: 'ok',
      consecutiveFailures: 0,
    });
  },

  _pollInterval: null,
  _pollRefs: 0,

  startPolling: () => {
    set({ _pollRefs: get()._pollRefs + 1 });
    // Already live — this consumer just joins the shared interval.
    if (get()._pollInterval) return;

    // Initial fetch
    get().fetch();

    // 10-minute polling
    const interval = setInterval(() => {
      get().fetch();
    }, 10 * 60 * 1000);

    set({ _pollInterval: interval });
  },

  stopPolling: () => {
    // Clamp at zero so a spurious extra stop can never wedge future starts.
    const refs = Math.max(0, get()._pollRefs - 1);
    set({ _pollRefs: refs });
    if (refs > 0) return; // another consumer still needs the poll

    const interval = get()._pollInterval;
    if (interval) {
      clearInterval(interval);
      set({ _pollInterval: null });
    }
  },
}));
