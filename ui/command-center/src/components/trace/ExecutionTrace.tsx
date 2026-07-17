import { useEffect } from 'react';
import { useCommandCenter } from '../../lib/store';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

/**
 * Execution trace — the last 50 events from the global daemon event bus
 * (useCommandCenter `events`). Reused as a workspace tool (no chrome) and as the
 * `activePanel:'trace'` overlay — when hosted as an overlay, `onClose` is
 * provided so it offers a Close button + Escape to dismiss back to the chat
 * (mirrors SkillsPanel/SessionsList/InboxPanel; a workspace host passes nothing
 * and shows no Close affordance).
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
          <button
            onClick={onClose}
            style={{ height: 26, padding: '0 10px', borderRadius: 8, background: 'transparent', border: `1px solid ${colors.border}`, color: colors.textMuted, cursor: 'pointer', fontFamily: font.body, fontSize: 12 }}
          >Close</button>
        )}
      </div>
      <div className="flex-1 overflow-y-auto px-3 py-2 space-y-1">
        {recentEvents.length === 0 ? (
          <p className="text-xs py-4 text-center" style={{ fontFamily: font.body, color: colors.textMuted }}>No events yet</p>
        ) : (
          recentEvents.map(ev => (
            <div
              key={ev.id}
              className="flex items-start gap-2 py-1"
              style={{ borderBottom: `1px solid ${colors.border}` }}
            >
              <span className="text-[10px] shrink-0" style={{ fontFamily: font.mono, color: colors.textMuted }}>
                {new Date(ev.timestamp).toLocaleTimeString()}
              </span>
              <span className="text-[11px]" style={{ fontFamily: font.mono, color: colors.cyan }}>{ev.event_type}</span>
            </div>
          ))
        )}
      </div>
    </div>
  );
}
