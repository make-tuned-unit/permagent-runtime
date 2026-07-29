import { useEffect, useRef } from 'react';
import { useTheme } from '../../styles/useTheme';
import { font, ease } from '../../styles/tokens';

/**
 * VoiceOrb — the full-window conversation-mode takeover.
 *
 * While hands-free (#19) is active, this overlay covers the entire chat
 * surface and renders one breathing orb driven by live audio:
 *   listening (ready/recording) → mic analyser, cyan
 *   thinking  (processing)      → autonomous slow pulse, violet
 *   speaking  (playing)         → TTS playback analyser, cyan→violet
 *
 * Purely visual — turn-taking stays with the VAD in useVoice; clicking
 * anywhere exits hands-free and returns to the normal chat.
 */
export function VoiceOrb({
  state,
  getPlaybackAnalyser,
  getMicAnalyser,
  onExit,
}: {
  state: string;
  getPlaybackAnalyser: () => AnalyserNode | null;
  getMicAnalyser: () => AnalyserNode | null;
  onExit: () => void;
}) {
  const { colors } = useTheme();
  const orbRef = useRef<HTMLDivElement>(null);
  const haloRef = useRef<HTMLDivElement>(null);
  const rafRef = useRef<number | null>(null);
  // Smoothed level so the orb glides between frames instead of jittering.
  const levelRef = useRef(0);
  const stateRef = useRef(state);
  stateRef.current = state;

  const speaking = state === 'playing';
  const thinking = state === 'processing' || state === 'connecting';
  const listening = state === 'recording';

  useEffect(() => {
    const reduce =
      typeof window !== 'undefined' &&
      window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
    if (reduce) return; // static orb — the state label still communicates

    const data = new Uint8Array(32); // frequencyBinCount for fftSize 64
    let phase = 0;
    let drift = 0;

    const tick = () => {
      const s = stateRef.current;
      let target = 0;

      const analyser =
        s === 'playing' ? getPlaybackAnalyser() : getMicAnalyser();
      if (s === 'processing' || s === 'connecting' || !analyser) {
        // No live signal — a slow autonomous breath (~6s cycle), barely there.
        phase += 0.016;
        target = 0.12 + 0.06 * (0.5 + 0.5 * Math.sin(phase));
      } else {
        analyser.getByteFrequencyData(data);
        // Voice energy sits in the low bins; average the lower ~60%.
        let sum = 0;
        const n = Math.max(1, Math.floor(data.length * 0.6));
        for (let i = 0; i < n; i++) sum += data[i];
        target = (sum / n / 255) * 1.2;
      }

      // Asymmetric smoothing: rise fast (feels responsive), fall slow (no flicker).
      const prev = levelRef.current;
      const level = prev + (target - prev) * (target > prev ? 0.3 : 0.06);
      levelRef.current = level;

      // Mostly-static presence: small scale swing, and a very slow rotation
      // drift so the asymmetric blob silhouette never reads as a stamped
      // circle. The glow (halo + box-shadow via opacity) carries the audio.
      drift += 0.03;
      if (orbRef.current) {
        orbRef.current.style.transform = `rotate(${drift}deg) scale(${1 + level * 0.1})`;
      }
      if (haloRef.current) {
        haloRef.current.style.transform = `rotate(${-drift * 0.6}deg) scale(${1 + level * 0.22})`;
        haloRef.current.style.opacity = `${0.3 + level * 0.5}`;
      }
      rafRef.current = requestAnimationFrame(tick);
    };
    rafRef.current = requestAnimationFrame(tick);

    return () => {
      if (rafRef.current != null) cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
    };
  }, [getPlaybackAnalyser, getMicAnalyser]);

  const stateColor = speaking
    ? colors.cyan
    : thinking
      ? colors.purple
      : listening
        ? colors.danger
        : colors.cyan;

  const label = speaking
    ? 'Speaking'
    : thinking
      ? 'Thinking…'
      : listening
        ? 'Listening'
        : 'Listening';

  return (
    <div
      onClick={onExit}
      title="Click anywhere to end the conversation"
      style={{
        position: 'absolute',
        inset: 0,
        zIndex: 60,
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        gap: 28,
        cursor: 'pointer',
        background: `radial-gradient(ellipse at center, ${colors.surfaceHi} 0%, ${colors.bg} 75%)`,
      }}
    >
      {/* Orb stack — halo behind, core in front, both audio-driven */}
      <div style={{ position: 'relative', width: 180, height: 180, display: 'grid', placeItems: 'center' }}>
        <div
          ref={haloRef}
          aria-hidden
          style={{
            position: 'absolute',
            inset: 0,
            // Squashed, off-round halo — the imperfection reads organic.
            borderRadius: '58% 42% 55% 45% / 45% 57% 43% 55%',
            background: `radial-gradient(ellipse at 45% 40%, ${stateColor}55 0%, transparent 70%)`,
            filter: 'blur(18px)',
            opacity: 0.3,
            willChange: 'transform, opacity',
            transition: `background 400ms ${ease.out}`,
          }}
        />
        <div
          ref={orbRef}
          aria-hidden
          style={{
            width: 124,
            height: 116,
            // Not a perfect orb: an asymmetric blob silhouette, with the slow
            // rAF rotation drift keeping the irregularity alive.
            borderRadius: '54% 46% 49% 51% / 47% 52% 48% 53%',
            background: `radial-gradient(circle at 38% 32%, ${colors.cyan}, ${colors.purple} 82%)`,
            boxShadow: `0 0 55px ${stateColor}55, inset 0 0 28px rgba(255,255,255,0.10)`,
            willChange: 'transform',
            transition: `box-shadow 400ms ${ease.out}`,
          }}
        />
      </div>

      <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 8 }}>
        <span
          style={{
            fontFamily: font.display,
            fontSize: 13,
            fontWeight: 600,
            letterSpacing: '0.1em',
            textTransform: 'uppercase',
            color: stateColor,
            transition: `color 400ms ${ease.out}`,
          }}
        >
          {label}
        </span>
        <span style={{ fontFamily: font.body, fontSize: 11, color: colors.textDim }}>
          click anywhere to end the conversation
        </span>
      </div>
    </div>
  );
}
