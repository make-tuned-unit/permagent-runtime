// Time of day — the rotunda keeps the hours of the person it serves.
//
// A REAL signal (the local clock — honest by construction, claims no agent
// state): the warm key light, cool fill, ambient floor, starfield, lanterns
// and the void nebula all drift through dawn / day / dusk / night with the
// user's actual local time. Values stay inside the bible §1 light formula
// (one warm key + one cool fill + near-black ambient; the §1 daytime values
// ARE the 'day' keyframe) — night cools and dims the key rather than adding
// lights, so the census and the single shadow caster are untouched.
//
// PURE CORE (unit-tested): sampleTimeOfDay(hours) interpolates a cyclic
// keyframe table with smoothstep easing — continuous across the whole 24h
// cycle including the midnight wrap. The store samples the wall clock once
// every 30s (never per frame); consumers damp toward the sampled targets in
// their own useFrame loops (zero allocation — plain numbers + hex strings).
//
// reduceMotion note: a lighting drift measured in HOURS is not motion; the
// rig stays active under reduceMotion (the per-frame damp only eases the
// 30s-cadence retargets, it does not animate).

import { getTheme, onThemeChange } from '../../../styles/tokens';

export type DayPhase = 'night' | 'dawn' | 'day' | 'dusk';

export interface TimeOfDayState {
  /** Input hours [0,24) echoed back. */
  hours: number;
  phase: DayPhase;
  /** Warm key light (the single shadow caster) — color + intensity. */
  keyColor: string;
  keyIntensity: number;
  /** Cool fill directional intensity (color fixed — §1 formula). */
  fillIntensity: number;
  /** Near-black ambient intensity. */
  ambientIntensity: number;
  /** Starfield opacity multiplier [0,1] (stars pale toward midday). */
  starOpacity: number;
  /** Colonnade-lantern emissive intensity (bronze fixtures glow at night). */
  lanternGlow: number;
  /** Void-nebula band opacity (night-weighted). */
  nebulaOpacity: number;
  /** Firefly activity gate [0,1] (also gated on real grove presence). */
  fireflies: number;
}

interface Keyframe {
  h: number;
  keyColor: string;
  keyIntensity: number;
  fillIntensity: number;
  ambientIntensity: number;
  starOpacity: number;
  lanternGlow: number;
  nebulaOpacity: number;
  fireflies: number;
}

// The cycle. 'day' (9h–17h) is EXACTLY the bible §1 baseline the scene shipped
// with (#FFF0D4 @ 1.6, fill 0.25, ambient 0.08) so midday is visually the
// established look; the other frames bend around it.
const FRAMES: Keyframe[] = [
  // deep night — cold moonlit key, lanterns carry the warmth
  { h: 0.0,  keyColor: '#C9D8EE', keyIntensity: 0.95, fillIntensity: 0.34, ambientIntensity: 0.055, starOpacity: 1.0,  lanternGlow: 1.5,  nebulaOpacity: 1.0,  fireflies: 1.0 },
  { h: 4.5,  keyColor: '#C9D8EE', keyIntensity: 0.95, fillIntensity: 0.34, ambientIntensity: 0.055, starOpacity: 1.0,  lanternGlow: 1.5,  nebulaOpacity: 1.0,  fireflies: 1.0 },
  // dawn — rose-amber key climbing, stars washing out
  { h: 6.5,  keyColor: '#FFD9AE', keyIntensity: 1.25, fillIntensity: 0.28, ambientIntensity: 0.07,  starOpacity: 0.45, lanternGlow: 0.85, nebulaOpacity: 0.5,  fireflies: 0.35 },
  // full day — the §1 baseline
  { h: 9.0,  keyColor: '#FFF0D4', keyIntensity: 1.6,  fillIntensity: 0.25, ambientIntensity: 0.08,  starOpacity: 0.28, lanternGlow: 0.25, nebulaOpacity: 0.18, fireflies: 0.0 },
  { h: 17.0, keyColor: '#FFF0D4', keyIntensity: 1.6,  fillIntensity: 0.25, ambientIntensity: 0.08,  starOpacity: 0.28, lanternGlow: 0.25, nebulaOpacity: 0.18, fireflies: 0.0 },
  // dusk — the warmest frame, lanterns waking
  { h: 19.5, keyColor: '#FFC89A', keyIntensity: 1.3,  fillIntensity: 0.3,  ambientIntensity: 0.07,  starOpacity: 0.55, lanternGlow: 1.05, nebulaOpacity: 0.55, fireflies: 0.6 },
  // settled night
  { h: 22.0, keyColor: '#C9D8EE', keyIntensity: 0.95, fillIntensity: 0.34, ambientIntensity: 0.055, starOpacity: 1.0,  lanternGlow: 1.5,  nebulaOpacity: 1.0,  fireflies: 1.0 },
];

function smoothstep(t: number): number {
  return t * t * (3 - 2 * t);
}

function hexToRgb(hex: string): [number, number, number] {
  const n = parseInt(hex.slice(1), 16);
  return [(n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff];
}

function rgbToHex(r: number, g: number, b: number): string {
  const c = (v: number) => Math.round(Math.min(255, Math.max(0, v))).toString(16).padStart(2, '0');
  return `#${c(r)}${c(g)}${c(b)}`.toUpperCase();
}

/** Linear hex color mix (pure — no THREE import so tests stay node-light). */
export function mixHex(a: string, b: string, t: number): string {
  const [ar, ag, ab] = hexToRgb(a);
  const [br, bg, bb] = hexToRgb(b);
  return rgbToHex(ar + (br - ar) * t, ag + (bg - ag) * t, ab + (bb - ab) * t);
}

/** Phase bands (for copy/tint decisions, not interpolation). */
export function phaseOf(hours: number): DayPhase {
  const h = ((hours % 24) + 24) % 24;
  if (h < 5.5 || h >= 21) return 'night';
  if (h < 8.5) return 'dawn';
  if (h < 18) return 'day';
  return 'dusk';
}

/**
 * Sample the cyclic keyframe table at `hours` (any real number; wraps mod 24).
 * Smoothstep-eased between neighboring frames; continuous across midnight.
 */
export function sampleTimeOfDay(hours: number): TimeOfDayState {
  const h = ((hours % 24) + 24) % 24;
  // Find the bracketing frames on the cycle.
  let i = FRAMES.length - 1;
  for (let k = 0; k < FRAMES.length; k++) {
    if (FRAMES[k].h <= h) i = k;
  }
  const a = FRAMES[i];
  const b = FRAMES[(i + 1) % FRAMES.length];
  const span = (b.h - a.h + 24) % 24 || 24;
  const t = smoothstep((((h - a.h + 24) % 24) / span) || 0);
  const lerp = (x: number, y: number) => x + (y - x) * t;
  return {
    hours: h,
    phase: phaseOf(h),
    keyColor: mixHex(a.keyColor, b.keyColor, t),
    keyIntensity: lerp(a.keyIntensity, b.keyIntensity),
    fillIntensity: lerp(a.fillIntensity, b.fillIntensity),
    ambientIntensity: lerp(a.ambientIntensity, b.ambientIntensity),
    starOpacity: lerp(a.starOpacity, b.starOpacity),
    lanternGlow: lerp(a.lanternGlow, b.lanternGlow),
    nebulaOpacity: lerp(a.nebulaOpacity, b.nebulaOpacity),
    fireflies: lerp(a.fireflies, b.fireflies),
  };
}

// ── Store: samples the real clock every 30s (never per frame) ───────────────

const SAMPLE_MS = 30_000;

let current: TimeOfDayState = sampleTimeOfDay(12); // neutral until first real sample
let timer: ReturnType<typeof setInterval> | undefined;
let started = false;
/** DEV override (evidence capture): when set, the clock is pinned. */
let overrideHours: number | null = null;

function realHours(): number {
  const d = new Date();
  return d.getHours() + d.getMinutes() / 60 + d.getSeconds() / 3600;
}

function sampleNow(): void {
  current = overrideHours !== null ? sampleTimeOfDay(overrideHours)
    : sampleAppearanceTime(realHours(), getTheme() === 'silver');
}

/** Existing resolved appearance is authoritative, including live OS changes.
 * Keep real dawn/dusk nuance only when it agrees with the selected appearance.
 * No second preference, geolocation, network request or sunrise service.
 */
export function sampleAppearanceTime(hours: number, light: boolean): TimeOfDayState {
  const phase = phaseOf(hours);
  const daylight = phase === 'day' || phase === 'dawn';
  return sampleTimeOfDay(daylight === light ? hours : light ? 12 : 0);
}

/** Shared daylight amount for the open conservatory sky and matching fog. */
export function daylightAmount(state: TimeOfDayState): number {
  return Math.max(0, Math.min(1, (1 - state.starOpacity) / .72));
}

/** Zero-alloc getter for useFrame consumers. */
export function getTimeOfDay(): TimeOfDayState {
  return current;
}

/** Idempotent start; returns a disposer. Mounted by the Atmosphere. */
export function startTimeOfDay(): () => void {
  if (started || typeof window === 'undefined') return () => {};
  started = true;
  sampleNow();
  timer = setInterval(sampleNow, SAMPLE_MS);
  const stopTheme = onThemeChange(sampleNow);
  return () => {
    started = false;
    if (timer) clearInterval(timer);
    stopTheme();
  };
}

// DEV-ONLY evidence harness: pin the world clock for captures (dawn/noon/dusk/
// night screenshots). Drives the SAME store the real clock writes. No-op in prod.
declare global {
  interface Window {
    __worldTime?: { set: (hours: number | null) => void; snapshot: () => TimeOfDayState };
  }
}
if (import.meta.env.DEV && typeof window !== 'undefined') {
  window.__worldTime = {
    set: (hours) => {
      overrideHours = hours;
      sampleNow();
    },
    snapshot: () => current,
  };
}
