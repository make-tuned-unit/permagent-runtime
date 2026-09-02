import { useEffect, useState, useCallback, useRef } from 'react';
import { useCommandCenter, navigateToTool } from './lib/store';
import { useTheme } from './styles/useTheme';
import { Sidebar } from './components/sidebar/Sidebar';
import { SettingsView } from './components/settings/SettingsView';
import { HistoryView, isHistoryTab, type HistoryTabKey } from './components/history/HistoryView';
import { SkillsPanel } from './components/skills/SkillsPanel';
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
import { getOpenOnLaunch } from './lib/openOnLaunch';
import { useVersionSkew } from './hooks/useVersionSkew';
import { onRepaintRegain, forceCompositorRepaint } from './lib/repaintOnRegain';
import { NATIVE_WINDOW_IS_OPAQUE, TITLEBAR_HEIGHT } from './lib/windowChrome';

function MainContent() {
  const activePanel = useCommandCenter(s => s.activePanel);
  const activeWorkspaceId = useCommandCenter(s => s.activeWorkspaceId);
  const workspaces = useCommandCenter(s => s.workspaces);
  const workspacesLoaded = useCommandCenter(s => s.workspacesLoaded);

  const showSettings = activePanel === 'settings';
  // Skills Library renders as a labeled overlay so accepting a skill proposal
  // — which sets activePanel:'skills' — lands on a real surface instead of a
  // blank/unchanged screen. Also the target of navigate_app("Skills").
  const showSkills = activePanel === 'skills';
  // History — Sessions, Downloads, Activity, Spend. The old Console overlay
  // was retired into Settings by the 2026-08 ruling because there was nowhere
  // else to put it; #1177 made the four one component, and this is the half
  // that finishes the move: a record of what your agent did is not a setting,
  // and it should not cost a trip through a configuration screen to read one.
  // The segment comes from the deep link's own section key (`pendingSettings
  // Section`), so "Settings → Spend" still opens Spend and not Sessions.
  const showHistory = activePanel === 'history';
  const pendingSection = useCommandCenter(s => s.pendingSettingsSection);
  const setPendingSection = useCommandCenter(s => s.setPendingSettingsSection);
  const setActivePanel = useCommandCenter(s => s.setActivePanel);

  // The deep link's segment, consumed ONCE — the same contract SettingsView
  // has always had with `pendingSettingsSection`. Consuming it matters here
  // for a reason that only shows up on the second visit: nothing else clears
  // the key now that History is not a Settings pane, so a stale "spend" would
  // make every later trip to History open on Spend. Holding the segment in
  // state after that is deliberate — coming back to History where you left it
  // is what the Settings pane did, and what a destination should do.
  const [historyTab, setHistoryTab] = useState<HistoryTabKey>('sessions');
  useEffect(() => {
    if (!showHistory || !isHistoryTab(pendingSection)) return;
    setHistoryTab(pendingSection);
    setPendingSection(null);
  }, [showHistory, pendingSection, setPendingSection]);

  if (!workspacesLoaded) {
    return (
      <div className="flex h-full items-center justify-center text-dark-muted text-xs font-mono">
        Loading workspaces...
      </div>
    );
  }

  if (!activeWorkspaceId && !showSettings && !showSkills && !showHistory) {
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
      {showHistory && (
        <div className="absolute inset-0 z-10">
          <ErrorBoundary surface="History">
            <HistoryView initialTab={isHistoryTab(pendingSection) ? pendingSection : historyTab} />
          </ErrorBoundary>
        </div>
      )}
      {workspaces.map(ws => (
        <div
          key={ws.id}
          className="absolute inset-0"
          style={{ display: (!showSettings && !showSkills && !showHistory && ws.id === activeWorkspaceId) ? 'block' : 'none' }}
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

  // One-time native window setup.
  //
  // The `setTitleBarStyle('overlay')` call that used to live here is GONE, and
  // its absence is the fix, not an omission. `titleBarStyle` is now declared in
  // tauri.conf.json alongside `hiddenTitle` and `trafficLightPosition` — the
  // only path that applies a traffic-light position at all while the `unstable`
  // cargo feature is on (A1a spike, see src-tauri/src/chrome.rs). Setting the
  // style from JS mutates the NSWindow's style mask, which rebuilds the frame
  // view and snaps the window controls back to AppKit's (9, 9): the app was
  // asking, once per launch, for exactly the reset the re-inset exists to undo.
  //
  // enable_media_capture_cmd stays: it reaches into the live WKWebView's
  // configuration through a private API and has to happen after mount.
  useEffect(() => {
    if (!('__TAURI_INTERNALS__' in window)) return;
    (async () => {
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
        // The NSWindow's own fill, which shows only in the frames between a
        // resize and the webview repainting it. It is kept — and it is kept
        // CONDITIONAL on the window being opaque, which it is and will stay:
        // this paint is what blocks a vibrancy/`NSVisualEffectView` layer from
        // being visible underneath, and A1a measured the transparent window
        // that vibrancy requires at ~+6 points of whole-GPU utilisation at idle
        // on a static page (0.12% -> 6.1%), on an always-on desktop agent. So
        // the opaque fill is the correct choice, not a leftover. If the
        // transparent path is ever taken, this call is the first thing that has
        // to go — hence the named flag rather than a bare call.
        if (NATIVE_WINDOW_IS_OPAQUE) await win.setBackgroundColor(gradient.shell);
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

  // "Open on launch" (Settings → Preferences, LIVE): once workspaces have
  // loaded, land on the user's chosen destination. 'default' keeps the
  // existing behavior (default workspace). Applied exactly once per launch.
  const workspacesLoaded = useCommandCenter(s => s.workspacesLoaded);
  const launchApplied = useRef(false);
  useEffect(() => {
    if (phase !== 'app' || !workspacesLoaded || launchApplied.current) return;
    launchApplied.current = true;
    const pref = getOpenOnLaunch();
    if (pref !== 'default') navigateToTool(pref);
  }, [phase, workspacesLoaded]);

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
    return <div style={{ background: gradient.shell, width: '100vw', height: '100vh', paddingTop: TITLEBAR_HEIGHT }} />;
  }

  if (phase === 'wizard') {
    return <WizardShell onComplete={() => { setPhase('app'); loadWorkspaces(); loadSkills(); }} />;
  }

  return (
    <div ref={shellRef} className={`flex h-screen density-${density}`} style={{ background: gradient.shell }}>
      {/* THE SILHOUETTE. The rail is the outer flex child and runs the FULL
          height of the window — from under the traffic lights to the bottom
          edge — which is what makes the window read as a Tahoe app rather than
          as a web page with a coloured strip on top. It replaced a 28px band
          that spanned the whole width: that band put a horizontal seam across
          the window at the one place macOS expects a continuous vertical one,
          and it left the rail starting 28px down from a corner the OS had
          already rounded.

          The rail owns the traffic lights' band (Sidebar draws its own drag
          region at the top); the content column below owns the rest of the
          band as a drag region of its own, so the whole titlebar is draggable
          without either side reaching into the other.

          The native browser/terminal webviews need NOTHING here. Their bounds
          come from `getBoundingClientRect()` on their container
          (`Browser.tsx` `syncBounds`), so moving <main> right by the rail's
          width and down by the titlebar's height is subtraction they already
          do — the rect they publish simply arrives correct. That is why this
          change does not touch the bounds pump, which lane R14 is restyling. */}
      <Sidebar />
      <div className="flex flex-col flex-1 min-w-0">
        <div data-tauri-drag-region style={{ height: TITLEBAR_HEIGHT, flexShrink: 0 }} />
        <VersionSkewBanner skew={versionSkew} />
        <div className="flex flex-1 min-h-0">
          <main className="flex-1 min-w-0 overflow-hidden relative">
            <DropZone onDrop={handleDrop} disabled={!!(activeWorkspace && (hasToolType(activeWorkspace.layoutJson, 'world') || hasToolType(activeWorkspace.layoutJson, 'memory')))}>
              <MainContent />
            </DropZone>
          </main>
          <ChatLauncher />
          <ChatDock />
        </div>
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
