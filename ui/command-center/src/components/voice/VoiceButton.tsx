/**
 * VoiceButton — minimal push-to-talk UI for Phase 1.
 *
 * Hold the button (or global hotkey Cmd+Shift+V) to record.
 * Release to send audio for transcription + reply + TTS playback.
 * Shows voice state and errors clearly. Backend-agnostic.
 */
import { useEffect, useRef, useCallback } from 'react';
import { useVoice, VoiceState } from '../../hooks/useVoice';
import { useTheme } from '../../styles/useTheme';
import { useCommandCenter } from '../../lib/store';

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

const STATE_LABELS: Record<VoiceState, string> = {
  idle: 'Voice off',
  connecting: 'Connecting...',
  ready: 'Hold to talk',
  recording: 'Listening...',
  processing: 'Thinking...',
  playing: 'Speaking...',
  error: 'Error',
  unavailable: 'Voice unavailable',
};

const STATE_COLORS: Record<VoiceState, string> = {
  idle: '#666',
  connecting: '#F5A623',
  ready: '#00BFEF',
  recording: '#FF4444',
  processing: '#F5A623',
  playing: '#00FF88',
  error: '#FF4444',
  unavailable: '#666',
};

export function VoiceButton() {
  const { colors } = useTheme();
  const chatSessionId = useCommandCenter(s => s.chatSessionId);
  const {
    state,
    lastTranscript,
    lastReply,
    error,
    connect,
    disconnect,
    startRecording,
    stopRecording,
  } = useVoice({ sessionId: chatSessionId ?? undefined });

  const isRecording = useRef(false);

  // Register global shortcut (Cmd+Shift+V on macOS)
  useEffect(() => {
    if (!isTauri) return;
    let cleanup: (() => void) | undefined;

    (async () => {
      try {
        const { register, unregister } = await import('@tauri-apps/plugin-global-shortcut');

        await register('CommandOrControl+Shift+V', (event) => {
          if (event.state === 'Pressed' && !isRecording.current) {
            isRecording.current = true;
            startRecording();
          } else if (event.state === 'Released' && isRecording.current) {
            isRecording.current = false;
            stopRecording();
          }
        });

        cleanup = () => {
          unregister('CommandOrControl+Shift+V').catch(() => {});
        };
      } catch (err) {
        console.warn('Global shortcut registration failed:', err);
      }
    })();

    return () => cleanup?.();
  }, [startRecording, stopRecording]);

  const handleToggle = useCallback(() => {
    if (state === 'idle' || state === 'error') {
      connect();
    } else if (state === 'ready' || state === 'playing') {
      disconnect();
    }
  }, [state, connect, disconnect]);

  const handlePointerDown = useCallback(() => {
    if (state === 'ready') {
      isRecording.current = true;
      startRecording();
    }
  }, [state, startRecording]);

  const handlePointerUp = useCallback(() => {
    if (isRecording.current) {
      isRecording.current = false;
      stopRecording();
    }
  }, [stopRecording]);

  const isActive = state !== 'idle' && state !== 'unavailable';
  const stateColor = STATE_COLORS[state];

  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 4, minWidth: 0 }}>
      <button
        onClick={handleToggle}
        onPointerDown={state === 'ready' ? handlePointerDown : undefined}
        onPointerUp={state === 'recording' ? handlePointerUp : undefined}
        onPointerLeave={state === 'recording' ? handlePointerUp : undefined}
        title={
          isActive
            ? state === 'ready' ? 'Hold to talk (or Cmd+Shift+V)' : STATE_LABELS[state]
            : 'Enable voice (Cmd+Shift+V)'
        }
        style={{
          width: 22,
          height: 22,
          flexShrink: 0,
          borderRadius: '50%',
          border: `1.5px solid ${stateColor}`,
          background: state === 'recording'
            ? 'rgba(255, 68, 68, 0.2)'
            : isActive
              ? `${stateColor}22`
              : 'transparent',
          cursor: 'pointer',
          display: 'grid',
          placeItems: 'center',
          padding: 0,
          transition: 'all 150ms ease-out',
        }}
      >
        <svg
          width="11"
          height="11"
          viewBox="0 0 24 24"
          fill="none"
          stroke={stateColor}
          strokeWidth={2.2}
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <path d="M12 1a3 3 0 0 0-3 3v8a3 3 0 0 0 6 0V4a3 3 0 0 0-3-3z" />
          <path d="M19 10v2a7 7 0 0 1-14 0v-2" />
          <line x1="12" y1="19" x2="12" y2="23" />
          <line x1="8" y1="23" x2="16" y2="23" />
        </svg>
      </button>

      {/* State label — hidden at very narrow widths via overflow */}
      {(error || state === 'recording' || state === 'processing' || state === 'playing') && (
        <span style={{
          fontSize: 10,
          color: error ? '#FF4444' : colors.textDim,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
          maxWidth: 120,
          minWidth: 0,
          flexShrink: 1,
        }}>
          {error || (lastTranscript && state === 'playing' ? lastReply || STATE_LABELS[state] : STATE_LABELS[state])}
        </span>
      )}
    </div>
  );
}
