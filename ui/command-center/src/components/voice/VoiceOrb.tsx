import { useEffect, useMemo, useRef } from 'react';
import { useTheme } from '../../styles/useTheme';
import { font, ease } from '../../styles/tokens';

/**
 * VoiceOrb — the full-window conversation-mode takeover.
 *
 * A pseudo-3D particle sphere (after the "abstract sphere with flowing
 * particles" reference): ~700 glowing dots on a noise-deformed rotating
 * sphere, wrapped in a scattered twinkling halo shell, lit blue→cyan on one
 * side flowing to violet→magenta on the other. Audio drives it live:
 *   listening (ready/recording) → mic analyser ripples the surface
 *   thinking  (processing)      → calm autonomous swell
 *   speaking  (playing)         → TTS analyser surges deformation + brightness
 * Electric static: per-point jitter scaled by level + random halo sparks.
 *
 * Purely visual — turn-taking stays with the VAD in useVoice; clicking
 * anywhere exits hands-free and returns to the normal chat.
 */

const N_SPHERE = 700;
const N_HALO = 240;
const SIZE = 360; // css px, square canvas

interface Pt {
  x: number; y: number; z: number; // unit sphere dir
  seed: number;
}

function makePoints(): { sphere: Pt[]; halo: Array<Pt & { r: number; tw: number }> } {
  // Fibonacci sphere — even coverage, no pole clustering.
  const sphere: Pt[] = [];
  const golden = Math.PI * (3 - Math.sqrt(5));
  for (let i = 0; i < N_SPHERE; i++) {
    const y = 1 - (i / (N_SPHERE - 1)) * 2;
    const rad = Math.sqrt(Math.max(0, 1 - y * y));
    const th = golden * i;
    sphere.push({ x: Math.cos(th) * rad, y, z: Math.sin(th) * rad, seed: (i * 2654435761) % 1000 / 1000 });
  }
  // Halo: a loose shell of drifting dust around the body.
  const halo: Array<Pt & { r: number; tw: number }> = [];
  let s = 42;
  const rnd = () => { s = (s * 16807) % 2147483647; return s / 2147483647; };
  for (let i = 0; i < N_HALO; i++) {
    const u = rnd() * 2 - 1;
    const th = rnd() * Math.PI * 2;
    const rad = Math.sqrt(Math.max(0, 1 - u * u));
    halo.push({
      x: Math.cos(th) * rad, y: u, z: Math.sin(th) * rad,
      seed: rnd(), r: 1.14 + rnd() * 0.42, tw: 1.5 + rnd() * 3.5,
    });
  }
  return { sphere, halo };
}

/** Lerp through the reference palette: cyan → electric blue → violet → magenta. */
function palette(g: number): [number, number, number] {
  const stops: Array<[number, number, number]> = [
    [0, 213, 255],   // cyan
    [64, 120, 255],  // electric blue
    [141, 68, 174],  // violet (brand)
    [255, 79, 216],  // magenta
  ];
  const t = Math.min(0.9999, Math.max(0, g)) * (stops.length - 1);
  const i = Math.floor(t);
  const f = t - i;
  const a = stops[i], b = stops[i + 1];
  return [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f];
}

export function VoiceOrb({
  state,
  getPlaybackAnalyser,
  getMicAnalyser,
  mirrorLevel,
  onExit,
}: {
  state: string;
  getPlaybackAnalyser: () => AnalyserNode | null;
  getMicAnalyser: () => AnalyserNode | null;
  /** Mirror mode (popped-out window): audio level relayed from the owning
   *  window, since the analysers live in that window's audio graph. */
  mirrorLevel?: number;
  onExit: () => void;
}) {
  const { colors } = useTheme();
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const rafRef = useRef<number | null>(null);
  const levelRef = useRef(0);
  const stateRef = useRef(state);
  stateRef.current = state;
  const mirrorRef = useRef(mirrorLevel);
  mirrorRef.current = mirrorLevel;

  const points = useMemo(makePoints, []);

  const speaking = state === 'playing';
  const thinking = state === 'processing' || state === 'connecting';
  const listening = state === 'recording';

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const dpr = Math.min(2, window.devicePixelRatio || 1);
    canvas.width = SIZE * dpr;
    canvas.height = SIZE * dpr;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    ctx.scale(dpr, dpr);

    const reduce =
      typeof window !== 'undefined' &&
      window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;

    const data = new Uint8Array(32); // frequencyBinCount for fftSize 64
    let phase = 0;
    let rotY = 0;
    let noiseT = 0; // accumulated surface-churn time — speeds up with speech
    let lastT = 0;
    // Per-band smoothed levels: lows drive the swell, mids the churn/spin,
    // highs the shimmer — so the orb dances WITH the speech, not just louder.
    let low = 0, mid = 0, high = 0;
    const cx = SIZE / 2;
    const cy = SIZE / 2;
    const R = SIZE * 0.30;

    const frame = (t: number) => {
      const s = stateRef.current;
      const dt = lastT ? Math.min(0.1, (t - lastT) / 1000) : 0.016;
      lastT = t;

      // ── Audio level, split into bands ──
      let tLow = 0, tMid = 0, tHigh = 0;
      const analyser = s === 'playing' ? getPlaybackAnalyser() : getMicAnalyser();
      const mirror = mirrorRef.current;
      if (typeof mirror === 'number' && !analyser) {
        // Mirror mode: one relayed level, spread across the bands with
        // synthetic detail so the orb still shimmers and churns convincingly.
        tLow = mirror;
        tMid = mirror * (0.7 + 0.3 * Math.sin(t * 0.011));
        tHigh = mirror * (0.5 + 0.5 * Math.sin(t * 0.019 + 1.7));
      } else if (s === 'processing' || s === 'connecting' || !analyser) {
        phase += 0.016;
        tLow = 0.10 + 0.06 * (0.5 + 0.5 * Math.sin(phase));
        tMid = tLow * 0.6;
        tHigh = tLow * 0.3;
      } else {
        analyser.getByteFrequencyData(data);
        const band = (a: number, b: number) => {
          let sum = 0;
          for (let i = a; i < b; i++) sum += data[i];
          return sum / (b - a) / 255;
        };
        tLow = band(0, 7) * 1.25;
        tMid = band(7, 15) * 1.5;
        tHigh = band(15, 26) * 1.9;
      }
      // Fast attack, slow release — syllables land visibly, decay is graceful.
      const smooth = (cur: number, target: number) =>
        cur + (target - cur) * (target > cur ? 0.5 : 0.08);
      low = smooth(low, tLow);
      mid = smooth(mid, tMid);
      high = smooth(high, tHigh);
      const level = Math.min(1.2, low * 0.5 + mid * 0.35 + high * 0.15);
      levelRef.current = level;

      rotY += dt * (0.2 + mid * 0.9); // mids spin it up
      noiseT += dt * (0.9 + mid * 4.5 + high * 2.0); // speech churns the surface
      const rotX = 0.42 + 0.06 * Math.sin(t * 0.00013);
      const cosY = Math.cos(rotY), sinY = Math.sin(rotY);
      const cosX = Math.cos(rotX), sinX = Math.sin(rotX);
      const amp = 0.045 + low * 0.34; // lows swell the whole body
      const tt = noiseT;

      // ── Backdrop glow ──
      //
      // Two constraints, both learned from the halo reading as a clipped grey
      // box on the light theme:
      //
      // 1. The gradient must reach zero alpha BEFORE the canvas edge. At the
      //    old outer radius (R * 1.9 = 0.57 * SIZE) the glow was still opaque
      //    where the square canvas cut it off, leaving straight vertical edges
      //    with transparent corners. SIZE * 0.5 lands exactly on the nearest
      //    edge, so the falloff completes inside the bitmap.
      // 2. It must fade to its OWN hue at zero alpha, never to `rgba(0,0,0,0)`.
      //    Canvas interpolates gradient stops per-channel, so fading toward
      //    transparent BLACK drags RGB down as alpha drops — a grey wash that
      //    is invisible on the dark themes and obvious over near-white.
      ctx.clearRect(0, 0, SIZE, SIZE);
      const glow = ctx.createRadialGradient(cx, cy, R * 0.2, cx, cy, SIZE * 0.5);
      glow.addColorStop(0, `rgba(64, 120, 255, ${0.10 + level * 0.14})`);
      glow.addColorStop(0.55, `rgba(141, 68, 174, ${0.05 + level * 0.08})`);
      glow.addColorStop(1, 'rgba(141, 68, 174, 0)');
      ctx.fillStyle = glow;
      ctx.fillRect(0, 0, SIZE, SIZE);

      // ── Halo dust (behind) then sphere points ──
      for (const p of points.halo) {
        // Slow orbital drift + twinkle; occasional electric spark pops bright.
        const a = tt * 0.12 * (0.5 + p.seed);
        const ca = Math.cos(a), sa = Math.sin(a);
        let x = p.x * ca - p.z * sa;
        let z = p.x * sa + p.z * ca;
        const y2 = p.y * cosX - z * sinX;
        z = p.y * sinX + z * cosX;
        const px = cx + x * R * p.r;
        const py = cy + y2 * R * p.r;
        const twinkle = 0.25 + 0.5 * (0.5 + 0.5 * Math.sin(tt * p.tw + p.seed * 40));
        const spark = Math.random() < 0.003 + level * 0.01 ? 0.9 : 0;
        const g = (px / SIZE) * 0.6 + (py / SIZE) * 0.4;
        const [r, gg, b] = palette(g);
        ctx.fillStyle = `rgba(${r | 0}, ${gg | 0}, ${b | 0}, ${Math.min(1, twinkle + spark)})`;
        const sz = spark ? 2.4 : 1.3;
        ctx.fillRect(px, py, sz, sz);
      }

      for (const p of points.sphere) {
        // Organic noise: three drifting harmonics along the unit dirs.
        const d =
          Math.sin(4.1 * p.x + tt * 0.9 + p.seed) * 0.5 +
          Math.sin(3.4 * p.y - tt * 0.7) * 0.3 +
          Math.sin(5.2 * p.z + tt * 1.2) * 0.2;
        // Electric static: per-point charge jitter scaled by the live level.
        const jitter = (Math.random() - 0.5) * level * 0.05;
        const rr = 1 + d * amp + jitter;

        let x = p.x * cosY - p.z * sinY;
        let z = p.x * sinY + p.z * cosY;
        const y2 = p.y * cosX - z * sinX;
        z = p.y * sinX + z * cosX;

        const px = cx + x * rr * R;
        const py = cy + y2 * rr * R;
        const depth = (z + 1) / 2; // 0 back … 1 front
        const g = (px / SIZE) * 0.62 + (py / SIZE) * 0.38;
        const [r, gg, b] = palette(g);
        // Highs shimmer: per-point brightness ripples race across the surface
        // while consonants land; silence leaves a calm, even glow.
        const shimmer = high * 0.5 * (0.5 + 0.5 * Math.sin(tt * 6 + p.seed * 40));
        const alpha = Math.min(1, 0.14 + depth * (0.6 + level * 0.35) + shimmer);
        ctx.fillStyle = `rgba(${r | 0}, ${gg | 0}, ${b | 0}, ${alpha})`;
        const sz = 0.9 + depth * (1.3 + level * 1.2) + shimmer * 1.1;
        ctx.fillRect(px - sz / 2, py - sz / 2, sz, sz);
      }

      if (!reduce) rafRef.current = requestAnimationFrame(frame);
    };

    if (reduce) {
      frame(0); // one static render — the label still communicates state
      return;
    }
    rafRef.current = requestAnimationFrame(frame);
    return () => {
      if (rafRef.current != null) cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
    };
  }, [getPlaybackAnalyser, getMicAnalyser, points]);

  const stateColor = speaking
    ? colors.cyan
    : thinking
      ? colors.purple
      : listening
        ? colors.danger
        : colors.cyan;

  // Be honest about the states that are NOT listening. This defaulted every
  // unrecognised state to "Listening", so a dropped socket ('idle') or a failed
  // one ('error') looked exactly like a live, armed mic — the agent appeared to
  // be listening and simply never answering, with nothing on screen to say
  // otherwise. A stalled conversation must be visibly stalled.
  const label = speaking
    ? 'Speaking'
    : thinking
      ? 'Thinking…'
      : state === 'error'
        ? 'Voice error — click to exit'
        : state === 'idle'
          ? 'Reconnecting…'
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
        gap: 12,
        cursor: 'pointer',
        background: `radial-gradient(ellipse at center, ${colors.surfaceHi} 0%, ${colors.bg} 75%)`,
      }}
    >
      <canvas
        ref={canvasRef}
        aria-hidden
        style={{ width: SIZE, height: SIZE, maxWidth: '90%', maxHeight: '60%' }}
      />

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
