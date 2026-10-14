// VoiceHost — THE voice engine singleton, main window only.
//
// The engine (mic, VAD, playback graph, WebSocket) lives here for the app's
// whole lifetime. The main window always exists, so a conversation can never
// be interrupted by chat-surface churn: dock open/close, pop-out, redock —
// audio just keeps flowing. The popped-out chat window renders a MIRROR orb
// from the polled live feed (lib/voiceHandoff) and sends start/end commands
// back; it never runs an engine of its own.
//
// Publishes: `voiceEngine` (controls for VoiceButton), `voiceConversation`
// (the orb takeover contract), and the localStorage live feed (state + audio
// level at ~150ms) that mirror orbs dance to.

import { useEffect, useRef, useState } from 'react';
import { useVoice } from '../../hooks/useVoice';
import { useCommandCenter } from '../../lib/store';
import { resolveVoiceHomeSession } from '../../lib/voiceHomeSession';
import {
  clearLiveConversation,
  consumeVoiceEnd,
  consumeVoiceInterrupt,
  consumeVoiceStart,
  publishLiveConversation,
} from '../../lib/voiceHandoff';

export function VoiceHost() {
  // Voice speaks into ONE dedicated, persistent, non-coding conversation
  // (decision D1) — never into whatever chat session the dock happens to be on.
  const [voiceSessionId, setVoiceSessionId] = useState<string | null>(null);
  const setVoiceEngine = useCommandCenter(s => s.setVoiceEngine);
  const setVoiceConversation = useCommandCenter(s => s.setVoiceConversation);
  const chatWindowOpen = useCommandCenter(s => s.chatWindowOpen);

  const {
    state,
    error,
    activate,
    deactivate,
    startRecording,
    stopRecording,
    interrupt,
    getAnalyser,
    getMicAnalyser,
    handsFree,
    setHandsFree,
    wakeWord,
  } = useVoice({ sessionId: voiceSessionId ?? undefined });

  // Resolve the current chat session at mount, retrying until it lands. Until
  // it does the engine is NOT published: a WebSocket opened without a session
  // id would let the daemon pick one, which is the bug this binding removes.
  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const attempt = () => {
      resolveVoiceHomeSession().then(
        id => {
          if (cancelled) return;
          setVoiceSessionId(id);
          // First sight of the current chat this launch: the dock adopts it too.
          if (!useCommandCenter.getState().currentChatSessionId) {
            useCommandCenter.getState().applyCurrentChatSession(id);
          }
        },
        () => { if (!cancelled) timer = setTimeout(attempt, 5000); },
      );
    };
    attempt();
    return () => { cancelled = true; if (timer) clearTimeout(timer); };
  }, []);

  // Follow the suite's current chat live. A voice turn in flight is never
  // re-bound mid-sentence (useVoice reconnects on a session change): the new
  // id is applied once the engine is between turns, and the dock follows after
  // any streaming reply ends too.
  const currentChatSessionId = useCommandCenter(s => s.currentChatSessionId);
  const pendingFollow = useCommandCenter(s => s.pendingFollowSessionId);
  const isStreaming = useCommandCenter(s => s.isStreaming);
  const voiceTurnBusy = state === 'recording' || state === 'processing' || state === 'playing';
  useEffect(() => {
    if (!currentChatSessionId || currentChatSessionId === voiceSessionId) return;
    if (voiceSessionId && voiceTurnBusy) return;
    setVoiceSessionId(currentChatSessionId);
  }, [currentChatSessionId, voiceSessionId, voiceTurnBusy]);
  useEffect(() => {
    if (!pendingFollow || voiceTurnBusy || isStreaming) return;
    const { chatSessionId, switchToSession } = useCommandCenter.getState();
    useCommandCenter.setState({ pendingFollowSessionId: null });
    if (chatSessionId !== pendingFollow) void switchToSession(pendingFollow);
  }, [pendingFollow, voiceTurnBusy, isStreaming]);

  useEffect(() => {
    if (!voiceSessionId) {
      setVoiceEngine(null);
      return;
    }
    setVoiceEngine({
      state,
      error,
      handsFree,
      activate,
      deactivate,
      startRecording,
      stopRecording,
      interrupt,
      getAnalyser,
      getMicAnalyser,
      setHandsFree,
      wakeWord,
    });
    return () => setVoiceEngine(null);
  }, [
    voiceSessionId, state, error, handsFree, wakeWord,
    activate, deactivate, startRecording, stopRecording, interrupt,
    getAnalyser, getMicAnalyser, setHandsFree, setVoiceEngine,
  ]);

  const handsFreeRef = useRef(handsFree);
  handsFreeRef.current = handsFree;
  const stateRef = useRef(state);
  stateRef.current = state;
  const voiceSessionRef = useRef(voiceSessionId);
  voiceSessionRef.current = voiceSessionId;

  // ── Live feed for mirror orbs (~150ms heartbeat while hands-free) ──
  useEffect(() => {
    if (!handsFree) {
      clearLiveConversation();
      return;
    }
    const data = new Uint8Array(32);
    let smoothed = 0;
    const id = setInterval(() => {
      const s = stateRef.current;
      const analyser = s === 'playing' ? getAnalyser() : getMicAnalyser();
      let level = 0;
      if (analyser) {
        analyser.getByteFrequencyData(data);
        let sum = 0;
        const n = Math.max(1, Math.floor(data.length * 0.6));
        for (let i = 0; i < n; i++) sum += data[i];
        level = (sum / n / 255) * 1.2;
      }
      smoothed = smoothed + (level - smoothed) * (level > smoothed ? 0.6 : 0.25);
      publishLiveConversation(s, smoothed);
    }, 150);
    return () => {
      clearInterval(id);
      clearLiveConversation();
    };
  }, [handsFree, getAnalyser, getMicAnalyser]);

  // ── Polled commands from mirror surfaces ──
  // (Polling, not `storage` events — WKWebView doesn't reliably deliver those
  // across webviews, which is what broke the old handoff design.)
  useEffect(() => {
    const id = setInterval(() => {
      if (consumeVoiceEnd() && handsFreeRef.current) {
        void setHandsFree(false);
        deactivate();
      }
      if (voiceSessionRef.current && consumeVoiceStart() && !handsFreeRef.current) {
        void setHandsFree(true);
      }
      if (consumeVoiceInterrupt()) interrupt();
    }, 400);
    return () => clearInterval(id);
  }, [setHandsFree, deactivate, interrupt]);

  // ── Closing the chat window brings the conversation home to the sidebar ──
  const prevChatWindowOpen = useRef(chatWindowOpen);
  useEffect(() => {
    const was = prevChatWindowOpen.current;
    prevChatWindowOpen.current = chatWindowOpen;
    if (was && !chatWindowOpen && handsFreeRef.current) {
      // The false-flip can be SPURIOUS: ChatLauncher's existence check races
      // window creation right after a pop-out, so confirm the window is
      // really gone before bringing the conversation home — acting on the
      // first flip re-opened the dock behind a live chat window (two
      // surfaces, and the dock X then killed the window's voice).
      void (async () => {
        try {
          const { WebviewWindow } = await import('@tauri-apps/api/webviewWindow');
          if (await WebviewWindow.getByLabel('chat')) return; // still alive
        } catch { /* non-tauri: no window to race */ }
        if (!handsFreeRef.current) return; // conversation ended meanwhile
        // setState directly, NOT openChatDock(): that helper focuses an
        // existing chat window when `chatWindowOpen` is still true, and
        // racing this very transition it would re-show the window the user
        // just closed — which is why closing took two clicks.
        useCommandCenter.setState({ chatDockOpen: true });
      })();
    }
  }, [chatWindowOpen]);

  // Conversation-mode takeover: while hands-free is on, publish the live voice
  // state + analyser taps for the orb. Exiting must actually STOP LISTENING:
  // full deactivate releases the mic and socket (macOS mic indicator off).
  useEffect(() => {
    if (!handsFree) {
      setVoiceConversation(null);
      return;
    }
    setVoiceConversation({
      state,
      getPlaybackAnalyser: getAnalyser,
      getMicAnalyser,
      exit: () => {
        void setHandsFree(false);
        deactivate();
      },
      wakeHint:
        wakeWord.active && wakeWord.gated && wakeWord.phrase
          ? `Say "${wakeWord.phrase}"`
          : null,
    });
    return () => setVoiceConversation(null);
  }, [handsFree, state, getAnalyser, getMicAnalyser, setHandsFree, deactivate, setVoiceConversation, wakeWord]);

  return null;
}
