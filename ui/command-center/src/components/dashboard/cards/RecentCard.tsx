import { useCallback } from 'react';
import { duration, ease, font, radius, space } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { useCommandCenter } from '../../../lib/store';
import { SectionTitle, StatusIcon, EmptyNote, CardScrollBody } from '../atoms';
import type { RecentSession } from '../useDashboard';
import { Tooltip } from '../../common/Tooltip';
import { classifySession, labelSource, sessionDisplayLabel } from '../../../lib/sessionLabel';

/** Home shows conversations — the same default History opens on. Probes,
 *  scheduled jobs and coding workers live under History's other filters. */
const RECENT_LIMIT = 4;

/** The daemon's full record when it sent one, else its cut title. */
function recentSource(item: RecentSession) {
  return labelSource({ ...item, name: item.name ?? item.title });
}

function timeAgo(iso: string): string {
  const ms = Date.now() - new Date(iso).getTime();
  const min = Math.floor(ms / 60000);
  if (min < 1) return 'just now';
  if (min < 60) return `${min}m ago`;
  const hrs = Math.floor(min / 60);
  if (hrs < 24) return `${hrs}h ago`;
  return `${Math.floor(hrs / 24)}d ago`;
}

interface Props {
  items: RecentSession[];
}

export function RecentCard({ items: all }: Props) {
  const { colors } = useTheme();
  const items = all.filter(item => classifySession(recentSource(item)) === 'conversation').slice(0, RECENT_LIMIT);
  const switchToSession = useCommandCenter(s => s.switchToSession);
  const openChatDock = useCommandCenter(s => s.openChatDock);

  // A recent item's id IS a session id (dashboard.rs builds it from sessions).
  // Open it in the chat dock so the past conversation is immediately visible.
  const openSession = useCallback((id: string) => {
    switchToSession(id).catch(err => console.error('[recent] open session failed:', err));
    openChatDock();
  }, [switchToSession, openChatDock]);

  return (
    <div style={{
      height: '100%', boxSizing: 'border-box',
      borderRadius: radius.lg,
      background: colors.surface,
      border: `1px solid ${colors.border}`,
      boxShadow: [colors.elevationRaised, colors.cardHighlight].filter(Boolean).join(', '),
      padding: space.xxl,
      display: 'flex', flexDirection: 'column',
      overflow: 'hidden',
    }}>
      {/* "While you were away" (UX_BENCHMARK_RESEARCH.md §1, Linear Priority's
          rest-split / ChatGPT Tasks framing): dashboard.rs already restricts
          this feed to non-active sessions and documents every one of them as
          settled ("all recent sessions are treated as completed" — no
          explicit pause/stop events exist to say otherwise), so this feed
          already IS the "happened while you were away" half of Home; it only
          needed the honest name. Renamed here rather than building a second,
          parallel card — R7 convergence, one concept in one place. */}
      <SectionTitle title="While you were away" right={items.length > 0 ? 'last 24h' : undefined} />
      {items.length === 0 ? (
        <EmptyNote hint="Sessions you run will appear here">No recent activity</EmptyNote>
      ) : (
        <CardScrollBody testId="recent-body" style={{ marginRight: -space.md, paddingRight: space.md }}>
          {items.map((item, i) => (
            <ActivityItem key={item.id} item={item} isLast={i === items.length - 1} onOpen={openSession} />
          ))}
        </CardScrollBody>
      )}
    </div>
  );
}

function ActivityItem({ item, isLast, onOpen }: { item: RecentSession; isLast: boolean; onOpen: (id: string) => void }) {
  const { colors, reduceMotion } = useTheme();
  const statusColor: Record<string, string> = {
    completed: colors.success,
    paused: colors.danger,
    awaiting_input: colors.cyan,
  };
  // Was `colors.border` as the hover fill — a hairline color reused as a
  // surface tint by coincidence of matching alpha. `fillHover`/`fillActive`
  // name what this row actually wants (D10), and give it real press feedback,
  // which it had none of before.
  // Never the raw title: a session named after tool-call args arrives as a cut
  // object literal, a placeholder or a quoted fragment (see lib/sessionLabel).
  const label = sessionDisplayLabel(recentSource(item));
  const fadeTransition = reduceMotion ? 'none' : `background ${duration.fast}ms ${ease.out}, box-shadow ${duration.fast}ms ${ease.out}`;
  return (
    <div
      data-card-row=""
      onClick={() => onOpen(item.id)}
      role="button"
      tabIndex={0}
      aria-label={`Open conversation: ${label}`}
      onKeyDown={e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); onOpen(item.id); } }}
      onMouseEnter={e => { e.currentTarget.style.background = colors.fillHover; e.currentTarget.style.transition = fadeTransition; }}
      onMouseLeave={e => { e.currentTarget.style.background = 'transparent'; }}
      onPointerDown={e => { e.currentTarget.style.background = colors.fillActive; }}
      onPointerUp={e => { e.currentTarget.style.background = colors.fillHover; }}
      onFocus={e => { e.currentTarget.style.background = colors.fillHover; e.currentTarget.style.boxShadow = `0 0 0 2px ${colors.cyanGlow}`; e.currentTarget.style.transition = fadeTransition; }}
      onBlur={e => { e.currentTarget.style.background = 'transparent'; e.currentTarget.style.boxShadow = 'none'; }}
      style={{
      display: 'flex', alignItems: 'center', padding: `${space.md}px`,
      borderBottom: isLast ? 'none' : `1px solid ${colors.border}`,
      cursor: 'pointer', borderRadius: radius.sm, outline: 'none',
      background: 'transparent', transition: fadeTransition,
    }}>
      {/* dagK coordinator, 2560px pass: title `flex:1` used to grow across
          the WHOLE row, and on a card stretched to 12 columns (fillHoles)
          that put the status word ~1,800px to the right of its label — two
          things that read as unrelated instead of one row. Capped at a
          readable width (R3; max-width 720 per the coordinator's finding),
          so title and status stay next to each other at any card width; the
          icon/hover/border still span the full row. */}
      <span style={{ display: 'flex', alignItems: 'center', gap: space.lg, width: '100%', maxWidth: 720, minWidth: 0 }}>
        <StatusIcon state={item.state} />
        {/* The title owns the row: it grows into every spare pixel and the
            status word is what gives way (below). The full title is one hover
            away — the server no longer cuts it at 40 characters either. */}
        <div style={{ flex: '1 1 auto', minWidth: 0 }}>
          <Tooltip content={label}>
            <div data-testid="recent-title" style={{
              fontFamily: font.body, fontSize: 12.5, fontWeight: 500, color: colors.text,
              overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
            }}>{label}</div>
          </Tooltip>
          <div style={{ fontSize: 10.5, color: colors.textDim, marginTop: space.xxs }}>
            {timeAgo(item.ended_at)}
          </div>
        </div>
        <span data-testid="recent-status" style={{
          fontFamily: font.body, fontSize: 9.5, fontWeight: 600,
          letterSpacing: '0.06em', textTransform: 'uppercase',
          flex: '0 1 auto', minWidth: 0, maxWidth: '40%',
          overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
          color: statusColor[item.state] || colors.textMuted,
        }}>{item.state.replace('_', ' ')}</span>
      </span>
    </div>
  );
}
