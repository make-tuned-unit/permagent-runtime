/** Pure helpers for Grow content-calendar grouping. Kept out of GrowView so
 *  the lens can stay honest about schedule/status without growing the view. */

export type PostStatus = 'draft' | 'scheduled' | 'posted';
export type MediaStatus = 'queued' | 'generating' | 'ready' | 'failed';

export interface SocialCard {
  id: string;
  title: string;
  description: string;
  /** CamelCase matches CardResponse (`rename_all = "camelCase"`). */
  metadataJson?: Record<string, unknown> | null;
}

const STATUSES: PostStatus[] = ['draft', 'scheduled', 'posted'];
const MEDIA_STATUSES: MediaStatus[] = ['queued', 'generating', 'ready', 'failed'];

export interface PostMedia {
  kind: string;
  file: string;
  source?: string;
}

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

export function readMediaMeta(card: SocialCard): {
  mediaStatus: MediaStatus;
  mediaError: string | null;
  stillFile: string | null;
  /**
   * The animated Reel, when the generator produced one.
   *
   * `format: "reel"` sends the still through Higgsfield and pushes a second
   * media item — `{"kind": "video", "file": "…mp4"}` — onto the card
   * (crates/goose/src/grow_media/mod.rs). This read used to match `still` and
   * nothing else, so every Reel the daemon generated was invisible in the app:
   * the calendar rendered the poster frame and the video was never asked for.
   * Null on a text or carousel post, and null on a reel whose animation failed
   * (`mediaError` carries the reason in that case).
   */
  videoFile: string | null;
  format: string | null;
  channel: string | null;
  mediaFeedback: string;
  /** When the still was generated (ISO), or null for a still made before the
   *  generator recorded it. */
  stillGeneratedAt: string | null;
  /** `mflux` (the image model) or `compose` (the branded fallback), when known. */
  stillSource: string | null;
} {
  const meta = card.metadataJson;
  const raw = meta && typeof meta.mediaStatus === 'string' ? meta.mediaStatus : null;
  const mediaStatus: MediaStatus =
    raw && (MEDIA_STATUSES as string[]).includes(raw) ? (raw as MediaStatus) : 'queued';
  const mediaError =
    meta && typeof meta.mediaError === 'string' && meta.mediaError.trim()
      ? meta.mediaError
      : null;
  const items = Array.isArray(meta?.media) ? (meta!.media as unknown[]) : [];
  const ofKind = (kind: string) => items.find((item) => {
    if (!item || typeof item !== 'object') return false;
    const rec = item as Record<string, unknown>;
    return rec.kind === kind && typeof rec.file === 'string';
  }) as Record<string, unknown> | undefined;
  const still = ofKind('still');
  const video = ofKind('video');
  const mediaFeedback =
    meta && typeof meta.mediaFeedback === 'string' ? meta.mediaFeedback : '';
  return {
    mediaStatus,
    mediaError,
    stillFile: still && typeof still.file === 'string' ? still.file : null,
    videoFile: video && typeof video.file === 'string' ? video.file : null,
    format: meta && typeof meta.format === 'string' ? meta.format : null,
    channel: meta && typeof meta.channel === 'string' ? meta.channel : null,
    mediaFeedback,
    stillGeneratedAt:
      still && typeof still.generatedAt === 'string' && !Number.isNaN(Date.parse(still.generatedAt))
        ? still.generatedAt
        : null,
    stillSource: still && typeof still.source === 'string' ? still.source : null,
  };
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

/**
 * The calendar's status filter (UX_BENCHMARK_RESEARCH §9, dagK 2026-09-25:
 * Buffer's queue — one list, filtered by status, rather than scattered
 * states). `attention` is not a `PostStatus` — a post's schedule state
 * (draft/scheduled/posted) and its still's generation state are two different
 * facts on the same card, and a still stuck on `failed` or `generating` needs
 * the user's eyes regardless of which schedule state the post is in. There is
 * no publish-failure status in the data model yet (`PostStatus` has no
 * `failed`; a scheduled post either becomes `posted` or stays `scheduled`),
 * so "failed" here means the nearest real thing: media generation failed.
 */
export type QueueFilter = 'all' | 'draft' | 'scheduled' | 'posted' | 'attention';

export const QUEUE_FILTERS: { id: QueueFilter; label: string }[] = [
  { id: 'all', label: 'All' },
  { id: 'draft', label: 'Draft' },
  { id: 'scheduled', label: 'Scheduled' },
  { id: 'posted', label: 'Posted' },
  { id: 'attention', label: 'Needs attention' },
];

/** Whether a card's still generation needs the user's eyes: failed outright,
 *  or generating past the point a card would normally have finished (the
 *  generator writes `stillGeneratedAt` on success; a still with none and no
 *  error is either brand new or stuck, and `mediaStatus` disambiguates). */
export function needsAttention(card: SocialCard): boolean {
  return readMediaMeta(card).mediaStatus === 'failed';
}

/** One bucket per card — `attention` takes priority over its schedule status,
 *  since a card whose still failed needs a look whatever its schedule state. */
export function queueBucket(card: SocialCard): Exclude<QueueFilter, 'all'> {
  if (needsAttention(card)) return 'attention';
  return readPostMeta(card).status;
}

export function queueCounts(posts: SocialCard[]): Record<Exclude<QueueFilter, 'all'>, number> {
  const counts: Record<Exclude<QueueFilter, 'all'>, number> = {
    draft: 0, scheduled: 0, posted: 0, attention: 0,
  };
  for (const post of posts) counts[queueBucket(post)] += 1;
  return counts;
}

export function filterByQueue(posts: SocialCard[], filter: QueueFilter): SocialCard[] {
  if (filter === 'all') return posts;
  return posts.filter((p) => queueBucket(p) === filter);
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

/**
 * The one line under a post's still: what made it and when.
 *
 * "compose" is the branded fallback the generator draws when the image model
 * is unavailable — worth saying, because it is the reason a still can look
 * plain. A still from before the generator recorded a time says so rather
 * than inventing one.
 */
export function stillProvenance(
  media: { stillGeneratedAt: string | null; stillSource: string | null },
  now: number = Date.now(),
): string {
  const what = media.stillSource === 'compose'
    ? 'Fallback still'
    : media.stillSource === 'mflux' ? 'AI still' : 'Still';
  if (!media.stillGeneratedAt) return `${what} · date not recorded`;
  const then = Date.parse(media.stillGeneratedAt);
  const days = Math.floor((now - then) / 86_400_000);
  const when = days <= 0
    ? 'today'
    : days === 1 ? 'yesterday' : `${days} days ago`;
  return `${what} · made ${when}`;
}
