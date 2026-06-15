/** Compact age string: "now", "42m", "3h 12m", "2d 4h". */
export function formatAge(iso: string): string {
  const ms = Date.now() - new Date(iso).getTime();
  const min = Math.floor(ms / 60_000);
  if (min < 1) return 'now';
  if (min < 60) return `${min}m`;
  const hrs = Math.floor(min / 60);
  if (hrs < 24) {
    const rem = min % 60;
    return rem > 0 ? `${hrs}h ${rem}m` : `${hrs}h`;
  }
  const days = Math.floor(hrs / 24);
  const remH = hrs % 24;
  return remH > 0 ? `${days}d ${remH}h` : `${days}d`;
}

/** "$4.02" — cost always leads with dollars (token counts live in raw detail). */
export function formatUsd(usd: number): string {
  return `$${usd.toFixed(2)}`;
}
