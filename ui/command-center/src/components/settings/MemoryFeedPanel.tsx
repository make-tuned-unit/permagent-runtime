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
 * States (R2): loading (skeleton rows in the final shape), error (StateBlock
 * with retry), empty (the endpoint always returns every source, so "empty" in
 * practice is zero rows returned — treated as an error-shaped surprise, not a
 * silent blank), stale (a source with a `last_error` reads as failing, not as
 * merely quiet), populated.
 */

import { useCallback, useEffect, useState } from 'react';
import { FiAlertTriangle } from 'react-icons/fi';
import { api } from '../../lib/api';
import { navigateToTool, type ToolType } from '../../lib/store';
import { space, radius, font, textSize, tabularNums } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { AsOf } from '../common/AsOf';
import { Chip } from '../common/Chip';
import { StateBlock } from '../common/StateBlock';
import { Tooltip } from '../common/Tooltip';

type FeedSourceStatus = Awaited<ReturnType<typeof api.getBrainFeedStatus>>['sources'][number];
type FeedStatusResponse = Awaited<ReturnType<typeof api.getBrainFeedStatus>>;

/**
 * "Where this lives" — the surface each source's facts are actually about,
 * for the reader who wants the source record rather than the memory of it.
 * `null` sources (council, briefing, skill, lesson, incident, model_review)
 * have no dedicated tab today; the row still shows counts/status honestly,
 * it just has one link instead of two (R3: never a dead link rendered live).
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

function rowTone(row: FeedSourceStatus): 'default' | 'warning' | 'danger' {
  if (!row.enabled) return 'warning';
  if (row.last_error) return 'danger';
  return 'default';
}

function SkeletonRow({ colors }: { colors: ReturnType<typeof useTheme>['colors'] }) {
  return (
    <div style={{
      display: 'flex', alignItems: 'center', gap: space.md, padding: `${space.sm}px 0`,
      borderBottom: `1px solid ${colors.border}`,
    }}>
      <div style={{ width: 160, height: 12, borderRadius: radius.sm, background: colors.surfaceHi }} />
      <div style={{ width: 40, height: 12, borderRadius: radius.sm, background: colors.surfaceHi }} />
      <div style={{ width: 90, height: 12, borderRadius: radius.sm, background: colors.surfaceHi, marginLeft: 'auto' }} />
    </div>
  );
}

export function MemoryFeedPanel() {
  const { colors } = useTheme();
  const [data, setData] = useState<FeedStatusResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

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

  if (loading && !data) {
    return (
      <div data-testid="memory-feed-panel">
        {[0, 1, 2, 3].map(i => <SkeletonRow key={i} colors={colors} />)}
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

  const sources = data?.sources ?? [];
  if (sources.length === 0) {
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

  return (
    <div data-testid="memory-feed-panel">
      <div style={{ display: 'flex', flexDirection: 'column' }}>
        {sources.map(row => {
          const tone = rowTone(row);
          const location = SOURCE_LOCATION[row.source];
          const isChat = row.source === 'chat';
          const pending = isChat ? (data?.chat_outbox.pending ?? 0) : row.pending;
          const dead = isChat ? (data?.chat_outbox.dead ?? 0) : row.dead;
          return (
            <div
              key={row.source}
              data-testid={`memory-feed-row-${row.source}`}
              style={{
                display: 'flex', alignItems: 'center', gap: space.md,
                padding: `${space.sm}px 0`, borderBottom: `1px solid ${colors.border}`,
                minHeight: 40,
              }}
            >
              <div style={{ minWidth: 0, flex: '1 1 200px' }}>
                <div style={{
                  fontFamily: font.body, fontSize: textSize.body, color: colors.text,
                  fontWeight: 600, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
                }}>
                  {row.label}
                </div>
                <div style={{ fontFamily: font.body, fontSize: textSize.micro, color: colors.textMuted }}>
                  {row.count > 0 ? (
                    <span style={tabularNums}>{row.count} fact{row.count === 1 ? '' : 's'} remembered</span>
                  ) : (
                    'Not yet — nothing fed'
                  )}
                  {dead > 0 && (
                    <span style={{ color: colors.danger }}> · {dead} failed permanently</span>
                  )}
                  {pending > 0 && (
                    <span> · {pending} queued</span>
                  )}
                </div>
              </div>

              <div style={{ minWidth: 110, textAlign: 'right' }}>
                <AsOf asOf={row.last_fed_at} prefix="Fed" />
              </div>

              <div style={{ display: 'flex', alignItems: 'center', gap: space.sm, minWidth: 140, justifyContent: 'flex-end' }}>
                {!row.enabled && (
                  <Chip kind="static" tone="warning" quiet data-testid={`memory-feed-disabled-${row.source}`}>
                    Off
                  </Chip>
                )}
                {row.last_error && (
                  <Tooltip content={row.last_error}>
                    <span style={{ display: 'inline-flex', color: colors.danger }} data-testid={`memory-feed-error-${row.source}`}>
                      <FiAlertTriangle size={14} />
                    </span>
                  </Tooltip>
                )}
                {tone === 'default' && row.count === 0 && !row.last_error && (
                  <span style={{ fontSize: textSize.micro, color: colors.textMuted }}>—</span>
                )}
              </div>

              <div style={{ display: 'flex', gap: space.sm, minWidth: 90, justifyContent: 'flex-end' }}>
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
