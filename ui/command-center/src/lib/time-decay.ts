/** Returns a human-readable relative time string.
 *  "" if within last 5 minutes, "N min ago", "Nh Mm ago" under a day, and
 *  "Nd ago" from a day on — the same day step History's "2d ago" uses.
 *  (It used to count hours forever: the Automate roster read "174h 3m ago"
 *  for a weekly job, QA 2026-09-24.) */
export function relativeTimeAgo(timestamp: string | Date): string {
  const then = typeof timestamp === 'string' ? new Date(timestamp) : timestamp;
  const diffMs = Date.now() - then.getTime();
  const diffMin = Math.floor(diffMs / 60_000);

  if (diffMin < 5) return '';
  if (diffMin < 60) return `${diffMin} min ago`;

  const hours = Math.floor(diffMin / 60);
  if (hours >= 24) return `${Math.floor(hours / 24)}d ago`;
  const mins = diffMin % 60;
  return mins > 0 ? `${hours}h ${mins}m ago` : `${hours}h ago`;
}
