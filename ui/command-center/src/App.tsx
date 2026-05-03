import { useEffect, useState } from 'react';
import { useCommandCenter } from './lib/store';
import { Sidebar } from './components/sidebar/Sidebar';
import { SettingsView } from './components/settings/SettingsView';
import { SessionsList } from './components/sessions/SessionsList';
import { WorkspaceRenderer } from './components/workspaces/WorkspaceRenderer';
import { WizardShell } from './components/wizard/WizardShell';
import { Splash } from './components/splash/Splash';
import { ChatWidget } from './components/chat/ChatWidget';
import { api } from './lib/api';

function MainContent() {
  const activePanel = useCommandCenter(s => s.activePanel);
  const activeWorkspaceId = useCommandCenter(s => s.activeWorkspaceId);
  const workspacesLoaded = useCommandCenter(s => s.workspacesLoaded);

  if (activePanel === 'settings') {
    return <SettingsView />;
  }

  if (activePanel === 'sessions') {
    return <SessionsList />;
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
  const loadWorkspaces = useCommandCenter(s => s.loadWorkspaces);
  const loadSkills = useCommandCenter(s => s.loadSkills);
  const setActivePanel = useCommandCenter(s => s.setActivePanel);
  const activePanel = useCommandCenter(s => s.activePanel);

  const [phase, setPhase] = useState<'splash' | 'loading' | 'wizard' | 'app'>('splash');

  useEffect(() => {
    if (phase !== 'loading') return;
    api.getConfig()
      .then((config: any) => {
        const wizardDone = config?.config?.wizard_complete === true;
        setPhase(wizardDone ? 'app' : 'wizard');
      })
      .catch(() => setPhase('wizard'));
  }, [phase]);

  useEffect(() => {
    if (phase === 'app') {
      loadWorkspaces();
      loadSkills();
    }
  }, [phase, loadWorkspaces, loadSkills]);

  // Reset activePanel from 'settings' when workspace loads
  // so workspaces render by default
  useEffect(() => {
    setActivePanel('chat');
  }, [setActivePanel]);

  // Cmd+, opens settings (macOS convention)
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === ',') {
        e.preventDefault();
        setActivePanel(activePanel === 'settings' ? 'chat' : 'settings');
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [setActivePanel, activePanel]);

  if (phase === 'splash') {
    return <Splash onDone={() => setPhase('loading')} />;
  }

  if (phase === 'loading') {
    return <div style={{ background: '#0B1220', width: '100vw', height: '100vh' }} />;
  }

  if (phase === 'wizard') {
    return <WizardShell onComplete={() => { setPhase('app'); loadWorkspaces(); loadSkills(); }} />;
  }

  return (
    <div className="flex h-screen bg-[#0B1120]">
      <Sidebar />
      <main className="flex-1 min-w-0 overflow-hidden">
        <MainContent />
      </main>
      <ChatWidget />
    </div>
  );
}

export default App;
