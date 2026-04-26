import { useEffect } from 'react';
import { useCommandCenter } from './lib/store';
import { Sidebar } from './components/sidebar/Sidebar';
import { SettingsView } from './components/settings/SettingsView';
import { WorkspaceRenderer } from './components/workspaces/WorkspaceRenderer';

function MainContent() {
  const activePanel = useCommandCenter(s => s.activePanel);
  const activeWorkspaceId = useCommandCenter(s => s.activeWorkspaceId);
  const workspacesLoaded = useCommandCenter(s => s.workspacesLoaded);

  // Settings is a special panel, not a workspace
  if (activePanel === 'settings') {
    return <SettingsView />;
  }

  if (!workspacesLoaded) {
    return (
      <div className="flex h-full items-center justify-center text-dark-muted text-xs font-mono">
        Loading workspaces...
      </div>
    );
  }

  if (!activeWorkspaceId) {
    return (
      <div className="flex h-full items-center justify-center text-dark-muted text-xs font-mono">
        No workspaces available
      </div>
    );
  }

  return <WorkspaceRenderer workspaceId={activeWorkspaceId} />;
}

function App() {
  const connect = useCommandCenter(s => s.connect);
  const disconnect = useCommandCenter(s => s.disconnect);
  const setActivePanel = useCommandCenter(s => s.setActivePanel);

  useEffect(() => {
    connect();
    return () => disconnect();
  }, [connect, disconnect]);

  // Reset activePanel from 'settings' when workspace loads
  // so workspaces render by default
  useEffect(() => {
    setActivePanel('chat');
  }, [setActivePanel]);

  return (
    <div className="flex h-screen bg-[#0B1120]">
      <Sidebar />
      <main className="flex-1 min-w-0 overflow-hidden">
        <MainContent />
      </main>
    </div>
  );
}

export default App;
