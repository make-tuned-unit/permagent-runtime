import { useState, useEffect } from 'react';
import { COLORS } from './constants';
import { api } from '../../lib/api';

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
}

// ── Henry's trim color ──────────────────────────────────────────────

const HENRY_TRIM = '#00D9FF';

// ── State badge colors ──────────────────────────────────────────────

const STATE_COLORS: Record<HenryState, { bg: string; text: string; border: string }> = {
  idle: { bg: 'rgba(107, 114, 128, 0.2)', text: '#9CA3AF', border: '#4B5563' },
  in_conversation: { bg: 'rgba(0, 217, 255, 0.12)', text: COLORS.neonCyan, border: '#0891B2' },
  tool_call: { bg: 'rgba(255, 179, 71, 0.15)', text: COLORS.neonAmber, border: '#D97706' },
};

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
  const [status, setStatus] = useState<HenryStatus | null>(null);

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

  // ESC to close — capture phase
  useEffect(() => {
    if (!visible) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopImmediatePropagation();
        onClose();
      }
    };
    window.addEventListener('keydown', handler, true);
    return () => window.removeEventListener('keydown', handler, true);
  }, [visible, onClose]);

  if (!visible || !status) return null;

  const state = (status.current_state as HenryState) || 'idle';
  const colors = STATE_COLORS[state] ?? STATE_COLORS.idle;
  const { identity, today_totals: today, lifetime_stats: lt } = status;

  return (
    <div style={panelStyle} onClick={(e) => e.stopPropagation()}>
      {/* Header */}
      <div style={headerStyle}>
        <span style={{ fontWeight: 600, letterSpacing: '0.05em' }}>
          {identity.name.toUpperCase()}
        </span>
        <button onClick={onClose} style={closeBtnStyle} title="Close (ESC)">✕</button>
      </div>

      {/* State badge */}
      <div style={{ padding: '8px 14px' }}>
        <div style={{
          display: 'inline-block',
          padding: '2px 10px',
          borderRadius: 3,
          fontSize: 11,
          fontWeight: 700,
          letterSpacing: '0.08em',
          background: colors.bg,
          color: colors.text,
          border: `1px solid ${colors.border}`,
        }}>
          {stateName(state)}
        </div>
        {status.current_tool && (
          <span style={{
            marginLeft: 10,
            fontSize: 11,
            color: COLORS.neonAmber,
          }}>
            <span style={spinnerStyle}>◠</span> processing…
          </span>
        )}
      </div>

      {/* Active sessions */}
      {status.active_sessions.length > 0 && (
        <Section title="ACTIVE SESSIONS" trimColor={HENRY_TRIM}>
          {status.active_sessions.slice(0, 3).map((s) => (
            <div key={s.id} style={{ fontSize: 11, lineHeight: 1.6 }}>
              <span style={{ color: COLORS.primaryMarble }}>{truncate(s.name, 32)}</span>
              <span style={{ color: '#6B7280', marginLeft: 8 }}>{relativeTime(s.started_at)}</span>
            </div>
          ))}
        </Section>
      )}

      {/* Tasks */}
      <Section title="TASKS" trimColor={HENRY_TRIM}>
        <StatRow label="In flight" value={status.tasks_in_flight} />
        {status.recent_tasks.length > 0 && (
          <div style={{ marginTop: 6 }}>
            <div style={{ fontSize: 10, color: '#6B7280', marginBottom: 3 }}>Recent:</div>
            {status.recent_tasks.slice(0, 5).map((t) => (
              <div key={t.id} style={{ fontSize: 10, lineHeight: 1.5, display: 'flex', justifyContent: 'space-between' }}>
                <span style={{ color: '#D1D5DB', maxWidth: 180, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                  {truncate(t.description, 30)}
                </span>
                <span style={{ color: t.status === 'completed' ? '#4ADE80' : '#EF4444', fontSize: 9 }}>
                  {t.status === 'completed' ? '✓' : '✗'}
                </span>
              </div>
            ))}
          </div>
        )}
      </Section>

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
    </div>
  );
}

// ── Sub-components ───────────────────────────────────────────────────

function Section({ title, trimColor, children }: {
  title: string;
  trimColor: string;
  children: React.ReactNode;
}) {
  return (
    <div style={{ padding: '0 14px 8px' }}>
      <div style={{
        fontSize: 10,
        fontWeight: 700,
        letterSpacing: '0.1em',
        color: trimColor,
        borderBottom: `1px solid ${trimColor}30`,
        paddingBottom: 3,
        marginBottom: 6,
      }}>
        {title}
      </div>
      {children}
    </div>
  );
}

function StatRow({ label, value }: { label: string; value: string | number }) {
  return (
    <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 11, lineHeight: 1.6 }}>
      <span style={{ color: '#9CA3AF' }}>{label}</span>
      <span style={{ color: COLORS.primaryMarble, fontWeight: 500 }}>{String(value)}</span>
    </div>
  );
}

// ── Styles ───────────────────────────────────────────────────────────

const panelStyle: React.CSSProperties = {
  position: 'absolute',
  top: 16,
  left: 16,
  width: 300,
  background: 'rgba(10, 14, 26, 0.88)',
  backdropFilter: 'blur(12px)',
  border: `1px solid ${COLORS.marbleVeining}25`,
  borderRadius: 8,
  fontFamily: 'monospace',
  color: COLORS.primaryMarble,
  zIndex: 20,
  pointerEvents: 'auto',
  overflow: 'hidden',
};

const headerStyle: React.CSSProperties = {
  display: 'flex',
  justifyContent: 'space-between',
  alignItems: 'center',
  padding: '10px 14px 6px',
  fontSize: 13,
  color: COLORS.primaryMarble,
  borderBottom: `1px solid ${COLORS.marbleVeining}20`,
  marginBottom: 4,
};

const closeBtnStyle: React.CSSProperties = {
  background: 'none',
  border: 'none',
  color: '#6B7280',
  cursor: 'pointer',
  fontSize: 14,
  padding: '2px 4px',
  lineHeight: 1,
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
