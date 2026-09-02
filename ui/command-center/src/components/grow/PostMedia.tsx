/**
 * The media a social post carries: the still, and — new in R9 — the Reel.
 *
 * WHY THE REEL WAS INVISIBLE. The generator has produced Reels for a while:
 * for `format: "reel"` it animates the still through Higgsfield and pushes
 * `{"kind": "video", "file": "…mp4"}` onto `metadata_json.media`
 * (crates/goose/src/grow_media/mod.rs), and the media route already serves it
 * with `Content-Type: video/mp4`
 * (crates/goose-server/src/routes/cards.rs). The UI simply never looked:
 * `readMediaMeta` matched `kind === 'still'` and nothing else, and the calendar
 * row rendered an `<img>`. There was no `<video>` element anywhere in the app,
 * so a generated Reel could be approved and published without ever being
 * watched by the person approving it. This file is the missing half.
 *
 * Both surfaces fetch through `api.fetchGrowMediaBlob` rather than pointing an
 * `src` at the route: the daemon needs an auth header, which an `<img>`/`<video>`
 * `src` cannot carry. The blob's own `type` is the daemon's declared
 * `Content-Type`, so the object URL plays with the right MIME without us
 * guessing from the filename — `mimeForFile` is only the fallback for a blob
 * that arrived as `application/octet-stream`.
 */

import { useEffect, useState } from 'react';
import { font, radius, space, textSize } from '../../styles/tokens';
import type { ThemeColors } from '../../styles/tokens';
import { api } from '../../lib/api';

/** Poster/still box, and the Reel's own width. One number, so the two line up. */
const MEDIA_W = 72;
const MEDIA_H = 90;
/** The Reel gets more room than the thumbnail: it is meant to be watched. */
const REEL_W = 132;
const REEL_H = 234;

/**
 * A last-resort MIME from the filename, for a daemon that answered
 * `application/octet-stream`. `video/mp4` is what the animator actually writes;
 * the other two are here so a future encoder does not silently render a dead
 * player.
 */
export function mimeForFile(filename: string): string {
  const ext = filename.toLowerCase().split('.').pop() ?? '';
  if (ext === 'webm') return 'video/webm';
  if (ext === 'mov') return 'video/quicktime';
  return 'video/mp4';
}

/**
 * One media file as an object URL, revoked on unmount.
 *
 * `cacheKey` is the card's media status: a regenerated still keeps its filename,
 * so without it the browser would hold the old bytes.
 */
function useGrowMediaUrl(
  projectId: string,
  cardId: string,
  filename: string | null,
  cacheKey?: string,
): { url: string | null; type: string | null } {
  const [state, setState] = useState<{ url: string | null; type: string | null }>({ url: null, type: null });
  useEffect(() => {
    if (!filename) { setState({ url: null, type: null }); return; }
    let live = true;
    let objectUrl: string | null = null;
    api.fetchGrowMediaBlob(projectId, cardId, filename)
      .then((blob) => {
        if (!live) return;
        objectUrl = URL.createObjectURL(blob);
        setState({ url: objectUrl, type: blob.type || null });
      })
      .catch(() => { /* still generating or missing */ });
    return () => {
      live = false;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [projectId, cardId, filename, cacheKey]);
  return state;
}

export function PostStill({
  projectId, cardId, filename, cacheKey, colors,
}: {
  projectId: string;
  cardId: string;
  filename: string;
  cacheKey?: string;
  colors: ThemeColors;
}) {
  const { url } = useGrowMediaUrl(projectId, cardId, filename, cacheKey);
  if (!url) {
    return (
      <div style={{
        width: MEDIA_W, height: MEDIA_H, borderRadius: radius.sm,
        background: colors.bgDeeper, border: `1px solid ${colors.border}`,
      }} />
    );
  }
  return (
    <img
      src={url}
      alt=""
      style={{
        width: MEDIA_W, height: MEDIA_H, objectFit: 'cover', borderRadius: radius.sm,
        border: `1px solid ${colors.border}`,
      }}
    />
  );
}

/**
 * The generated Reel, playable in place.
 *
 * `controls` and nothing else: no autoplay (a calendar of five Reels all
 * playing at once is the reason `preload="metadata"` is here too), no loop, no
 * muted-autoplay trick. The poster is the post's own still when there is one,
 * so the card looks the same before the first frame decodes as after.
 *
 * The chrome is content chrome — an opaque surface, the theme's hairline, a
 * radius off the scale. Never glass: a video IS content, and Apple's rule is
 * that glass belongs to the floating control layer.
 */
export function PostVideo({
  projectId, cardId, filename, posterFilename, cacheKey, colors,
}: {
  projectId: string;
  cardId: string;
  filename: string;
  /** The still for this post, used as the poster frame. */
  posterFilename?: string | null;
  cacheKey?: string;
  colors: ThemeColors;
}) {
  const { url, type } = useGrowMediaUrl(projectId, cardId, filename, cacheKey);
  const poster = useGrowMediaUrl(projectId, cardId, posterFilename ?? null, cacheKey);
  const shell = {
    width: REEL_W, height: REEL_H, borderRadius: radius.sm,
    background: colors.bgDeeper, border: `1px solid ${colors.border}`,
    flexShrink: 0,
  } as const;

  if (!url) {
    return (
      <div
        style={{
          ...shell,
          display: 'flex', alignItems: 'center', justifyContent: 'center',
          textAlign: 'center', padding: space.md, boxSizing: 'border-box',
          fontSize: textSize.micro, fontFamily: font.body, color: colors.textDim,
        }}
      >
        Reel loading…
      </div>
    );
  }
  return (
    <video
      data-testid="post-reel"
      controls
      preload="metadata"
      playsInline
      poster={poster.url ?? undefined}
      style={{ ...shell, objectFit: 'cover', display: 'block' }}
    >
      <source src={url} type={type && type.startsWith('video/') ? type : mimeForFile(filename)} />
    </video>
  );
}
