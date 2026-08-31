import { useEffect, type CSSProperties } from 'react';
import { useCommandCenter } from '../../lib/store';
import { summarizeTraceEvent } from '../../lib/traceEvents';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';

/**
 * Execution trace — the last 50 events in the `useCommandCenter.events` buffer,
 * fed by BOTH runtime wires (see lib/traceEvents.ts): the global `/events` bus
 * (navigations, worker/agent activity, goal/decision/skill lifecycle, activity
 * usage signals — recorded with their real wire types) and the per-session chat
 * SSE (tool_call / Message / Error / Finish). Reused as a workspace tool (no
 * chrome) and as the `activePanel:'trace'` overlay — when hosted as an overlay,
 * `onClose` is provided so it offers a Close button + Escape to dismiss back to
 * the chat (mirrors SkillsPanel/SessionsList/InboxPanel; a workspace host
 * passes nothing and shows no Close affordance).
 */
export function ExecutionTrace({ onClose }: { onClose?: () => void } = {}) {
  const { colors } = useTheme();
  const events = useCommandCenter(s => s.events);
  const recentEvents = events.slice(0, 50);

  // Overlay dismissal — Escape closes back to chat, but only when hosted as an
  // overlay (onClose provided).
  useEffect(() => {
    if (!onClose) return;
    const h = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.preventDefault(); onClose(); } };
    window.addEventListener('keydown', h);
    return () => window.removeEventListener('keydown', h);
  }, [onClose]);

  return (
    <div className="flex h-full flex-col overflow-hidden" style={{ backgroundColor: colors.bg }}>
      <div className="px-4 py-2 flex items-center justify-between" style={{ borderBottom: `1px solid ${colors.border}` }}>
        <h2 className="text-xs tracking-wider" style={{ fontFamily: font.display, fontWeight: 600, color: colors.text }}>TRACE</h2>
        {onClose && (
          <Button
            colors={colors}
            onClick={onClose}
            style={{
              '--pa-btn-fg': colors.textMuted,
              '--pa-btn-fg-hover': colors.text,
              '--pa-btn-pad': '0 10px',
              '--pa-btn-radius': `${radius.md}px`,
              height: 26,
              fontFamily: font.body,
              fontSize: 12,
            } as CSSProperties}
          >Close</Button>
        )}
      </div>
      <div className="flex-1 overflow-y-auto px-3 py-2 space-y-1">
        {recentEvents.length === 0 ? (
          <p className="text-xs py-4 text-center" style={{ fontFamily: font.body, color: colors.textMuted }}>No events yet</p>
        ) : (
          recentEvents.map(ev => {
            const summary = summarizeTraceEvent(ev);
            return (
              <div
                key={ev.id}
                className="flex items-start gap-2 py-1"
                style={{ borderBottom: `1px solid ${colors.border}` }}
              >
                <span className="text-[10px] shrink-0" style={{ fontFamily: font.mono, color: colors.textMuted }}>
                  {new Date(ev.timestamp).toLocaleTimeString()}
                </span>
                <span className="text-[11px] shrink-0" style={{ fontFamily: font.mono, color: ev.severity === 'error' ? colors.danger : colors.cyan }}>{ev.event_type}</span>
                {summary && (
                  <span className="text-[10px] truncate min-w-0" style={{ fontFamily: font.mono, color: colors.textMuted }} title={summary}>
                    {summary}
                  </span>
                )}
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}
