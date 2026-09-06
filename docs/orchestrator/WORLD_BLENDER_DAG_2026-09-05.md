# Blender World authoring DAG

Owner: root / Astra. User requested Blender-authored environments and characters;
no renderer, live-state, navigation, memory, or animation framework replacement.

`W0 audit → W1 editable asset → W2 export validation → W3 runtime mount → W4 visual/performance → W5 app release`

- W0: installed Blender 5.2.1 CLI verified; existing World bible and scene inspected.
  Retain cyber-classical material tiers, meter scale, colonnade radius 14, mezzanine
  height 10, Mesh opening at 225°, all existing live props and characters.
- W1: author the Observatory Vault in Blender: carved fluted stone, bronze ribs,
  coffered open vault, restrained engraved light. Save editable `.blend` and generator.
- W2: export GLB with bundled PBR materials, no remote textures or dependencies;
  assert size/triangle/material budgets, axis conversion, named architecture root.
  Static repeated geometry is material-batched at export rather than separate
  meshes: a scoped authoring amendment to the old instancing-only rule, preserving
  its draw-call objective. Editable source objects remain separate in the blend.
- W3: replace only static hall shell through existing R3F/drei GLTF loader. Keep
  procedural shell as loading/error fallback. No silent blank World on asset failure.
- W4: inspect actual Blender render and actual runtime; run regression/type checks;
  measure draw calls/triangles with existing probe. Render != WebView performance.
- W5: release only with existing app build gate; do not claim shipped before install.

Future character authoring uses Blender source + GLB, meter scale, named pivots and
state-independent identity materials. Existing live poses, identity/state separation,
anchors, and honest idle/working rules remain authoritative. Replacing all character
rigs is a separate migration gate, not an excuse to disconnect live characters now.

iOS companion fix: Home and Control now match Decisions' existing brandTitle and
6/8 top/bottom spacing. Voice reconnect investigation runs in its own worker lane.

Status: audit, authored asset and live binding gates passed. Visual/motion
qualification active; installed macOS acceptance remains open. Machine-readable
status is in WORLD_BLENDER_MASTER_PROGRAM_DAG.yaml.

## Expanded direction — user amendment, September 5

The first small vault is a pipeline proof, not final creative acceptance. Enlarge
the Rotunda into a 51-meter institution with a second architectural order and
36-meter vault. Preserve the functioning inner library, task floor and anchors
while expanding outward. Intricacy must survive export and runtime lighting.

Additional child DAGs, each audit → implement → regression → runtime evidence:

1. **Architecture**: monumental double colonnade, recessed coffers, promenade,
   layered bronze and carved stone. Root/Astra authors in Blender.
2. **Characters**: distinct role-specific silhouettes and equipment authored in
   Blender; adapt to existing rig/pose/state contracts, never replace truthful
   state with pre-rendered fake work. Preserve custom identity and operator name.
3. **Interaction**: hands/tools meet existing real work surfaces; actions and
   completion artifacts stay bound to actual backend signals and provenance.
4. **Performance**: visible 60 Hz target, hidden render pause preserved; batch
   staircase/railings and repeated furniture before reducing design quality.
   Current initial capture: 30 fps (intentional cap), 612 draws, 275065 triangles.
   Source cap is now 60; measured 60 fps is not yet claimed.
5. **Acceptance**: flythrough, close character views, portal/station navigation,
   reduced motion, failed-asset fallback, steady-state/motion frame-time samples,
   native WKWebView qualification and bundled production asset verification.

Pipeline proof evidence: Blender5.2.1 render/export ran; GLB 2027688 bytes,
74876 triangles, 5 batches; three asset/fallback tests + seven World HUD tests
passed. Actual browser capture had no page exceptions. Expanded art supersedes
these preliminary asset numbers and must be measured again.

## Refinement and solarpunk amendment

Root/Astra authored twelve named role outfits: ambassador frock, archivist
vestments, scholar waistcoat, surveyor field jacket, artisan apron, sentinel
cuirass, treasurer double breast, celestial navigator, consular stole,
fabricator harness, curator expedition vest, and botanical survey tunic.
Existing live skeleton and identity/state materials remain authoritative;
no additional per-character draw channels or cloth simulation.

Suspended conservatory gardens and photovoltaic petals add living green to
the outer order without obstructing ground navigation or the Mesh approach.
The vault exports six material batches, 143000 triangles, 4404564 bytes.
Existing timeOfDay consumers now follow the existing resolved appearance:
system changes and explicit app themes share one source of truth. Local
dawn/dusk nuance is retained when it agrees with that appearance. No new
theme preference, geolocation or network dependency.

Verification: 188 World tests passed, TypeScript passed. Browser live OS
appearance change reached the shared World state (day → dusk), with all
twelve authored agents and the garden vault loaded and no scene errors.
Refined Chromium motion test held 60fps, p95 16.7–16.8ms at DPR1.5,
275 draw calls and 321425 scene triangles. Daylight now also colors the
background and matching fog through that same store, with no extra draws.
Standalone WKWebView reported 30fps on a display reporting maximum 30Hz;
this is NOT native 60fps acceptance and not an installed Tauri test.

## Independent W3 closure and movement regression

Root's rerun did not accept the initial WALKING label as motion evidence.
It exposed a real selection binding defect: a ref filled only by `useFrame`
left the camera's selected-agent prop null until an unrelated React render.
The existing proxy now initializes synchronously for each selected identity,
then updates its stable position in place each frame. A hook regression proves
immediate selection, live updates, identity changes and null/unknown selection.

Real pointer/key journey after the fix: library/Build station camera glide
1.532 units; Henry pointer hit opens HUD and walking mode; holding W moves the
live agent 2.25 units; Escape closes HUD and returns to orbit. No page errors.
The evidence camera remains pinned only during target acquisition and is
released before testing actual walking and camera return. Targets are freshly
projected rather than probed through a stale pixel grid.

Pointer drag and activated tour both moved the camera. After the actual
ResizeObserver visibility gate acknowledged hidden layout, zero perf windows
were produced; restoring layout produced three. Activated reduced-motion tour
held its establishing pose. The gate acknowledgment removes the old measurement
race with the final pre-hide frame; it does not relax the no-hidden-render rule.
W3 is passed. W4 final production/performance receipt and W5 native installed
acceptance remain explicit successors.

### W4 final source-build receipt

After the selection fix, **189 World regressions passed**. Production build
passed after removing the newly unused hook import; Vite retains its existing
large-chunk warnings. All **13 GLBs** in `public/world` match `dist/world`
byte-for-byte by SHA-256. Final browser run: five static 60fps windows and ten
moving-camera windows at 60fps, p95 16.7–16.8ms, 274–275 draws, at most 321425
triangles, DPR1.5; twelve characters and one vault loaded. OS appearance switched
day to night correctly, with no page errors or warnings. W4 passed; W5 remains
unpassed and requires a fresh native application and suitable display/device
measurement. This receipt does not replace the existing human release gate.

### Art-direction receipt, September 6 (creative refinement after W4)

Audit before any change, against a fresh Cycles render of the shipping .blend
(the on-disk preview predated the conservatory pass) and real Chrome captures
at five fixed viewpoints, day and night. Judgement: the material hierarchy had
collapsed at runtime. Bronze at metallic .72 under two directionals and no
environment map keeps ~28% diffuse and no reflections, so it rendered as the
same khaki as key-lit limestone and the whole hall read as one tan wireframe.
The vault read as a birdcage from the default orbit view: uniform four-sided
rib pipes, single-sided coffer fields seen through culled back faces as unlit
dark squares, diagonal tracery reading as scribble, no weight at the spring.
The six entablature "cornices" were round tori with no horizontal mass. The
outer arcade sprang mid-shaft from nothing, its piers (14:1) slenderer than the
intimate order they should outweigh; the bronze reveals stood at a fixed radius
half a metre in front of the tapering piers, and the bronze soffit was buried
inside the arch tube, never visible. The green did not read at 51 m (0.95 m
pots, 1.5 m fronds, two flat black kites per pot). The alternating pale/dark
promenade wedges were the loudest element in the frame. At night bronze
vanished and nothing carried engraved light into the dome. Characters: distinct
silhouettes, runtime-owned materials; the only fault is jacket lapels floating
~2 cm off the chest in close-up. Left untouched: reworking a live-skeleton
contract is not worth that return.

Changes, all in `scripts/blender/build_world_vault.py`, regenerated through the
documented command with `--render`. The previous .blend, GLB, manifest and
preview were copied aside before regeneration; nothing was hand-edited; the
twelve character .blend/GLB pairs are untouched (byte-identical SHA-256).
Six material batches kept. Bronze retuned dark and saturated (#6B4720,
metallic .5, roughness .34), stone matte (.56), basalt polished (.38), all for
the forward renderer rather than Cycles. Entablatures are swept rectangular
profiles with fascia steps, a projecting cornice and a bronze fillet; the warm
hairline now sits under the inner cornice soffit. Outer piers .88→.74 radius,
bronze impost rings at the 17.0 spring line, eight-sided archivolts (.34) with
the bronze intrados hung visibly below; reveals follow the taper on the pier
surface, as does the inner column light channel. Vault ribs taper .30→.13 by
per-point bevel radius (no polygon cost), eight-sided, each with one engraved
warm hairline on its inner face replacing the buried bronze seam; coffer
diamonds replaced by stone lips plus an outward-facing shell so the orbit view
sees a pale coffered dome. Conservatory: faceted canopy masses, cascading vines
(lowest point 2.66 m, above head height, Mesh approach clear), fronds, and
five-petal photovoltaic parasols on bronze ribs. Promenade: all-basalt wedges,
bronze hairline rays, one pale stone ray per pier axis. Hairline rings under
3 cm dropped to four minor segments; a first export at 150672 triangles
tripped the existing 150k test guard, and that trim (not a raised guard)
brought it back under.

Asset, old → new: 143000 → 141456 triangles, 4404564 → 4692432 bytes, 6 → 6
batches. Colonnade radius 14, mezzanine 10, Mesh opening 225°, promenade
25.5 and the live floor inside r=15 are unchanged.

Gates: `tsc --noEmit` clean; `vitest run` **1930/1930** (261 files); World
**189/189** (23 files); `vite build` passed with its pre-existing large-chunk
warning; all **13 GLBs** in `public/world` match `dist/world` by SHA-256 (vault
b41578e0…b9721).

Runtime evidence (real headless Chrome 1440×1000, worldcensus surface, DPR1.5;
a render is not performance evidence): the final export held 60fps in five
static windows and ten moving-camera windows, p95 16.7–16.8 ms, 275 draws
(274 in one window), **319881** scene triangles against 321425 before this
node; twelve authored agents and one vault loaded; OS appearance day → night
reached the World store; no page errors or `[world]` warnings. Reduced-motion
run: identical counts, 60fps in all fifteen windows. Failed-asset run (vault
GLB request aborted): the procedural hall mounted with vaults=0 and twelve
agents still authored, the expected "[world] Blender vault unavailable"
warning plus the loader's two "Could not load" errors surfaced through the
boundary, 322 draws, 192829 triangles, 60fps in all windows: not a blank World.

Caveats. The first "before" capture of the untouched scene showed static
windows at 39/20fps and eight seconds of 18–24fps camera motion while three
unrelated python3 processes ran at 99% CPU (load ~6); every after capture held
60 under the same background load, so that dip was contention, not the scene,
but a quiet-machine run would be cleaner evidence. The Cycles preview at
`assets/world/blender/observatory-vault-preview.png` is inspection only. The
stone still reads khaki under the runtime's warm key and ACES exposure; that is
the atmosphere lane's light rig, not this asset, and was left alone. Native
WKWebView was not measured; W5 keeps its human gate. Nothing committed.
