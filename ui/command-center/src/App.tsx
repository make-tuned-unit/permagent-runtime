import { useEffect, useState, useCallback } from 'react';
import { useCommandCenter } from './lib/store';
import { useTheme } from './styles/useTheme';
import { Sidebar } from './components/sidebar/Sidebar';
import { SettingsView } from './components/settings/SettingsView';
import { WorkspaceRenderer } from './components/workspaces/WorkspaceRenderer';
import { WizardShell } from './components/wizard/WizardShell';
import { Splash } from './components/splash/Splash';
import { ChatLauncher } from './components/chat/ChatLauncher';
import { DropZone } from './components/chat/DropZone';
import { api, fileToBase64 } from './lib/api';
import type { LayoutNode } from './lib/store';

function MainContent() {
  const activePanel = useCommandCenter(s => s.activePanel);
  const activeWorkspaceId = useCommandCenter(s => s.activeWorkspaceId);
  const workspaces = useCommandCenter(s => s.workspaces);
  const workspacesLoaded = useCommandCenter(s => s.workspacesLoaded);

  const showSettings = activePanel === 'settings';

  if (!workspacesLoaded) {
    return (
      <div className="flex h-full items-center justify-center text-dark-muted text-xs font-mono">
        Loading workspaces...
      </div>
    );
  }

  if (!activeWorkspaceId && !showSettings) {
    return (
      <div className="flex h-full items-center justify-center text-dark-muted text-xs font-mono">
        No workspaces available
      </div>
    );
  }

  // Render ALL workspaces simultaneously, hiding inactive ones.
  // This prevents Terminal/Browser from unmounting and losing sessions
  // when switching between workspace tabs or opening settings.
  return (
    <div className="h-full w-full relative">
      {showSettings && (
        <div className="absolute inset-0 z-10">
          <SettingsView />
        </div>
      )}
      {workspaces.map(ws => (
        <div
          key={ws.id}
          className="absolute inset-0"
          style={{ display: (!showSettings && ws.id === activeWorkspaceId) ? 'block' : 'none' }}
        >
          <WorkspaceRenderer workspaceId={ws.id} />
        </div>
      ))}
    </div>
  );
}

function App() {
  const loadWorkspaces = useCommandCenter(s => s.loadWorkspaces);
  const loadSkills = useCommandCenter(s => s.loadSkills);
  const setActivePanel = useCommandCenter(s => s.setActivePanel);
  const activePanel = useCommandCenter(s => s.activePanel);
  const activeWorkspace = useCommandCenter(s => s.workspaces.find(w => w.id === s.activeWorkspaceId));
  const { gradient, density } = useTheme();

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

  const handleDrop = useCallback(async (files: File[]) => {
    if (!('__TAURI_INTERNALS__' in window)) return;
    // Don't accept drops on World View — it's a watch-only surface
    if (activeWorkspace && hasToolType(activeWorkspace.layoutJson, 'world')) return;
    const { WebviewWindow } = await import('@tauri-apps/api/webviewWindow');
    const { emit, once } = await import('@tauri-apps/api/event');

    const payload = await Promise.all(
      files.map(async (f) => ({ name: f.name, mime_type: f.type, data_b64: await fileToBase64(f) }))
    );

    const existing = await WebviewWindow.getByLabel('chat');
    if (existing) {
      await emit('chat_drop_files', { files: payload });
      return;
    }

    // Open a new chat window and wait for it to signal readiness
    const chatWindow = new WebviewWindow('chat', {
      url: 'index.html?view=chat', title: 'Permagent Chat',
      width: 480, height: 700, minWidth: 360, minHeight: 400,
      center: true, decorations: true, resizable: true, focus: true,
    });
    chatWindow.once('tauri://error', (e) => console.error('Chat window error:', e));

    const ready = await Promise.race([
      new Promise<true>((resolve) => { once('chat_ready', () => resolve(true)); }),
      new Promise<false>((resolve) => setTimeout(() => resolve(false), 3000)),
    ]);

    if (ready) {
      await emit('chat_drop_files', { files: payload });
    } else {
      console.error('[drop] chat window did not become ready');
      window.alert('Could not deliver files to chat — please try again');
    }
  }, [activeWorkspace]);

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
    <div className={`flex h-screen density-${density}`} style={{ background: gradient.shell }}>
      <Sidebar />
      <main className="flex-1 min-w-0 overflow-hidden relative">
        <DropZone onDrop={handleDrop} disabled={!!(activeWorkspace && hasToolType(activeWorkspace.layoutJson, 'world'))}>
          <MainContent />
        </DropZone>
      </main>
      <ChatLauncher />
    </div>
  );
}

function hasToolType(node: LayoutNode, tool: string): boolean {
  if (node.type === 'panel') return (node as { tool: string }).tool === tool;
  if (node.type === 'split') return (node as { children: LayoutNode[] }).children.some(c => hasToolType(c, tool));
  return false;
}

export default App;
