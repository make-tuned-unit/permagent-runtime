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

import { Bloom, BrightnessContrast, EffectComposer, HueSaturation, ToneMapping } from '@react-three/postprocessing';
import { ToneMappingMode } from 'postprocessing';

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
 * (Those figures were taken with AgX silently OFF: see THE WHITE BLOCK below.
 * With AgX actually in the chain, and SATURATION retuned 0.4 -> 0.5, the day
 * frame measures 0.435 / 0.199 / 0.354 at 1512x945, display DPR 2.)
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
 *
 * THE WHITE BLOCK (the user, 2026-09-18, real app, night): a hard-edged white
 * shape over the sky beside the gas giant. Cause: this composer threw away the
 * AgX view transform. ForumView sets `toneMapping: AgXToneMapping` on the
 * Canvas, and two things switch it off while the composer is mounted. First and
 * most directly, @react-three/postprocessing (2.19.1) sets
 * `gl.toneMapping = NoToneMapping` outright for as long as EffectComposer is
 * mounted (dist/EffectComposer.js:117-122), so the Canvas's AgX never runs,
 * not even for a draw to the screen. Second, three (0.184) only tone-maps a
 * draw to the SCREEN anyway, and the scene now draws into the composer's
 * HalfFloat target; postprocessing (6.39) never tone-maps unless it is asked to
 * (every effect material is `toneMapped: false`). So rendering the scene
 * straight to the screen would NOT bring AgX back: the fix has to be a
 * ToneMapping effect inside the chain. The frame reaching the grade was
 * linear HDR, and everything above 1.0 was clipped to flat 255 at the end. The
 * gas giant is the brightest thing at night; bloom adds its own halo on top,
 * and the brighter bands and the halo fuse into one flat white patch whose
 * edge is wherever the clip happens to land: a white block beside the gas giant.
 * The fix is `<ToneMapping mode={AGX}>` after Bloom and before the grade:
 * bloom still works on scene-linear light (that is what its threshold is in),
 * and the grade lands on display-referred values, which is what a grade is for.
 *
 * A/B: one change at a time, measured with `scripts/probe-black-rect.mjs` in
 * Playwright WebKit (webkit-2359, a stand-in for the system WKWebView the app
 * embeds; see the WKWebView rows below), night via
 * colorScheme dark, 1512x945 CSS px at display DPR 2 (the Canvas's dpr [1, 1.5]
 * clamps it to 1.5 internally), default vista held still, 3 frames per row,
 * identical across frames. Counted over the canvas rect in device px:
 * `white` = pure 255,255,255 pixels, `any` = pixels with ANY channel at 255,
 * `blob` = the largest connected pure-white shape. The floor is the HUD's
 * white text over the canvas (no-composer row: 12,020 white, 184 blob = glyph).
 *
 *   chain                                   white    any       blob   rect
 *   committed: Bloom > grade (no AgX)       21,450  125,975    5,822    0
 *   no composer (renderer AgX, no bloom)    12,020   75,994      184    0
 *   Bloom alone                             33,585  111,266   16,538    0   <- 331x356 px flat white over the gas giant
 *   grade alone                             11,764  108,594      184    0
 *   Bloom thresh 0.72 -> 0.90, + grade      13,918  114,818      536    0
 *   Bloom levels 5 -> 3, + grade            22,161  122,633    6,063    0
 *   Bloom intensity 0.55 -> 0.25, + grade   14,175  116,381    1,152    0
 *   Bloom > AgX (no grade)                  11,382   12,159      184    0
 *   AgX > grade (no bloom)                  11,382   13,801      184    0
 *   Bloom > grade > AgX                     11,382   12,115      184    3   <- black runs: grade crushes the sky before AgX
 *   Bloom > AgX > grade      (sat 0.4)      11,382   13,962      184    0
 *   Bloom > AgX > grade      (sat 0.5)      11,382   14,329      184    0   <- SHIPPED
 *   same, contrast 0.075 -> 0.12            11,382   24,476      184    3   <- black runs: night sky crushed
 *
 * The same check in the SYSTEM WKWebView that Tauri embeds
 * (scripts/probe-wkwebview.swift at 1512x945, night, display DPR 2 and canvas
 * DPR 1.5, whole-window snapshot so the HUD text counts), then run through
 * `probe-black-rect.mjs --analyze`:
 *
 *   committed: Bloom > grade (no AgX)       33,233  138,183    5,822    0
 *   SHIPPED: Bloom > AgX > grade (sat 0.5)  24,265   27,630      436    0   <- blob is desk heading text
 *
 * Bloom settings only shrink the clipped patch; they do not remove it, because
 * the gas giant itself sits above 1.0. Only the view transform does. Every
 * row with AgX in the chain drops to the HUD floor, and the gas-giant crop
 * goes from 9.2% clipped to 0.0%. The rectangle detector (>=200 CSS px run of
 * pure white/black) never fired white in any row, at this pose. The block in
 * the real app was never captured, so that its pose and window size produce
 * the same clip at block size is the leading explanation, not a confirmation.
 *
 * Keeping the look: AgX desaturates on the way in, so with the grade unchanged
 * the day scene fell from saturation 0.365 to 0.314 (imgstats, world area).
 * SATURATION 0.4 -> 0.5 brings it back to 0.354 (night 0.654 against 0.643
 * before); luma is unchanged (day 0.435 against 0.434). Day contrast settles at
 * 0.199 against 0.224, and it stays there, because raising CONTRAST crushes
 * the night sky to pure black runs (last row).
 *
 * Earlier the same day, before this was found, the chain as committed was run
 * without a single white rectangle hit: Playwright WebKit at 1280x1000 @1 and
 * five sizes @2 (about 100 frames, orbit / still / resize), the system
 * WKWebView via scripts/probe-wkwebview.swift at 1512x945, 1280x800 and
 * 1440x900 (display DPR 2, canvas DPR 1.5; opaque and Tauri-transparent
 * windows; about 40 frames), and Chromium ANGLE Metal (16 frames). A scratch
 * readback found no NaN or Inf before or after Bloom, and alpha 255
 * everywhere. So a non-finite value spreading through the mip chain, and a
 * see-through canvas, are both ruled out.
 *
 * Is it in the app? Every probe above renders the vite DEV page, which is not
 * the bundle Permagent.app serves. `probe-black-rect.mjs --shipped` reads the
 * chunk names the installed binary embeds and this chain out of those same
 * chunks in dist/. On 2026-09-19 it found the app packaged at 23:53 the night
 * before embeds ForumAppView-BAoLuAgn.js with Bloom > AgX > grade,
 * multisampling 0 (verdict fix-shipped). Run it after every app rebuild; only
 * the look in the app itself, at night beside the gas giant, is still owed.
 *
 * Two more gaps between "shipped" and "on the user's screen", closed the
 * same day (the screen was locked again, so the window itself could not be
 * captured):
 *  - The binary on disk is not what a RUNNING app shows: it serves the
 *    assets of the binary it was launched from. `--shipped` now also reads
 *    the running permagent-app's start time and fails (exit 3) if it predates
 *    the binary. At 07:03Z the app running was launched 06:07:32Z, 22 s after
 *    the binary holding ForumAppView-BPc9qO6y.js was written: fix-shipped.
 *  - Every probe rendered EMPTY storage, i.e. Agent Desk and place rail
 *    open. The user's app has both closed (read from its WebKit store), so
 *    his canvas is the full window: another aspect, another framing of the
 *    gas giant, other target sizes. `--app-state` renders that layout. In
 *    the system WKWebView at 1512x945, night, display DPR 2: 0 pure-white
 *    run pixels, largest white blob 73 px (a HUD glyph), gas giant banded.
 *    It also found 9,876 exact-black run pixels, which is the far tower
 *    facade taken to 0 by BRIGHTNESS: 7 clusters, none taller than 7 device
 *    rows. Texture, not a rectangle (see SOLID_FILL and MIN_BLOCK_ROWS in
 *    extremeRuns.ts, which judge each block on its own so that texture can
 *    never dilute a real block into a clean verdict). With a job-19-sized
 *    block planted over that same frame (PLANT=black|white) the probe reports
 *    it on every frame, so the clean verdict is not a blind instrument.
 *  - The app's window is 1280x800 pt (CGWindowList, pid 76299), so the World
 *    panel is ~1072x748 with the rail open: at that size, night, DPR 2, the
 *    same result (0 white run pixels, largest white blob 73 px).
 *  The in-window capture itself is `probe-black-rect.mjs --app-window`; it
 *  refuses on a locked screen, which is all it could do on 2026-09-19.
 *  It also used to call ANY view clean, so a Dashboard capture could have
 *  closed this. It now reads the World's own labels off the capture with
 *  on-device Vision (worldFromOcr in extremeRuns.ts) and refuses without
 *  them, and `--app-window --wait <s>` polls through the lock so the check
 *  runs the first time the screen is unlocked with the night World open.
 *  That capture is the one confirmation still owed.
 */
// Keep the stronger night grade; daylight foliage needs less saturation.
const SATURATION = { day: 0.16, night: 0.5 };
const CONTRAST = 0.075;
const BRIGHTNESS = -0.035;

export function ForumPostProcessing({ appearance = 'day' }: { appearance?: 'day' | 'night' }) {
  return (
    <EffectComposer multisampling={0}>
      <Bloom mipmapBlur intensity={appearance === 'day' ? 0.3 : 0.55} radius={0.6} levels={5} luminanceThreshold={0.72} luminanceSmoothing={0.3} />
      {/* AgX here, not on the Canvas: the Canvas's toneMapping is skipped while
          this composer is mounted, which is what clipped the gas giant white. */}
      <ToneMapping mode={ToneMappingMode.AGX} />
      <HueSaturation saturation={SATURATION[appearance]} />
      <BrightnessContrast brightness={BRIGHTNESS} contrast={CONTRAST} />
    </EffectComposer>
  );
}
