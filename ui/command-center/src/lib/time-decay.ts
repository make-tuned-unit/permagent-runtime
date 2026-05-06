/** Returns a human-readable relative time string.
 *  "" if within last 5 minutes, "N min ago" or "Nh Mm ago" otherwise. */
export function relativeTimeAgo(timestamp: string | Date): string {
  const then = typeof timestamp === 'string' ? new Date(timestamp) : timestamp;
  const diffMs = Date.now() - then.getTime();
  const diffMin = Math.floor(diffMs / 60_000);

  if (diffMin < 5) return '';
  if (diffMin < 60) return `${diffMin} min ago`;

  const hours = Math.floor(diffMin / 60);
  const mins = diffMin % 60;
  return mins > 0 ? `${hours}h ${mins}m ago` : `${hours}h ago`;
}
