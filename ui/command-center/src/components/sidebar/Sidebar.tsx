import { useEffect, useCallback } from 'react';
import { Home, LayoutDashboard, Globe, Code, Brain, Settings, Wifi, WifiOff, Loader, MessageSquare } from 'lucide-react';
import { useCommandCenter, type ConnectionStatus } from '../../lib/store';

const WORKSPACE_ICONS: Record<string, typeof LayoutDashboard> = {
  'home': Home,
  'layout-dashboard': LayoutDashboard,
  'globe': Globe,
  'code': Code,
  'brain': Brain,
};

function ConnectionIndicator({ status }: { status: ConnectionStatus }) {
  switch (status) {
    case 'connected':
      return (
        <div className="flex items-center gap-2">
          <Wifi size={12} className="text-emerald-400" />
          <span className="text-[10px] font-mono text-emerald-400">Connected</span>
        </div>
      );
    case 'connecting':
      return (
        <div className="flex items-center gap-2">
          <Loader size={12} className="text-amber-400 animate-spin" />
          <span className="text-[10px] font-mono text-amber-400">Connecting...</span>
        </div>
      );
    case 'error':
      return (
        <div className="flex items-center gap-2">
          <WifiOff size={12} className="text-red-400 animate-pulse" />
          <span className="text-[10px] font-mono text-red-400">Error</span>
        </div>
      );
    case 'disconnected':
    default:
      return (
        <div className="flex items-center gap-2">
          <WifiOff size={12} className="text-dark-muted" />
          <span className="text-[10px] font-mono text-dark-muted">Disconnected</span>
        </div>
      );
  }
}

export function Sidebar() {
  const workspaces = useCommandCenter(s => s.workspaces);
  const activeWorkspaceId = useCommandCenter(s => s.activeWorkspaceId);
  const activePanel = useCommandCenter(s => s.activePanel);
  const switchWorkspace = useCommandCenter(s => s.switchWorkspace);
  const setActivePanel = useCommandCenter(s => s.setActivePanel);
  const connectionStatus = useCommandCenter(s => s.connectionStatus);

  const isSettingsOpen = activePanel === 'settings';
  const isSessionsOpen = activePanel === 'sessions';

  // Switch to a workspace AND leave settings if we're in it
  const goToWorkspace = useCallback((workspaceId: string) => {
    switchWorkspace(workspaceId);
    if (isSettingsOpen || isSessionsOpen) {
      setActivePanel('chat');
    }
  }, [switchWorkspace, setActivePanel, isSettingsOpen, isSessionsOpen]);

  // Keyboard shortcuts: Cmd+1, Cmd+2, Cmd+3
  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      if (!e.metaKey && !e.ctrlKey) return;
      const num = parseInt(e.key, 10);
      if (num >= 1 && num <= workspaces.length) {
        e.preventDefault();
        const ws = workspaces[num - 1];
        if (ws) goToWorkspace(ws.id);
      }
    }
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [workspaces, goToWorkspace]);

  return (
    <div className="flex h-full w-[200px] shrink-0 flex-col border-r border-dark-border bg-[#0B1120]">
      {/* Brand */}
      <div className="border-b border-dark-border px-4 py-3">
        <span className="text-[11px] font-mono font-semibold tracking-wider text-accent">
          PERMAGENT
        </span>
      </div>

      {/* Workspace navigation */}
      <nav className="flex-1 px-2 py-3 space-y-1">
        <div className="px-3 pb-2">
          <span className="text-[9px] font-mono font-semibold tracking-widest text-dark-muted/60 uppercase">
            Workspaces
          </span>
        </div>
        {workspaces.map((ws, i) => {
          const isActive = activeWorkspaceId === ws.id && !isSettingsOpen && !isSessionsOpen;
          const IconComponent = WORKSPACE_ICONS[ws.icon] || LayoutDashboard;
          return (
            <button
              key={ws.id}
              onClick={() => goToWorkspace(ws.id)}
              className={`flex w-full items-center gap-2.5 rounded-md px-3 py-2 text-xs font-medium transition ${
                isActive
                  ? 'bg-accent/10 text-accent'
                  : 'text-dark-muted hover:bg-white/5 hover:text-dark-text'
              }`}
              title={`${ws.name} (${navigator.platform.includes('Mac') ? '\u2318' : 'Ctrl+'}${i + 1})`}
            >
              <IconComponent size={14} />
              <span className="flex-1 text-left">{ws.name}</span>
              <kbd className="text-[9px] font-mono text-dark-muted/40">
                {navigator.platform.includes('Mac') ? '\u2318' : '^'}{i + 1}
              </kbd>
            </button>
          );
        })}
      </nav>

      {/* Bottom: Sessions + Settings + Connection */}
      <div className="border-t border-dark-border px-2 py-2 space-y-1">
        <button
          onClick={() => setActivePanel(isSessionsOpen ? 'chat' : 'sessions')}
          className={`flex w-full items-center gap-2.5 rounded-md px-3 py-2 text-xs font-medium transition ${
            isSessionsOpen
              ? 'bg-accent/10 text-accent'
              : 'text-dark-muted hover:bg-white/5 hover:text-dark-text'
          }`}
        >
          <MessageSquare size={14} />
          Sessions
        </button>
        <button
          onClick={() => setActivePanel(isSettingsOpen ? 'chat' : 'settings')}
          className={`flex w-full items-center gap-2.5 rounded-md px-3 py-2 text-xs font-medium transition ${
            isSettingsOpen
              ? 'bg-accent/10 text-accent'
              : 'text-dark-muted hover:bg-white/5 hover:text-dark-text'
          }`}
        >
          <Settings size={14} />
          Settings
        </button>
        <div className="px-3 py-1">
          <ConnectionIndicator status={connectionStatus} />
        </div>
      </div>
    </div>
  );
}
