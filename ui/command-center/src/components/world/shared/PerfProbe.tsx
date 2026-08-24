// DEV-ONLY measurement harness (research note THREEJS_WORLD_2026-08-24 §4 M1).
//
// Why this exists: every recorded World perf number came from headless
// Chrome/ANGLE. The shipped app runs WKWebView, and WKWebView exposes no
// remote-debug hook we can read `window.__worldPerf` through. So the probe
// publishes its rolling numbers into `document.title`, which AppleScript can
// read from Safari with no extra permissions — that is the whole trick.
//
// Not mounted unless the URL carries `?perf=1`, and stripped from production
// builds by the `import.meta.env.DEV` guard at the mount site.
//
// It deliberately does NOT read `gl.info` itself: <PerfSampler/> already owns
// that (and resets it every frame). This reads PerfSampler's published snapshot
// and adds what the snapshot lacks — a frame-time distribution, which is the
// number that actually tells you whether a scene is smooth.
import { useRef } from 'react';
import { useFrame, useThree } from '@react-three/fiber';
import { getPerfSnapshot } from './perf';

export interface PerfSample {
  t: number;
  fps: number;
  p50: number;
  p95: number;
  calls: number;
  triangles: number;
  geometries: number;
  dpr: number;
  gpuMs: number | null;
}

declare global {
  interface Window {
    __worldPerfLog?: PerfSample[];
    __worldPerfCsv?: () => string;
  }
}

const RING = 240; // frame-time samples kept per second-window (plenty at 30-120fps)

function percentile(sorted: number[], p: number): number {
  if (sorted.length === 0) return 0;
  const i = Math.min(sorted.length - 1, Math.floor((p / 100) * sorted.length));
  return sorted[i];
}

export function perfProbeEnabled(): boolean {
  if (typeof window === 'undefined') return false;
  return new URLSearchParams(window.location.search).get('perf') === '1';
}

// Fill-rate headroom, measured honestly on a vsync-capped display.
//
// This machine's 5K display runs at 30.00 Hz, so "we hit 30 fps" says only
// that we hit the ceiling — it cannot tell you whether there is 1ms of
// headroom or 15ms. Raising the pixel count until the frame falls off vsync
// can. `?dpr=2.5` fixes the device pixel ratio so a sweep is reproducible;
// the shipped app never reads it (DEV-only mount site, and the param is
// absent in normal use).
export function devDprOverride(): number | null {
  if (typeof window === 'undefined') return null;
  const raw = new URLSearchParams(window.location.search).get('dpr');
  if (!raw) return null;
  const n = Number(raw);
  return Number.isFinite(n) && n > 0 && n <= 4 ? n : null;
}

export function PerfProbe() {
  const gl = useThree((s) => s.gl);
  const deltas = useRef<number[]>([]);
  const windowStart = useRef(0);
  const sortBuf = useRef<number[]>([]);
  const gpuTimer = useRef<{ ext: unknown; supported: boolean } | null>(null);

  useFrame(({ clock }, delta) => {
    if (gpuTimer.current === null) {
      (window as unknown as { __worldGl?: unknown }).__worldGl = gl;
      // WebKit has historically not exposed EXT_disjoint_timer_query_webgl2.
      // Record the fact rather than guessing at GPU time.
      const ctx = gl.getContext() as WebGL2RenderingContext;
      const ext = ctx.getExtension('EXT_disjoint_timer_query_webgl2');
      gpuTimer.current = { ext, supported: !!ext };
    }

    const d = deltas.current;
    if (d.length < RING) d.push(delta * 1000);

    const t = clock.elapsedTime;
    if (t < windowStart.current) {
      // Same clock-resnap trap PerfSampler documents: r3f restarts the clock
      // when the frameloop leaves 'never'.
      windowStart.current = t;
      d.length = 0;
      return;
    }
    if (t - windowStart.current < 1) return;

    const snap = getPerfSnapshot();
    const s = sortBuf.current;
    s.length = 0;
    for (let i = 0; i < d.length; i++) s.push(d[i]);
    s.sort((a, b) => a - b);

    const sample: PerfSample = {
      t: Math.round(t),
      fps: snap?.fps ?? 0,
      p50: Math.round(percentile(s, 50) * 10) / 10,
      p95: Math.round(percentile(s, 95) * 10) / 10,
      calls: snap?.calls ?? 0,
      triangles: snap?.triangles ?? 0,
      geometries: snap?.geometries ?? 0,
      dpr: snap?.dpr ?? 0,
      gpuMs: null, // see gpuTimer probe above; null means "not measurable here"
    };

    const log = (window.__worldPerfLog ??= []);
    log.push(sample);
    if (log.length > 900) log.shift(); // 15 minutes is more than enough

    // The readable-from-AppleScript channel.
    document.title =
      `PERF t=${sample.t} fps=${sample.fps} p50=${sample.p50} p95=${sample.p95} ` +
      `dc=${sample.calls} tri=${sample.triangles} geo=${sample.geometries} dpr=${sample.dpr}`;

    d.length = 0;
    windowStart.current = t;
  });

  if (typeof window !== 'undefined' && !window.__worldPerfCsv) {
    window.__worldPerfCsv = () =>
      ['t,fps,p50,p95,calls,triangles,geometries,dpr']
        .concat((window.__worldPerfLog ?? []).map((r) =>
          [r.t, r.fps, r.p50, r.p95, r.calls, r.triangles, r.geometries, r.dpr].join(','),
        ))
        .join('\n');
  }

  return null;
}
