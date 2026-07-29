// VoiceHost — the per-window voice engine singleton.
//
// useVoice owns real resources (mic stream, WebSocket, playback graph), so it
// must live where surface churn can't reach it: mounted ONCE at the window
// root (App for the main window, ChatApp for the popped-out chat window).
// Closing the dock, detaching chat, or switching views only unmounts the
// VIEWS — the conversation keeps running here, uninterrupted.
//
// Publishes two store slices: `voiceEngine` (state + controls for VoiceButton)
// and `voiceConversation` (the hands-free orb takeover contract).

import { useEffect, useRef } from 'react';
import { useVoice } from '../../hooks/useVoice';
import { useCommandCenter } from '../../lib/store';
import {
  VOICE_HANDOFF_KEY,
  VOICE_END_KEY,
  clearLiveConversation,
  consumeVoiceHandoff,
  publishLiveConversation,
  requestVoiceHandoff,
} from '../../lib/voiceHandoff';

/** True inside the popped-out chat WebviewWindow (index.html?view=chat). */
const isChatWindow =
  typeof location !== 'undefined' &&
  new URLSearchParams(location.search).get('view') === 'chat';

export function VoiceHost() {
  const chatSessionId = useCommandCenter(s => s.chatSessionId);
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
  } = useVoice({ sessionId: chatSessionId ?? undefined });

  useEffect(() => {
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
    });
    return () => setVoiceEngine(null);
  }, [
    state, error, handsFree,
    activate, deactivate, startRecording, stopRecording, interrupt,
    getAnalyser, getMicAnalyser, setHandsFree, setVoiceEngine,
  ]);

  // ── Cross-window handoff ─────────────────────────────────────────────
  // The conversation follows the chat surface. When the chat window opens
  // while a conversation runs HERE (main window), hand it off: end locally,
  // post the ticket; the chat window consumes it and resumes hands-free on
  // the same session. Mirrored on chat-window close via beforeunload.
  const handsFreeRef = useRef(handsFree);
  handsFreeRef.current = handsFree;

  useEffect(() => {
    if (isChatWindow) return;
    // Deferred handoff: never yank the conversation mid-turn. While Henry is
    // thinking/speaking (or the user is mid-utterance) the conversation stays
    // HERE and he finishes his sentence; the moment the turn settles back to
    // 'ready', the mic moves to the chat window. state is a dep, so the
    // playing→ready transition re-fires this.
    if (handsFree && chatWindowOpen && state === 'ready') {
      requestVoiceHandoff('chat');
      void setHandsFree(false);
      deactivate();
    }
  }, [handsFree, chatWindowOpen, state, setHandsFree, deactivate]);

  useEffect(() => {
    const target = isChatWindow ? 'chat' : 'main';
    const tryTake = () => {
      if (consumeVoiceHandoff(target)) void setHandsFree(true);
    };
    tryTake(); // covers a window created AFTER the ticket was posted
    const onStorage = (e: StorageEvent) => {
      if (e.key === VOICE_HANDOFF_KEY) tryTake();
    };
    window.addEventListener('storage', onStorage);
    return () => window.removeEventListener('storage', onStorage);
  }, [setHandsFree]);

  useEffect(() => {
    if (!isChatWindow) return;
    const onUnload = () => {
      if (handsFreeRef.current) requestVoiceHandoff('main');
    };
    window.addEventListener('beforeunload', onUnload);
    return () => window.removeEventListener('beforeunload', onUnload);
  }, []);

  // Live-conversation mirror: while this window OWNS a conversation,
  // heartbeat its state so another window can render the orb in mirror mode
  // (the pop-out opens straight into orb view while audio finishes here).
  useEffect(() => {
    if (!handsFree) {
      clearLiveConversation();
      return;
    }
    publishLiveConversation(state);
    const id = setInterval(() => publishLiveConversation(state), 3000);
    return () => clearInterval(id);
  }, [handsFree, state]);

  // A mirror-orb click in the other window asks the owner to end it.
  useEffect(() => {
    const onStorage = (e: StorageEvent) => {
      if (e.key === VOICE_END_KEY && handsFreeRef.current) {
        void setHandsFree(false);
        deactivate();
      }
    };
    window.addEventListener('storage', onStorage);
    return () => window.removeEventListener('storage', onStorage);
  }, [setHandsFree, deactivate]);

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
    });
    return () => setVoiceConversation(null);
  }, [handsFree, state, getAnalyser, getMicAnalyser, setHandsFree, deactivate, setVoiceConversation]);

  return null;
}
