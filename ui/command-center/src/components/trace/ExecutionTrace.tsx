import { humanizeId } from '../../lib/humanLabel';
import { useEffect, useState, type CSSProperties } from 'react';
import { useCommandCenter } from '../../lib/store';
import { summarizeTraceEvent } from '../../lib/traceEvents';
import { font, radius, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';

import { Tooltip } from '../common/Tooltip';
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
export function ExecutionTrace({ onClose, activity = false }: { onClose?: () => void; activity?: boolean } = {}) {
  const { colors } = useTheme();
  const events = useCommandCenter(s => s.events);
  const [scope, setScope] = useState<'work' | 'trace'>(activity ? 'work' : 'trace');
  const [source, setSource] = useState('all');
  const [status, setStatus] = useState('all');
  const recentEvents = events.filter(ev =>
    (scope === 'trace' || ev.severity === 'error' || /(?:completed|failed|finished|Finish|decision|goal_state|task_|run_|agent_state)/i.test(ev.event_type))
    && (source === 'all' || ev.source === source)
    && (status !== 'errors' || ev.severity === 'error'),
  ).slice(0, 50);

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
        <h2 className="text-xs tracking-wider" style={{ fontFamily: font.display, fontWeight: 600, color: colors.text }}>{scope === 'work' ? 'WORK ACTIVITY' : 'EVENT TRACE'}</h2>
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
              fontSize: textSize.caption,
            } as CSSProperties}
          >Close</Button>
        )}
      </div>
      {activity && <div className="flex flex-wrap gap-2 px-4 py-2" style={{ color: colors.text, fontSize: textSize.caption }}>
        <label>View <select aria-label="Activity view" value={scope} onChange={e => setScope(e.target.value as 'work' | 'trace')} style={{ background: colors.inputBg }}><option value="work">Work outcomes</option><option value="trace">All diagnostic events</option></select></label>
        <label>Source <select aria-label="Activity source" value={source} onChange={e => setSource(e.target.value)} style={{ background: colors.inputBg }}><option value="all">All sources</option>{[...new Set(events.map(ev => ev.source))].map(value => <option key={value} value={value}>{value}</option>)}</select></label>
        <label>Status <select aria-label="Activity status" value={status} onChange={e => setStatus(e.target.value)} style={{ background: colors.inputBg }}><option value="all">All statuses</option><option value="errors">Errors</option></select></label>
        <span style={{ color: colors.textMuted }}>Latest {recentEvents.length} matching events from this session’s buffer</span>
      </div>}
      <div className="flex-1 overflow-y-auto px-3 py-2 space-y-1">
        {recentEvents.length === 0 ? (
          <p className="text-xs py-4 text-center" style={{ fontFamily: font.body, color: colors.textMuted }}>{events.length > 0 ? 'No events match this view. Diagnostic events remain available above.' : 'No events yet'}</p>
        ) : (
          recentEvents.map(ev => {
            const summary = summarizeTraceEvent(ev);
            const context = ev.run_id ?? ev.correlation_id ?? ev.task_id;
            return (
              <div
                key={ev.id}
                className="flex items-start gap-2 py-1"
                style={{ borderBottom: `1px solid ${colors.border}` }}
              >
                <span className="text-[10px] shrink-0" style={{ fontFamily: font.mono, color: colors.textMuted }}>
                  {new Date(ev.timestamp).toLocaleString()}
                </span>
                <Tooltip content={ev.event_type}>
                  <span tabIndex={0} className="text-[11px] shrink-0" style={{ fontFamily: font.body, outline: 'none', color: ev.severity === 'error' ? colors.danger : colors.cyan }}>{humanizeId(ev.event_type)}</span>
                </Tooltip>
                {activity && context && <Tooltip content={context}><span tabIndex={0} style={{ color: colors.textDim, fontSize: textSize.micro }}>Run / task {context.slice(0, 12)}</span></Tooltip>}
                {summary && (
                  <Tooltip content={summary}>
                    <span tabIndex={0} style={{ outline: 'none' }}>
                      <span className="text-[10px] truncate min-w-0" style={{ fontFamily: font.mono, color: colors.textMuted }}>
                        {summary}
                      </span>
                    </span>
                  </Tooltip>
                )}
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}
