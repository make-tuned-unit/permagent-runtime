/** Last-contact decay for the People directory and graph. */

export const QUIET_AFTER_DAYS = 30;

export function daysSince(iso: string | null | undefined, now = Date.now()): number | null {
  if (!iso) return null;
  const t = new Date(iso).getTime();
  if (Number.isNaN(t)) return null;
  return Math.floor((now - t) / 86_400_000);
}

export function isQuiet(lastContact: string | null | undefined, now = Date.now()): boolean {
  const d = daysSince(lastContact, now);
  return d === null || d >= QUIET_AFTER_DAYS;
}

export function contactLabel(lastContact: string | null | undefined, now = Date.now()): string {
  const d = daysSince(lastContact, now);
  if (d === null) return 'never';
  if (d <= 0) return 'today';
  if (d === 1) return 'yesterday';
  if (d < 14) return `${d}d ago`;
  if (d < 60) return `${Math.floor(d / 7)}w ago`;
  return `${Math.floor(d / 30)}mo ago`;
}

export function isFollowUpDue(at: string | null | undefined, now = Date.now()): boolean {
  if (!at) return false;
  const t = new Date(at).getTime();
  return !Number.isNaN(t) && t <= now;
}
