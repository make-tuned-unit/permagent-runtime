# Solar Forum — Permagent World exploration

Branch: `feat/solarpunk-forum`, cloned from `1b142785` on 2026-09-10.
Persistent recovery checkout: `/Users/j/Documents/dev/permagent-runtime/recovery/solar-forum`.
See [RECOVERY.md](RECOVERY.md) for crash recovery and fresh validation. The original checkout and the
concurrent Claude Code session's files were not edited.

## Open the work

From `ui/command-center`, run:

```sh
npm run dev -- --host 127.0.0.1 --port 5284
```

- Forum: http://127.0.0.1:5284/ui/forum.html
- Original World comparison: http://127.0.0.1:5284/ui/worldcensus.html
- Standalone build: `./node_modules/.bin/vite build --config vite.forum.config.ts`
  writes `dist/forum`. The shipping app entry remains unchanged.
- Blender: `assets/world/forum/solar-forum.blend` (generated — see **Generated assets**).
- Unreal: `unreal/SolarForum/SolarForum.uproject`, map `/Game/SolarForum/Maps/Forum`.
  The generated map and local Epic content exist in this worktree but are not
  committed. Recreate them with `./scripts/unreal/build-forum.sh` from the repo root.

## Generated assets

Everything the Blender builder writes is generated output, not source. The
source of truth is `scripts/blender/build_solar_forum.py` plus the module
scripts it executes; regenerate with:

```sh
blender --background --factory-startup --python-exit-code 1 \
  --python scripts/blender/build_solar_forum.py
```

Add `--no-render` to skip the Cycles evidence passes, or set
`FORUM_SCRATCH=<dir>` to redirect every output (blend, FBX, GLB, manifest,
renders) into a scratch directory so a modeling dry run never touches the
shipped asset.

| Path | Tracked in git? | Notes |
| --- | --- | --- |
| `assets/world/forum/solar-forum.blend` | No — `.gitignore` | 8 MB editable source rebuilt on every run. |
| `assets/world/forum/solar-forum.fbx` | No — `.gitignore` | 19 MB; the Unreal pipeline (`scripts/unreal/`) reads it from disk, so keep it locally. Run the builder before an Unreal import on a fresh checkout. |
| `ui/command-center/public/world/solar-forum.glb` | Yes | The runtime asset. Shipped compressed — see below. |
| `ui/command-center/public/world/solar-forum.manifest.json` | Yes | `bytes` is the compressed file, `rawBytes` the Blender export. |
| `assets/world/forum/**/*.webp` | Yes | Evidence renders, converted from PNG by `scripts/blender/shrink-evidence.py`. |

### Runtime GLB compression

Blender's GLB export is ~62 MB — float32 geometry plus embedded JPEG textures.
GitHub warns above 50 MB, and the Tauri bundle grows by the whole file. Git LFS
is deliberately not used (not installed here, and it bills CI bandwidth).

The builder therefore pipes its export through
`npm run world:compress` (`ui/command-center/scripts/compress-world-glb.mjs`),
which re-encodes the embedded JPEG textures at quality 90 — same format, same
resolution — and then writes the geometry as **EXT_meshopt_compression** plus
**KHR_mesh_quantization** with `@gltf-transform/cli`. Result: **62,151,688 →
14,118,352 bytes (4.4x)**, with mesh count, triangle count and material names
unchanged.

Decoding needs a meshopt decoder on every loader that reads the file.
`src/components/world/forum/forumGltf.ts` supplies three's own
`MeshoptDecoder` to the browser loaders (`ForumView`, `ForumSky`) and to the
Node loaders in `forumWalkRoutes.test.ts` and `forumAssets.test.ts`. Draco is
deliberately not used: it fetches a decoder from a CDN in the browser and needs
Web Workers that Node does not have, so the same file could not be verified by
the walk-route tests. The legacy World's loaders are unchanged; those assets are
still uncompressed.

`forumAssets.test.ts` fails the build if the shipped GLB exceeds 20 MB or loses
the meshopt extension.

### Evidence renders

Blender and Unreal can only write PNG, and ~40 of those frames had reached
63 MB. `python3 scripts/blender/shrink-evidence.py` converts every PNG under
`assets/world/forum/` to WebP at max 1400 px, quality 82, deletes the PNG and
rewrites the `.png` references in `docs/design/solar-forum/**/*.{md,json}`. It
is idempotent; re-run it after any render pass. `north-star.png` stays a PNG
(it is the art-direction reference) and is capped at 1600 px.
`assets/world/forum/third-party/` is skipped — those are CC0 source textures
Blender reads at build time.


## Review and design direction

The existing World already represents real activity: it has daemon-backed agent
sources, individual HUDs, active-goal plaques, pending decisions and product
navigation. The historical baseline audit is stale on this point. The current
roster explicitly distinguishes daemon-backed, ambient and static identities.

The gap is the path from understanding to action. HenryHUD's Chat and Tools tabs
are disabled; information is fragmented across panels; selecting an agent also
switches the camera to third-person. A request to learn unexpectedly changes the
controls. This study keeps selection and camera mode separate and offers one desk.

The solarpunk direction extends Permagent's existing cyber-classical language:
pale stone, dark structural accents, bronze mechanisms, engraved cyan light and
preserved agent identity trims. Green belongs to vegetation, never to a fabricated
agent status. Blender reads the shared World palette directly, including the
canonical `NEON_ACCENT`, instead of maintaining a competing palette.

The expanded architecture adds Build, Brain, Automate and Mesh thresholds, information
steles, bronze lecterns, fine inlaid conduits, hanging vines, lantern housings,
pollinator pots and a shaded upper gallery, twin stairs and an observatory prospect. The UI uses
shared theme colors, font/radius tokens and Button
primitive. The configured orchestrator name is used consistently. Forum CSS is
scoped to prevent it from repainting other product views.

## Implemented behavior

- Orbitable Blender scene with 12 material batches, a campus roughly 134 meters across. The manifest records the current triangle
  count and GLB size. Editable source retains individual architectural objects.
- Twelve distinct, separately authored forum outfits and portraits, plus unique
  character/voice briefs for every real roster identity. The original production
  armor assets remain separate. The Librarian occupies the gallery and the
  Forecaster the observatory. Existing rigs and live status semantics are retained.
- Browser walk mode: WASD, Shift for faster movement, mouse or drag/arrow look,
  E to inspect a nearby agent, and Escape for overview. Grounded movement checks
  authored geometry using BVH-accelerated body and floor probes; it blocks walls,
  water, high ledges and gaps and follows stair-height changes. This lightweight
  controller is not a general rigid-body physics engine. Collision route tests
  exercise the real exported model in addition to simple fixture geometry.
- World lighting follows `(prefers-color-scheme: dark)` live: neutral marble by
  day, Permagent deep blue by night, with moonlight and warm local lighting. No
  green backdrop or separate logo. The user's global app theme preference is not
  overwritten. The OS determines when automatic appearance changes.
- Ten accessible place controls and matching 3D waypoints. District selection
  moves the camera without an animated transition and opens a relevant agent.
- Meet / Learn / Give a brief / Conversation desk. Four lessons cover planning,
  tools, memory provenance and completion evidence. Lessons become editable queries.
- **Ask** reuses the existing authenticated conversation, event stream, message
  bubbles, decision UI and cancellation. The orchestrator receives the selected
  specialist as context; it does not claim to dispatch directly to that specialist.
  No request is sent on mount or selection. Clicking Ask is the send action.
- **Jobs remain drafts.** Query/job briefs include acceptance criteria and export
  to Markdown. No goal/run is created merely by preparing or exporting a brief.
- Live jobs use the existing feed, with an unavailable/connecting message until
  loaded. No invented activity counts. The existing feed retains its last snapshot
  after a later disconnect; explicit freshness tracking remains a follow-up.
- `ForumView({ visible, onNavigate })` supports host integration. Hidden views stop
  rendering. A supplied navigation callback maps Brain and the gallery to `memory`, Build to
  `build`, and Automate to `automate`. Mesh has no invented destination.
- A fallback floor handles unavailable authored geometry. Narrow screens stack
  the desk below the scene; keyboard users can access districts without the canvas.

## Unreal integration

Unreal **5.8.2** successfully imported the regenerated FBX, saved the map, and
compiled the first-person character Blueprint to `BS_UP_TO_DATE`.
`unreal-import.json` records the verified campus diameter, 12 material
slots, collision mode, map, game mode and starting position.

The import script uses the installed engine's Python API. An initial commandlet
attempt failed because the legacy FBX importer expected Slate initialization.
The working invocation uses full editor initialization with `-ExecutePythonScript`
and `-NullRHI`. The builder now checks a fresh completion report because Unreal
can exit zero even when a Python assertion fails.

The first-person template and shared character/input assets are copied from the
locally installed Epic engine by `prepare_forum.py`; they are not distributed in
git. Rebuild imports the FBX and updates the generated actors. Treat this map as
generated output: duplicate it before hand-editing. `FORUM_ENGINE` overrides the
installed engine directory when necessary.

The map includes static architecture collision, a first-person spawn, native
material slots, directional light, sky and a review camera. Triangle collision
preserves the open court and colonnade instead of wrapping the whole scene in a
convex hull. The character Blueprint and input assets compile/load successfully.
**Play-in-editor, player movement, rendered Unreal quality and frame time have not
been verified.** `NullRHI` validates assets and map creation, not rendered output.
The scene has no agent/daemon bridge in Unreal yet. Navigation mesh, interact
prompts, playable agent encounters, native water/foliage materials and a rendered
lighting pass remain work for the next playable slice.

API references: [Epic FBX import options](https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/FbxImportUI)
and [Epic editor level API migration notes](https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/EditorLevelLibrary?application_version=5.0).
Installed 5.8 headers and live Python execution verified the APIs used here.

## Make the forum useful

| Place | User action | Evidence to inspect |
| --- | --- | --- |
| Learning stoa | Work through brief → plan → tool → result | Clearly labeled lesson trace, distinct from a live job |
| Agent desk | Understand capabilities and ask a question | Actual configuration, tools, limits and conversation |
| Petition tables | Scope a job, choose a project, review and submit | Server-acknowledged run ID, owner, approvals and cancellation |
| Build workshop | Follow work and inspect artifacts | Existing run/task events, diffs and checks |
| Brain archive | Explore and correct memory | Sources, observation time and provenance |
| Mesh court | Compare proposals and model perspectives | Individual arguments, synthesis and unresolved uncertainty |

Next integration should bind job creation to the canonical project/goal service,
then show acknowledged runs in the plaza. Keep credentials, authorization and
execution in the daemon. Unreal should consume the same commands and event stream
as the web client, with request IDs, event sequencing and connection freshness.
Task progress must never be inferred from an animation timer.

## Validation and limits

- Blender source regenerated; GLB/FBX exported; Cycles preview rendered and inspected.
- Export budget assertions pass. Unreal additionally verified dimensions and materials.
- TypeScript and focused tests cover draft export, lessons, district routing,
  system appearance changes, character coverage and rig binding, grounded movement,
  session failure, active-turn blocking and conversation connection-before-send.
- Standalone forum production bundle built separately from the shipping app.
- No live query was sent during verification. The conversation integration is tested
  with mocks, not an end-to-end model run. Job dispatch is not implemented.
- Browser/native automation remains unavailable (`apps=[]`, `browsers=[]`, native
  pipe startup failed). Browser rendering, responsive visual QA, native WKWebView,
  sustained GPU cost and Unreal play testing remain unverified.


## Current vision and next art pass

[WORLD_VISION.md](WORLD_VISION.md) describes the intended experience: a living civic
garden for intelligence. The central forum is the social hub of a larger campus,
with a garden promenade and four connected destinations: Arrival gardens, Reading
grove, Maker court and Council garden. An upper archive and observatory add vertical
exploration. The 3D asset URL carries a generated content revision so geometry
updates do not silently remain in the browser's old loader cache.

Character source generator: `scripts/blender/build_forum_characters.py`.
Outfits/portraits: `public/world/forum-characters/`; editable source:
`assets/world/forum/characters/`. Personality briefs are design/presentation direction,
not changes to persistent specialist system prompts. The visible voice preference
can accompany a user-initiated query to the orchestrator.

The target Unreal finish is substantially beyond this procedural art pass: authored
stone/metal materials, naturalistic foliage, water reflections, atmospheric depth,
careful lighting composition and human-scale traversal. The current Unreal import
is an executable starting scene, not a claim of finished visual polish. Native
agent interactions and the daemon bridge still need implementation there.
