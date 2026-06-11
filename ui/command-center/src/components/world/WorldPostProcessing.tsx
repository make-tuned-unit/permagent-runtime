import { useSyncExternalStore } from 'react';
import { Bloom, EffectComposer, Noise } from '@react-three/postprocessing';
import { BlendFunction, KernelSize } from 'postprocessing';

// W4-owned (bible §7/§8). Bloom is the FIRST cut if the frame budget fails;
// Noise is cheap and stays. The dev-only toggles below exist so perf evidence
// can compare post on / bloom off / post off at runtime without rebuilding.

interface PostFlags {
  enabled: boolean;
  bloom: boolean;
}

let flags: PostFlags = { enabled: true, bloom: true };
const listeners = new Set<() => void>();

function setFlags(next: Partial<PostFlags>): void {
  flags = { ...flags, ...next };
  listeners.forEach((l) => l());
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

// DEV-ONLY measurement harness (bible §8 item 5: measure with/without Bloom).
declare global {
  interface Window {
    __worldPost?: {
      setBloom: (on: boolean) => void;
      setEnabled: (on: boolean) => void;
    };
  }
}
if (import.meta.env.DEV && typeof window !== 'undefined') {
  window.__worldPost = {
    setBloom: (on: boolean) => setFlags({ bloom: on }),
    setEnabled: (on: boolean) => setFlags({ enabled: on }),
  };
}

export function WorldPostProcessing() {
  const { enabled, bloom } = useSyncExternalStore(subscribe, () => flags);

  if (!enabled) return null;

  // multisampling=0: the canvas runs antialias:false (bible §8 item 1) and the
  // composer's default MSAA(8) would silently reintroduce the same fill-rate
  // cost in its offscreen buffers.
  if (!bloom) {
    return (
      <EffectComposer multisampling={0}>
        <Noise premultiply blendFunction={BlendFunction.ADD} opacity={0.12} />
      </EffectComposer>
    );
  }

  return (
    <EffectComposer multisampling={0}>
      <Bloom
        intensity={0.8}
        luminanceThreshold={0.4}
        luminanceSmoothing={0.4}
        kernelSize={KernelSize.LARGE}
      />
      <Noise premultiply blendFunction={BlendFunction.ADD} opacity={0.12} />
    </EffectComposer>
  );
}
