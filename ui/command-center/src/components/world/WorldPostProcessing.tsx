import { useEffect, useRef, useSyncExternalStore, type RefObject } from 'react';
import { useThree } from '@react-three/fiber';
import { Bloom, EffectComposer, Noise } from '@react-three/postprocessing';
import { BlendFunction, type EffectComposer as EffectComposerImpl } from 'postprocessing';

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

// AdaptiveResolution (WorldView.tsx) calls r3f's setDpr at runtime to trade
// resolution for frame time. @react-three/postprocessing's <EffectComposer>
// only calls composer.setSize() from an effect keyed on r3f's `size` (the
// CSS box) — never on `viewport.dpr` — even though a dpr change makes r3f
// call gl.setPixelRatio + gl.setSize on the renderer itself. So the composer's
// offscreen render targets (Bloom's mip chain included) are left at whatever
// drawing-buffer resolution was current when EffectComposer last mounted, and
// every later dpr change silently stops reaching the post chain — softness
// after the scene adapts. This resyncs the composer once dpr changes: the
// width/height passed in are the same CSS-size values EffectComposer's own
// effect uses, and composer.setSize() reads the renderer's actual (now
// current) drawing-buffer size to resize its buffers — this call only forces
// that read to happen again.
function ComposerDprSync({ composerRef }: { composerRef: RefObject<EffectComposerImpl | null> }) {
  const dpr = useThree((s) => s.viewport.dpr);
  const size = useThree((s) => s.size);
  useEffect(() => {
    composerRef.current?.setSize(size.width, size.height);
  }, [dpr, size, composerRef]);
  return null;
}

export function WorldPostProcessing() {
  const { enabled, bloom } = useSyncExternalStore(subscribe, () => flags);
  const composerRef = useRef<EffectComposerImpl | null>(null);

  if (!enabled) return null;

  // multisampling=0: the canvas runs antialias:false (bible §8 item 1) and the
  // composer's default MSAA(8) would silently reintroduce the same fill-rate
  // cost in its offscreen buffers.
  if (!bloom) {
    return (
      <>
        <EffectComposer ref={composerRef} multisampling={0}>
          <Noise premultiply blendFunction={BlendFunction.ADD} opacity={0.12} />
        </EffectComposer>
        <ComposerDprSync composerRef={composerRef} />
      </>
    );
  }

  return (
    <>
      <EffectComposer ref={composerRef} multisampling={0}>
        {/* mipmapBlur instead of the old kernelSize={KernelSize.LARGE}.
            KernelSize.LARGE is a wide gaussian convolved at full resolution — a
            lot of texture taps per pixel on a scene that is already fill-rate
            bound. mipmapBlur builds the same halo out of a mip chain instead:
            each level is a quarter of the pixels of the one above it, so the
            whole blur costs a fraction of one full-resolution pass. postprocessing
            marks kernelSize deprecated in favour of exactly this.

            `levels={6}` rather than the default 8 caps how far the glow spreads —
            at 8 the halo reaches across the rotunda and the engraved-circuitry
            read the bible asks for (§1: light is engraved INTO the stone, never
            free-floating) starts to dissolve into a general wash. `radius` and
            `intensity` are tuned to land on the same brightness the old kernel
            gave; the threshold and smoothing are untouched, so exactly the same
            pixels bloom as before. */}
        <Bloom
          mipmapBlur
          intensity={1.0}
          radius={0.7}
          levels={6}
          luminanceThreshold={0.4}
          luminanceSmoothing={0.4}
        />
        <Noise premultiply blendFunction={BlendFunction.ADD} opacity={0.12} />
      </EffectComposer>
      <ComposerDprSync composerRef={composerRef} />
    </>
  );
}
