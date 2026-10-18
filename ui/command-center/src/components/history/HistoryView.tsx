/**
 * History — Sessions, Downloads, Activity and Spend.
 *
 * These four are not settings. A setting is a value you change that changes
 * what the app does next; these are RECORDS of what already happened, and the
 * only thing they had in common with Preferences or API keys was that the
 * 2026-08 ruling had nowhere else to put them when the Console overlay was
 * retired. Living under Settings made the Settings rail twenty rows long and
 * made "where do I see what my agent did" a question about configuration.
 *
 * So they are one destination, with four segments, in one component — and this
 * component deliberately lives OUTSIDE `components/settings/` so that hoisting
 * it to the sidebar rail is an import and a branch, not a move.
 *
 * WHAT IS NOT DONE HERE, and why: the rail row and the destination switch live
 * in `components/sidebar/Sidebar.tsx` and `App.tsx`, which lane A1c owns this
 * wave (#1165). This lane does not touch them. Until A1c lands the two lines,
 * Settings keeps ONE row — "History" — that renders this view, so nothing is
 * unreachable and no surface exists twice. The moment the rail row lands, that
 * Settings row is deleted and the deep-link keys route to the destination.
 *
 * Segment ids are the OLD Settings section keys on purpose: `sessions`,
 * `inbox`, `activity`, `spend` are what `app_navigate` deep links carry
 * (app_conductor.rs), what `resolveSettingsSection` accepts, and what
 * Autonomy's "Set spend caps in Spend →" asks for. Renaming them would break
 * live callers for a tidier-looking enum.
 */

import { useEffect, useState, type CSSProperties } from 'react';
import { FiActivity, FiClock, FiDollarSign, FiInbox } from 'react-icons/fi';
import { api, type IncidentView } from '../../lib/api';
import { radius, space, textSize, type } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { useNotifications } from '../../lib/notifications';
import { Button } from '../common/Button';
import { SEGMENT_STRIP_PAD, SEGMENT_STRIP_RADIUS, segmentDirection, segmentedTab } from '../common/segmented';
import { runViewTransition } from '../../lib/viewTransition';
import { Tooltip } from '../common/Tooltip';
import { H1 } from '../common/H1';
import { SessionsList } from '../sessions/SessionsList';
import { InboxPanel } from '../inbox/InboxPanel';
import { ExecutionTrace } from '../trace/ExecutionTrace';
import { SpendPanel } from './SpendPanel';

/** The four records, in the order they are shown. */
export const HISTORY_TABS = [
  {
    key: 'sessions', label: 'Sessions', icon: FiClock,
    sub: 'Your past conversations — reopen one to pick up where you left off, or rename and delete old ones. Picking a session opens it in the chat.',
  },
  {
    key: 'inbox', label: 'Downloads', icon: FiInbox,
    sub: 'Files you download in the in-app browser land here — send them to the Brain, a project, or the post scheduler. You choose; nothing is routed for you.',
  },
  {
    key: 'activity', label: 'Activity', icon: FiActivity,
    sub: "The runtime's most recent events, live off the running system's event streams — tool calls, worker activity, navigations, and lifecycle signals as they happen.",
  },
  {
    key: 'spend', label: 'Spend', icon: FiDollarSign,
    sub: 'What you run costs money — everything you have spent, per project and per session, plus the caps the cost router enforces. Enforced locally, not by a cloud admin.',
  },
] as const;

export type HistoryTabKey = (typeof HISTORY_TABS)[number]['key'];

export const HISTORY_TAB_KEYS: readonly string[] = HISTORY_TABS.map(t => t.key);

/** Is this deep-link section one of History's segments? */
export function isHistoryTab(key: string | null | undefined): key is HistoryTabKey {
  return !!key && HISTORY_TAB_KEYS.includes(key);
}

/**
 * Open-incident triage (wave-1 item 2): the failure-learning loop files
 * incidents, workers read them into every plan — this is the missing half
 * where a human closes them out. Honest quiet state: renders nothing when
 * there are none.
 */
function IncidentsStrip() {
  const { colors } = useTheme();
  const [incidents, setIncidents] = useState<IncidentView[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    api.getIncidents().then(i => { if (live) setIncidents(i); }).catch(() => { if (live) setIncidents([]); });
    return () => { live = false; };
  }, []);

  const resolve = async (id: string) => {
    setBusy(id);
    let resolved = true;
    try {
      await api.resolveIncident(id);
      setIncidents(prev => (prev ?? []).filter(i => i.id !== id));
    } catch {
      // Leave the row; the next load retells the truth. `false` is what keeps
      // the button from ticking over an incident that is still open.
      resolved = false;
    }
    setBusy(null);
    return resolved;
  };

  if (!incidents || incidents.length === 0) return null;
  return (
    <div style={{
      marginBottom: space.xl, border: `1px solid ${colors.border}`,
      borderRadius: radius.lg, padding: `${space.lg}px ${space.xxl}px`,
      background: colors.surface,
    }}>
      {/* `stale` rather than `warning`: an open incident is a thing whose AGE
          is the point — it keeps feeding worker plans until someone closes it
          — not an alarm competing with a real fault. */}
      <div style={{ ...type.label, color: colors.stale, marginBottom: space.sm }}>
        Open incidents — feeding every worker plan until resolved
      </div>
      {incidents.map(i => (
        <div key={i.id} style={{ display: 'flex', alignItems: 'baseline', gap: space.lg, padding: `${space.sm}px 0`, borderBottom: `1px solid ${colors.border}` }}>
          <Tooltip content={`${i.user_goal} — ${i.observation}`}>
            <span tabIndex={0} style={{ fontSize: textSize.caption, color: colors.text, flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
              [{i.surface}] {i.observation}
            </span>
          </Tooltip>
          <span style={{ fontSize: textSize.micro, color: colors.textDim, fontFamily: 'monospace' }}>{i.mechanism}</span>
          <Button
            colors={colors}
            variant="ghostOn"
            onClick={() => resolve(i.id)}
            disabled={busy === i.id}
            style={{
              '--pa-btn-border': colors.borderHi,
              '--pa-btn-border-hover': colors.cyan,
              '--pa-btn-pad': '2px 10px',
              '--pa-btn-radius': `${radius.sm}px`,
            } as CSSProperties}
          >
            {busy === i.id ? '…' : 'Resolve'}
          </Button>
        </div>
      ))}
    </div>
  );
}

/** A framed, scrolling host for one of the embedded record panels. */
function Frame({ children }: { children: React.ReactNode }) {
  const { colors } = useTheme();
  return (
    <div style={{
      flex: 1, minHeight: 320,
      border: `1px solid ${colors.border}`, borderRadius: radius.lg,
      background: colors.surface, overflow: 'hidden',
    }}>
      {children}
    </div>
  );
}

/** The page inset every other top-level view has (Grow's body, ViewHeader). */
export const HISTORY_PAGE_PADDING = `${space.xxxl}px ${space.huge}px`;

export function HistoryView({ initialTab = 'sessions' }: { initialTab?: HistoryTabKey }) {
  const { colors } = useTheme();
  const [tab, setTab] = useState<HistoryTabKey>(initialTab);
  // A deep link that arrives while the view is already mounted must move the
  // segment, not be ignored — `initialTab` is the live section, not a seed.
  useEffect(() => { setTab(initialTab); }, [initialTab]);
  // R8 item 5: a clicked tab crossfades + slides toward the direction the
  // user just picked (view-transition, index.css). A deep-link-driven switch
  // (the effect above) does NOT go through this — it isn't a click the user
  // is watching happen, it's the view arriving already on the right tab.
  const selectTab = (next: HistoryTabKey) => {
    runViewTransition(() => setTab(next), { direction: segmentDirection(HISTORY_TAB_KEYS as HistoryTabKey[], tab, next) });
  };

  // The Downloads segment carries an unread count of 'download' notifications
  // (files landed via `inbox_file_received`), fed by the same notification
  // stream the toast/tray read — not a second poll.
  const { items: notificationItems } = useNotifications();
  const downloadUnread = notificationItems.filter(n => n.kind === 'download' && !n.read).length;

  const active = HISTORY_TABS.find(t => t.key === tab) ?? HISTORY_TABS[0];

  return (
    // Page padding (QA 2026-09-24): History is mounted straight into the
    // workspace area (App.tsx), with no ViewHeader or scroll body around it,
    // so its title, segments and list sat flush against the rail. Same inset
    // as Grow's body and ViewHeader's horizontal padding.
    <div data-testid="history-view" style={{
      height: '100%', display: 'flex', flexDirection: 'column', boxSizing: 'border-box',
      padding: HISTORY_PAGE_PADDING,
    }}>
      <H1 sub={active.sub}>History</H1>

      {/* The app's segmented control (components/common/segmented.ts), the
          same one Grow wears. History used to style its own, and painted the
          selected tab with `colors.surface` — darker than this strip in the
          dark theme, so the chosen tab looked recessed and a hovered one
          looked chosen. */}
      <div
        role="tablist"
        aria-label="History"
        style={{
          display: 'inline-flex', gap: SEGMENT_STRIP_PAD, alignSelf: 'flex-start',
          padding: SEGMENT_STRIP_PAD, marginBottom: space.xxl,
          borderRadius: SEGMENT_STRIP_RADIUS, background: colors.bgDeeper,
          border: `1px solid ${colors.border}`,
        }}
      >
        {HISTORY_TABS.map(t => {
          const on = t.key === tab;
          return (
            <Button
              key={t.key}
              colors={colors}
              role="tab"
              id={'history-tab-' + t.key}
              aria-controls={'history-panel-' + t.key}
              aria-selected={on}
              data-testid={`history-tab-${t.key}`}
              onClick={() => selectTab(t.key)}
              flashSuccess={false}
              style={{ ...segmentedTab(colors, on), gap: space.sm } as CSSProperties}
            >
              <t.icon size={13} />
              {t.label}
              {t.key === 'inbox' && downloadUnread > 0 && (
                <span style={{
                  minWidth: 16, height: 16, padding: '0 4px', borderRadius: radius.pill,
                  background: colors.cyan, color: colors.textOnCyan,
                  fontSize: textSize.micro, fontWeight: 700,
                  display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
                }}>{downloadUnread > 9 ? '9+' : downloadUnread}</span>
              )}
            </Button>
          );
        })}
      </div>

      {tab === 'sessions' && (
        <div role="tabpanel" id="history-panel-sessions" aria-labelledby="history-tab-sessions" tabIndex={0} style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}>
          <Frame><SessionsList /></Frame>
        </div>
      )}
      {tab === 'inbox' && (
        <div role="tabpanel" id="history-panel-inbox" aria-labelledby="history-tab-inbox" tabIndex={0} style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}>
          <Frame><InboxPanel embedded /></Frame>
        </div>
      )}
      {tab === 'activity' && (
        <div role="tabpanel" id="history-panel-activity" aria-labelledby="history-tab-activity" tabIndex={0} style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}>
          <IncidentsStrip />
          <Frame><ExecutionTrace activity /></Frame>
        </div>
      )}
      {tab === 'spend' && (
        <div role="tabpanel" id="history-panel-spend" aria-labelledby="history-tab-spend" tabIndex={0} style={{ flex: 1, minHeight: 0 }}>
          <SpendPanel />
        </div>
      )}
    </div>
  );
}
