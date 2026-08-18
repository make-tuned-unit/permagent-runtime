/** Pure helpers for Grow content-calendar grouping. Kept out of GrowView so
 *  the lens can stay honest about schedule/status without growing the view. */

export type PostStatus = 'draft' | 'scheduled' | 'posted';

export interface SocialCard {
  id: string;
  title: string;
  description: string;
  /** CamelCase matches CardResponse (`rename_all = "camelCase"`). */
  metadataJson?: Record<string, unknown> | null;
}

const STATUSES: PostStatus[] = ['draft', 'scheduled', 'posted'];

/** Tolerant read: unknown/absent status → draft; unparseable date → unscheduled. */
export function readPostMeta(card: SocialCard): {
  scheduledFor: string | null;
  status: PostStatus;
} {
  const meta = card.metadataJson;
  const rawStatus = meta && typeof meta.postStatus === 'string' ? meta.postStatus : null;
  const status: PostStatus =
    rawStatus && (STATUSES as string[]).includes(rawStatus) ? (rawStatus as PostStatus) : 'draft';
  const rawWhen = meta && typeof meta.scheduledFor === 'string' ? meta.scheduledFor : null;
  const scheduledFor =
    rawWhen && !Number.isNaN(Date.parse(rawWhen)) ? rawWhen : null;
  return { scheduledFor, status };
}

/** Local calendar day key (`YYYY-MM-DD`), or null when unscheduled. */
export function localDayKey(iso: string | null): string | null {
  if (!iso) return null;
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return null;
  const d = new Date(t);
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${y}-${m}-${day}`;
}

export interface DayGroup {
  /** `YYYY-MM-DD` or the sentinel `"unscheduled"`. */
  day: string;
  label: string;
  posts: SocialCard[];
}

/** Group soonest scheduled day first; Unscheduled last. */
export function groupPostsByDay(posts: SocialCard[]): DayGroup[] {
  const buckets = new Map<string, SocialCard[]>();
  for (const post of posts) {
    const { scheduledFor } = readPostMeta(post);
    const key = localDayKey(scheduledFor) ?? 'unscheduled';
    const list = buckets.get(key) ?? [];
    list.push(post);
    buckets.set(key, list);
  }
  const days = [...buckets.keys()].sort((a, b) => {
    if (a === 'unscheduled') return 1;
    if (b === 'unscheduled') return -1;
    return a.localeCompare(b);
  });
  return days.map((day) => ({
    day,
    label: day === 'unscheduled' ? 'Unscheduled' : formatDayHeading(day),
    posts: (buckets.get(day) ?? []).slice().sort((a, b) => {
      const aa = readPostMeta(a).scheduledFor ?? '';
      const bb = readPostMeta(b).scheduledFor ?? '';
      return aa.localeCompare(bb);
    }),
  }));
}

function formatDayHeading(day: string): string {
  const [y, m, d] = day.split('-').map(Number);
  if (!y || !m || !d) return day;
  return new Date(y, m - 1, d).toLocaleDateString(undefined, {
    weekday: 'short',
    month: 'short',
    day: 'numeric',
    year: 'numeric',
  });
}

/** `datetime-local` value from an RFC-3339 instant (local wall clock). */
export function toDatetimeLocalValue(iso: string | null): string {
  if (!iso) return '';
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return '';
  const d = new Date(t);
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** Local `datetime-local` → RFC-3339 UTC instant. */
export function fromDatetimeLocalValue(local: string): string | null {
  if (!local) return null;
  const t = Date.parse(local);
  if (Number.isNaN(t)) return null;
  return new Date(t).toISOString();
}
