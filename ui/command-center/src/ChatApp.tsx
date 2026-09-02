/**
 * ChatApp — standalone chat view rendered in the separate chat window.
 * Loaded when the URL contains ?view=chat.
 * All state flows through the daemon (HTTP/SSE), not the main window.
 */
import { useEffect, useRef, useState, useCallback, type CSSProperties } from 'react';
import { FiEye } from 'react-icons/fi';
import { font, radius } from './styles/tokens';
import { useTheme } from './styles/useTheme';
import { Button } from './components/common/Button';
import { Tooltip } from './components/common/Tooltip';
import { Mobius } from './components/mobius/Mobius';
import { useCommandCenter } from './lib/store';
import { api } from './lib/api';
import { MessageList } from './components/chat/MessageList';
import { ChatInput } from './components/chat/ChatInput';
import type { ChatInputHandle } from './components/chat/ChatInput';
import { DropZone } from './components/chat/DropZone';
import { ModelPicker } from './components/chat/ModelPicker';
import { SessionPicker } from './components/chat/SessionPicker';
import { InspectionPanel } from './components/inspection/InspectionPanel';
import { AwarenessIndicator } from './components/awareness/AwarenessIndicator';
import { PreTurnPreview } from './components/awareness/PreTurnPreview';
import { trackChatGeometry } from './lib/chatWindow';
import { VoiceOrb } from './components/voice/VoiceOrb';
import { readLiveConversation, requestVoiceEnd } from './lib/voiceHandoff';
// VoiceButton moved to ChatInput row (beside send button)

export default function ChatApp() {
  const { gradient, colors, theme } = useTheme();
  const setAgentName = useCommandCenter(s => s.setAgentName);
  const [inspectionOpen, setInspectionOpen] = useState(false);
  const [inputFocused, setInputFocused] = useState(false);

  // Sync native window titlebar appearance with theme
  useEffect(() => {
    if (!('__TAURI_INTERNALS__' in window)) return;
    (async () => {
      try {
        const { getCurrentWindow } = await import('@tauri-apps/api/window');
        const win = getCurrentWindow();
        await win.setTheme(theme === 'silver' ? 'light' : 'dark');
        await win.setBackgroundColor(gradient.shell);
      } catch { /* ignore */ }
    })();
  }, [theme]);

  // NOTE: nothing here (or anywhere) may register `onCloseRequested` on this
  // window. Tauri core does:
  //
  //   WindowEvent::CloseRequested { api } =>
  //     if window.has_js_listener(CLOSE_REQUESTED) { api.prevent_close() }
  //
  // — the mere EXISTENCE of a JS listener suppresses the native close, so the
  // window then closes only if some JS path calls destroy(). Two windows used
  // to register one (this one, and ChatLauncher in main), and when the destroy
  // failed to land the window became unclosable no matter how many times the X
  // was pressed. With zero listeners the first press closes it natively, always.
  // Main learns it is gone from `tauri://destroyed` + an existence poll.

  // Persist this window's position/size so it reopens where the user left it.
  useEffect(() => {
    let cleanup: (() => void) | undefined;
    trackChatGeometry().then(fn => { cleanup = fn; });
    return () => { cleanup?.(); };
  }, []);

  const ensureSession = useCommandCenter(s => s.ensureSession);
  const connectSession = useCommandCenter(s => s.connectSession);
  const loadSessionMessages = useCommandCenter(s => s.loadSessionMessages);

  const chatInputRef = useRef<ChatInputHandle>(null);
  const connectedRef = useRef(false);

  const handleDrop = useCallback((files: File[]) => {
    chatInputRef.current?.addFiles(files);
  }, []);

  // Listen for cross-window file drops and signal readiness
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    (async () => {
      const { emit, listen } = await import('@tauri-apps/api/event');
      unlisten = await listen<{ files: { name: string; mime_type: string; data_b64: string }[] }>('chat_drop_files', (event) => {
        const files = event.payload.files.map((f) => {
          const bytes = Uint8Array.from(atob(f.data_b64), (c) => c.charCodeAt(0));
          return new File([bytes], f.name, { type: f.mime_type });
        });
        chatInputRef.current?.addFiles(files);
      });
      await emit('chat_ready', {});
    })();
    return () => { unlisten?.(); };
  }, []);

  useEffect(() => {
    api.getIdentity().then(id => setAgentName(id.first_name)).catch(() => {});
    // Enable media capture (getUserMedia) on this webview window.
    // WKWebView doesn't expose navigator.mediaDevices by default.
    if ('__TAURI_INTERNALS__' in window) {
      import('@tauri-apps/api/core').then(({ invoke }) => {
        invoke('enable_media_capture_cmd').catch(() => {});
      });
    }
  }, []);

  // Connect on mount
  useEffect(() => {
    if (connectedRef.current) return;
    connectedRef.current = true;
    (async () => {
      // Adopt the session the main window handed over (see createChatWindow).
      // Without this the pop-out would ensureSession() a NEW one and replay the
      // greeting over the dock conversation the user was mid-way through.
      const handedOff = new URLSearchParams(location.search).get('session');
      if (handedOff) {
        useCommandCenter.setState({ chatSessionId: handedOff });
        try { localStorage.setItem('permagent-chat-session-id', handedOff); } catch { /* */ }
      }
      const sid = await ensureSession();
      if (sid) {
        await loadSessionMessages(sid);
        connectSession(sid);
      }
    })();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  return (
    <div style={{
      width: '100vw', height: '100vh',
      background: gradient.shell,
      display: 'flex', flexDirection: 'column',
      fontFamily: font.body,
      // The chat window must NEVER side-scroll: wide content (code blocks,
      // long tokens) scrolls inside its own block, not the window.
      overflowX: 'hidden',
    }}>
      {/* Toolbar */}
      <div style={{
        height: 36, flexShrink: 0,
        display: 'flex', alignItems: 'center',
        padding: '0 12px', gap: 8,
        borderBottom: `1px solid ${colors.border}`,
      }}>
        {/* NO `overflow: hidden` on this row. It is the positioning ancestor
            of the session dropdown, which opens BELOW the 36px toolbar — so
            clipping the row hides the panel entirely and the sessions list
            becomes unreachable. A long agent name is kept from pushing the
            model picker off the bar by truncating the NAME (below), which is
            the only thing that can grow, rather than by clipping the row. */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, flex: 1, minWidth: 0 }}>
          <Mobius size={16} state="idle" glow={0.6} />

          <SessionPicker />

          <div style={{ flex: 1 }} />

          <ModelPicker />

          <Tooltip content="What your agent sees">
            <Button
              colors={colors}
              variant="bare"
              onClick={() => setInspectionOpen(!inspectionOpen)}
              aria-label="What your agent sees"
              style={{
                // Bare glyph (2026-07-27): the boxed version overflowed the
                // header bar. Open state reads through the icon color alone, and
                // so does hover — no fill, or the box comes back.
                '--pa-btn-fg': inspectionOpen ? colors.cyan : colors.textMuted,
                '--pa-btn-fg-hover': colors.cyan,
                '--pa-btn-bg-hover': 'transparent',
                '--pa-btn-pad': '0',
                '--pa-btn-radius': `${radius.sm}px`,
                width: 22, height: 22, flexShrink: 0,
              } as CSSProperties}
            >
              <FiEye size={12} />
            </Button>
          </Tooltip>
        </div>
      </div>

      {/* Chat body */}
      <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
        <DropZone onDrop={handleDrop}>
          <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
            <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
              <MessageList />
            </div>
            <AwarenessIndicator onOpenInspection={() => setInspectionOpen(true)} />
            <PreTurnPreview visible={inputFocused} />
            <div onFocusCapture={() => setInputFocused(true)} onBlurCapture={() => setInputFocused(false)}>
              <ChatInput ref={chatInputRef} />
            </div>
          </div>
        </DropZone>
      </div>

      {inspectionOpen && <InspectionPanel onClose={() => setInspectionOpen(false)} />}

      {/* MIRROR ONLY — the engine lives in the main window for the app's
          lifetime (see VoiceHost), so opening/closing this window can never
          interrupt the audio. This just renders the live feed. */}
      <ChatWindowVoiceOrb />
    </div>
  );
}

// Mirror orb for the popped-out chat window. The conversation runs in the
// main window's engine; this polls the live feed (state + audio level, ~150ms)
// so the orb appears the instant the window opens and dances with the real
// audio. Clicking it asks the owner to end the conversation.
const noAnalyser = () => null;

function ChatWindowVoiceOrb() {
  const [live, setLive] = useState(() => readLiveConversation());

  useEffect(() => {
    // Poll — WKWebView does not reliably deliver `storage` events between
    // separate webviews, which is what made the earlier design flaky.
    const id = setInterval(() => setLive(readLiveConversation()), 200);
    return () => clearInterval(id);
  }, []);

  if (!live) return null;
  return (
    <div style={{ position: 'fixed', inset: 0, zIndex: 90 }}>
      <VoiceOrb
        state={live.state}
        getPlaybackAnalyser={noAnalyser}
        getMicAnalyser={noAnalyser}
        mirrorLevel={live.level}
        onExit={requestVoiceEnd}
        teachWord={live.teachWord}
      />
    </div>
  );
}
