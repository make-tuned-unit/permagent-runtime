import { useCommandCenter } from '../../lib/store';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

export function ExecutionTrace() {
  const { colors } = useTheme();
  const events = useCommandCenter(s => s.events);
  const recentEvents = events.slice(0, 50);

  return (
    <div className="flex h-full flex-col overflow-hidden" style={{ backgroundColor: colors.bg }}>
      <div className="px-4 py-2" style={{ borderBottom: `1px solid ${colors.border}` }}>
        <h2 className="text-xs tracking-wider" style={{ fontFamily: font.display, fontWeight: 600, color: colors.text }}>TRACE</h2>
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
