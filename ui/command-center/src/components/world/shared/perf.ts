// Shared perf probe — WORLD_VIEW_BIBLE.md §6, §8. FROZEN after Phase 0
// (amended 2026-06-10: priority-0 sampling — a positive useFrame priority takes
// over r3f's render loop and blanks any Canvas without a post chain).
// (amended 2026-06-11, W4: clock-reset resnap — r3f resets the clock when the
// frameloop is re-enabled after gating, which stalled the sampler ~15-20s with
// stale reads after every World tab re-show).
// One measurement method for all lane evidence so numbers are comparable.
// Mount <PerfSampler/> inside the Canvas; read window.__worldPerf or getPerfSnapshot().

import { useEffect, useRef } from 'react';
import { useFrame, useThree } from '@react-three/fiber';

export interface PerfSnapshot {
  fps: number;
  calls: number;
  triangles: number;
  geometries: number;
  textures: number;
  programs: number;
  dpr: number;
}

declare global {
  interface Window {
    __worldPerf?: PerfSnapshot;
  }
}

let latest: PerfSnapshot | null = null;
export function getPerfSnapshot(): PerfSnapshot | null {
  return latest;
}

export function PerfSampler() {
  const gl = useThree((s) => s.gl);
  const frames = useRef(0);
  const last = useRef(0);

  // Manual reset gives a stable read point regardless of who drives rendering
  // (auto-render or EffectComposer) and where three places its auto-reset.
  useEffect(() => {
    const prev = gl.info.autoReset;
    gl.info.autoReset = false;
    return () => {
      gl.info.autoReset = prev;
    };
  }, [gl]);

  // Priority 0: observes the loop without taking it over. The callback runs
  // pre-render, so gl.info holds the previous frame's full counts — read, then
  // reset so the next frame accumulates from zero.
  useFrame(({ clock }) => {
    frames.current += 1;
    const t = clock.elapsedTime;
    if (t < last.current) {
      // r3f restarts (resets) the clock whenever the frameloop is re-enabled
      // after 'never' (frameloop gating, bible §8 item 2). Without this
      // resnap the sampler silently reports stale numbers until elapsedTime
      // climbs past the pre-gating read point again (measured: ~15-20s of
      // stale reads after every tab switch back to World). Same
      // measurement-corruption class as the priority-0 amendment.
      last.current = t;
      frames.current = 1;
      gl.info.reset();
      return;
    }
    if (t - last.current >= 1) {
      const info = gl.info;
      latest = {
        fps: Math.round((frames.current / (t - last.current)) * 10) / 10,
        calls: info.render.calls,
        triangles: info.render.triangles,
        geometries: info.memory.geometries,
        textures: info.memory.textures,
        programs: info.programs?.length ?? 0,
        dpr: gl.getPixelRatio(),
      };
      window.__worldPerf = latest;
      frames.current = 0;
      last.current = t;
    }
    gl.info.reset();
  });

  return null;
}
