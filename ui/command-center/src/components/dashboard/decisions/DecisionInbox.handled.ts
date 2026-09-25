/**
 * The header's quiet "handled" link (U18). It replaces both the old
 * "handled overnight" row — which labelled an ALL-TIME, any-actor count as
 * "overnight" — and the History footer.
 *
 * With the daemon's windowed agent-only count (`handled: {count, since}`,
 * daemon lane D3) it reads "4 handled since 9 PM". Without it, it falls back
 * to "N handled" from the legacy total and names no window at all: saying
 * "overnight" or "since…" about a count that has no window is the defect.
 */
import type { DecisionsResponse } from './types';

export interface HandledSummary {
  count: number;
  /** "since 9 PM", "since yesterday 9 PM", or null when there is no window. */
  since: string | null;
}

/** "9 PM" / "9:30 PM" — minutes only when they are not zero. */
function clock(t: Date): string {
  return t.toLocaleTimeString('en-US', {
    hour: 'numeric', ...(t.getMinutes() ? { minute: '2-digit' } : {}),
  });
}

export function windowStartLabel(sinceIso: string, now: Date = new Date()): string | null {
  const t = new Date(sinceIso);
  if (Number.isNaN(t.getTime())) return null;
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const day = 86_400_000;
  if (t.getTime() >= startOfToday) return clock(t);
  if (t.getTime() >= startOfToday - day) return `yesterday ${clock(t)}`;
  return t.toLocaleDateString('en-US', { weekday: 'short', month: 'short', day: 'numeric' });
}

export function handledSummary(data: Pick<DecisionsResponse, 'handled_count' | 'handled'> | null, now: Date = new Date()): HandledSummary {
  const w = data?.handled;
  if (w && typeof w.count === 'number' && Number.isFinite(w.count) && typeof w.since === 'string') {
    const since = windowStartLabel(w.since, now);
    return { count: Math.max(0, w.count), since: since ? `since ${since}` : null };
  }
  return { count: Math.max(0, data?.handled_count ?? 0), since: null };
}

/** Link text: "4 handled since 9 PM", "4 handled", or "History" at zero. */
export function handledLinkLabel(summary: HandledSummary): string {
  if (summary.count === 0) return 'History';
  return summary.since ? `${summary.count} handled ${summary.since}` : `${summary.count} handled`;
}

/** Empty-state detail: "Henry handled 4 routine items since 9 PM." */
export function handledSentence(summary: HandledSummary, agentName: string): string | null {
  if (summary.count === 0) return null;
  const items = `${summary.count} item${summary.count === 1 ? '' : 's'}`;
  return summary.since
    ? `${agentName} handled ${items} ${summary.since}.`
    : `${agentName} has handled ${items} so far.`;
}
