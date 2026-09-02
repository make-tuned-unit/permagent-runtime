// Formatting helpers for the Grow analytics panels. Pure functions, unit
// tested — the staleness rule ("a stale figure must never read as a quiet
// day", the botsExcluded pattern) lives here rather than inline in JSX.

/** A drain older than this is stale enough to warn about.
 *
 *  The poller polls each site once a day (`analytics_drain::POLL_INTERVAL` —
 *  anything faster keeps the site's Postgres from ever scaling to zero). So a
 *  drain that is several hours old is the NORMAL state, not a fault: this has
 *  to sit past a full day or the panel would fly the warning tint for ~23
 *  hours out of every 24 and mean nothing. A day plus two hours leaves room
 *  for the wake schedule to drift without crying wolf, while still catching a
 *  poller that has genuinely stopped. */
export const DRAIN_STALE_MS = 26 * 60 * 60 * 1000;

export interface DrainFreshness {
  /** e.g. "drained 2m ago" */
  label: string;
  /** True when the last successful drain is over an hour old — the figures on
   *  screen may be arbitrarily behind, and must not read as a quiet day. */
  stale: boolean;
}

/** Freshness of the drain loop, from the stats payload's `lastDrainAt`.
 *  Returns null when the timestamp is absent or unparsable — an honest gap,
 *  rendered as nothing rather than as "fresh". */
export function drainFreshness(
  lastDrainAt: string | null | undefined,
  now: Date = new Date(),
): DrainFreshness | null {
  if (!lastDrainAt) return null;
  const at = new Date(lastDrainAt).getTime();
  if (Number.isNaN(at)) return null;
  const ageMs = Math.max(0, now.getTime() - at);
  const mins = Math.floor(ageMs / 60_000);
  const label = mins < 1
    ? 'drained just now'
    : mins < 60
      ? `drained ${mins}m ago`
      : mins < 48 * 60
        ? `drained ${Math.floor(mins / 60)}h ago`
        : `drained ${Math.floor(mins / (24 * 60))}d ago`;
  return { label, stale: ageMs > DRAIN_STALE_MS };
}
