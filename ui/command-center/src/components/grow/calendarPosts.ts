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
