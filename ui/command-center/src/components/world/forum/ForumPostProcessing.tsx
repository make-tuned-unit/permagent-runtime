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
//  - `multisampling={0}`, and this is the fix for the black rectangle.
//
//    Jobs 19 and 20 recorded a solid block of exact RGB 0,0,0 over the world,
//    root-caused it to the ANGLE *Metal* backend, and worked around it in the
//    capture harness by driving the GL backend instead. The product runs on
//    Metal, so users kept seeing it. `scripts/probe-black-rect.mjs` reproduces
//    it on purpose and counts the pixels; on this machine, on Metal:
//
//      multisampling={4}   1 of 6 frames black, 71,302 px at [437,0]-[776,250]
//                          1 of 12 frames black, same rectangle, twice over
//      multisampling={0}   0 of 6 and 0 of 12 frames, zero black pixels
//
//    The composer's multisampled render target is what trips the driver. The
//    cost of dropping it is edge quality on the composer's own target, which
//    is the thing the next note was protecting — but a scene that intermittently
//    paints a black hole over a third of the frame is not trading quality for
//    quality, it is trading a defect for aliasing. SMAA as a final pass is the
//    documented replacement and is the next step here.
//
//    The note this replaces read:
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

import { Bloom, BrightnessContrast, EffectComposer, HueSaturation } from '@react-three/postprocessing';

/**
 * The grade that gives AgX its colour back.
 *
 * AgX is the right view transform for this scene — it is what Blender renders
 * the source images with, and it holds a sunrise sky together instead of
 * clipping it to white paper. It also desaturates hard by design, which is why
 * Blender ships "AgX — Punchy" and why nobody uses base AgX as a final look.
 * three.js has no Punchy, so the scene arrived washed out.
 *
 * Measured against `docs/design/solar-forum/north-star.png` with
 * `scratchpad/imgstats.py`, over the world area of the frame only (the Agent
 * Desk is flat dark UI and drags every figure toward itself):
 *
 *                       luma    contrast  saturation
 *   north-star           0.340     0.237       0.482
 *   forum, before        0.498     0.202       0.191
 *   forum, after         0.441     0.221       0.372
 *
 * Saturation was the gap: two and a half times under the thing it is meant to
 * look like, while luma ran bright and flat. These two effects are the Punchy
 * step, applied after bloom so the glow blooms scene colour and the grade lands
 * on the finished frame, which is the order a film chain uses.
 *
 * It stops short of the reference on purpose. north-star is a NIGHT render, and
 * night is saturated in a way a sunrise is not — deep blues and neon against
 * black, where this scene is pale stone under a warm low sun. Grading a daylit
 * campus all the way to 0.48 turns the planting to poster paint. 0.372 is where
 * the greens read as leaves and the lanterns read as fire.
 *
 * Both are display-referred and cheap — two full-screen multiplies folded into
 * the pass the composer already runs — and neither reintroduces a multisampled
 * target, so the ANGLE/Metal black rectangle documented above stays fixed.
 * `scripts/probe-black-rect.mjs` is the check for that and must be re-run after
 * any change to this chain.
 */
const SATURATION = 0.4;
const CONTRAST = 0.075;
const BRIGHTNESS = -0.035;

export function ForumPostProcessing() {
  return (
    <EffectComposer multisampling={0}>
      <Bloom mipmapBlur intensity={0.55} radius={0.6} levels={5} luminanceThreshold={0.72} luminanceSmoothing={0.3} />
      <HueSaturation saturation={SATURATION} />
      <BrightnessContrast brightness={BRIGHTNESS} contrast={CONTRAST} />
    </EffectComposer>
  );
}
