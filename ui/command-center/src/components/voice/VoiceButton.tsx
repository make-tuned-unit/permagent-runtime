/**
 * VoiceButton — push-to-talk mic button for the chat input row.
 *
 * Triggers: hold button OR hold spacebar (when text input is not focused).
 * Connection: persistent socket, auto-connects on first use.
 * State: idle → click to activate → ready → hold to talk → recording → processing → playing → ready.
 */
import { useEffect, useRef, useState, useCallback, type CSSProperties } from 'react';
import { FiMic } from 'react-icons/fi';
import { VoiceState, isInterruptibleState } from '../../hooks/useVoice';
import { useCommandCenter } from '../../lib/store';
import {
  readLiveConversation,
  requestVoiceEnd,
  requestVoiceInterrupt,
  requestVoiceStart,
} from '../../lib/voiceHandoff';

/** True inside the popped-out chat WebviewWindow (index.html?view=chat). */
const isChatWindow =
  typeof location !== 'undefined' &&
  new URLSearchParams(location.search).get('view') === 'chat';
import { useTheme } from '../../styles/useTheme';
import type { ThemeColors } from '../../styles/useTheme';
import { VoiceVisualizer } from './VoiceVisualizer';
import { handsFreeStatusLabel } from './voiceStatus';
import { radius, space } from '../../styles/tokens';
import { Button } from '../common/Button';
import { Tooltip } from '../common/Tooltip';

/** The mic and its sibling chip are toolbar-dense controls, not prominent
 *  ones, so they stay rounded rectangles rather than capsules (D5). */
const CONTROL = 28;

const STATE_LABELS: Record<VoiceState, string> = {
  idle: '',
  connecting: 'Connecting...',
  ready: 'Hold to talk',
  recording: 'Listening...',
  processing: 'Thinking...',
  playing: 'Speaking...',
  error: 'Error',
};

function stateColorMap(colors: ThemeColors): Record<VoiceState, string> {
  return {
    idle: colors.textDim,
    connecting: colors.warning,
    ready: colors.cyan,
    recording: colors.danger,
    processing: colors.warning,
    playing: colors.success,
    error: colors.danger,
  };
}

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
  const { colors } = useTheme();
  // Pure view over the MAIN window's VoiceHost engine (store slice). In the
  // popped-out chat window there is no local engine — this becomes a remote
  // control: it reads the polled live feed and sends start/end commands, so
  // one conversation runs in one place no matter which surface drives it.
  const engine = useCommandCenter(s => s.voiceEngine);
  const agentName = useCommandCenter(s => s.agentName);
  const [remote, setRemote] = useState(() => (isChatWindow ? readLiveConversation() : null));
  useEffect(() => {
    if (!isChatWindow) return;
    const id = setInterval(() => setRemote(readLiveConversation()), 300);
    return () => clearInterval(id);
  }, []);

  const state = (engine?.state ?? remote?.state ?? 'idle') as VoiceState;
  const error = engine?.error ?? null;
  const handsFree = engine?.handsFree ?? (isChatWindow && remote !== null);
  const gatedWakePhrase =
    engine?.wakeWord.active && engine.wakeWord.gated
      ? engine.wakeWord.phrase
      : null;
  const noop = () => {};
  const activate = engine?.activate ?? (isChatWindow ? requestVoiceStart : noop);
  const deactivate = engine?.deactivate ?? (isChatWindow ? requestVoiceEnd : noop);
  const startRecording = engine?.startRecording ?? noop;
  const stopRecording = engine?.stopRecording ?? noop;
  const interrupt = engine?.interrupt ?? (isChatWindow ? requestVoiceInterrupt : noop);
  const getAnalyser = engine?.getAnalyser ?? (() => null);
  const setHandsFree =
    engine?.setHandsFree ??
    (isChatWindow ? ((on: boolean) => (on ? requestVoiceStart() : requestVoiceEnd())) : noop);

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
      // Barge-in (#398): Space while Henry is thinking/speaking STOPS him and
      // returns the mic to listening, instead of doing nothing.
      if (isInterruptibleState(s)) {
        e.preventDefault();
        interrupt();
        return;
      }
      // Push-to-talk: only start recording from a connected, ready socket.
      // First activation requires clicking the mic button.
      if (s !== 'ready') return;

      e.preventDefault(); // prevent page scroll
      isRecordingRef.current = true;
      startRecording();
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
  }, [activate, startRecording, stopRecording, interrupt]);

  // --- Button handlers ---
  const handleClick = useCallback(() => {
    if (handsFree) {
      // In hands-free the mic button is the OFF switch, whatever the state —
      // previously a click during 'recording' did nothing and a click during
      // playing only interrupted, so there was no way to stop it listening.
      // Leave hands-free AND fully deactivate (release mic + socket).
      void setHandsFree(false);
      deactivate();
    } else if (state === 'idle' || state === 'error') {
      activate();
    } else if (isInterruptibleState(state)) {
      // Barge-in (#398): the mic button doubles as a Stop button while Henry is
      // thinking/speaking — click halts the reply and returns to listening.
      interrupt();
    } else if (state === 'ready') {
      deactivate();
    }
  }, [handsFree, state, activate, deactivate, interrupt, setHandsFree]);

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
  const stateColor = stateColorMap(colors)[state];
  const showLabel = error || state === 'recording' || state === 'processing' || state === 'playing' || state === 'connecting';

  const isBusy = state === 'processing' || state === 'playing';
  // The mic's four resting faces, unchanged — now expressed as `.pa-btn` custom
  // properties so the state that was only reachable through a pair of mouse
  // handlers (the muted glyph coming up to full while idle) is CSS, and the
  // press give arrives with it. `bgHover`/`borderHover` deliberately repeat the
  // resting values: this control already signals its state through its fill, so
  // a hover fill on top of "recording" would be a second, competing signal.
  const face =
    state === 'recording'
      ? { bg: `${colors.danger}33`, fg: colors.danger, border: colors.danger, fgHover: colors.danger }
      : isBusy
        ? { bg: colors.surfaceHi, fg: colors.textMuted, border: colors.border, fgHover: colors.textMuted }
        : isActive
          ? { bg: colors.cyanSoft, fg: colors.cyan, border: `${colors.cyan}80`, fgHover: colors.cyan }
          : { bg: colors.surfaceHi, fg: colors.textMuted, border: colors.border, fgHover: colors.text };

  const micTip =
    state === 'idle' ? 'Enable voice (spacebar to talk)'
    : state === 'ready' ? 'Hold to talk (spacebar)'
    : isInterruptibleState(state) ? `Stop ${agentName} — click or press space to interrupt`
    : STATE_LABELS[state];

  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: space.xs }}>
      <Tooltip content={micTip}>
        <Button
          colors={colors}
          variant="bare"
          onClick={handleClick}
          onPointerDown={state === 'ready' ? handlePointerDown : undefined}
          onPointerUp={state === 'recording' ? handlePointerUp : undefined}
          onPointerLeave={state === 'recording' ? handlePointerUp : undefined}
          aria-label="Voice"
          style={{
            '--pa-btn-bg': face.bg,
            '--pa-btn-fg': face.fg,
            '--pa-btn-border': face.border,
            '--pa-btn-bg-hover': face.bg,
            '--pa-btn-border-hover': face.border,
            '--pa-btn-fg-hover': face.fgHover,
            '--pa-btn-bg-active': face.bg,
            '--pa-btn-pad': '0',
            '--pa-btn-radius': `${radius.sm}px`,
            width: CONTROL,
            height: CONTROL,
            flexShrink: 0,
          } as CSSProperties}
        >
          <FiMic
            size={12}
            color={isActive ? stateColor : 'currentColor'}
            style={{ display: 'block' }}
          />
        </Button>
      </Tooltip>

      {/* While the agent speaks, the waveform IS the label — a live frequency
          visualization instead of static "Speaking..." text. Clicking it
          enters HANDS-FREE (#19): the mic stays open, turns are taken by
          voice-activity detection (silence ends your turn, loud speech barges
          in), no spacebar needed. Click again to leave. */}
      {handsFree ? (
        <Tooltip content="Hands-free conversation is ON — click to stop listening">
          <Button
            colors={colors}
            onClick={() => {
              void setHandsFree(false);
              deactivate();
            }}
            style={{
              '--pa-btn-bg': colors.cyanSoft,
              '--pa-btn-fg': 'inherit',
              '--pa-btn-border': `${colors.cyan}80`,
              '--pa-btn-bg-hover': colors.cyanSoft,
              '--pa-btn-border-hover': colors.cyan,
              '--pa-btn-bg-active': colors.cyanSoft,
              '--pa-btn-pad': `${space.xs}px ${space.md}px`,
              '--pa-btn-radius': `${radius.sm}px`,
              gap: space.sm,
            } as CSSProperties}
          >
            {state === 'playing' ? (
              <VoiceVisualizer getAnalyser={getAnalyser} active />
            ) : (
              <span style={{ fontSize: 10, color: colors.cyan, whiteSpace: 'nowrap' }}>
                {handsFreeStatusLabel(state, gatedWakePhrase)}
              </span>
            )}
          </Button>
        </Tooltip>
      ) : state === 'playing' && !error ? (
        // The waveform IS the control here: it has no padding of its own, so a
        // hover fill would sit flush against the bars and read as a glitch.
        // Press give and focus still arrive from `.pa-btn`.
        <Tooltip content="Click to go hands-free — always listening, talk naturally">
          <Button
            colors={colors}
            variant="bare"
            onClick={() => void setHandsFree(true)}
            aria-label="Go hands-free"
            style={{
              '--pa-btn-bg': 'transparent',
              '--pa-btn-border': 'transparent',
              '--pa-btn-bg-hover': 'transparent',
              '--pa-btn-border-hover': 'transparent',
              '--pa-btn-bg-active': 'transparent',
              '--pa-btn-pad': '0',
            } as CSSProperties}
          >
            <VoiceVisualizer getAnalyser={getAnalyser} active />
          </Button>
        </Tooltip>
      ) : showLabel ? (
        <span style={{
          fontSize: 10,
          color: error ? colors.danger : stateColor,
          whiteSpace: 'nowrap',
          maxWidth: 80,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
        }}>
          {error || STATE_LABELS[state]}
        </span>
      ) : null}
    </div>
  );
}
