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
import { Button } from '../common/Button';
import type { Project } from '../projects/types';
import {
  fromDatetimeLocalValue,
  groupPostsByDay,
  readMediaMeta,
  readPostMeta,
  toDatetimeLocalValue,
  type PostStatus,
  type SocialCard,
} from './calendarPosts';
import { draftPostPrompt } from './growStrategy';
import { growChip } from './growStyles';
import { FIELD_CLASS, growCard, growField, growLabel } from './growChrome';
import { ROW_INNER_R, ROW_PAD, ROW_R, CARD_R } from './growGeometry';
import { ErrorState, LoadingState } from './GrowStateBlocks';
import { HiggsfieldConnect, PostizConnect, ProjectChannels } from './PublisherSettings';
import { PostStill, PostVideo } from './PostMedia';
import type { LoadState } from './growTypes';

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
          textAlign: 'center', fontSize: textSize.caption, color: colors.textDim,
        }}>
          No posts yet. Draft one with {agentName} above — it is written in this project's voice, a still is generated, and Approve schedules it on this project's connected accounts when you are ready.
        </div>
      ) : (
        <CalendarLens
          projectId={active.id}
          posts={posts}
          colors={colors}
          onMutate={onMutate}
          onReload={() => onReload({ silent: true })}
        />
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
              />
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}

function CalendarPostRow({
  projectId, post, colors, onMutate, onReload,
}: {
  projectId: string;
  post: SocialCard;
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
  const chip: CSSProperties = {
    ...growLabel(colors),
    background: colors.bgDeeper, padding: `1px ${space.sm}px`, borderRadius: radius.pill,
  };

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
      await apiFetch(`/api/projects/${encodeURIComponent(projectId)}/cards/${encodeURIComponent(post.id)}/approve`, {
        method: 'POST',
      });
      onReload();
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
  const canRetryStill = status === 'draft' && media.mediaStatus !== 'generating';

  return (
    <div style={growCard(colors, { r: ROW_R, pad: ROW_PAD })}>
      <div style={{ display: 'flex', alignItems: 'center', gap: space.md, marginBottom: space.sm }}>
        <span style={chip}>{status}</span>
        <span style={chip}>{media.mediaStatus}</span>
        {media.channel && <span style={chip}>{media.channel}</span>}
        {media.format && <span style={chip}>{media.format}</span>}
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
          <PostStill
            projectId={projectId}
            cardId={post.id}
            filename={media.stillFile}
            cacheKey={media.mediaStatus}
            colors={colors}
          />
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
        {status !== 'draft' && (
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
          >Approve</Button>
        )}
      </div>
      {canRetryStill && (
        <div style={{ display: 'flex', gap: space.md, marginTop: space.lg, alignItems: 'flex-start' }}>
          <textarea
            value={feedback}
            onChange={(e) => setFeedback(e.target.value)}
            disabled={busy}
            aria-label="Still taste notes"
            placeholder="Taste notes for a new still — copy stays"
            rows={2}
            className={FIELD_CLASS}
            style={{
              ...growField(colors), flex: 1, fontSize: textSize.micro,
              borderRadius: ROW_INNER_R, resize: 'vertical',
            }}
          />
          <Button
            colors={colors}
            type="button"
            style={btn}
            disabled={busy}
            onClick={() => retryMedia()}
          >Regenerate still</Button>
        </div>
      )}
    </div>
  );
}
