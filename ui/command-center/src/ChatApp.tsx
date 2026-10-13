/**
 * ChatApp — standalone chat view rendered in the separate chat window.
 * Loaded when the URL contains ?view=chat.
 * All state flows through the daemon (HTTP/SSE), not the main window.
 */
import { useEffect, useRef, useState, useCallback, type CSSProperties } from 'react';
import { FiArrowLeft, FiEye } from 'react-icons/fi';
import { font, radius, space } from './styles/tokens';
import { useTheme } from './styles/useTheme';
import { Button } from './components/common/Button';
import { Tooltip } from './components/common/Tooltip';
import { Mobius } from './components/mobius/Mobius';
import { useLiveSessionSync } from './lib/liveSessionSync';
import { useCommandCenter } from './lib/store';
import {
  clearPageBackgroundForVibrancy, NATIVE_WINDOW_IS_OPAQUE, TITLEBAR_HEIGHT,
} from './lib/windowChrome';
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
import { ChatSetupCard } from './components/chat/ChatSetupCard';
import { chatSetupState, modelSetupReadiness } from './components/chat/chatSetupState';
import { CHAT_DRAFT_FINAL, CHAT_DRAFT_FINAL_ACK, CHAT_DRAFT_HANDOFF, CHAT_DRAFT_HANDOFF_ACK, decodeDraftFiles, encodeDraftFiles, type DraftHandoff } from './lib/chatDraftHandoff';
import { CHAT_SETUP_INTENT_EVENT, lastSeenChatSetupIntentId, peekChatSetupIntentRecord, receiveChatSetupIntentChange, type ChatSetupIntentChange } from './lib/wizardIntent';
// VoiceButton moved to ChatInput row (beside send button)

export default function ChatApp() {
  const { gradient, colors, theme } = useTheme();
  const setAgentName = useCommandCenter(s => s.setAgentName);
  const providers = useCommandCenter(s => s.providers);
  const providersError = useCommandCenter(s => s.providersError);
  const chatResolvedRoute = useCommandCenter(s => s.chatResolvedRoute);
  const chatSessionRouteEvidence = useCommandCenter(s => s.chatSessionRouteEvidence);
  const loadProviders = useCommandCenter(s => s.loadProviders);
  const configRev = useCommandCenter(s => s.configRev);
  const chatMessages = useCommandCenter(s => s.chatMessages);
  const chatHistoryLoaded = useCommandCenter(s => s.chatHistoryLoaded);
  const isStreaming = useCommandCenter(s => s.isStreaming);
  const sessionLoadError = useCommandCenter(s => s.sessionLoadError);
  const daemonAuthError = useCommandCenter(s => s.daemonAuthError);
  const [providerCheckKnown, setProviderCheckKnown] = useState(false);
  const [inspectionOpen, setInspectionOpen] = useState(false);
  const [inputFocused, setInputFocused] = useState(false);
  const [returning, setReturning] = useState(false);
  const [returnError, setReturnError] = useState<string | null>(null);
  const returningRef = useRef(false);

  useEffect(() => {
    let active = true;
    setProviderCheckKnown(false);
    void loadProviders().then(() => { if (active) setProviderCheckKnown(true); });
    return () => { active = false; };
  }, [loadProviders, configRev]);

  const modelReadiness = modelSetupReadiness({
    providers: providers ?? [], providerCheckKnown, providersError: !!providersError,
    chatRoute: chatResolvedRoute, sessionRouteEvidence: chatSessionRouteEvidence,
    hasConversation: (chatMessages?.length ?? 0) > 0,
  });
  const setupState = chatSetupState({
    modelReadiness, historyLoaded: !!chatHistoryLoaded,
    messageCount: chatMessages?.length ?? 0, streaming: !!isStreaming,
    sessionLoadError: sessionLoadError ?? null, daemonAuthError: daemonAuthError ?? null,
    voiceActive: !!readLiveConversation(),
  });

  // Same as the main window: clear the inline page paint so the material shows.
  useEffect(() => clearPageBackgroundForVibrancy(), []);

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

  useLiveSessionSync();
  const chatInputRef = useRef<ChatInputHandle>(null);
  const connectedRef = useRef(false);

  const handleDrop = useCallback((files: File[]) => {
    chatInputRef.current?.addFiles(files);
  }, []);

  const returnToDock = useCallback(async () => {
    if (returningRef.current) return;
    const draft = chatInputRef.current?.freezeDraft();
    if (!draft) {
      setReturnError('Wait for your draft to finish restoring, then try again.');
      return;
    }
    returningRef.current = true;
    setReturning(true);
    setReturnError(null);
    const id = globalThis.crypto?.randomUUID?.() ?? `${Date.now()}-${Math.random()}`;
    const deadline = Date.now() + 8000;
    let unack: (() => void) | undefined;
    const bounded = <T,>(work: Promise<T>): Promise<T> => new Promise((resolve, reject) => {
      const remaining = deadline - Date.now();
      if (remaining <= 0) { reject(new Error('Draft transfer timed out')); return; }
      const timer = setTimeout(() => reject(new Error('Draft transfer timed out')), remaining);
      work.then(value => { clearTimeout(timer); resolve(value); }, error => { clearTimeout(timer); reject(error); });
    });
    try {
      const files = await bounded(encodeDraftFiles(draft.files));
      const { emitTo, listen } = await bounded(import('@tauri-apps/api/event'));
      let resolveAck: (() => void) | undefined;
      let rejectAck: ((error: Error) => void) | undefined;
      const ack = new Promise<void>((resolve, reject) => { resolveAck = resolve; rejectAck = reject; });
      unack = await bounded(listen<{ id: string; ok: boolean }>(CHAT_DRAFT_FINAL_ACK, event => {
        if (event.payload?.id !== id) return;
        if (event.payload.ok) resolveAck?.();
        else rejectAck?.(new Error('The main window could not save your draft'));
      }));
      await bounded(emitTo('main', CHAT_DRAFT_FINAL, {
        id, source: draft.source, revision: draft.revision, text: draft.text, files,
        setup: peekChatSetupIntentRecord(), lastSeenSetupId: lastSeenChatSetupIntentId(),
      }));
      await bounded(ack);
      const { getCurrentWindow } = await bounded(import('@tauri-apps/api/window'));
      await bounded(getCurrentWindow().close());
    } catch {
      chatInputRef.current?.releaseDraft();
      setReturnError('Your draft is still here. Could not confirm its transfer to the main window. Try Return to dock again.');
    } finally {
      unack?.();
      returningRef.current = false;
      setReturning(false);
    }
  }, []);

  // Listen for cross-window file drops and signal readiness
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let unlistenDraft: (() => void) | undefined;
    let unlistenSetup: (() => void) | undefined;
    let disposed = false;
    (async () => {
      const { emit, emitTo, listen } = await import('@tauri-apps/api/event');
      if (disposed) return;
      unlisten = await listen<{ files: { name: string; mime_type: string; data_b64: string }[] }>('chat_drop_files', (event) => {
        const files = event.payload.files.map((f) => {
          const bytes = Uint8Array.from(atob(f.data_b64), (c) => c.charCodeAt(0));
          return new File([bytes], f.name, { type: f.mime_type });
        });
        chatInputRef.current?.addFiles(files);
      });
      if (disposed) { unlisten(); return; }
      unlistenSetup = await listen<ChatSetupIntentChange>(CHAT_SETUP_INTENT_EVENT, event => {
        receiveChatSetupIntentChange(event.payload);
      });
      if (disposed) { unlistenSetup(); return; }
      const handoffId = new URLSearchParams(window.location.search).get('draft_handoff');
      if (handoffId) {
        unlistenDraft = await listen<DraftHandoff>(CHAT_DRAFT_HANDOFF, (event) => {
          if (event.payload.id !== handoffId) return;
          try {
            if (!chatInputRef.current) return;
            chatInputRef.current.restoreDraft(event.payload.text, decodeDraftFiles(event.payload.files), event.payload.pendingSetup);
            void emitTo('main', CHAT_DRAFT_HANDOFF_ACK, { id: handoffId });
          } catch { /* sender keeps the original draft and times out safely */ }
        });
        if (disposed) { unlistenDraft(); return; }
      }
      if (!disposed) await emit('chat_ready', { draft_handoff: handoffId });
    })().catch(() => {});
    return () => { disposed = true; unlisten?.(); unlistenDraft?.(); unlistenSetup?.(); };
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
      // Transparent over the native material, the same treatment the main
      // window gets; the message bubbles and the composer carry their own
      // fills, so the panel reads as glass rather than as a flat sheet.
      background: NATIVE_WINDOW_IS_OPAQUE ? gradient.shell : 'transparent',
      display: 'flex', flexDirection: 'column',
      fontFamily: font.body,
      // The chat window must NEVER side-scroll: wide content (code blocks,
      // long tokens) scrolls inside its own block, not the window.
      overflowX: 'hidden',
    }}>
      {/* The window wears an overlay titlebar (OVERLAY_TITLEBAR in
          chatWindow.ts), so the traffic lights sit over our own HTML: the
          toolbar starts below them and the band above is the drag region. */}
      {!NATIVE_WINDOW_IS_OPAQUE && (
        <div data-tauri-drag-region style={{ height: TITLEBAR_HEIGHT, flexShrink: 0 }} />
      )}
      {/* Toolbar */}
      <div style={{
        height: 36, flexShrink: 0,
        display: 'flex', alignItems: 'center',
        padding: '0 12px', gap: space.md,
        borderBottom: `1px solid ${colors.border}`,
      }}>
        {/* NO `overflow: hidden` on this row. It is the positioning ancestor
            of the session dropdown, which opens BELOW the 36px toolbar — so
            clipping the row hides the panel entirely and the sessions list
            becomes unreachable. A long agent name is kept from pushing the
            model picker off the bar by truncating the NAME (below), which is
            the only thing that can grow, rather than by clipping the row. */}
        <div style={{ display: 'flex', alignItems: 'center', gap: space.md, flex: 1, minWidth: 0 }}>
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
          <Button colors={colors} variant="bare" type="button" disabled={returning}
            onClick={() => { void returnToDock(); }} aria-label="Return to dock"
            style={{ display: 'inline-flex', alignItems: 'center', gap: space.xs, whiteSpace: 'nowrap' }}>
            <FiArrowLeft size={13} /> Return to dock
          </Button>
        </div>
      </div>

      {/* Chat body */}
      <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
        {returning && <div role="status" style={{ padding: space.sm, color: colors.textMuted }}>Saving your unsent draft and attachments…</div>}
        {returnError && <div role="alert" style={{ padding: space.sm, color: colors.warning }}>
          {returnError} <Button colors={colors} variant="bare" type="button" onClick={() => { void returnToDock(); }}>Retry</Button>
        </div>}
        <DropZone onDrop={handleDrop}>
          <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
            <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
              {setupState ? <ChatSetupCard state={setupState} onRetry={() => {
                setProviderCheckKnown(false);
                void loadProviders().then(() => setProviderCheckKnown(true));
              }} /> : <MessageList />}
            </div>
            <AwarenessIndicator onOpenInspection={() => setInspectionOpen(true)} />
            <PreTurnPreview visible={inputFocused} />
            <div onFocusCapture={() => setInputFocused(true)} onBlurCapture={() => setInputFocused(false)}>
              <ChatInput ref={chatInputRef} modelReadiness={modelReadiness} />
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
