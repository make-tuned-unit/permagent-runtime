import { useCommandCenter } from '../../lib/store';

export function ExecutionTrace() {
  const events = useCommandCenter(s => s.events);
  const recentEvents = events.slice(0, 50);

  return (
    <div className="flex h-full flex-col bg-[#0A0E17] overflow-hidden">
      <div className="border-b border-dark-border px-4 py-2">
        <h2 className="text-xs font-mono font-semibold text-dark-text tracking-wider">TRACE</h2>
      </div>
      <div className="flex-1 overflow-y-auto px-3 py-2 space-y-1">
        {recentEvents.length === 0 ? (
          <p className="text-xs text-dark-muted py-4 text-center">No events yet</p>
        ) : (
          recentEvents.map(ev => (
            <div
              key={ev.id}
              className="flex items-start gap-2 py-1 border-b border-dark-border/50"
            >
              <span className="text-[10px] font-mono text-dark-muted shrink-0">
                {new Date(ev.timestamp).toLocaleTimeString()}
              </span>
              <span className="text-[11px] font-mono text-accent">{ev.event_type}</span>
            </div>
          ))
        )}
      </div>
    </div>
  );
}
