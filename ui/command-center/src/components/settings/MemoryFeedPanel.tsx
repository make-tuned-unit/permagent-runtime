/**
 * "What Henry learns from" — Settings → Memory's transparency panel
 * (memory-feed lane, 2026-09-25; UX_STANDARD.md R5 agent transparency).
 *
 * Every structured data source that can reach the Brain
 * (`crates/goose/src/brain_feed.rs::FeedSource`) gets one row: how many facts
 * it has fed, when it last fed one, whether it is failing, and — R5 — where
 * that data lives and (when a filtered view exists) a way into the Brain.
 * Chat turns are fed through their own, older outbox
 * (`chat_memory_outbox`), so the "Conversations" row folds that queue's depth
 * in too rather than silently reading zero pending/dead forever.
 *
 * Layout is one CSS grid shared by every row (a coordinator review, 2026-09-25,
 * caught per-row flex drifting column widths row to row — "Fed 2 months ago"
 * sitting further right than "Fed 5m ago"). Each row is a `display: contents`
 * wrapper so its six cells land in the SAME grid columns as every other row,
 * which is what makes the columns actually line up rather than each row
 * negotiating its own widths from its own content.
 *
 * Global control, in the panel itself (R5 "one click away", not a link to a
 * page that doesn't have the control either — a second coordinator finding,
 * 2026-09-25): each row's `On` column is a real [`Toggle`], writing
 * `BRAIN_FEED_DISABLED_SOURCES` through the SAME generic config-upsert path
 * every other Settings control already uses (`api.upsertConfig` ->
 * `POST /config/upsert` -> `Config::set` -> `crates/goose/src/config/base.rs`).
 * No new backend route: that one already accepts an arbitrary key/value pair.
 * `disabled_sources()` (`crates/goose/src/brain_feed.rs`) reads this key fresh
 * via `Config::global().get_param` on every drain pass — `get_param` re-reads
 * the config file (or an env override) from scratch on every call, nothing is
 * cached at boot — so the toggle takes effect on the daemon's next drain pass
 * with no restart (see `brain_feed.rs`'s
 * `disabled_sources_reads_live_not_cached_at_boot` test).
 *
 * States (R2): loading (skeleton rows in the final grid shape), error
 * (StateBlock with retry), empty (the endpoint always returns every source,
 * so "empty" in practice is zero rows returned — treated as an error-shaped
 * surprise, not a silent blank), stale (a source with a `last_error` reads as
 * failing via the status chip, never as merely quiet), populated.
 */

import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from 'react';
import { api } from '../../lib/api';
import { toast } from '../../lib/notifications';
import { navigateToTool, useCommandCenter, type ToolType } from '../../lib/store';
import { space, radius, font, textSize, tabularNums } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { AsOf } from '../common/AsOf';
import { Chip } from '../common/Chip';
import { StateBlock } from '../common/StateBlock';
import { Toggle } from '../common/Toggle';

/**
 * Sources whose toggle is locked on. No backend enforcement refuses to
 * disable ANY source today (`parse_disabled` treats every short name
 * uniformly) — this is a product-level guardrail, not a technical one:
 * `chat` is the one source that predates `brain_feed` entirely (chat
 * persistence writes memories directly, tagged plainly `"chat"`, not under
 * this outbox), and it is the Brain's primary intake. Turning it off would
 * silently stop the agent learning from every future conversation. The daemon
 * enforces the same rule (brain_feed::disabled_sources strips `chat`, ruling
 * 2026-09-25), so this lock only mirrors it; to make chat toggleable, both
 * sides have to change.
 */
const NEVER_DISABLE = new Set(['chat']);

type FeedSourceStatus = Awaited<ReturnType<typeof api.getBrainFeedStatus>>['sources'][number];
type FeedStatusResponse = Awaited<ReturnType<typeof api.getBrainFeedStatus>>;

/**
 * "Where this lives" — the surface each source's facts are actually about,
 * for the reader who wants the source record rather than the memory of it.
 * `null` sources (council, briefing, skill, lesson, incident, model_review)
 * have no dedicated tab today; the row still shows counts/status honestly,
 * with an empty (same-width) Source cell rather than a dead link (R3).
 */
const SOURCE_LOCATION: Partial<Record<string, ToolType>> = {
  chat: 'chat',
  goal: 'dashboard',
  decision_asked: 'dashboard',
  decision_auto: 'dashboard',
  card: 'dashboard',
  project: 'projects',
  meeting: 'people',
  growth: 'grow',
  analytics: 'grow',
  finance: 'finance',
  watcher_insight: 'projects',
};

const LOCATION_LABEL: Partial<Record<ToolType, string>> = {
  chat: 'Open Chat',
  dashboard: 'Open Home',
  projects: 'Open Projects',
  people: 'Open People',
  grow: 'Open Grow',
  finance: 'Open Finance',
};

/** The grid every row (and the skeleton) shares: label / fed / on / source / brain.
 *  A failure chip lives in the label cell: its own column sat empty on every
 *  healthy row, a bordered blank between the switch and the Source button. */
const GRID_COLUMNS = 'minmax(0,1fr) 92px 44px 84px 60px';

function isFailing(row: FeedSourceStatus, chatDead: number): boolean {
  if (row.source === 'chat') return chatDead > 0;
  return row.dead > 0 || !!row.last_error;
}

function SkeletonRow({ colors }: { colors: ReturnType<typeof useTheme>['colors'] }) {
  const bar = (w: number) => (
    <div style={{ height: 12, width: w, borderRadius: radius.sm, background: colors.surfaceHi }} />
  );
  return (
    <div style={{ display: 'contents' }}>
      <div style={{ padding: `${space.sm}px 0`, borderBottom: `1px solid ${colors.border}` }}>{bar(160)}</div>
      <div style={{ padding: `${space.sm}px 0`, borderBottom: `1px solid ${colors.border}` }}>{bar(60)}</div>
      <div style={{ padding: `${space.sm}px 0`, borderBottom: `1px solid ${colors.border}` }} />
      <div style={{ padding: `${space.sm}px 0`, borderBottom: `1px solid ${colors.border}` }} />
      <div style={{ padding: `${space.sm}px 0`, borderBottom: `1px solid ${colors.border}` }} />
    </div>
  );
}

export function MemoryFeedPanel() {
  const { colors } = useTheme();
  const [data, setData] = useState<FeedStatusResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [failingOnly, setFailingOnly] = useState(false);
  const agentName = useCommandCenter(s => s.agentName);
  /** Writes enqueued but not yet settled. A reload must not re-base the queue
   *  while one is in flight: the server snapshot it fetched can predate that
   *  write, and a write queued after the reload would then compute its payload
   *  without it — the same lost update the queue exists to prevent. */
  const inFlightWritesRef = useRef(0);

  /** The disabled-sources baton, threaded THROUGH the write queue itself
   *  rather than read from a ref every writes' callbacks share. Each queued
   *  write receives the previous write's *committed* disabled-set (its
   *  server-confirmed result on success, or the set unchanged on failure —
   *  a rollback), computes its own delta on top of that, fires the request,
   *  and hands the result to whichever write is queued behind it.
   *
   *  This is deliberately NOT a plain mutable ref that every in-flight write
   *  reads at request time: two toggles clicked back to back (no await in
   *  between) both run their click handlers before either write's callback
   *  gets a turn, so a ref updated synchronously on click would already
   *  show BOTH toggles flipped by the time the FIRST write goes to compute
   *  its payload — sending 'finance,meeting' for a write that should only
   *  ever have said 'finance', with the second write then redundantly
   *  resending the same full set once it got the chance. Threading the set
   *  through the promise chain instead means write N only ever sees writes
   *  1..N-1's outcomes, never a later write's not-yet-applicable toggle —
   *  and still sees an earlier write's rollback, so a write queued behind a
   *  failed one recomputes from the state that failure actually left behind
   *  rather than the stale pre-rollback snapshot that existed when it was
   *  queued. Reseeded from the server's own view on every load. */
  const writeQueueRef = useRef<Promise<Set<string>>>(Promise.resolve(new Set<string>()));

  const load = useCallback(() => {
    setLoading(true);
    setError(null);
    api.getBrainFeedStatus()
      .then(res => {
        if (inFlightWritesRef.current === 0) {
          writeQueueRef.current = Promise.resolve(
            new Set(res.sources.filter(s => !s.enabled).map(s => s.source)),
          );
        }
        setData(res);
        setLoading(false);
      })
      .catch((e: unknown) => {
        setError(e instanceof Error ? e.message : 'Could not reach the Brain feed status.');
        setLoading(false);
      });
  }, []);

  useEffect(() => { load(); }, [load]);

  /** Flip one source's enabled state: enqueue the write behind any
   *  still-in-flight one, and only commit the new `enabled` locally once
   *  THIS write's own outcome is known — Toggle's own optimistic display
   *  covers the in-flight gap, this is what makes the committed value the
   *  daemon's, not the click's. Returns a boolean outcome (Toggle's own
   *  "did it fail" signal) rather than rejecting, so a failure reverts and
   *  shows a message without the queue itself ever entering a rejected
   *  state (which would need its own recovery before the next write could
   *  chain onto it). */
  const setSourceEnabled = useCallback((source: string, nextEnabled: boolean) => {
    let succeeded = true;
    inFlightWritesRef.current += 1;
    const nextQueue = writeQueueRef.current.then(async (baseDisabled) => {
      const disabledAfter = new Set(baseDisabled);
      if (nextEnabled) disabledAfter.delete(source); else disabledAfter.add(source);
      try {
        // Literal, not `DISABLED_SOURCES_KEY`: `settingsWrites.test.ts` scans
        // Settings for the config keys it writes and can only read a literal
        // string here, not an identifier — see that file's own doc comment.
        await api.upsertConfig('BRAIN_FEED_DISABLED_SOURCES', Array.from(disabledAfter).join(','), false);
        setData(prev => prev && {
          ...prev,
          sources: prev.sources.map(s => s.source === source ? { ...s, enabled: nextEnabled } : s),
        });
        return disabledAfter;
      } catch (e) {
        succeeded = false;
        toast(
          `Couldn't ${nextEnabled ? 'enable' : 'disable'} that source`,
          e instanceof Error ? e.message : 'The setting was not saved.',
        );
        return baseDisabled;
      } finally {
        inFlightWritesRef.current -= 1;
      }
    });

    writeQueueRef.current = nextQueue;

    return nextQueue.then(() => succeeded);
  }, []);

  const chatDead = data?.chat_outbox.dead ?? 0;
  const chatPending = data?.chat_outbox.pending ?? 0;

  const summary = useMemo(() => {
    const sources = data?.sources ?? [];
    const totalFacts = sources.reduce((sum, s) => sum + s.count, 0);
    const failingCount = sources.filter(s => isFailing(s, chatDead)).length;
    const offCount = sources.filter(s => !s.enabled).length;
    return { total: sources.length, totalFacts, failingCount, offCount };
  }, [data, chatDead]);

  if (loading && !data) {
    return (
      <div data-testid="memory-feed-panel">
        <div style={{ display: 'grid', gridTemplateColumns: GRID_COLUMNS }}>
          {[0, 1, 2, 3].map(i => <SkeletonRow key={i} colors={colors} />)}
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div data-testid="memory-feed-panel">
        <StateBlock tone="error" title={`Couldn't load what ${agentName} learns from`} detail={error} onRetry={load} />
      </div>
    );
  }

  const allSources = data?.sources ?? [];
  if (allSources.length === 0) {
    return (
      <div data-testid="memory-feed-panel">
        <StateBlock
          tone="empty"
          title="No feed sources reported"
          detail="The Brain feed status endpoint returned nothing to show."
          onRetry={load}
        />
      </div>
    );
  }

  const sources = failingOnly ? allSources.filter(s => isFailing(s, chatDead)) : allSources;

  return (
    <div data-testid="memory-feed-panel">
      <div style={{
        display: 'flex', alignItems: 'center', gap: space.sm, flexWrap: 'wrap',
        fontFamily: font.body, fontSize: textSize.caption, color: colors.textMuted,
        marginBottom: space.sm,
      }}>
        <span style={tabularNums}>{summary.total} sources</span>
        <span aria-hidden="true">·</span>
        <span style={tabularNums}>{summary.totalFacts.toLocaleString()} facts remembered</span>
        {summary.failingCount > 0 && (
          <>
            <span aria-hidden="true">·</span>
            <Chip
              kind="filter"
              tone="danger"
              quiet
              pressed={failingOnly}
              onClick={() => setFailingOnly(v => !v)}
              data-testid="memory-feed-failing-filter"
              title={failingOnly ? 'Showing only failing sources — click to show all' : 'Show only failing sources'}
            >
              {summary.failingCount} failing
            </Chip>
          </>
        )}
        {summary.offCount > 0 && (
          <>
            <span aria-hidden="true">·</span>
            <span>{summary.offCount} turned off</span>
          </>
        )}
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: GRID_COLUMNS, alignItems: 'stretch' }}>
        {sources.map(row => {
          const location = SOURCE_LOCATION[row.source];
          const isChat = row.source === 'chat';
          const pending = isChat ? chatPending : row.pending;
          const dead = isChat ? chatDead : row.dead;
          const failing = isFailing(row, chatDead);
          const cellStyle: CSSProperties = {
            padding: `${space.sm}px ${space.md}px ${space.sm}px 0`, borderBottom: `1px solid ${colors.border}`, minHeight: 40,
            display: 'flex', alignItems: 'center',
          };

          return (
            <div key={row.source} data-testid={`memory-feed-row-${row.source}`} style={{ display: 'contents' }}>
              {/* Label */}
              <div style={{ ...cellStyle, alignItems: 'flex-start', flexDirection: 'column', gap: space.xxs }}>
                <div style={{
                  fontFamily: font.body, fontSize: textSize.body, color: colors.text, fontWeight: 600,
                  display: '-webkit-box', WebkitLineClamp: 2, WebkitBoxOrient: 'vertical', overflow: 'hidden',
                }}>
                  {row.label}
                </div>
                <div style={{ fontFamily: font.body, fontSize: textSize.micro, color: colors.textMuted }}>
                  {row.count > 0 ? (
                    <span style={tabularNums}>{row.count.toLocaleString()} fact{row.count === 1 ? '' : 's'} remembered</span>
                  ) : (
                    'Not yet — nothing fed'
                  )}
                  {dead > 0 && <span style={{ color: colors.danger }}> · {dead} failed</span>}
                  {pending > 0 && <span> · {pending} queued</span>}
                </div>
                {row.enabled && failing && (
                  <div style={{ marginTop: space.xxs }}>
                    <Chip
                      kind="static"
                      tone="danger"
                      quiet
                      data-testid={`memory-feed-error-${row.source}`}
                      title={row.last_error ?? 'Delivery to the Brain has been failing.'}
                    >
                      {dead > 0 ? `${dead} failed` : 'Failing'}
                    </Chip>
                  </div>
                )}
              </div>

              {/* Fed */}
              <div style={{ ...cellStyle, fontSize: textSize.micro, ...tabularNums, color: colors.textMuted, justifyContent: 'flex-start' }}>
                <AsOf asOf={row.last_fed_at} prefix="Fed" unknownLabel="Never" />
              </div>

              {/* On */}
              <div style={cellStyle}>
                <Toggle
                  on={row.enabled}
                  onChange={next => setSourceEnabled(row.source, next)}
                  disabled={NEVER_DISABLE.has(row.source)}
                  disabledReason={
                    NEVER_DISABLE.has(row.source)
                      ? `Conversations must stay on — turning this off would stop ${agentName} learning from any future chat.`
                      : undefined
                  }
                  label={`Feed ${row.label} to the Brain`}
                />
              </div>

              {/* Source */}
              <div style={cellStyle}>
                {location && (
                  <Chip
                    kind="link"
                    quiet
                    onClick={() => navigateToTool(location)}
                    title={LOCATION_LABEL[location]}
                  >
                    Source
                  </Chip>
                )}
              </div>

              {/* Brain */}
              <div style={cellStyle}>
                <Chip
                  kind="link"
                  quiet
                  onClick={() => navigateToTool('memory')}
                  title="Open the Brain view (whole-brain search; not yet filtered to this source)"
                >
                  Brain
                </Chip>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
