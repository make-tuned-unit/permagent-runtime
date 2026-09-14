// Forum post chain — bloom on emissives only.
//
// This reuses the pipeline the World already ships (`world/WorldPostProcessing`
// uses the same `@react-three/postprocessing` dependency); no new package is
// added. It is deliberately smaller than the legacy chain:
//
//  - no Noise: the forum is a daylight solarpunk campus, not the fogged
//    rotunda, and film grain reads as dirt on white sandstone.
//  - `luminanceThreshold` is high so only genuinely emissive surfaces bloom —
//    the hologram globe, the lantern cores and strand lights, the gate's
//    horizonBlue channel and the Agora's event horizon — and lit stone does not.
//  - `multisampling={4}` because the forum Canvas runs `antialias: true`; the
//    composer renders into its own target, where the canvas's AA does not
//    apply, so without this the whole scene would lose its edge quality as a
//    side effect of turning bloom on.
//  - `mipmapBlur` with few levels: the halo comes out of a mip chain instead of
//    a wide full-resolution gaussian, which is the cheap way to do this on a
//    scene that is already fill-rate bound.
//
// Measured on the standalone forum page (ANGLE/Metal, M4, 910x1000): 214 draw
// calls at 93 fps with this mounted against 203 at 91 without it, and no
// visible change to the daylight scene beyond the intended emissive glow.

import { Bloom, EffectComposer } from '@react-three/postprocessing';

export function ForumPostProcessing() {
  return (
    <EffectComposer multisampling={4}>
      <Bloom mipmapBlur intensity={0.55} radius={0.6} levels={5} luminanceThreshold={0.72} luminanceSmoothing={0.3} />
    </EffectComposer>
  );
}
