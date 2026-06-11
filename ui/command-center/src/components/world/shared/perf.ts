// Shared perf probe — WORLD_VIEW_BIBLE.md §6, §8. FROZEN after Phase 0.
// One measurement method for all lane evidence so numbers are comparable.
// Mount <PerfSampler/> inside the Canvas; read window.__worldPerf or getPerfSnapshot().

import { useRef } from 'react';
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

  // Priority 1000: runs after the scene renders so gl.info reflects a full frame.
  useFrame(({ clock }) => {
    frames.current += 1;
    const t = clock.elapsedTime;
    if (t - last.current < 1) return;
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
  }, 1000);

  return null;
}
