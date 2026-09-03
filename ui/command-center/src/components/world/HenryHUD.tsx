import { useState, useEffect, type CSSProperties } from 'react';
import { COLORS } from './constants';
import { AGENT_TRIM } from './shared/palette';
import { api } from '../../lib/api';
import { radius, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import { HudShell, Section, StatRow, useTabReset } from './HudShell';
import type { HudTab } from './HudShell';
import { HenryIdentityTab } from './HenryIdentityTab';
import { useOrchestratorName } from './shared/useOrchestratorName';

// ── Types ────────────────────────────────────────────────────────────

type HenryState = 'idle' | 'in_conversation' | 'tool_call';

interface HenryStatus {
  identity: { name: string; traits: string[]; tone: string };
  current_state: HenryState;
  active_sessions: { id: string; name: string; started_at: string }[];
  current_tool: string | null;
  tasks_in_flight: number;
  recent_tasks: { id: string; description: string; status: string; tool_used: string | null; completed_at: string | null }[];
  today_totals: { messages_sent: number; tasks_dispatched: number; scheduled_fires: number; memories_formed: number };
  lifetime_stats: { total_memories: number; total_sessions: number; days_active: number; first_active: string | null };
  next_scheduled: { id: string; cron: string; currently_running: boolean } | null;
  /** Unread reports from the worker agents — the same list, in the same
   *  severity order, that Henry sees in his own brief. Optional so an older
   *  daemon (no `briefings` field) renders without the panel rather than
   *  crashing on undefined. */
  briefings?: { id: string; from: string; severity: string; summary: string; created_at: string }[];
}

// ── Henry's trim color ──────────────────────────────────────────────

// Warm white-gold (#F0E6D0) per WORLD_VIEW_BIBLE.md §2 / §9 D2 — resolves issue #87.
const HENRY_TRIM = AGENT_TRIM.henry;

// ── State badge colors ──────────────────────────────────────────────

function stateColors(c: { fillSubtle: string; textMuted: string; border: string; cyanSoft: string; cyan: string; borderHi: string; staleSoft: string; warning: string }) {
  return {
    idle: { bg: c.fillSubtle, text: c.textMuted, border: c.border },
    in_conversation: { bg: c.cyanSoft, text: c.cyan, border: c.borderHi },
    tool_call: { bg: c.staleSoft, text: c.warning, border: c.warning },
  } as const;
}

// ── Tab definitions ─────────────────────────────────────────────────

function henryTabs(success: string): HudTab[] {
  return [
    { id: 'status', label: 'STATUS', accentColor: COLORS.neonCyan },
    { id: 'identity', label: 'IDENTITY', accentColor: success },
    { id: 'chat', label: 'CHAT', accentColor: COLORS.neonAmber, disabled: true, disabledLabel: 'SOON' },
    { id: 'tools', label: 'TOOLS', accentColor: AGENT_TRIM.felix, disabled: true, disabledLabel: 'SOON' },
  ];
}

// ── Helpers ──────────────────────────────────────────────────────────

function stateName(state: HenryState): string {
  switch (state) {
    case 'idle': return 'IDLE';
    case 'in_conversation': return 'IN CONVERSATION';
    case 'tool_call': return 'TOOL CALL';
  }
}

function relativeTime(iso: string): string {
  const diff = Date.now() - new Date(iso).getTime();
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return 'just now';
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  const days = Math.floor(hrs / 24);
  return `${days}d ago`;
}

function formatDate(iso: string | null): string {
  if (!iso) return '—';
  const d = new Date(iso);
  return d.toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' });
}

function truncate(s: string, max: number): string {
  return s.length <= max ? s : s.slice(0, max) + '…';
}

// ── Component ────────────────────────────────────────────────────────

interface HenryHUDProps {
  visible: boolean;
  onClose: () => void;
}

export function HenryHUD({ visible, onClose }: HenryHUDProps) {
  const { colors: theme } = useTheme();
  const [status, setStatus] = useState<HenryStatus | null>(null);
  const [activeTab, setActiveTab] = useTabReset(visible, 'status');
  const orchestratorName = useOrchestratorName();

  // Poll /api/henry/status at 1s
  useEffect(() => {
    if (!visible) return;

    let cancelled = false;
    const poll = async () => {
      try {
        const s = await api.getHenryStatus();
        if (!cancelled) setStatus(s as HenryStatus);
      } catch {
        // silently retry next tick
      }
    };
    poll();
    const id = setInterval(poll, 1000);
    return () => { cancelled = true; clearInterval(id); };
  }, [visible]);

  if (!visible) return null;

  const state = status ? ((status.current_state as HenryState) || 'idle') : 'idle';
  const colors = stateColors(theme)[state] ?? stateColors(theme).idle;
  const displayName = (status?.identity?.name ?? orchestratorName ?? 'AGENT').toUpperCase();
  const tabs = henryTabs(theme.success);

  const statusPill = (
    <div style={{
      display: 'inline-block',
      padding: '2px 8px',
      borderRadius: radius.xs,
      fontSize: textSize.micro,
      fontWeight: 700,
      letterSpacing: '0.08em',
      background: colors.bg,
      color: colors.text,
      border: `1px solid ${colors.border}`,
    }}>
      {stateName(state)}
    </div>
  );

  return (
    <HudShell
      visible={visible}
      onClose={onClose}
      title={displayName}
      statusPill={statusPill}
      tabs={tabs}
      activeTab={activeTab}
      onTabChange={setActiveTab}
    >
      {activeTab === 'status' && <HenryStatusBody status={status} />}
      {activeTab === 'identity' && <HenryIdentityTab />}
    </HudShell>
  );
}

// ── Status tab body (pixel-identical to previous full HUD) ──────────

function HenryStatusBody({ status }: { status: HenryStatus | null }) {
  // Locally hide acked briefings until the next status poll confirms.
  const [ackedIds, setAckedIds] = useState<string[]>([]);
  const { colors: themeColors } = useTheme();
  if (!status) return null;

  const { today_totals: today, lifetime_stats: lt } = status;
  const briefings = (status.briefings ?? []).filter((b) => !ackedIds.includes(b.id));

  return (
    <>
      {/* State badge area (current tool indicator) */}
      {status.current_tool && (
        <div style={{ padding: '4px 14px 0' }}>
          <span style={{ fontSize: textSize.micro, color: COLORS.neonAmber }}>
            <span style={spinnerStyle}>◠</span> processing…
          </span>
        </div>
      )}

      {/* Active sessions */}
      {status.active_sessions.length > 0 && (
        <Section title="ACTIVE SESSIONS" trimColor={HENRY_TRIM}>
          {status.active_sessions.slice(0, 3).map((s) => (
            <div key={s.id} style={{ fontSize: textSize.micro, lineHeight: 1.6 }}>
              <span style={{ color: themeColors.text }}>{truncate(s.name, 32)}</span>
              <span style={{ color: themeColors.textDim, marginLeft: 8 }}>{relativeTime(s.started_at)}</span>
            </div>
          ))}
        </Section>
      )}

      {/* Tasks */}
      <Section title="TASKS" trimColor={HENRY_TRIM}>
        <StatRow label="Tasks running" value={status.tasks_in_flight} />
        {status.recent_tasks.length > 0 && (
          <div style={{ marginTop: 6 }}>
            <div style={{ fontSize: textSize.micro, color: themeColors.textDim, marginBottom: 3 }}>Recent:</div>
            {status.recent_tasks.slice(0, 5).map((t) => (
              <div key={t.id} style={{ fontSize: textSize.micro, lineHeight: 1.5, display: 'flex', justifyContent: 'space-between' }}>
                <span style={{ color: themeColors.text, maxWidth: 180, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                  {truncate(t.description, 30)}
                </span>
                <span style={{ color: t.status === 'completed' ? themeColors.success : themeColors.dangerStrong, fontSize: textSize.micro }}>
                  {t.status === 'completed' ? '✓' : '✗'}
                </span>
              </div>
            ))}
          </div>
        )}
      </Section>

      {/* Briefings from the worker agents. Placed above TODAY because this is
          the only panel that can be waiting on someone — the same reason it
          renders first in Henry's own brief. Omitted entirely when nothing is
          unread rather than showing an empty box. */}
      {briefings.length > 0 && (
        <Section title="BRIEFINGS" trimColor={COLORS.neonAmber}>
          {briefings.map((b) => (
            <div key={b.id} style={briefingRowStyle}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                <span
                  style={{
                    ...briefingSeverityStyle,
                    color:
                      b.severity === 'action required'
                        ? COLORS.neonAmber
                        : COLORS.marbleVeining,
                    opacity: b.severity === 'info' ? 0.7 : 1,
                  }}
                >
                  {b.severity}
                </span>
                <span style={briefingFromStyle}>{b.from}</span>
                {/* Ack = "seen", never approval (briefings.rs contract).
                    Wave-1 item 5: the unacked list previously grew forever —
                    no route, no button. */}
                <Button
                  colors={themeColors}
                  variant="bare"
                  type="button"
                  title="Mark as seen"
                  aria-label="Mark as seen"
                  onClick={() => api.ackBriefings([b.id])
                    .then(() => { setAckedIds((ids) => [...ids, b.id]); return true; })
                    // The row stays and the next status load retells the truth,
                    // so `false` keeps the button from ticking over a failure.
                    .catch(() => false)}
                  style={{
                    '--pa-btn-fg': themeColors.textMuted,
                    '--pa-btn-fg-hover': themeColors.text,
                    '--pa-btn-bg-hover': themeColors.fillHover,
                    '--pa-btn-pad': '2px 4px',
                    '--pa-btn-radius': `${radius.xs}px`,
                    marginLeft: 'auto', fontSize: textSize.micro, lineHeight: 1,
                  } as CSSProperties}
                >
                  ✓
                </Button>
              </div>
              <div style={briefingSummaryStyle}>{b.summary}</div>
            </div>
          ))}
        </Section>
      )}

      {/* Today */}
      <Section title="TODAY" trimColor={COLORS.neonAmber}>
        <StatRow label="Messages sent" value={today.messages_sent} />
        <StatRow label="Tasks dispatched" value={today.tasks_dispatched} />
        <StatRow label="Scheduled fires" value={today.scheduled_fires} />
        <StatRow label="Memories formed" value={today.memories_formed} />
      </Section>

      {/* Lifetime */}
      <Section title="LIFETIME" trimColor={COLORS.marbleVeining}>
        <StatRow label="Total memories" value={lt.total_memories} />
        <StatRow label="Total sessions" value={lt.total_sessions} />
        <StatRow label="Days active" value={lt.days_active} />
        <StatRow label="Member since" value={formatDate(lt.first_active)} />
      </Section>

      {/* Schedule */}
      {status.next_scheduled && (
        <Section title="NEXT SCHEDULED" trimColor={COLORS.marbleVeining}>
          <StatRow label="Job" value={status.next_scheduled.id} />
          <StatRow label="Cron" value={status.next_scheduled.cron} />
          <StatRow
            label="Status"
            value={status.next_scheduled.currently_running ? 'Running' : 'Waiting'}
          />
        </Section>
      )}
    </>
  );
}

// ── Styles ───────────────────────────────────────────────────────────

const briefingRowStyle: React.CSSProperties = {
  padding: '6px 0',
  borderBottom: `1px solid ${COLORS.marbleVeining}22`,
};

const briefingSeverityStyle: React.CSSProperties = {
  fontSize: 9,
  letterSpacing: '0.08em',
  textTransform: 'uppercase',
  fontWeight: 600,
};

const briefingFromStyle: React.CSSProperties = {
  fontSize: textSize.micro,
  color: COLORS.marbleVeining,
};

const briefingSummaryStyle: React.CSSProperties = {
  fontSize: textSize.micro,
  marginTop: 2,
  lineHeight: 1.4,
};

const spinnerStyle: React.CSSProperties = {
  display: 'inline-block',
  animation: 'henrySpin 1s linear infinite',
};

// Inject keyframes once
if (typeof document !== 'undefined' && !document.getElementById('henry-hud-styles')) {
  const style = document.createElement('style');
  style.id = 'henry-hud-styles';
  style.textContent = `@keyframes henrySpin { to { transform: rotate(360deg); } }`;
  document.head.appendChild(style);
}
