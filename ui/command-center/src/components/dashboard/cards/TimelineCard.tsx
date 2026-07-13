/**
 * Activity Timeline — home card ("timeline" registry entry, #619).
 *
 * Reads the durable activity journal (`GET /api/activity`), the append-only
 * record of what the agents did: goal transitions, decisions requested and
 * resolved, librarian describe runs, Watcher nudges, task failures.
 * Day-grouped, newest first, with "Show earlier" keyset pagination and
 * server-side kind/actor filters (chips + actor select) so pagination stays
 * correct under a filter. Rows deep-link to their evidence: goal rows open
 * the goal detail overlay, decision rows open the Decision Inbox, memory
 * rows jump to the Memory tool.
 */

import { useState, useEffect, useRef, useCallback } from 'react';
import {
  FiFlag, FiHelpCircle, FiCheckCircle, FiBookOpen, FiBell, FiAlertTriangle, FiActivity,
} from 'react-icons/fi';
import { font, radius } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { SectionTitle } from '../atoms';
import { apiFetch } from '../../../lib/api';
import { useCommandCenter, navigateToTool } from '../../../lib/store';
import { useDecisions } from '../decisions/useDecisions';
import { DecisionInbox } from '../decisions/DecisionInbox';

interface JournalItem {
  id: string;
  ts: string;
  kind: string;
  actor: string;
  title: string;
  detail: string | null;
  ref_kind: string | null;
  ref_id: string | null;
  goal_project_id: string | null;
}

interface JournalPage {
  items: JournalItem[];
  next_before?: string | null;
}

const PAGE_SIZE = 50;

/** Kind-filter chips. `kinds: null` = no kind filter (All). */
const KIND_CHIPS: { label: string; kinds: string[] | null }[] = [
  { label: 'All', kinds: null },
  { label: 'Goals', kinds: ['goal_state_changed'] },
  { label: 'Decisions', kinds: ['decision_created', 'decision_resolved'] },
  { label: 'Librarian', kinds: ['librarian_describe_completed'] },
  { label: 'Watcher', kinds: ['proactive_nudge'] },
  { label: 'Failures', kinds: ['task_failed'] },
];

function journalUrl(kinds: string[] | null, actor: string | null, before?: string): string {
  const params = new URLSearchParams({ limit: String(PAGE_SIZE) });
  if (kinds && kinds.length > 0) params.set('kind', kinds.join(','));
  if (actor) params.set('actor', actor);
  if (before) params.set('before', before);
  return `/api/activity?${params.toString()}`;
}

function useActivityJournal(kinds: string[] | null, actor: string | null) {
  const [items, setItems] = useState<JournalItem[]>([]);
  const [nextBefore, setNextBefore] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadingMore, setLoadingMore] = useState(false);
  // Guards against a slow response landing after the filter changed.
  const fetchSeq = useRef(0);

  const fetchFirstPage = useCallback(async (initial: boolean) => {
    const seq = fetchSeq.current;
    try {
      const page = await apiFetch<JournalPage>(journalUrl(kinds, actor));
      if (seq !== fetchSeq.current) return; // filter changed mid-flight
      setItems(prev => {
        if (initial || prev.length === 0) return page.items;
        // Refresh: prepend rows newer than what we already show, keeping
        // any older pages the user has loaded.
        const known = new Set(prev.map(i => i.id));
        const fresh = page.items.filter(i => !known.has(i.id));
        return fresh.length > 0 ? [...fresh, ...prev] : prev;
      });
      if (initial) setNextBefore(page.next_before ?? null);
    } catch { /* ignore — stale data stays, matching useDashboard */ }
    if (seq === fetchSeq.current) setLoading(false);
  }, [kinds, actor]);

  // Filter change (or mount): reset and refetch; keep a 30s refresh going.
  useEffect(() => {
    fetchSeq.current += 1;
    setItems([]);
    setNextBefore(null);
    setLoading(true);
    setLoadingMore(false);
    fetchFirstPage(true);
    const interval = setInterval(() => fetchFirstPage(false), 30_000);
    return () => clearInterval(interval);
  }, [fetchFirstPage]);

  const loadMore = useCallback(async () => {
    if (!nextBefore || loadingMore) return;
    setLoadingMore(true);
    const seq = fetchSeq.current;
    try {
      const page = await apiFetch<JournalPage>(journalUrl(kinds, actor, nextBefore));
      if (seq !== fetchSeq.current) return; // filter changed mid-flight
      setItems(prev => {
        const known = new Set(prev.map(i => i.id));
        return [...prev, ...page.items.filter(i => !known.has(i.id))];
      });
      setNextBefore(page.next_before ?? null);
    } catch { /* keep current cursor so the user can retry */ }
    if (seq === fetchSeq.current) setLoadingMore(false);
  }, [nextBefore, loadingMore, kinds, actor]);

  return { items, loading, loadMore, hasMore: nextBefore !== null, loadingMore };
}

function dayLabel(ts: string): string {
  const d = new Date(ts);
  const today = new Date();
  const yesterday = new Date(today);
  yesterday.setDate(today.getDate() - 1);
  if (d.toDateString() === today.toDateString()) return 'Today';
  if (d.toDateString() === yesterday.toDateString()) return 'Yesterday';
  return d.toLocaleDateString(undefined, { weekday: 'short', month: 'short', day: 'numeric' });
}

function timeLabel(ts: string): string {
  return new Date(ts).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });
}

function groupByDay(items: JournalItem[]): { label: string; items: JournalItem[] }[] {
  const groups: { label: string; items: JournalItem[] }[] = [];
  for (const item of items) {
    const label = dayLabel(item.ts);
    const last = groups[groups.length - 1];
    if (last && last.label === label) last.items.push(item);
    else groups.push({ label, items: [item] });
  }
  return groups;
}

export function TimelineCard() {
  const { colors } = useTheme();
  const [chipIdx, setChipIdx] = useState(0);
  const [actor, setActor] = useState<string | null>(null);
  const { items, loading, loadMore, hasMore, loadingMore } =
    useActivityJournal(KIND_CHIPS[chipIdx].kinds, actor);
  const [inboxOpen, setInboxOpen] = useState(false);
  // Actor options accumulate across pages and filters so the select doesn't
  // collapse to just the currently filtered actor.
  const [actorOptions, setActorOptions] = useState<string[]>([]);
  useEffect(() => {
    setActorOptions(prev => {
      const set = new Set(prev);
      let grew = false;
      for (const i of items) if (!set.has(i.actor)) { set.add(i.actor); grew = true; }
      return grew ? [...set].sort() : prev;
    });
  }, [items]);
  const filtered = chipIdx !== 0 || actor !== null;
  const groups = groupByDay(items);

  return (
    <>
      <div style={{
        height: '100%', boxSizing: 'border-box',
        borderRadius: radius.lg,
        background: colors.surface,
        border: `1px solid ${colors.border}`,
        boxShadow: [colors.cardShadow, colors.cardHighlight].filter(Boolean).join(', '),
        padding: '18px 20px',
        display: 'flex', flexDirection: 'column',
        overflow: 'hidden',
      }}>
        <SectionTitle title="Timeline" right={items.length > 0 ? 'last 90 days' : undefined} />
        <div style={{
          display: 'flex', alignItems: 'center', gap: 5, flexWrap: 'wrap',
          margin: '2px 0 8px',
        }}>
          {KIND_CHIPS.map((chip, idx) => {
            const active = idx === chipIdx;
            return (
              <button
                key={chip.label}
                onClick={() => setChipIdx(idx)}
                aria-pressed={active}
                style={{
                  padding: '3px 9px',
                  borderRadius: 999,
                  border: `1px solid ${active ? colors.cyan : colors.border}`,
                  background: active ? colors.cyanSoft : 'transparent',
                  color: active ? colors.cyan : colors.textDim,
                  fontFamily: font.body, fontSize: 11, fontWeight: 500,
                  cursor: 'pointer',
                  transition: 'color 120ms ease, border-color 120ms ease, background 120ms ease',
                }}
              >
                {chip.label}
              </button>
            );
          })}
          <div style={{ flex: 1 }} />
          {actorOptions.length > 1 && (
            <select
              value={actor ?? ''}
              onChange={e => setActor(e.target.value || null)}
              aria-label="Filter by actor"
              style={{
                padding: '3px 6px',
                borderRadius: radius.sm,
                border: `1px solid ${actor ? colors.cyan : colors.border}`,
                background: 'transparent',
                color: actor ? colors.cyan : colors.textDim,
                fontFamily: font.body, fontSize: 11,
                cursor: 'pointer',
              }}
            >
              <option value="">All actors</option>
              {actorOptions.map(a => <option key={a} value={a}>{a}</option>)}
            </select>
          )}
        </div>
        {items.length === 0 ? (
          <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
            <div style={{ textAlign: 'center' }}>
              <div style={{ fontSize: 13, color: colors.textMuted, marginBottom: 4 }}>
                {loading ? 'Loading activity…' : filtered ? 'Nothing matches this filter' : 'No activity yet'}
              </div>
              {!loading && !filtered && (
                <div style={{ fontSize: 11, color: colors.textDim }}>
                  Goal moves, decisions, and librarian runs will appear here
                </div>
              )}
            </div>
          </div>
        ) : (
          <div style={{ flex: 1, overflow: 'auto' }}>
            {groups.map(group => (
              <div key={group.label}>
                <div style={{
                  position: 'sticky', top: 0, zIndex: 1,
                  padding: '6px 0 4px',
                  background: colors.surface,
                  fontFamily: font.body, fontSize: 11, fontWeight: 600,
                  letterSpacing: '0.10em', textTransform: 'uppercase',
                  color: colors.textDim,
                }}>{group.label}</div>
                {group.items.map((item, i) => (
                  <TimelineRow
                    key={item.id}
                    item={item}
                    isLast={i === group.items.length - 1}
                    onOpenDecisions={() => setInboxOpen(true)}
                  />
                ))}
              </div>
            ))}
            {hasMore && (
              <button
                onClick={loadMore}
                disabled={loadingMore}
                style={{
                  width: '100%', margin: '10px 0 4px', padding: '7px 0',
                  borderRadius: radius.md,
                  border: `1px solid ${colors.border}`,
                  background: colors.cyanSoft,
                  color: colors.textMuted,
                  fontFamily: font.body, fontSize: 12, fontWeight: 500,
                  cursor: loadingMore ? 'default' : 'pointer',
                  transition: 'border-color 150ms ease',
                }}
              >
                {loadingMore ? 'Loading…' : 'Show earlier'}
              </button>
            )}
          </div>
        )}
      </div>
      {inboxOpen && <DecisionOverlayHost onClose={() => setInboxOpen(false)} />}
    </>
  );
}

/** Mounted only while open, so the decisions poll doesn't run idle. */
function DecisionOverlayHost({ onClose }: { onClose: () => void }) {
  const inbox = useDecisions();
  return <DecisionInbox inbox={inbox} onClose={onClose} />;
}

const KIND_META: Record<string, { icon: React.ComponentType<{ size?: number }>; colorKey: 'cyan' | 'success' | 'danger' | 'warning' }> = {
  goal_state_changed: { icon: FiFlag, colorKey: 'cyan' },
  decision_created: { icon: FiHelpCircle, colorKey: 'warning' },
  decision_resolved: { icon: FiCheckCircle, colorKey: 'success' },
  librarian_describe_completed: { icon: FiBookOpen, colorKey: 'cyan' },
  proactive_nudge: { icon: FiBell, colorKey: 'cyan' },
  task_failed: { icon: FiAlertTriangle, colorKey: 'danger' },
};

function TimelineRow({ item, isLast, onOpenDecisions }: {
  item: JournalItem;
  isLast: boolean;
  onOpenDecisions: () => void;
}) {
  const { colors } = useTheme();
  const openGoalDetail = useCommandCenter(s => s.openGoalDetail);
  const [hover, setHover] = useState(false);

  const meta = KIND_META[item.kind] ?? { icon: FiActivity, colorKey: 'cyan' as const };
  const Icon = meta.icon;
  const accent = colors[meta.colorKey];

  // Deep link to the evidence, via the existing navigation seams.
  const onClick =
    item.ref_kind === 'goal' && item.goal_project_id && item.ref_id
      ? () => openGoalDetail(item.goal_project_id!, item.ref_id!)
      : item.ref_kind === 'decision'
        ? onOpenDecisions
        : item.ref_kind === 'memory'
          ? () => navigateToTool('memory')
          : undefined;

  return (
    <div
      onClick={onClick}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      title={onClick ? 'View evidence' : undefined}
      style={{
        display: 'flex', alignItems: 'center', gap: 12, padding: '9px 6px',
        borderBottom: isLast ? 'none' : `1px solid ${colors.border}`,
        borderRadius: radius.sm,
        cursor: onClick ? 'pointer' : 'default',
        background: onClick && hover ? colors.cyanSoft : 'transparent',
        transition: 'background 120ms ease',
      }}
    >
      <div style={{
        width: 26, height: 26, borderRadius: '50%', flexShrink: 0,
        background: accent + '26', color: accent,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}>
        <Icon size={13} />
      </div>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{
          fontFamily: font.body, fontSize: 13, fontWeight: 500, color: colors.text,
          overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
        }}>{item.title}</div>
        {item.detail && (
          <div style={{
            fontSize: 11, color: colors.textMuted, marginTop: 1,
            overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
          }}>{item.detail}</div>
        )}
      </div>
      <span style={{
        fontFamily: font.mono, fontSize: 10, color: colors.textDim,
        flexShrink: 0,
      }}>{item.actor}</span>
      <span style={{
        fontFamily: font.body, fontSize: 11, color: colors.textDim,
        flexShrink: 0, minWidth: 44, textAlign: 'right',
      }}>{timeLabel(item.ts)}</span>
    </div>
  );
}
