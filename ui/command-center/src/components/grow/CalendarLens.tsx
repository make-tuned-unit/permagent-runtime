/**
 * The Calendar lens — the content calendar section, its day groups, and the
 * per-post row that edits, schedules, approves and regenerates one post.
 *
 * Split out of GrowView.tsx (R9). The section wrapper moved down with the rows
 * it renders so the whole calendar is one file; GrowView owns the post STATE
 * (it is shared with the analytics funnel) and hands it in.
 */

import { useEffect, useState } from 'react';
import type { CSSProperties } from 'react';
import { font, radius, space, textSize } from '../../styles/tokens';
import type { ThemeColors } from '../../styles/tokens';
import { apiFetch } from '../../lib/api';
import { AsOf } from '../common/AsOf';
import { Button } from '../common/Button';
import type { Project } from '../projects/types';
import {
  fromDatetimeLocalValue,
  groupPostsByDay,
  filterByQueue,
  queueBucket,
  queueCounts,
  readMediaMeta,
  readPostMeta,
  toDatetimeLocalValue,
  QUEUE_FILTERS,
  type PostStatus,
  type QueueFilter,
  type SocialCard,
  stillProvenance,
} from './calendarPosts';
import { draftPostPrompt } from './growStrategy';
import { growChip } from './growStyles';
import { Chip, type ChipTone } from '../common/Chip';
import { FIELD_CLASS, growCard, growField, growLabel } from './growChrome';
import { ROW_INNER_R, ROW_PAD, ROW_R, CARD_R } from './growGeometry';
import { SEGMENT_STRIP_PAD, SEGMENT_STRIP_RADIUS, segmentedTab } from './growStyles';

/** R6: one chip style per meaning. A post's schedule state — Draft / Scheduled
 *  / Posted / Needs attention — is the semantic status and gets the tone; the
 *  same buckets `queueBucket()` uses for the Calendar's filter tabs, so a row
 *  can never disagree with its own filter. Format and channel ("carousel",
 *  "ig") are neutral tags, not statuses, and stay a plain outline whatever
 *  they say. This replaces four look-alike chips (POSTED / READY / IG /
 *  CAROUSEL, QA 2026-09-25) that read as one undifferentiated row of pills. */
const STATUS_CHIP_LABEL: Record<ReturnType<typeof queueBucket>, string> = {
  draft: 'Draft', scheduled: 'Scheduled', posted: 'Posted', attention: 'Needs attention',
};
const STATUS_CHIP_TONE: Record<ReturnType<typeof queueBucket>, ChipTone> = {
  draft: 'neutral', scheduled: 'accent', posted: 'success', attention: 'danger',
};
import { ErrorState, LoadingState } from './GrowStateBlocks';
import { HiggsfieldConnect, NETWORKS, PostizConnect, ProjectChannels, useConnectedChannels } from './PublisherSettings';
import { PostStill, PostVideo } from './PostMedia';
import type { LoadState } from './growTypes';

/** The still's column: wide enough for its provenance line and the button
 *  under the 72px thumbnail without the text column jumping around. */
const STILL_COLUMN_W = 112;

/**
 * The lens section: the header and its hand-off, the three connection rows, and
 * whichever of loading / error / empty / calendar the post state calls for.
 */
export function CalendarSection({
  active, colors, agentName, send, posts, postsState, postsMutationError, onReload, onMutate,
}: {
  active: Project;
  colors: ThemeColors;
  agentName: string;
  send: (prompt: string) => void;
  posts: SocialCard[];
  postsState: LoadState;
  postsMutationError: string | null;
  onReload: (opts?: { silent?: boolean }) => void;
  onMutate: (projectId: string, post: SocialCard, body: Record<string, unknown> | null) => Promise<void>;
}) {
  // UX_BENCHMARK_RESEARCH §9 (dagK 2026-09-25: Buffer's queue): one list,
  // filtered by status, instead of every post's state only ever readable by
  // scanning its own chip. Reset to "All" on a project switch — a filter
  // chosen on one project silently applied to the next would hide posts with
  // no sign anything was filtered.
  const [queueFilter, setQueueFilter] = useState<QueueFilter>('all');
  useEffect(() => { setQueueFilter('all'); }, [active.id]);
  const counts = queueCounts(posts);
  const filtered = filterByQueue(posts, queueFilter);

  return (
    <section>
      <div style={{ display: 'flex', alignItems: 'center', gap: space.lg, margin: `0 0 ${space.xl}px` }}>
        <h3 style={{ ...growLabel(colors), margin: 0 }}>Content calendar</h3>
        <span style={{ fontSize: textSize.micro, color: colors.textDim, background: colors.bgDeeper, padding: `1px ${space.sm}px`, borderRadius: radius.pill, fontVariantNumeric: 'tabular-nums' }}>{posts.length}</span>
        <div style={{ flex: 1 }} />
        <Button
          colors={colors}
          onClick={() => send(draftPostPrompt(active.name))}
          style={{
            '--pa-btn-pad': `${space.sm}px ${space.xl}px`,
            '--pa-btn-radius': `${radius.md}px`,
            fontFamily: font.body,
          } as CSSProperties}
        >+ Draft a post with {agentName}</Button>
      </div>
      <HiggsfieldConnect colors={colors} />
      <PostizConnect colors={colors} />
      <ProjectChannels projectId={active.id} colors={colors} />
      {postsMutationError && (
        <div role="alert" style={{
          fontSize: textSize.caption, color: colors.danger, marginBottom: space.lg,
          background: colors.bgDeeper, border: `1px solid ${colors.border}`,
          borderRadius: ROW_R, padding: `${space.md}px ${space.lg}px`,
        }}>
          Couldn&apos;t save changes: {postsMutationError}
        </div>
      )}
      {postsState === 'error' ? (
        <ErrorState
          colors={colors}
          inline
          message="Couldn't load the content calendar."
          onRetry={() => onReload()}
        />
      ) : postsState === 'loading' ? (
        <LoadingState colors={colors} inline label="Loading posts…" />
      ) : posts.length === 0 ? (
        <div style={{
          border: `1px dashed ${colors.border}`, borderRadius: CARD_R, padding: space.huge,
          fontSize: textSize.caption, color: colors.textDim,
        }}>
          No posts yet. Draft one with {agentName} above — it is written in this project's voice, a still is generated, and Approve schedules it on this project's connected accounts when you are ready.
        </div>
      ) : (
        <>
          <div
            role="tablist"
            aria-label="Filter posts by status"
            data-testid="queue-filter"
            style={{
              display: 'flex', gap: SEGMENT_STRIP_PAD, flexWrap: 'wrap',
              background: colors.bgDeeper, borderRadius: SEGMENT_STRIP_RADIUS, padding: SEGMENT_STRIP_PAD,
              marginBottom: space.lg,
            }}
          >
            {QUEUE_FILTERS.map(({ id, label }) => {
              const selected = queueFilter === id;
              const n = id === 'all' ? posts.length : counts[id];
              // "Needs attention" only earns a tab when something is actually
              // in it — an always-present empty bucket for a state most
              // projects never hit is noise, not a queue.
              if (id === 'attention' && n === 0) return null;
              return (
                <button
                  key={id}
                  className="pa-btn"
                  role="tab"
                  aria-selected={selected}
                  tabIndex={0}
                  onClick={() => setQueueFilter(id)}
                  style={segmentedTab(colors, selected)}
                >
                  {label} ({n})
                </button>
              );
            })}
          </div>
          {filtered.length === 0 ? (
            <div style={{
              border: `1px dashed ${colors.border}`, borderRadius: CARD_R, padding: space.huge,
              fontSize: textSize.caption, color: colors.textDim,
            }}>
              No posts in {QUEUE_FILTERS.find((f) => f.id === queueFilter)?.label.toLowerCase()}.
            </div>
          ) : (
            <CalendarLens
              projectId={active.id}
              posts={filtered}
              colors={colors}
              onMutate={onMutate}
              onReload={() => onReload({ silent: true })}
            />
          )}
        </>
      )}
    </section>
  );
}

function CalendarLens({
  projectId, posts, colors, onMutate, onReload,
}: {
  projectId: string;
  posts: SocialCard[];
  colors: ThemeColors;
  onMutate: (projectId: string, post: SocialCard, body: Record<string, unknown> | null) => Promise<void>;
  onReload: () => void;
}) {
  const groups = groupPostsByDay(posts);
  const connected = useConnectedChannels(projectId, posts);
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: space.xxl }}>
      {groups.map((group) => (
        <div key={group.day}>
          <div style={{ ...growLabel(colors), marginBottom: space.md }}>{group.label}</div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: space.md }}>
            {group.posts.map((post) => (
              <CalendarPostRow
                key={post.id}
                projectId={projectId}
                post={post}
                colors={colors}
                onMutate={onMutate}
                onReload={onReload}
                connected={connected}
              />
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}

function CalendarPostRow({
  projectId, post, colors, onMutate, onReload, connected,
}: {
  projectId: string;
  post: SocialCard;
  /** Channel ids this project can publish to; null while unknown. */
  connected: Set<string> | null;
  colors: ThemeColors;
  onMutate: (projectId: string, post: SocialCard, body: Record<string, unknown> | null) => Promise<void>;
  onReload: () => void;
}) {
  const meta = readPostMeta(post);
  const media = readMediaMeta(post);
  const [editing, setEditing] = useState(false);
  const [title, setTitle] = useState(post.title);
  const [body, setBody] = useState(post.description ?? '');
  const [when, setWhen] = useState(toDatetimeLocalValue(meta.scheduledFor));
  const [status, setStatus] = useState<PostStatus>(meta.status);
  const [busy, setBusy] = useState(false);
  const [feedback, setFeedback] = useState(media.mediaFeedback);

  useEffect(() => {
    if (editing) return;
    setTitle(post.title);
    setBody(post.description ?? '');
    setWhen(toDatetimeLocalValue(meta.scheduledFor));
    setStatus(meta.status);
    setFeedback(media.mediaFeedback);
  }, [post.title, post.description, meta.scheduledFor, meta.status, media.mediaFeedback, editing]);

  // `.pa-btn` owns the cursor now: `wait` while busy was the only feedback this
  // row had, and it said nothing about hover, press or disabled.
  const btn: CSSProperties = growChip();
  const statusBucket = queueBucket(post);
  const channelLabel = media.channel
    ? (NETWORKS.find((n) => n.id === media.channel)?.label ?? media.channel)
    : null;

  const saveEdit = async () => {
    setBusy(true);
    try {
      await onMutate(projectId, post, { title, description: body });
      setEditing(false);
      return true;
    } catch { /* surfaced by parent */ return false; }
    finally { setBusy(false); }
  };

  const saveSchedule = async (nextWhen: string, nextStatus: PostStatus) => {
    setBusy(true);
    setWhen(nextWhen);
    setStatus(nextStatus);
    const scheduledFor = fromDatetimeLocalValue(nextWhen);
    const metadataJson = {
      ...(post.metadataJson ?? {}),
      postStatus: nextStatus,
      ...(scheduledFor ? { scheduledFor } : {}),
    };
    if (!scheduledFor) delete (metadataJson as Record<string, unknown>).scheduledFor;
    try {
      await onMutate(projectId, post, { metadataJson });
    } catch { /* surfaced by parent */ }
    finally { setBusy(false); }
  };

  const approve = async () => {
    setBusy(true);
    try {
      const updated = await apiFetch<{ metadataJson?: Record<string, unknown> }>(`/api/projects/${encodeURIComponent(projectId)}/cards/${encodeURIComponent(post.id)}/approve`, {
        method: 'POST',
      });
      onReload();
      if (updated?.metadataJson?.postStatus === 'publish_failed') {
        throw new Error(String(updated.metadataJson.lastPublishError || 'Publishing failed. Please retry.'));
      }
    } finally { setBusy(false); }
  };

  /** Re-attempt a publish that previously failed (`postStatus === 'publish_failed'`).
   *  Same submission path Approve uses; the outcome — scheduled again, or
   *  failed again with a fresh error — comes back on the reloaded card, so
   *  there is nothing to reconcile locally beyond refetching. */
  const retryPublish = async () => {
    setBusy(true);
    try {
      const updated = await apiFetch<{ metadataJson?: Record<string, unknown> }>(`/api/projects/${encodeURIComponent(projectId)}/cards/${encodeURIComponent(post.id)}/publish/retry`, {
        method: 'POST',
      });
      onReload();
      if (updated?.metadataJson?.postStatus === 'publish_failed') {
        throw new Error(String(updated.metadataJson.lastPublishError || 'Publishing failed. Please retry.'));
      }
      return true;
    } finally { setBusy(false); }
  };

  const retryMedia = async () => {
    setBusy(true);
    try {
      await apiFetch(`/api/projects/${encodeURIComponent(projectId)}/cards/${encodeURIComponent(post.id)}/media/retry`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ feedback: feedback.trim() || undefined }),
      });
      onReload();
      return true;
    } catch { /* parent */ return false; }
    finally { setBusy(false); }
  };

  const remove = async () => {
    setBusy(true);
    try { await onMutate(projectId, post, null); return true; }
    catch { /* surfaced by parent */ return false; }
    finally { setBusy(false); }
  };

  const canApprove = status === 'draft' && media.mediaStatus === 'ready';
  // Approve on an unconnected channel still marks the post scheduled, but
  // nothing publishes (publisher::submit_approved returns None). The card
  // said READY + Approve with no hint of that, so say it where the choice is.
  const unconnectedNetwork = media.channel && connected && !connected.has(media.channel)
    ? (NETWORKS.find((n) => n.id === media.channel)?.label ?? media.channel)
    : null;
  const canRetryStill = status === 'draft' && media.mediaStatus !== 'generating';

  return (
    <div style={growCard(colors, { r: ROW_R, pad: ROW_PAD })}>
      <div style={{ display: 'flex', alignItems: 'center', gap: space.md, marginBottom: space.sm }}>
        <Chip kind="static" tone={STATUS_CHIP_TONE[statusBucket]}>
          {STATUS_CHIP_LABEL[statusBucket]}
        </Chip>
        {/* A still in flight is worth a quiet word; "ready" is the expected
            resting state and says nothing a silent thumbnail doesn't already
            say, so it earns no chip. A failed still is already covered by
            "Needs attention" above — this never doubles that claim. */}
        {media.mediaStatus === 'generating' && (
          <Chip kind="static" tone="warning">Generating still…</Chip>
        )}
        {channelLabel && <Chip kind="static" tone="neutral">{channelLabel}</Chip>}
        {media.format && <Chip kind="static" tone="neutral">{media.format}</Chip>}
        <div style={{ flex: 1 }} />
        {!editing ? (
          <>
            <Button colors={colors} type="button" style={btn} disabled={busy} onClick={() => setEditing(true)}>Edit</Button>
            <Button colors={colors} type="button" style={btn} disabled={busy} onClick={() => remove()}>Delete</Button>
          </>
        ) : (
          <>
            <Button colors={colors} type="button" style={btn} disabled={busy} onClick={() => saveEdit()}>Save</Button>
            <Button colors={colors} type="button" style={btn} disabled={busy} onClick={() => setEditing(false)}>Cancel</Button>
          </>
        )}
      </div>
      <div style={{ display: 'flex', gap: space.xl, alignItems: 'flex-start' }}>
        {/* The Reel wins the slot when there is one: it is the artefact being
            approved, and the still is only its first frame — showing the poster
            beside the video would be the same picture twice. A post with no
            video keeps the thumbnail it always had. */}
        {media.videoFile ? (
          <PostVideo
            projectId={projectId}
            cardId={post.id}
            filename={media.videoFile}
            posterFilename={media.stillFile}
            cacheKey={media.mediaStatus}
            colors={colors}
          />
        ) : media.stillFile ? (
          // The still and the control that replaces it, together.
          // "Regenerate still" used to live only at the far right of the
          // card's last row, a screen-width away from the image it acts on.
          // Provenance ("AI still · made 8 days ago") moved OUT of this
          // column (QA 2026-09-25): at the 112px thumbnail width it wrapped
          // to three lines and pushed the button down unpredictably. It now
          // sits in the meta row under the title, which has the row's full
          // width to say it in one line.
          <div
            data-testid="post-still-column"
            style={{ display: 'flex', flexDirection: 'column', alignItems: 'flex-start', gap: space.xs, flexShrink: 0, width: STILL_COLUMN_W }}
          >
            <PostStill
              projectId={projectId}
              cardId={post.id}
              filename={media.stillFile}
              cacheKey={media.mediaStatus}
              colors={colors}
              alt={post.title}
            />
            {canRetryStill && (
              <Button
                colors={colors}
                type="button"
                style={{ ...btn, '--pa-btn-pad': `2px ${space.md}px` } as CSSProperties}
                disabled={busy}
                pending={busy}
                onClick={() => retryMedia()}
              >Regenerate still</Button>
            )}
          </div>
        ) : null}
        <div style={{ flex: 1, minWidth: 0 }}>
      {editing ? (
        <>
          <input
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            aria-label="Post title"
            className={FIELD_CLASS}
            style={{
              ...growField(colors), width: '100%', fontSize: textSize.small, fontWeight: 600,
              borderRadius: ROW_INNER_R, marginBottom: space.sm,
            }}
          />
          <textarea
            value={body}
            onChange={(e) => setBody(e.target.value)}
            aria-label="Post body"
            rows={3}
            className={FIELD_CLASS}
            style={{
              ...growField(colors), width: '100%', lineHeight: 1.5,
              color: colors.textMuted, borderRadius: ROW_INNER_R, resize: 'vertical',
            }}
          />
        </>
      ) : (
        <>
          <div style={{ fontSize: textSize.small, fontWeight: 600, color: colors.text }}>{post.title}</div>
          {/* Moved from the 112px thumbnail column, where it wrapped to three
              lines (QA 2026-09-25) — the meta row has the row's full width. */}
          {!media.videoFile && media.stillFile && (
            <div data-testid="still-provenance" style={{ fontSize: textSize.micro, color: colors.textDim, marginTop: space.xs / 2 }}>
              {stillProvenance(media)}
            </div>
          )}
          {post.description && (
            <div style={{ fontSize: textSize.caption, color: colors.textMuted, marginTop: space.xs, lineHeight: 1.5 }}>{post.description}</div>
          )}
        </>
      )}
        </div>
      </div>
      {media.mediaError && (
        <div style={{ fontSize: textSize.micro, color: colors.textDim, marginTop: space.md }}>{media.mediaError}</div>
      )}
      {status === 'publish_failed' && (
        <div
          role="alert"
          data-testid="publish-error"
          style={{
            display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: space.md,
            fontSize: textSize.micro, color: colors.danger, marginTop: space.md,
            background: colors.bgDeeper, border: `1px solid ${colors.border}`,
            borderRadius: ROW_INNER_R, padding: `${space.sm}px ${space.md}px`,
          }}
        >
          <span style={{ flex: 1, minWidth: 0 }}>
            {meta.lastPublishError ?? 'Publishing failed.'}
            {meta.lastPublishAttemptAt && (
              <>
                {' · '}
                <AsOf asOf={meta.lastPublishAttemptAt} prefix="Last tried" />
              </>
            )}
          </span>
          <Button
            colors={colors}
            type="button"
            style={btn}
            disabled={busy}
            onClick={() => retryPublish()}
          >Retry publish</Button>
        </div>
      )}
      <div style={{ display: 'flex', flexWrap: 'wrap', gap: space.md, marginTop: space.lg, alignItems: 'center' }}>
        <label style={{ fontSize: textSize.micro, color: colors.textDim, display: 'flex', alignItems: 'center', gap: space.sm }}>
          Schedule
          <input
            type="datetime-local"
            aria-label="Reschedule post"
            value={when}
            disabled={busy}
            onChange={(e) => setWhen(e.target.value)}
            onBlur={() => {
              if (when === toDatetimeLocalValue(meta.scheduledFor)) return;
              void saveSchedule(when, status);
            }}
            className={FIELD_CLASS}
            style={{ ...growField(colors), fontSize: textSize.micro, borderRadius: ROW_INNER_R }}
          />
        </label>
        {status !== 'draft' && status !== 'publish_failed' && (
        <label style={{ fontSize: textSize.micro, color: colors.textDim, display: 'flex', alignItems: 'center', gap: space.sm }}>
          Status
          <select
            aria-label="Post status"
            value={status}
            disabled={busy}
            onChange={(e) => void saveSchedule(when, e.target.value as PostStatus)}
            className={FIELD_CLASS}
            style={{ ...growField(colors), fontSize: textSize.micro, borderRadius: ROW_INNER_R }}
          >
            <option value="scheduled">scheduled</option>
            <option value="posted">posted</option>
          </select>
        </label>
        )}
        {canApprove && (
          <Button
            colors={colors}
            type="button"
            disabled={busy}
            onClick={() => approve()}
            style={{
              ...btn,
              '--pa-btn-fg': colors.cyan,
              '--pa-btn-border': colors.borderHi,
              '--pa-btn-border-hover': colors.cyan,
              '--pa-btn-weight': 600,
            } as CSSProperties}
          >{unconnectedNetwork ? "Approve (won't publish)" : 'Approve'}</Button>
        )}
      </div>
      {canApprove && unconnectedNetwork && (
        <div role="note" style={{ fontSize: textSize.micro, color: colors.warning, marginTop: space.sm }}>
          {unconnectedNetwork} isn&apos;t connected for this project, so approving marks this post
          scheduled here but won&apos;t publish it. Connect {unconnectedNetwork} above first.
        </div>
      )}
      {canRetryStill && (
        <div style={{ display: 'flex', gap: space.md, marginTop: space.lg, alignItems: 'flex-start' }}>
          {/* With a still on screen the button sits beside it and these notes
              feed it; a post with no still yet keeps the button here. */}
          <textarea
            value={feedback}
            onChange={(e) => setFeedback(e.target.value)}
            disabled={busy}
            aria-label="Still taste notes"
            placeholder="Notes for the next still (used by Regenerate still) — copy stays"
            rows={2}
            className={FIELD_CLASS}
            style={{
              ...growField(colors), flex: 1, fontSize: textSize.micro,
              borderRadius: ROW_INNER_R, resize: 'vertical',
            }}
          />
          {!media.stillFile || media.videoFile ? (
            <Button
              colors={colors}
              type="button"
              style={btn}
              disabled={busy}
              onClick={() => retryMedia()}
            >Regenerate still</Button>
          ) : null}
        </div>
      )}
    </div>
  );
}
