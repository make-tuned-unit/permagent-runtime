// Reattaching the terminal to PTYs that are still running (reported
// 2026-08-19: "I minimised the window for ten minutes and an hour of work in
// my coding session was gone").
//
// IT WAS NOT GONE. The PTY was alive the whole time — a `claude` process under
// `/bin/zsh -l`, 51 minutes old, owned by the Permagent app, on ttys006.
// Nothing killed it: `kill_pty` is only ever called from an explicit tab close
// (TerminalManager's two-click confirm) and from a detached window's real
// teardown. What died was the FRONTEND'S MEMORY OF ITS ID.
//
// `persistedTabs` is a module-level variable. It survives a React unmount —
// which is what it was written for, and it does that job — but it does not
// survive re-evaluation of the JS realm, and a WKWebView whose window is fully
// occluded for minutes is exactly when macOS reclaims the WebContent process
// and the page comes back freshly evaluated. (An accidental Cmd+R does the same
// thing: it is a native menu accelerator, so it fires even while the terminal
// has keyboard focus.) With the ids gone the manager fabricated one empty tab,
// `Terminal` saw no `sessionId` and spawned a brand-new shell at `$HOME`, and
// the live session became unreachable — addressable by nothing, still burning
// its work.
//
// Two things were missing, and both are here:
//
//   1. DURABLE tab records, so a re-evaluated realm still knows the ids.
//   2. A way to ASK the backend what is running, so ids that were lost anyway
//      can be adopted instead of orphaned. `browser.rs` learned this for native
//      webviews ("the React shell's memory of which webview_ids exist dies with
//      the page") and reaps them. A shell is not a webview: it holds work, so
//      the terminal's version of that operation adopts rather than reaps.
//
// The reconciliation is a pure function so it can be tested without a PTY.

import type { TerminalTab } from './TerminalManager';

/** One live PTY as `list_pty_sessions` reports it (see terminal.rs). */
export interface PtySessionInfo {
  session_id: string;
  cwd: string;
  started_at: string;
  alive: boolean;
  produced: number;
}

/** What survives a realm re-evaluation. */
export interface StoredTerminalState {
  tabs: TerminalTab[];
  activeTabId: string | null;
  /**
   * Sessions handed to a detached pane window. That window has its own realm
   * and its own memory of them, so the docked manager must never adopt one —
   * two panes writing to one PTY is worse than an orphan.
   */
  detachedSessionIds: string[];
}

export const TERMINAL_STATE_KEY = 'permagent.terminal.state.v1';

/**
 * Read the durable tab record. Returns null for absent, unparseable or
 * structurally wrong data — a corrupt record must degrade to "cold start", not
 * throw during a render-phase initializer.
 */
export function readStoredState(storage: Storage | null | undefined): StoredTerminalState | null {
  if (!storage) return null;
  let raw: string | null = null;
  try {
    raw = storage.getItem(TERMINAL_STATE_KEY);
  } catch {
    return null;
  }
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as Partial<StoredTerminalState>;
    if (!parsed || !Array.isArray(parsed.tabs)) return null;
    const tabs = parsed.tabs.filter(
      (t): t is TerminalTab => !!t && typeof t.id === 'string' && typeof t.label === 'string',
    );
    if (tabs.length === 0) return null;
    return {
      tabs,
      activeTabId: typeof parsed.activeTabId === 'string' ? parsed.activeTabId : null,
      detachedSessionIds: Array.isArray(parsed.detachedSessionIds)
        ? parsed.detachedSessionIds.filter((s): s is string => typeof s === 'string')
        : [],
    };
  } catch {
    return null;
  }
}

/** Persist the tab record. Storage failures are non-fatal by design. */
export function writeStoredState(
  storage: Storage | null | undefined,
  state: StoredTerminalState,
): void {
  if (!storage) return;
  try {
    storage.setItem(TERMINAL_STATE_KEY, JSON.stringify(state));
  } catch {
    /* quota, private mode, a locked profile — never worth breaking the pane */
  }
}

/** `~/Documents/dev/GetLadle` -> `GetLadle`. Falls back to the whole string. */
export function labelForCwd(cwd: string): string {
  const trimmed = String(cwd || '').replace(/\/+$/, '');
  if (!trimmed) return 'Terminal';
  const base = trimmed.slice(trimmed.lastIndexOf('/') + 1);
  return base || trimmed;
}

export interface ReconcileArgs {
  /** The tabs this manager currently believes in. */
  tabs: TerminalTab[];
  /** What the backend reports. Ignored unless `listed`. */
  live: PtySessionInfo[];
  /**
   * Whether `live` is a real answer. A FAILED listing must change nothing:
   * treating "I could not ask" as "nothing is running" would clear every
   * session id and respawn every shell — the exact damage being fixed.
   */
  listed: boolean;
  /**
   * Adopt live sessions no tab claims. Only true on a COLD start (no tab record
   * from either the module variable or storage), because that is the only
   * situation in which an unclaimed session means "the UI forgot me" rather
   * than "that one belongs to somebody else".
   */
  adopt: boolean;
  detachedSessionIds: string[];
  /** Factory for a fresh, unspawned tab. */
  makeTab: () => TerminalTab;
}

/**
 * Bring the tab list back into agreement with the PTYs that actually exist.
 *
 * - a tab whose session has exited loses its id, so it gets a working shell
 *   instead of a pane wired to nothing;
 * - a live session no tab claims is adopted on a cold start, so the session the
 *   user has been working in comes back instead of being buried under a new one;
 * - the result is never empty — an empty pane is not a state the UI has.
 */
export function reconcileTabs(args: ReconcileArgs): TerminalTab[] {
  const { tabs, live, listed, adopt, detachedSessionIds, makeTab } = args;

  if (!listed) {
    return tabs.length > 0 ? tabs : [makeTab()];
  }

  const aliveIds = new Set(live.filter(s => s.alive).map(s => s.session_id));
  const detached = new Set(detachedSessionIds);

  const kept = tabs.map(tab =>
    tab.sessionId && !aliveIds.has(tab.sessionId) ? { ...tab, sessionId: null } : tab,
  );

  const claimed = new Set(kept.map(t => t.sessionId).filter((s): s is string => !!s));
  const adopted: TerminalTab[] = [];
  if (adopt) {
    for (const session of live) {
      if (!session.alive) continue;
      if (claimed.has(session.session_id)) continue;
      if (detached.has(session.session_id)) continue;
      adopted.push({
        id: `tab-adopted-${session.session_id}`,
        label: labelForCwd(session.cwd),
        sessionId: session.session_id,
        cwd: session.cwd,
      });
    }
  }

  const result = [...kept, ...adopted];
  return result.length > 0 ? result : [makeTab()];
}

/**
 * The active tab after reconciliation: keep the caller's choice when it still
 * exists, otherwise fall back to the first tab.
 */
export function resolveActiveTabId(tabs: TerminalTab[], preferred: string | null): string {
  if (preferred && tabs.some(t => t.id === preferred)) return preferred;
  return tabs[0]?.id ?? '';
}
