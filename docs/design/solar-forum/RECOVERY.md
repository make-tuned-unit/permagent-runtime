# Crash recovery — 2026-09-10

The restart cleared `/private/tmp/permagent-solar-forum`. The saved branch at
`dc8c6bd3` survived in the original repository. Source-only file creation and edit
steps were recovered in order from the prior session records; build, deployment,
network and interactive commands were not replayed as part of source restoration.

Persistent project: `/Users/j/Documents/dev/permagent-runtime/recovery/solar-forum`.
The original checkout's iOS edits were preserved.

Recovered: expanded campus geometry and collision, gallery/observatory routes,
system appearance, character silhouettes/portraits, conversation integration,
Unreal import/environment/render scripts, tests and design notes.

Generated assets and Unreal Content were lost and are being regenerated from the
recovered generators and hash-verified Poly Haven sources. Prior validation claims
in README/PLAYTEST describe the pre-crash session; new results will be recorded here.

Fresh validation: TypeScript passes; all 44 focused World tests pass. Regenerated
campus: 134 m diameter, 99,176 triangles, 6,759,000-byte GLB. All 12 agent models
and portraits regenerated. All 26 Poly Haven files passed MD5 verification.

The standalone Vite production build passes and the restarted preview returns HTTP
200 at http://127.0.0.1:5284/ui/forum.html. Unreal 5.8.2 imported the 13,400 cm
campus with 12 material slots and compiled the first-person Blueprint successfully.

Recovery checkpoint: `b191d181`, also saved in the original repository as
`recovery/solar-forum-20260910`. Native browser automation stalled during this
session and then reported concurrent user interaction, so browser visual QA has
not been claimed.

Unreal environment build now passes: `ForumCinematic`, 112 environment actors,
scanned limestone, animated water, bronze, cutout vegetation, rocks and atmosphere.
Fixed two recovered script defects: texture discovery now includes JPEG and EXR,
and fog uses the engine's `enable_volumetric_fog` property. Reusing already-assigned
mesh materials avoids redundant static mesh rebuilds. Native play-in-editor and
live agent interaction in Unreal remain unverified.

The first Metal exports exposed incorrect positional Rotator arguments in the
recovered scripts. Cameras, sun, foliage/bank yaw, and first-person start now use
explicit pitch/yaw/roll fields. Both the base-map import and environment build pass
with this correction. See the engine's [Rotator API](https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/Rotator?application_version=5.1).

Visual review of the corrected Metal images confirms a level horizon, visible
forum, upper gallery/observatory, and connected campus courts. This is an Unreal
environment prototype, not a claim of finished AAA quality. Remaining art work
includes richer terrain/shore transitions, less repetitive building silhouettes,
material variation, more believable water, and in-engine agent presence. Native
player-input testing and GPU frame-time measurement are still outstanding.

The capture now explicitly requests Lumen global illumination/reflections, as
required for scene captures, and refreshes a movable skylight. The lagoon extends
past the wide review camera to avoid a visible rectangular water boundary.

Final result: all three 1600×1000 Metal renders exported successfully and were
visually inspected after the rotation and capture-lighting corrections. See
`assets/world/forum/unreal/` and `unreal-render.json`. Reflections and shaded detail
are improved; shaded areas still need lighting balance. The recovered preview is
running at http://127.0.0.1:5284/ui/forum.html.
