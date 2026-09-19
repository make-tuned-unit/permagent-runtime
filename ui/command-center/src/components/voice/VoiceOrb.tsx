import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { orbAmp, orbBands, orbMotionFor, orbSpin } from './orbDrive';
import { useTheme } from '../../styles/useTheme';
import type { ThemeColors } from '../../styles/useTheme';
import { duration, ease, font, radius, space, textSize, type } from '../../styles/tokens';
import { useGlass } from '../common/Glass';
import { Tooltip } from '../common/Tooltip';
import { useCommandCenter } from '../../lib/store';
import { WAKE_DEAF_HINT, readVoiceVerdict, type useVoice, type VoiceVerdict } from '../../hooks/useVoice';

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

type RGB = [number, number, number];

/**
 * `#RGB` / `#RRGGBB` -> channels. Canvas is the one place in the app that
 * cannot read a CSS custom property, so the theme has to arrive as numbers.
 */
function channels(hex: string): RGB {
  const s = hex.replace('#', '');
  const full = s.length === 3 ? s[0] + s[0] + s[1] + s[1] + s[2] + s[2] : s;
  return [
    parseInt(full.substring(0, 2), 16),
    parseInt(full.substring(2, 4), 16),
    parseInt(full.substring(4, 6), 16),
  ];
}

const mix = (a: RGB, b: RGB, t: number): RGB =>
  [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];

/**
 * The orb's four gradient stops, DERIVED from the active theme.
 *
 * They used to be four hand-written rgb triplets — cyan `[0,213,255]`, an
 * "electric blue" `[64,120,255]`, the brand violet `[141,68,174]` and a magenta
 * `[255,79,216]` — which meant the orb wore the dark theme's brand colours on
 * every theme, including the pearl one, where a magenta halo over near-white is
 * not the same picture at all. Canvas cannot read a CSS variable, so the fix is
 * not a variable: it is to compute the ramp from `useTheme().colors` in JS and
 * hand the numbers to the gradient.
 *
 * Every stop traces to a token:
 *   0  `colors.cyan`                        — the accent, unchanged
 *   1  45% of the way from cyan to purple   — the old "electric blue" step;
 *                                             mix(#00D5FF,#8D44AE,0.45) lands
 *                                             at #3F82D4, within a few points
 *                                             of the #4078FF that was typed
 *   2  `colors.purple`                      — the brand violet, unchanged
 *   3  `colors.purpleBright` pulled 35%
 *      toward `colors.text`                 — the hot far end. Toward WHITE on
 *                                             the dark themes (as the magenta
 *                                             was), toward graphite on silver,
 *                                             where lighter would mean fainter
 */
export function paletteStops(colors: ThemeColors): RGB[] {
  const cyan = channels(colors.cyan);
  const purple = channels(colors.purple);
  const bright = channels(colors.purpleBright);
  const ink = channels(colors.text);
  return [cyan, mix(cyan, purple, 0.45), purple, mix(bright, ink, 0.35)];
}

/** `[r,g,b]` -> the channel triplet a canvas `rgba(...)` string wants. */
const rgb = (c: RGB): string => `${c[0] | 0}, ${c[1] | 0}, ${c[2] | 0}`;

/** Lerp through the theme-derived ramp: accent → blue → violet → hot end. */
function palette(stops: RGB[], g: number): RGB {
  const t = Math.min(0.9999, Math.max(0, g)) * (stops.length - 1);
  const i = Math.floor(t);
  const f = t - i;
  const a = stops[i], b = stops[i + 1];
  return [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f];
}

/**
 * The orb's state word. Pure, so what the user reads for each state is
 * testable without a canvas.
 *
 * Be honest about the states that are NOT listening. This defaulted every
 * unrecognised state to "Listening", so a dropped socket ('idle') or a failed
 * one ('error') looked exactly like a live, armed mic — the agent appeared to
 * be listening and simply never answering, with nothing on screen to say
 * otherwise. A stalled conversation must be visibly stalled.
 *
 * `errorText` is the engine's own diagnosis. It matters most for the mic
 * watchdog in useVoice: "isn't responding and just gets stuck on listening"
 * (2026-09-18) was a hands-free input that had gone deaf (a Bluetooth profile
 * flip suspended the VAD graph) while the orb kept saying "Listening". The
 * watchdog now parks the engine in 'error' with "Not hearing the microphone —
 * check the input device", and that sentence — not a generic "Voice error" —
 * is what the user needs in order to act. It also outranks the wake hint: a
 * `Say "Hey Henry"` over a dead mic is the same lie in different words.
 *
 * `spotterDeaf` is the other half of that report: a live mic whose wake-word
 * spotter never fired, so the orb asked for "Hey Henry" forever. useVoice now
 * fails the gate open after repeated ignored speech; the resting word must then
 * say that bare speech works, or the user keeps repeating the phrase.
 */
export function orbLabel({
  state,
  teachWord,
  wakeHint,
  errorText,
  spotterDeaf,
}: {
  state: string;
  teachWord?: string | null;
  wakeHint?: string | null;
  errorText?: string | null;
  spotterDeaf?: boolean;
}): string {
  const speaking = state === 'playing';
  const thinking = state === 'processing' || state === 'connecting';
  if (teachWord) return speaking ? 'Placing a word on the Orb' : 'Say the word on the Orb';
  if (speaking) return 'Speaking';
  if (thinking) return 'Thinking…';
  if (state === 'error') return errorText || 'Voice error — click to exit';
  if (state === 'idle') return 'Reconnecting…';
  if (wakeHint) return wakeHint;
  // Only in the resting state: once a turn is open, "Listening" is the truth.
  if (spotterDeaf && state === 'ready') return WAKE_DEAF_HINT;
  return 'Listening';
}

export function VoiceOrb({
  state,
  getPlaybackAnalyser,
  getMicAnalyser,
  mirrorLevel,
  onExit,
  wakeHint,
  teachWord,
  footer,
}: {
  state: string;
  getPlaybackAnalyser: () => AnalyserNode | null;
  getMicAnalyser: () => AnalyserNode | null;
  /** Mirror mode (popped-out window): audio level relayed from the owning
   *  window, since the analysers live in that window's audio graph. */
  mirrorLevel?: number;
  onExit: () => void;
  /** Armed wake gate: what to say to open a turn (e.g. `Say "Hey Henry"`). */
  wakeHint?: string | null;
  /** Word placed on the Orb for a listen-once pronunciation. Never spoken. */
  teachWord?: string | null;
  /**
   * Content that must stay READABLE while the takeover is up — today the
   * pending-decision cards.
   *
   * It is a slot rather than a sibling on purpose. ChatView used to float the
   * decisions in their own `position: absolute; bottom` box at `zIndex: 70`
   * over this one at `zIndex: 60`: two layers with no layout relationship, the
   * upper one transparent. The orb centres its label column in the FULL
   * surface height, so on a normal dock the centred `SAY "HEY HENRY"` and its
   * `click anywhere to end the conversation` caption landed inside the
   * decisions' band and the two texts drew straight through each other
   * (reported 2026-09-18). Rendered here they are flex children of one column:
   * the orb yields its share of the height, and no arithmetic has to agree.
   */
  footer?: ReactNode;
}) {
  const { colors, reduceMotion } = useTheme();
  // Where the engine's verdicts come from depends on the window.
  //
  // Main window: the store VoiceHost publishes to, so the fault reaches the
  // takeover without every host having to thread it through.
  const storeError = useCommandCenter(s => s.voiceEngine?.error ?? null);
  // useVoice publishes `spotterDeaf` on the same wakeWord object VoiceHost
  // hands the store; the store's declared shape predates it, hence the read
  // through useVoice's own type — a rename there fails the type check here
  // instead of silently dropping the verdict.
  const storeSpotterDeaf = useCommandCenter(s => Boolean(
    (s.voiceEngine?.wakeWord as Partial<Pick<ReturnType<typeof useVoice>['wakeWord'], 'spotterDeaf'>> | undefined)
      ?.spotterDeaf,
  ));
  // Mirror mode (`mirrorLevel` set — the popped-out chat window): there is no
  // engine in this window's store, so the store says nothing. Before this the
  // mirror said "Voice error — click to exit" over a dead mic, plain
  // "Listening" over a failed-open gate, and plain "Listening" while the gate
  // was armed (its host never passed a wake hint) — the exact "stuck on
  // listening" of the 2026-09-18 report, in the window it was read in. Read the
  // verdict the engine relays instead (useVoice, VOICE_VERDICT_KEY), polled on
  // the live feed's cadence for the same WKWebView reason.
  const mirror = mirrorLevel !== undefined;
  const [relayed, setRelayed] = useState<VoiceVerdict | null>(() => (mirror ? readVoiceVerdict() : null));
  useEffect(() => {
    if (!mirror) { setRelayed(null); return; }
    const poll = () => setRelayed(prev => {
      const next = readVoiceVerdict();
      if (!prev || !next) return next;
      const same = prev.error === next.error && prev.wakeHint === next.wakeHint
        && prev.spotterDeaf === next.spotterDeaf;
      return same ? prev : next;
    });
    poll();
    const id = setInterval(poll, 200);
    return () => clearInterval(id);
  }, [mirror]);
  const engineError = mirror ? relayed?.error ?? null : storeError;
  const spotterDeaf = mirror ? relayed?.spotterDeaf ?? false : storeSpotterDeaf;
  // A host-passed hint wins (the main window computes its own from the store);
  // the mirror's host has none to pass.
  const shownWakeHint = wakeHint ?? (mirror ? relayed?.wakeHint ?? null : null);
  const wordGlass = useGlass('glass');
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const rafRef = useRef<number | null>(null);
  const levelRef = useRef(0);
  const stateRef = useRef(state);
  stateRef.current = state;

  // The theme's colours as canvas numbers, held in a ref so a theme switch
  // repaints from the next frame WITHOUT tearing down the animation loop —
  // restarting it would reset the band smoothing and the rotation mid-turn.
  const stops = useMemo(() => paletteStops(colors), [colors]);
  const stopsRef = useRef(stops);
  stopsRef.current = stops;

  // Reduced motion DAMPS the orb; it does not stop it. It used to draw a
  // single static frame, and the state — listening vs thinking vs speaking —
  // is carried by this canvas, so the one audience that most needs the state
  // to be legible got the least of it. The damping mirrors how Mobius answers
  // the same setting (Mobius.tsx:74): the AMBIENT motion goes (the autonomous
  // spin, the surface churn, the random sparks, the per-point electric
  // jitter), and the motion that carries meaning — amplitude and brightness
  // following the voice — stays. Calmer, not dead.
  //
  // It also reads the app's own setting rather than `matchMedia` directly, so
  // an explicit choice in Settings wins over the OS the way it does everywhere
  // else (`getReduceMotion`, tokens.ts:722).
  const reduceRef = useRef(reduceMotion);
  reduceRef.current = reduceMotion;
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
      } else if (analyser) {
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
      // State shaping — floors, residuals, and the thinking state's distinct
      // motion — lives in orbDrive.ts so it can be unit-tested. `phase` keeps
      // the synthetic clocks advancing at the frame rate rather than reading
      // wall time, so a paused tab does not jump the animation.
      phase += dt;
      const shaped = orbBands(
        orbMotionFor(s),
        { low: tLow, mid: tMid, high: tHigh },
        phase,
      );
      tLow = shaped.low;
      tMid = shaped.mid;
      tHigh = shaped.high;
      // Fast attack, slow release — syllables land visibly, decay is graceful.
      const smooth = (cur: number, target: number) =>
        cur + (target - cur) * (target > cur ? 0.5 : 0.08);
      low = smooth(low, tLow);
      mid = smooth(mid, tMid);
      high = smooth(high, tHigh);
      const level = Math.min(1.2, low * 0.5 + mid * 0.35 + high * 0.15);
      levelRef.current = level;

      // Reduce Motion scales the AMBIENT terms only. `orbSpin`, `orbAmp` and
      // `orbBands` are untouched — the drive is the same drive, and what it
      // drives is quieter.
      const calm = reduceRef.current;
      const ambient = calm ? 0.25 : 1;

      rotY += dt * orbSpin(mid, s === 'playing') * ambient; // mids spin it up
      noiseT += dt * ((calm ? 0.15 : 0.9) + (mid * 4.5 + high * 2.0) * ambient); // speech churns the surface
      const rotX = 0.42 + 0.06 * Math.sin(t * 0.00013) * ambient;
      const cosY = Math.cos(rotY), sinY = Math.sin(rotY);
      const cosX = Math.cos(rotX), sinX = Math.sin(rotX);
      const amp = orbAmp(low, s === 'playing'); // lows swell the whole body
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
      //
      // The two hues are stops 1 and 2 of the theme-derived ramp — the blue
      // step and the brand violet — so the halo is the same material as the
      // points it surrounds, on every theme.
      const ramp = stopsRef.current;
      const blue = rgb(ramp[1]);
      const violet = rgb(ramp[2]);
      ctx.clearRect(0, 0, SIZE, SIZE);
      const glow = ctx.createRadialGradient(cx, cy, R * 0.2, cx, cy, SIZE * 0.5);
      glow.addColorStop(0, `rgba(${blue}, ${0.10 + level * 0.14})`);
      glow.addColorStop(0.55, `rgba(${violet}, ${0.05 + level * 0.08})`);
      glow.addColorStop(1, `rgba(${violet}, 0)`);
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
        const twinkle = 0.25 + 0.5 * (0.5 + 0.5 * Math.sin(tt * p.tw + p.seed * 40) * ambient);
        // The sparks are the one purely random motion in the picture, and
        // random flicker is exactly what Reduce Motion is for.
        const spark = !calm && Math.random() < 0.003 + level * 0.01 ? 0.9 : 0;
        const g = (px / SIZE) * 0.6 + (py / SIZE) * 0.4;
        ctx.fillStyle = `rgba(${rgb(palette(ramp, g))}, ${Math.min(1, twinkle + spark)})`;
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
        const jitter = calm ? 0 : (Math.random() - 0.5) * level * 0.05;
        const rr = 1 + d * amp + jitter;

        let x = p.x * cosY - p.z * sinY;
        let z = p.x * sinY + p.z * cosY;
        const y2 = p.y * cosX - z * sinX;
        z = p.y * sinX + z * cosX;

        const px = cx + x * rr * R;
        const py = cy + y2 * rr * R;
        const depth = (z + 1) / 2; // 0 back … 1 front
        const g = (px / SIZE) * 0.62 + (py / SIZE) * 0.38;
        // Highs shimmer: per-point brightness ripples race across the surface
        // while consonants land; silence leaves a calm, even glow. The RIPPLE
        // is ambient; the brightness the level buys is not, so under Reduce
        // Motion the surface still lights up with the voice, evenly.
        const shimmer = high * 0.5 * (0.5 + 0.5 * Math.sin(tt * 6 + p.seed * 40) * ambient);
        const alpha = Math.min(1, 0.14 + depth * (0.6 + level * 0.35) + shimmer);
        ctx.fillStyle = `rgba(${rgb(palette(ramp, g))}, ${alpha})`;
        const sz = 0.9 + depth * (1.3 + level * 1.2) + shimmer * 1.1;
        ctx.fillRect(px - sz / 2, py - sz / 2, sz, sz);
      }

      rafRef.current = requestAnimationFrame(frame);
    };

    rafRef.current = requestAnimationFrame(frame);
    return () => {
      if (rafRef.current != null) cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
    };
    // No `state` and no `reduce` in the deps, deliberately. Both are read from
    // refs inside the frame, so a state change, a theme switch or a Reduce
    // Motion toggle changes what the NEXT frame draws instead of tearing the
    // loop down and resetting the band smoothing and the rotation mid-turn.
  }, [getPlaybackAnalyser, getMicAnalyser, points]);

  // A fault is amber, not the accent: in cyan, "Not hearing the microphone"
  // read as one more resting state. Not red either — red on this screen means
  // "recording".
  const stateColor = speaking
    ? colors.cyan
    : thinking
      ? colors.purple
      : listening
        ? colors.danger
        : state === 'error'
          ? colors.warning
          : colors.cyan;

  const label = orbLabel({ state, teachWord, wakeHint: shownWakeHint, errorText: engineError, spotterDeaf });

  return (
    <Tooltip content="Click anywhere to end the conversation">
      <div
        data-testid="voice-orb-takeover"
        onClick={onExit}
        style={{
          position: 'absolute',
          inset: 0,
          zIndex: 60,
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          // The orb block gets the room the footer does not want. `center`
          // here (with the orb block on `flex: 1`) keeps the sphere optically
          // centred in ITS share of the surface instead of the whole of it,
          // which is what let the centred labels sit under floating content.
          justifyContent: 'center',
          cursor: 'pointer',
          background: `radial-gradient(ellipse at center, ${colors.surfaceHi} 0%, ${colors.bg} 75%)`,
        }}
      >
      {/* Orb + state labels: one block, so the footer below can only ever take
          height AWAY from it rather than be drawn across it. */}
      <div style={{
        flex: 1,
        minHeight: 0,
        alignSelf: 'stretch',
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        gap: space.xl,
      }}>
      <div style={{ position: 'relative', width: SIZE, maxWidth: '90%', maxHeight: '60%' }}>
        <canvas
          ref={canvasRef}
          aria-hidden
          style={{ width: '100%', height: 'auto', display: 'block' }}
        />
        {teachWord ? (
          <div
            aria-label={`Say ${teachWord}`}
            style={{
              position: 'absolute',
              inset: 0,
              display: 'flex',
              flexDirection: 'column',
              alignItems: 'center',
              justifyContent: 'center',
              pointerEvents: 'none',
              textAlign: 'center',
              padding: space.huge,
            }}
          >
            {/* The word-pill is the ONE glass plane on this screen. It floats
                over the orb canvas, and a canvas is content — so this is the
                floating control layer sitting on content, which is exactly
                where Apple puts glass (D1), and there is only one of it, so no
                glass sits on glass (D2). It was hand-written as a flat
                `rgba(0,0,0,0.72)` box with `#fff` text: black-on-any-theme,
                which on the pearl theme is a hole punched in the picture.
                `radius.glass` because this is a top-level floating surface. */}
            <div
              style={{
                ...type.display,
                fontFamily: font.display,
                color: colors.text,
                ...wordGlass,
                borderRadius: radius.glass,
                padding: `${space.xl}px ${space.xxl}px`,
              }}
            >
              {teachWord}
            </div>
            <div
              style={{
                marginTop: space.md,
                fontFamily: font.body,
                fontSize: textSize.micro,
                fontWeight: 700,
                letterSpacing: '0.16em',
                textTransform: 'uppercase',
                color: colors.textMuted,
              }}
            >
              Say this
            </div>
          </div>
        ) : null}
      </div>

      <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: space.md }}>
        <span
          data-testid="voice-orb-label"
          role="status"
          style={{
            fontFamily: font.display,
            fontSize: textSize.small,
            fontWeight: 600,
            letterSpacing: '0.1em',
            textTransform: 'uppercase',
            color: stateColor,
            // The state word is the one thing on this screen that must not
            // snap between colours, so it gets the no-overshoot spring at its
            // settle time — 320ms, inside Apple's 500ms ceiling. Reduce Motion
            // takes the cross-fade off; the colour still changes.
            transition: reduceMotion ? 'none' : `color ${duration.smooth}ms ${ease.smooth}`,
          }}
        >
          {label}
        </span>
        <span style={{ fontFamily: font.body, fontSize: textSize.micro, color: colors.textDim }}>
          click anywhere to end the conversation
        </span>
      </div>
      </div>

      {/* Actionable content, in flow beneath the orb block — never floated over
          it. See the `footer` prop for the overlap this replaces. */}
      {footer && (
        <div
          data-testid="voice-orb-footer"
          // The slot's content decides whether it exists (ChatPendingDecisions
          // renders nothing with an empty inbox), so an empty slot must not
          // still reserve its padding and nudge the orb off centre.
          className="empty:hidden"
          style={{
            flexShrink: 0,
            alignSelf: 'stretch',
            // No scroller here — the footer's content owns its own cap, so a
            // pair of nested scroll areas never appears.
            padding: `0 ${space.xl}px ${space.huge}px`,
            // Default the cursor back: the takeover is click-to-exit, but the
            // footer's own controls are not.
            cursor: 'auto',
          }}
        >
          {footer}
        </div>
      )}
    </div>
    </Tooltip>
  );
}
