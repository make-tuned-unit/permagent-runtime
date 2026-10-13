/**
 * World W1 — size bounds and freshness thresholds for World evidence.
 *
 * Every number here is a DISPLAY bound, not a claim about how much exists.
 * When a source holds more than a bound, the section says `truncated: true`
 * and the UI must say "showing N of more", never imply the list is complete
 * (Astra 2026-10-12, goalActivity.ts finding: a list length must not
 * masquerade as the complete current workload).
 *
 * Why these values:
 * - A place panel is a reading surface at 1280×800 beside a 3D scene, not a
 *   table view. Eight records is roughly what fits in one scroll of a desk
 *   panel at the type ramp the Forum desk uses; the canonical home view
 *   (Brain / History / Grow / Projects) is one record link away for the rest.
 * - Text caps keep one pathological record (a 40 kB memory, a long market
 *   history) from dominating layout or memory. The full text stays one
 *   `Open in Brain` away.
 * - The `/work` limit is small because the World only shows recent attributed
 *   work; the daemon clamps 1..200 (agents_surface.rs:750) and a smaller page is
 *   cheaper for the daemon (it reads activity, goals and decision history).
 */
export const BOUNDS = {
  /** Memories shown in the Garden archive folio. */
  memories: 8,
  /** Characters of memory text kept per record (full text lives in Brain). */
  memoryTextChars: 600,
  /** `limit` sent to GET /api/agents/{id}/work (daemon clamps 1..200). */
  workLimit: 10,
  /** Attributed activity rows shown per agent. */
  activity: 10,
  /** Goals shown per list. */
  goals: 8,
  /** Open decisions shown (the daemon already ranks and caps at 10). */
  decisions: 5,
  /** Orchestrator active sessions offered as "Continue with…". */
  sessions: 3,
  /** Orchestrator recent tasks inspected for failed / cancelled requests. */
  requests: 6,
  /** Saved skills shown in the Maker workshop. */
  skills: 6,
  /** Market rows shown on the Observatory instrument. */
  forecasts: 6,
  /** Most-recent history points kept per market row. */
  forecastHistoryPoints: 60,
  /** Council reports requested and shown (`?limit=` on /api/council/reports). */
  councilReports: 3,
  /** Consensus lines kept per Council report. */
  councilConsensus: 5,
  /** Characters kept for a Council headline. */
  headlineChars: 200,
  /** Growth results shown (the daemon itself keeps only 12 fleet rows). */
  growthResults: 8,
  /** Project notes / hypotheses shown. */
  notes: 6,
  /** Characters of note body kept per record. */
  noteBodyChars: 600,
  /** Characters kept for any one-line title. */
  titleChars: 160,
} as const;

/** One minute: matches the existing World goal backstop poll (goalActivity.ts:70). */
const LIVE_MS = 60_000;
/** Five minutes: memories and notes change on user / Librarian action, not per tick. */
const SLOW_MS = 5 * 60_000;
/** Fifteen minutes: market and growth data are daily-cadence series. */
const SERIES_MS = 15 * 60_000;

/**
 * Freshness thresholds per evidence family. A section older than this is
 * rendered STALE (still visible, explicitly marked) — never silently dropped,
 * never silently turned into "nothing happened".
 */
export const STALE_AFTER_MS = {
  orchestrator: LIVE_MS,
  goals: LIVE_MS,
  decisions: LIVE_MS,
  work: LIVE_MS,
  agent: SLOW_MS,
  memories: SLOW_MS,
  notes: SLOW_MS,
  skills: SLOW_MS,
  council: SERIES_MS,
  market: SERIES_MS,
  growth: SERIES_MS,
} as const;

export type StaleFamily = keyof typeof STALE_AFTER_MS;

/**
 * After a request fails, a revisit inside this window reuses the failure
 * instead of re-requesting. Stops a user flicking between places from
 * hammering a daemon that is down; the next visit after it retries.
 */
export const FAILURE_RETRY_MS = 5_000;

/**
 * The hard ceiling on network requests one place visit may cause. Every place
 * plan in snapshotLoader.ts fits under it; snapshotLoader.test.ts proves it.
 */
export const MAX_REQUESTS_PER_PLACE_VISIT = 3;

/** Trim to `max` characters on a whitespace boundary where one is close. */
export function clipText(text: string, max: number): { text: string; clipped: boolean } {
  const chars = Array.from(text);
  if (chars.length <= max) return { text, clipped: false };
  const cut = chars.slice(0, max).join('');
  const space = cut.lastIndexOf(' ');
  const body = space > max * 0.6 ? cut.slice(0, space) : cut;
  return { text: `${body.trimEnd()}…`, clipped: true };
}
