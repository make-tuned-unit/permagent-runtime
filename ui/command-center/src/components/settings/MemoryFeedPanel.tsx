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
 * wrapper so its five cells land in the SAME grid columns as every other row,
 * which is what makes the columns actually line up rather than each row
 * negotiating its own widths from its own content.
 *
 * States (R2): loading (skeleton rows in the final grid shape), error
 * (StateBlock with retry), empty (the endpoint always returns every source,
 * so "empty" in practice is zero rows returned — treated as an error-shaped
 * surprise, not a silent blank), stale (a source with a `last_error` reads as
 * failing via the status chip, never as merely quiet), populated.
 */

import { useCallback, useEffect, useMemo, useState, type CSSProperties } from 'react';
import { api } from '../../lib/api';
import { navigateToTool, useCommandCenter, type ToolType } from '../../lib/store';
import { space, radius, font, textSize, tabularNums } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { AsOf } from '../common/AsOf';
import { Chip } from '../common/Chip';
import { StateBlock } from '../common/StateBlock';

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

/** The grid every row (and the skeleton) shares: label / fed / status / source / brain. */
const GRID_COLUMNS = 'minmax(0,1fr) 92px 190px 84px 60px';

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
  const setPendingSettingsSection = useCommandCenter(s => s.setPendingSettingsSection);
  const [data, setData] = useState<FeedStatusResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [failingOnly, setFailingOnly] = useState(false);

  const load = useCallback(() => {
    setLoading(true);
    setError(null);
    api.getBrainFeedStatus()
      .then(res => { setData(res); setLoading(false); })
      .catch((e: unknown) => {
        setError(e instanceof Error ? e.message : 'Could not reach the Brain feed status.');
        setLoading(false);
      });
  }, []);

  useEffect(() => { load(); }, [load]);

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
        <div style={{ display: 'grid', gridTemplateColumns: GRID_COLUMNS, columnGap: space.md }}>
          {[0, 1, 2, 3].map(i => <SkeletonRow key={i} colors={colors} />)}
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div data-testid="memory-feed-panel">
        <StateBlock tone="error" title="Couldn't load what Henry learns from" detail={error} onRetry={load} />
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

      <div style={{ display: 'grid', gridTemplateColumns: GRID_COLUMNS, columnGap: space.md, alignItems: 'center' }}>
        {sources.map(row => {
          const location = SOURCE_LOCATION[row.source];
          const isChat = row.source === 'chat';
          const pending = isChat ? chatPending : row.pending;
          const dead = isChat ? chatDead : row.dead;
          const failing = isFailing(row, chatDead);
          const cellStyle: CSSProperties = {
            padding: `${space.sm}px 0`, borderBottom: `1px solid ${colors.border}`, minHeight: 40,
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
              </div>

              {/* Fed */}
              <div style={{ ...cellStyle, fontSize: textSize.micro, ...tabularNums, color: colors.textMuted, justifyContent: 'flex-start' }}>
                <AsOf asOf={row.last_fed_at} prefix="Fed" unknownLabel="Never" />
              </div>

              {/* Status */}
              <div style={cellStyle}>
                {!row.enabled && (
                  <Chip
                    kind="link"
                    tone="warning"
                    quiet
                    onClick={() => setPendingSettingsSection('privacy')}
                    data-testid={`memory-feed-disabled-${row.source}`}
                    title="Disabled from Settings → Privacy & data"
                  >
                    Off
                  </Chip>
                )}
                {row.enabled && failing && (
                  <Chip
                    kind="static"
                    tone="danger"
                    quiet
                    data-testid={`memory-feed-error-${row.source}`}
                    title={row.last_error ?? 'Delivery to the Brain has been failing.'}
                  >
                    {dead > 0 ? `${dead} failed` : 'Failing'}
                  </Chip>
                )}
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
