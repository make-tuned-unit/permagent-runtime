/**
 * VoiceButton — push-to-talk mic button for the chat input row.
 *
 * Triggers: hold button OR hold spacebar (when text input is not focused).
 * Connection: persistent socket, auto-connects on first use.
 * State: idle → click to activate → ready → hold to talk → recording → processing → playing → ready.
 */
import { useEffect, useRef, useCallback } from 'react';
import { useVoice, VoiceState } from '../../hooks/useVoice';
import { useCommandCenter } from '../../lib/store';

const STATE_LABELS: Record<VoiceState, string> = {
  idle: '',
  connecting: 'Connecting...',
  ready: 'Hold to talk',
  recording: 'Listening...',
  processing: 'Thinking...',
  playing: 'Speaking...',
  error: 'Error',
};

const STATE_COLORS: Record<VoiceState, string> = {
  idle: '#666',
  connecting: '#F5A623',
  ready: '#00BFEF',
  recording: '#FF4444',
  processing: '#F5A623',
  playing: '#00FF88',
  error: '#FF4444',
};

/** Returns true if the currently focused element is a text input (textarea, input, contenteditable). */
function isTextInputFocused(): boolean {
  const el = document.activeElement;
  if (!el) return false;
  const tag = el.tagName.toLowerCase();
  if (tag === 'textarea') return true;
  if (tag === 'input' && (el as HTMLInputElement).type !== 'button') return true;
  if ((el as HTMLElement).isContentEditable) return true;
  return false;
}

export function VoiceButton() {
  const chatSessionId = useCommandCenter(s => s.chatSessionId);
  const {
    state,
    error,
    activate,
    deactivate,
    startRecording,
    stopRecording,
  } = useVoice({ sessionId: chatSessionId ?? undefined });

  const isRecordingRef = useRef(false);
  const stateRef = useRef(state);
  stateRef.current = state;

  // --- Spacebar push-to-talk ---
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.code !== 'Space') return;
      if (e.repeat) return; // ignore key-repeat
      if (isTextInputFocused()) return; // let the user type spaces
      if (isRecordingRef.current) return; // already recording

      const s = stateRef.current;
      // Only trigger on ready or idle (idle will auto-connect)
      if (s !== 'ready' && s !== 'idle') return;

      e.preventDefault(); // prevent page scroll
      isRecordingRef.current = true;

      if (s === 'idle') {
        // First use: activate then record
        activate().then(() => startRecording());
      } else {
        startRecording();
      }
    };

    const handleKeyUp = (e: KeyboardEvent) => {
      if (e.code !== 'Space') return;
      if (!isRecordingRef.current) return;

      e.preventDefault();
      isRecordingRef.current = false;
      stopRecording();
    };

    window.addEventListener('keydown', handleKeyDown);
    window.addEventListener('keyup', handleKeyUp);
    return () => {
      window.removeEventListener('keydown', handleKeyDown);
      window.removeEventListener('keyup', handleKeyUp);
    };
  }, [activate, startRecording, stopRecording]);

  // --- Button handlers ---
  const handleClick = useCallback(() => {
    if (state === 'idle' || state === 'error') {
      activate();
    } else if (state === 'ready' || state === 'playing') {
      deactivate();
    }
  }, [state, activate, deactivate]);

  const handlePointerDown = useCallback(() => {
    if (state === 'ready') {
      isRecordingRef.current = true;
      startRecording();
    }
  }, [state, startRecording]);

  const handlePointerUp = useCallback(() => {
    if (isRecordingRef.current) {
      isRecordingRef.current = false;
      stopRecording();
    }
  }, [stopRecording]);

  const isActive = state !== 'idle';
  const stateColor = STATE_COLORS[state];
  const showLabel = error || state === 'recording' || state === 'processing' || state === 'playing' || state === 'connecting';

  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
      <button
        onClick={handleClick}
        onPointerDown={state === 'ready' ? handlePointerDown : undefined}
        onPointerUp={state === 'recording' ? handlePointerUp : undefined}
        onPointerLeave={state === 'recording' ? handlePointerUp : undefined}
        title={
          state === 'idle' ? 'Enable voice (spacebar to talk)'
          : state === 'ready' ? 'Hold to talk (spacebar)'
          : STATE_LABELS[state]
        }
        className={`rounded-lg border px-2 py-2 transition ${
          state === 'recording'
            ? 'border-red-500 bg-red-500/20 text-red-400'
            : isActive
              ? 'border-accent/50 bg-accent/10 text-accent'
              : 'border-dark-border bg-dark-surface-2 text-dark-muted hover:text-dark-text'
        }`}
        style={{ flexShrink: 0 }}
      >
        <svg
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="none"
          stroke={isActive ? stateColor : 'currentColor'}
          strokeWidth={2}
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <path d="M12 1a3 3 0 0 0-3 3v8a3 3 0 0 0 6 0V4a3 3 0 0 0-3-3z" />
          <path d="M19 10v2a7 7 0 0 1-14 0v-2" />
          <line x1="12" y1="19" x2="12" y2="23" />
          <line x1="8" y1="23" x2="16" y2="23" />
        </svg>
      </button>

      {showLabel && (
        <span style={{
          fontSize: 10,
          color: error ? '#FF4444' : stateColor,
          whiteSpace: 'nowrap',
          maxWidth: 80,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
        }}>
          {error || STATE_LABELS[state]}
        </span>
      )}
    </div>
  );
}
