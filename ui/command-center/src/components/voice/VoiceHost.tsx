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

import { useEffect } from 'react';
import { useVoice } from '../../hooks/useVoice';
import { useCommandCenter } from '../../lib/store';

export function VoiceHost() {
  const chatSessionId = useCommandCenter(s => s.chatSessionId);
  const setVoiceEngine = useCommandCenter(s => s.setVoiceEngine);
  const setVoiceConversation = useCommandCenter(s => s.setVoiceConversation);

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
