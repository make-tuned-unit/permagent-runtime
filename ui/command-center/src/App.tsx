import { useEffect, useState, useCallback, useRef } from 'react';
import { useCommandCenter } from './lib/store';
import { useTheme } from './styles/useTheme';
import { Sidebar } from './components/sidebar/Sidebar';
import { SettingsView } from './components/settings/SettingsView';
import { WorkspaceRenderer } from './components/workspaces/WorkspaceRenderer';
import { WizardShell } from './components/wizard/WizardShell';
import { Splash } from './components/splash/Splash';
import { ChatLauncher } from './components/chat/ChatLauncher';
import { ChatDock } from './components/chat/ChatDock';
import { GoalDetailModalHost } from './components/goals/GoalDetailModal';
import { PersonDetailModalHost } from './components/projects/PersonDetailModal';
import { DropZone } from './components/chat/DropZone';
import { NotificationHost } from './components/notifications/NotificationHost';
import { toast } from './lib/notifications';
import { VersionSkewBanner } from './components/version/VersionSkewBanner';
import { api, fileToBase64 } from './lib/api';
import { createChatWindow } from './lib/chatWindow';
import type { LayoutNode } from './lib/store';
import { useAppNavigate } from './hooks/useAppNavigate';
import { useVersionSkew } from './hooks/useVersionSkew';
import { onRepaintRegain, forceCompositorRepaint } from './lib/repaintOnRegain';

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
  const { gradient, density, theme } = useTheme();

  // Sync native window titlebar appearance with theme
  useEffect(() => {
    if (!('__TAURI_INTERNALS__' in window)) return;
    (async () => {
      try {
        const { getCurrentWindow } = await import('@tauri-apps/api/window');
        const win = getCurrentWindow();
        await win.setTheme(theme === 'silver' ? 'light' : 'dark');
        await win.setTitleBarStyle('overlay');
        await win.setBackgroundColor(gradient.shell);
      } catch { /* older Tauri or permission not available */ }
      // Enable media capture (getUserMedia) on this window's WKWebView.
      try {
        const { invoke } = await import('@tauri-apps/api/core');
        await invoke('enable_media_capture_cmd');
      } catch { /* voice mic capture unavailable — graceful */ }
    })();
  }, [theme]);

  const [phase, setPhase] = useState<'splash' | 'loading' | 'wizard' | 'app'>('splash');

  // Subscribe to AppNavigate events from the agent
  useAppNavigate();

  // Throttle-immune render self-heal (Phase 2.5 S1). On focus/visibility
  // regain, force the shell to repaint so React surfaces that froze under
  // macOS occlusion throttling (sidebar blanks in fullscreen #517) recover
  // immediately instead of waiting for an interaction. Terminals self-heal
  // their own xterm renderer via the same regain hook.
  const shellRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    return onRepaintRegain(() => {
      if (shellRef.current) forceCompositorRepaint(shellRef.current);
    });
  }, []);

  // App↔daemon version-skew detection — only once we're in the running app.
  const versionSkew = useVersionSkew(phase === 'app');

  useEffect(() => {
    if (phase !== 'loading') return;
    let cancelled = false;
    // Retry getConfig — the daemon may still be starting after a reinstall.
    // Without retry, a transient connection failure sends us to the wizard
    // even though wizard_complete=true in ~/.permagent/config.yaml.
    (async () => {
      const maxAttempts = 10;
      const delayMs = 1000;
      for (let attempt = 1; attempt <= maxAttempts; attempt++) {
        if (cancelled) return;
        try {
          const config: any = await api.getConfig();
          if (cancelled) return;
          const wizardDone = config?.config?.wizard_complete === true;
          setPhase(wizardDone ? 'app' : 'wizard');
          return;
        } catch {
          if (attempt < maxAttempts) {
            await new Promise(r => setTimeout(r, delayMs));
          }
        }
      }
      if (!cancelled) setPhase('wizard');
    })();
    return () => { cancelled = true; };
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
    const chatWindow = await createChatWindow(theme);
    chatWindow.once('tauri://error', (e) => console.error('Chat window error:', e));

    const ready = await Promise.race([
      new Promise<true>((resolve) => { once('chat_ready', () => resolve(true)); }),
      new Promise<false>((resolve) => setTimeout(() => resolve(false), 3000)),
    ]);

    if (ready) {
      await emit('chat_drop_files', { files: payload });
    } else {
      console.error('[drop] chat window did not become ready');
      toast('Could not deliver files to chat', 'The chat window did not answer — please try the drop again.');
    }
  }, [activeWorkspace, theme]);

  if (phase === 'splash') {
    return <Splash onDone={() => setPhase('loading')} />;
  }

  if (phase === 'loading') {
    return <div style={{ background: gradient.shell, width: '100vw', height: '100vh', paddingTop: 28 }} />;
  }

  if (phase === 'wizard') {
    return <WizardShell onComplete={() => { setPhase('app'); loadWorkspaces(); loadSkills(); }} />;
  }

  return (
    <div ref={shellRef} className={`flex flex-col h-screen density-${density}`} style={{ background: gradient.shell }}>
      {/* Title-bar strip — overlay mode makes native bar transparent; this fills
          the area behind the traffic lights with the sidebar color across full width */}
      <div data-tauri-drag-region style={{ height: 28, flexShrink: 0, background: gradient.sidebar }} />
      <VersionSkewBanner skew={versionSkew} />
      <div className="flex flex-1 min-h-0">
        <Sidebar />
        <main className="flex-1 min-w-0 overflow-hidden relative">
          <DropZone onDrop={handleDrop} disabled={!!(activeWorkspace && (hasToolType(activeWorkspace.layoutJson, 'world') || hasToolType(activeWorkspace.layoutJson, 'memory')))}>
            <MainContent />
          </DropZone>
        </main>
        <ChatLauncher />
        <ChatDock />
      </div>
      <GoalDetailModalHost />
      <NotificationHost />
      <PersonDetailModalHost />
    </div>
  );
}

function hasToolType(node: LayoutNode, tool: string): boolean {
  if (node.type === 'panel') return (node as { tool: string }).tool === tool;
  if (node.type === 'split') return (node as { children: LayoutNode[] }).children.some(c => hasToolType(c, tool));
  return false;
}

export default App;
