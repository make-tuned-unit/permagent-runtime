/**
 * @vitest-environment jsdom
 *
 * The orb's state word when the engine is in a fault.
 *
 * User report (2026-09-18): voice "isn't responding and just gets stuck on
 * listening". useVoice's mic watchdog now parks a deaf hands-free input in
 * 'error' with "Not hearing the microphone — check the input device"; this
 * checks the takeover actually SAYS that (read from the engine VoiceHost
 * publishes to the store) instead of a generic "Voice error", and that the
 * fault outranks the wake hint. And the other half of the report: once the
 * spotter has ignored repeated speech, the orb says the wake word is not being
 * heard and what to check. The gate stays armed, so it never claims bare
 * speech works.
 *
 * jsdom has no canvas: the sphere never draws here. This proves the label
 * text and its wiring to the store, not the look.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

import { VoiceOrb, orbLabel } from './VoiceOrb';
import { useCommandCenter } from '../../lib/store';
import { MIC_FAULT_MESSAGE, WAKE_DEAF_HINT, publishVoiceVerdict } from '../../hooks/useVoice';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

describe('orbLabel', () => {
  it('shows the engine diagnosis in error instead of a generic word', () => {
    expect(orbLabel({ state: 'error', errorText: MIC_FAULT_MESSAGE })).toBe(MIC_FAULT_MESSAGE);
    expect(orbLabel({ state: 'error', errorText: MIC_FAULT_MESSAGE, wakeHint: 'Say "Hey Henry"' }))
      .toBe(MIC_FAULT_MESSAGE);
  });

  it('falls back to the generic error word without a diagnosis (mirror window)', () => {
    expect(orbLabel({ state: 'error' })).toBe('Voice error — click to exit');
  });

  it('keeps the live states unchanged', () => {
    expect(orbLabel({ state: 'ready' })).toBe('Listening');
    expect(orbLabel({ state: 'ready', wakeHint: 'Say "Hey Henry"' })).toBe('Say "Hey Henry"');
    expect(orbLabel({ state: 'playing' })).toBe('Speaking');
    expect(orbLabel({ state: 'processing' })).toBe('Thinking…');
    expect(orbLabel({ state: 'idle' })).toBe('Reconnecting…');
  });
});

describe('orbLabel — deaf wake spotter', () => {
  it('says the wake word is not being heard, and never that bare speech works', () => {
    expect(orbLabel({ state: 'ready', spotterDeaf: true })).toBe(WAKE_DEAF_HINT);
    expect(WAKE_DEAF_HINT).not.toMatch(/just talk/i);
  });

  it('replaces the still-armed gate hint, but only in the resting state and never over a fault', () => {
    expect(orbLabel({ state: 'ready', spotterDeaf: true, wakeHint: 'Say "Hey Henry"' })).toBe(WAKE_DEAF_HINT);
    expect(orbLabel({ state: 'recording', spotterDeaf: true })).toBe('Listening');
    expect(orbLabel({ state: 'error', spotterDeaf: true, errorText: MIC_FAULT_MESSAGE })).toBe(MIC_FAULT_MESSAGE);
  });
});

describe('VoiceOrb — fault label from the engine', () => {
  let container: HTMLDivElement;
  let root: Root;
  const noAnalyser = () => null;

  beforeEach(() => {
    // jsdom logs "not implemented" for canvas; the orb already bails on a null context.
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null);
    vi.stubGlobal('requestAnimationFrame', () => 0);
    vi.stubGlobal('cancelAnimationFrame', () => {});
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
    useCommandCenter.setState({ voiceEngine: null });
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('renders the mic-fault sentence the engine published, not "Listening"', () => {
    useCommandCenter.setState({
      voiceEngine: {
        state: 'error',
        error: MIC_FAULT_MESSAGE,
        handsFree: true,
        activate: () => {},
        deactivate: () => {},
        startRecording: () => {},
        stopRecording: () => {},
        interrupt: () => {},
        getAnalyser: noAnalyser,
        getMicAnalyser: noAnalyser,
        setHandsFree: () => {},
        wakeWord: { enabled: true, setEnabled: () => {}, active: true, phrase: 'Hey Henry', gated: true },
      },
    });
    act(() => root.render(
      <VoiceOrb
        state="error"
        getPlaybackAnalyser={noAnalyser}
        getMicAnalyser={noAnalyser}
        onExit={() => {}}
        wakeHint={'Say "Hey Henry"'}
      />,
    ));
    // Literal on purpose: the sentence the user reads is the contract.
    expect(container.textContent).toContain('Not hearing the microphone — check the input device');
    expect(container.textContent).not.toContain('Voice error');
    expect(container.textContent).not.toContain('Hey Henry');
  });

  it('renders the fail-open hint from the wakeWord object the engine published', () => {
    // Built as a variable: `spotterDeaf` rides on the same object useVoice
    // returns and VoiceHost publishes, beyond the store's declared shape.
    const wakeWord = {
      enabled: true, setEnabled: () => {}, active: true, phrase: 'Hey Henry', gated: false, spotterDeaf: true,
    };
    useCommandCenter.setState({
      voiceEngine: {
        state: 'ready',
        error: null,
        handsFree: true,
        activate: () => {},
        deactivate: () => {},
        startRecording: () => {},
        stopRecording: () => {},
        interrupt: () => {},
        getAnalyser: noAnalyser,
        getMicAnalyser: noAnalyser,
        setHandsFree: () => {},
        wakeWord,
      },
    });
    // The gate is still armed, so VoiceHost still passes the wake hint.
    act(() => root.render(
      <VoiceOrb state="ready" wakeHint='Say "Hey Henry"' getPlaybackAnalyser={noAnalyser} getMicAnalyser={noAnalyser} onExit={() => {}} />,
    ));
    expect(container.textContent).toContain(WAKE_DEAF_HINT);
  });
  // The popped-out chat window renders a MIRROR orb (ChatApp's
  // ChatWindowVoiceOrb passes state, mirrorLevel and onExit — no
  // wake hint, no error). There is no engine in that window's store, so before
  // the relay it said "Voice error — click to exit" over a dead mic, and plain
  // "Listening" both while wake-gated and over a spotter that looks deaf. These render
  // the orb exactly as that host does and write the relay with the engine's own
  // publisher. They prove the mirror reads the relay and ignores its empty
  // store; the hook tests prove useVoice writes it. Neither proves the two
  // WKWebViews share localStorage — the live feed already depends on that.
  describe('mirror window (no engine in the store) reads the relayed verdict', () => {
    beforeEach(() => {
      vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] });
      localStorage.clear();
    });
    afterEach(() => {
      vi.useRealTimers();
      localStorage.clear();
    });

    /** Render the orb the way ChatApp's mirror does. */
    function renderMirror(state: string) {
      act(() => root.render(
        <VoiceOrb
          state={state}
          getPlaybackAnalyser={noAnalyser}
          getMicAnalyser={noAnalyser}
          mirrorLevel={0}
          onExit={() => {}}
        />,
      ));
    }
    const label = () => container.querySelector('[data-testid="voice-orb-label"]')?.textContent;

    it('shows the wake hint while the gate is armed, not a bare "Listening"', () => {
      expect(useCommandCenter.getState().voiceEngine).toBeNull();
      publishVoiceVerdict({ error: null, wakeHint: 'Say "Hey Henry"', spotterDeaf: false });
      renderMirror('ready');
      expect(label()).toBe('Say "Hey Henry"');
    });

    it('shows the mic-fault sentence instead of the generic error word', () => {
      publishVoiceVerdict({ error: MIC_FAULT_MESSAGE, wakeHint: 'Say "Hey Henry"', spotterDeaf: false });
      renderMirror('error');
      expect(label()).toBe('Not hearing the microphone — check the input device');
    });

    it('shows the deaf hint over the still-armed gate hint', () => {
      publishVoiceVerdict({ error: null, wakeHint: 'Say "Hey Henry"', spotterDeaf: true });
      renderMirror('ready');
      expect(label()).toBe(WAKE_DEAF_HINT);
    });

    it('follows the engine as the verdict changes (polled, like the live feed)', () => {
      publishVoiceVerdict({ error: null, wakeHint: 'Say "Hey Henry"', spotterDeaf: false });
      renderMirror('ready');
      expect(label()).toBe('Say "Hey Henry"');
      publishVoiceVerdict({ error: null, wakeHint: 'Say "Hey Henry"', spotterDeaf: true });
      act(() => { vi.advanceTimersByTime(250); });
      expect(label()).toBe(WAKE_DEAF_HINT);
    });

    it('the main-window orb ignores the relay and reads its own engine', () => {
      publishVoiceVerdict({ error: MIC_FAULT_MESSAGE, wakeHint: null, spotterDeaf: true });
      act(() => root.render(
        <VoiceOrb state="ready" getPlaybackAnalyser={noAnalyser} getMicAnalyser={noAnalyser} onExit={() => {}} />,
      ));
      expect(label()).toBe('Listening');
    });
  });
});
