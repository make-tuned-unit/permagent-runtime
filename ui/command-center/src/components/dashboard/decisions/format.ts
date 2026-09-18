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

/**
 * Derive a translucent tint from a theme token (R7 glass pass, D7). Replaces
 * this directory's old `colors.token` + a two-digit hex suffix, glued on by
 * plain string concatenation — an idiom that only "worked" because the token
 * happened to be a bare `#rrggbb`, and would silently produce garbage the
 * moment it wasn't (an `rgba(...)` token, a gradient). Same shape as
 * automate's local `withAlpha` (tokens.ts is the shell lane's file, not this
 * lane's to add a second derivation helper to).
 */
export function withAlpha(hex: string, alpha: number): string {
  const m = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return hex;
  let h = m[1];
  if (h.length === 3) h = h.split('').map(c => c + c).join('');
  const r = parseInt(h.slice(0, 2), 16);
  const g = parseInt(h.slice(2, 4), 16);
  const b = parseInt(h.slice(4, 6), 16);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}
