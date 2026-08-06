import { useEffect, useState, useCallback, useRef } from 'react';
import { useCommandCenter } from './lib/store';
import { useTheme } from './styles/useTheme';
import { Sidebar } from './components/sidebar/Sidebar';
import { ConsoleTabStrip, isConsolePanel } from './components/console/ConsoleTabs';
import { SettingsView } from './components/settings/SettingsView';
import { InboxPanel } from './components/inbox/InboxPanel';
import { SkillsPanel } from './components/skills/SkillsPanel';
import { SessionsList } from './components/sessions/SessionsList';
import { ExecutionTrace } from './components/trace/ExecutionTrace';
import { GovernanceView } from './components/governance/GovernanceView';
import { WorkspaceRenderer } from './components/workspaces/WorkspaceRenderer';
import { WorkspaceSaveErrorChip } from './components/workspaces/WorkspaceSaveErrorChip';
import { ErrorBoundary } from './components/common/ErrorBoundary';
import { WizardShell } from './components/wizard/WizardShell';
import { Splash } from './components/splash/Splash';
import { ChatLauncher } from './components/chat/ChatLauncher';
import { ChatDock } from './components/chat/ChatDock';
import { VoiceHost } from './components/voice/VoiceHost';
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
  // Skills Library renders as a labeled overlay (mirrors inbox) so accepting a
  // skill proposal — which sets activePanel:'skills' — lands on a real surface
  // instead of a blank/unchanged screen. Also the target of navigate_app("Skills").
  const showSkills = activePanel === 'skills';
  // Session history renders as a labeled overlay (mirrors skills/inbox) so a user
  // can browse/switch/rename/delete past conversations. Reached from the sidebar
  // "Sessions" row; selecting a session loads it into the chat dock.
  // Execution trace renders as a labeled overlay (mirrors sessions/skills/inbox).
  // ExecutionTrace reads the global `events` buffer and needs no session id, so a
  // global overlay is the honest entry point. Reached from the sidebar "Trace" row.
  // Governance renders as a labeled overlay (mirrors trace/sessions/inbox): a
  // global, session-less surface, so a global overlay is the honest entry point.
  // Reached from the sidebar "Governance" row and navigate_app("Governance").
  const setActivePanel = useCommandCenter(s => s.setActivePanel);

  if (!workspacesLoaded) {
    return (
      <div className="flex h-full items-center justify-center text-dark-muted text-xs font-mono">
        Loading workspaces...
      </div>
    );
  }

  if (!activeWorkspaceId && !showSettings && !showSkills && !isConsolePanel(activePanel)) {
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
      {showSkills && (
        <div className="absolute inset-0 z-10">
          <SkillsPanel onClose={() => setActivePanel('chat')} />
        </div>
      )}
      {/* Console — Sessions/Inbox/Trace/Governance as tabs of ONE overlay
          (2026-07-27 consolidation). Same panel components, shared chrome. */}
      {isConsolePanel(activePanel) && (
        <div className="absolute inset-0 z-10" style={{ display: 'flex', flexDirection: 'column' }}>
          <ConsoleTabStrip active={activePanel} />
          <div style={{ flex: 1, minHeight: 0, overflow: 'hidden' }}>
            {activePanel === 'sessions' && <SessionsList onClose={() => setActivePanel('chat')} />}
            {activePanel === 'inbox' && <InboxPanel />}
            {activePanel === 'trace' && <ExecutionTrace onClose={() => setActivePanel('chat')} />}
            {activePanel === 'governance' && <GovernanceView />}
          </div>
        </div>
      )}
      {workspaces.map(ws => (
        <div
          key={ws.id}
          className="absolute inset-0"
          style={{ display: (!showSettings && !showSkills && !isConsolePanel(activePanel) && ws.id === activeWorkspaceId) ? 'block' : 'none' }}
        >
          <ErrorBoundary surface="the workspace">
            <WorkspaceRenderer workspaceId={ws.id} />
          </ErrorBoundary>
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
  const { gradient, density, theme, themePref } = useTheme();

  // One-time native window setup. Deliberately NOT in the theme effect below:
  // both of these are heavyweight and unrelated to colour, and re-running them
  // on every appearance change tore the window down and rebuilt it hard enough
  // to look like the app had restarted. setTitleBarStyle mutates the NSWindow's
  // style mask (a frame-view rebuild), and enable_media_capture_cmd reaches
  // into the live WKWebView's configuration through a private API.
  useEffect(() => {
    if (!('__TAURI_INTERNALS__' in window)) return;
    (async () => {
      try {
        const { getCurrentWindow } = await import('@tauri-apps/api/window');
        await getCurrentWindow().setTitleBarStyle('overlay');
      } catch { /* older Tauri or permission not available */ }
      // Enable media capture (getUserMedia) on this window's WKWebView.
      try {
        const { invoke } = await import('@tauri-apps/api/core');
        await invoke('enable_media_capture_cmd');
      } catch { /* voice mic capture unavailable — graceful */ }
    })();
  }, []);

  // Colour only — the part that genuinely tracks the theme.
  useEffect(() => {
    if (!('__TAURI_INTERNALS__' in window)) return;
    (async () => {
      try {
        const { getCurrentWindow } = await import('@tauri-apps/api/window');
        const win = getCurrentWindow();
        // `null` hands the decision back to macOS. tao's set_theme sets the
        // APPLICATION-wide NSAppearance, so pinning it to a concrete value
        // under a "System" preference would fight the OS on every light/dark
        // flip instead of following it.
        await win.setTheme(themePref === 'system' ? null : theme === 'silver' ? 'light' : 'dark');
        await win.setBackgroundColor(gradient.shell);
      } catch { /* older Tauri or permission not available */ }
    })();
  }, [theme, themePref, gradient.shell]);

  const [phase, setPhase] = useState<'splash' | 'loading' | 'wizard' | 'app'>('splash');

  // Install the bundled dictation model on first run so the mic "just works"
  // offline, with no download and no setup. Resolves the bundled Whisper model
  // (Tauri resource) and asks the daemon to install it + set LOCAL_WHISPER_MODEL.
  // Idempotent, fire-and-forget; a dev/unbundled build simply no-ops. Gated on
  // `phase === 'app'` so the daemon is known ready (mount is too early — the app
  // is still waiting for the daemon, and provisioning runs only once).
  const dictationProvisioned = useRef(false);
  useEffect(() => {
    if (phase !== 'app' || dictationProvisioned.current) return;
    if (!('__TAURI_INTERNALS__' in window)) return;
    dictationProvisioned.current = true;
    (async () => {
      try {
        const { resolveResource } = await import('@tauri-apps/api/path');
        const modelPath = await resolveResource('whisper/whisper-base-q8_0.gguf');
        await api.provisionDictationModel(modelPath);
      } catch { /* no bundled model (dev build) — dictation stays opt-in */ }
    })();
  }, [phase]);

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

    // The DOCK is a chat surface too: with it open, hand the files straight to
    // its composer instead of popping a whole chat window out over the app —
    // that pop-out was the reported bug ("the drop goes over the main app").
    // With the dock closed, open it and queue; the window path below stays the
    // route only when a detached window already exists.
    {
      const { chatWindowOpen, chatDockOpen, queueChatFiles, openChatDock } =
        useCommandCenter.getState();
      if (!chatWindowOpen) {
        queueChatFiles(files);
        if (!chatDockOpen) openChatDock();
        return;
      }
    }

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
      <WorkspaceSaveErrorChip />
      {/* Per-window voice engine — survives dock close/detach/view switches. */}
      <VoiceHost />
      <VoiceConversationFallback />
    </div>
  );
}

// The orb lives INSIDE a chat surface — the dock (ChatView) or the detached
// window's mirror — never as a standalone takeover of the whole app.
//
// There used to be a full-window fallback here for "conversation live but no
// chat surface open". It made closing the sidebar explode the orb across the
// entire app, which reads as a bug rather than a feature. Closing the last
// chat surface now ends the conversation (see ChatDock), so this state cannot
// arise and the takeover has nowhere to leak to.
function VoiceConversationFallback() {
  return null;
}

function hasToolType(node: LayoutNode, tool: string): boolean {
  if (node.type === 'panel') return (node as { tool: string }).tool === tool;
  if (node.type === 'split') return (node as { children: LayoutNode[] }).children.some(c => hasToolType(c, tool));
  return false;
}

export default App;
