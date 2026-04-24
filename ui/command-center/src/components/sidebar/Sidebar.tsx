import { FiMessageSquare, FiZap, FiList, FiWifi, FiWifiOff } from 'react-icons/fi';
import { useCommandCenter, type ActivePanel } from '../../lib/store';

const NAV_ITEMS: Array<{ panel: ActivePanel; icon: typeof FiMessageSquare; label: string }> = [
  { panel: 'chat', icon: FiMessageSquare, label: 'Chat' },
  { panel: 'skills', icon: FiZap, label: 'Skills' },
  { panel: 'events', icon: FiList, label: 'Event Log' },
];

export function Sidebar() {
  const activePanel = useCommandCenter(s => s.activePanel);
  const setActivePanel = useCommandCenter(s => s.setActivePanel);
  const wsConnected = useCommandCenter(s => s.wsConnected);
  const wsReconnecting = useCommandCenter(s => s.wsReconnecting);

  return (
    <div className="flex h-full w-[200px] shrink-0 flex-col border-r border-dark-border bg-[#0B1120]">
      {/* Brand */}
      <div className="border-b border-dark-border px-4 py-3">
        <span className="text-[11px] font-mono font-semibold tracking-wider text-accent">
          PERMAGENT
        </span>
      </div>

      {/* Navigation */}
      <nav className="flex-1 px-2 py-3 space-y-1">
        {NAV_ITEMS.map(({ panel, icon: Icon, label }) => {
          const isActive = activePanel === panel;
          return (
            <button
              key={panel}
              onClick={() => setActivePanel(panel)}
              className={`flex w-full items-center gap-2.5 rounded-md px-3 py-2 text-xs font-medium transition ${
                isActive
                  ? 'bg-accent/10 text-accent'
                  : 'text-dark-muted hover:bg-white/5 hover:text-dark-text'
              }`}
            >
              <Icon size={14} />
              {label}
            </button>
          );
        })}
      </nav>

      {/* Connection status */}
      <div className="border-t border-dark-border px-4 py-3">
        <div className="flex items-center gap-2">
          {wsConnected ? (
            <>
              <FiWifi size={12} className="text-emerald-400" />
              <span className="text-[10px] font-mono text-emerald-400">Connected</span>
            </>
          ) : wsReconnecting ? (
            <>
              <FiWifiOff size={12} className="text-amber-400 animate-pulse" />
              <span className="text-[10px] font-mono text-amber-400">Reconnecting...</span>
            </>
          ) : (
            <>
              <FiWifiOff size={12} className="text-red-400" />
              <span className="text-[10px] font-mono text-red-400">Disconnected</span>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
