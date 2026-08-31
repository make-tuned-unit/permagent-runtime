import { useEffect, useRef } from 'react';
import { useTheme } from '../../styles/useTheme';
import { radius } from '../../styles/tokens';

const BAR_COUNT = 7;
const MIN_SCALE = 0.16; // resting height so bars never fully collapse

/**
 * VoiceVisualizer — a live frequency waveform drawn while the agent speaks.
 *
 * Reads byte-frequency data off the TTS playback analyser (from useVoice) on
 * each animation frame and drives the height of a small row of cyan→violet
 * bars. It renders nothing when inactive, tears the rAF loop down when the
 * agent stops, and honors reduced-motion (a static gentle row, no loop).
 *
 * Purely visual — the bars mirror the audio, they don't touch it.
 */
export function VoiceVisualizer({
  getAnalyser,
  active,
}: {
  getAnalyser: () => AnalyserNode | null;
  active: boolean;
}) {
  const { colors } = useTheme();
  const barsRef = useRef<Array<HTMLDivElement | null>>([]);
  const rafRef = useRef<number | null>(null);

  useEffect(() => {
    const setAll = (scale: number) => {
      for (const el of barsRef.current) {
        if (el) el.style.transform = `scaleY(${scale})`;
      }
    };

    if (!active) {
      setAll(MIN_SCALE);
      return;
    }

    const reduce =
      typeof window !== 'undefined' &&
      window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
    if (reduce) {
      // No animation loop — a calm, static "speaking" row.
      setAll(0.45);
      return;
    }

    const data = new Uint8Array(32); // frequencyBinCount for fftSize 64
    const tick = () => {
      const analyser = getAnalyser();
      if (analyser) {
        analyser.getByteFrequencyData(data);
        for (let i = 0; i < BAR_COUNT; i++) {
          // Spread bars across the lower ~60% of bins, where voice energy sits.
          const bin = 1 + Math.floor((i / BAR_COUNT) * data.length * 0.6);
          const v = data[bin] / 255; // 0..1
          const el = barsRef.current[i];
          if (el) el.style.transform = `scaleY(${MIN_SCALE + v * (1 - MIN_SCALE)})`;
        }
      }
      rafRef.current = requestAnimationFrame(tick);
    };
    rafRef.current = requestAnimationFrame(tick);

    return () => {
      if (rafRef.current != null) cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
    };
  }, [active, getAnalyser]);

  if (!active) return null;

  return (
    <div
      aria-hidden
      style={{ display: 'flex', alignItems: 'center', gap: 2.5, height: 16 }}
    >
      {Array.from({ length: BAR_COUNT }).map((_, i) => (
        <div
          key={i}
          ref={(el) => {
            barsRef.current[i] = el;
          }}
          style={{
            width: 2.5,
            height: 16,
            borderRadius: radius.pill,
            transformOrigin: 'center',
            transform: `scaleY(${MIN_SCALE})`,
            background: `linear-gradient(to top, ${colors.cyan}, ${colors.purple})`,
            willChange: 'transform',
          }}
        />
      ))}
    </div>
  );
}
