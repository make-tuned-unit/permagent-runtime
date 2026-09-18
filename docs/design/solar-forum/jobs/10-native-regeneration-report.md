# Job 10 — native Unreal regeneration report

Date: 2026-09-14

Branch: `feat/solarpunk-forum`

Export: `assets/world/forum/solar-forum.fbx` (current worktree export)

Engine: Unreal Engine 5.8.2 / Metal

## Result

The native pipeline completed end to end for the current export. Import/build,
day/night native material polish, day/night inhabitant authoring, day/night
atmosphere, and day/night offscreen rendering all produced their own complete
artifacts. The renders are technically verified, but the seven-item visual
target in Job 04 remains materially unmet; this is a pipeline proof, not a
north-star approval.

## Inherited process audit

The requested inherited-process checks were attempted first. `pgrep -fl
UnrealEditor-Cmd` and the related `pgrep -af` check failed because this managed
environment cannot access the macOS process service (`sysmond service not
found` / `pgrep: Cannot get process list`). `ps -axo ...` was also blocked with
`operation not permitted`. The PID present in the inherited import log was
checked directly with `kill -0 24508` and was absent. No current Unreal process
was therefore running to kill, and the inherited wrapper was not active.

The inherited `unreal-import.log` was 494 bytes and unchanged at the check;
its complete tail was:

```text
2026-09-14 12:10:38.656 UnrealEditor-Cmd[24508:223294] Failure on line 688 in function id scheduleApplicationNotification(LSNotificationCode, NSWorkspaceNotificationCenter *): noErr == _LSModifyNotification(notificationID, 1, &code, 0, NULL, NULL, NULL)
2026-09-14 12:10:38.913 UnrealEditor-Cmd[24508:223316] Connection Invalid error for service com.apple.hiservices-xpcservice.
2026-09-14 12:10:38.913 UnrealEditor-Cmd[24508:223294] Error received in message reply handler: Connection invalid
```

`unreal-import-manual.log` was empty and no newer project log was found in
`~/Library/Logs/Unreal Engine/SolarForumEditor/`. The documented wrapper was
rerun unchanged first; it progressed into normal Unreal initialization and
completed, so `-NoSourceControl` was not added. The existing
`-unattended -NullRHI` flags were sufficient for headless import/build.

## Exact pipeline commands, durations, and evidence

Durations below are the observed wrapper durations where the wrapper returned;
render durations are the exact `elapsedSeconds` values written by the render
JSON reports. Every command was run sequentially; no two Unreal processes were
launched concurrently.

| Step | Exact command | Duration | Result and verification artifact |
| --- | --- | ---: | --- |
| Import/build | `./scripts/unreal/build-forum.sh` | 52.6 s | PASS — `unreal-import.json`: `status=complete`, `diameterCm=44830.193359375`, 12 material slots, `BS_UP_TO_DATE`; `unreal-import.log` has normal Unreal shutdown. |
| Polish day | `./scripts/unreal/polish-forum.sh day` | 150.2 s until wrapper stop | PASS at Unreal/report level — `unreal-environment-day.json` was complete and `unreal-environment-day.log` ended after Unreal shutdown. The wrapper shell itself remained open after Unreal PID 68897 had exited and was stopped with Ctrl-C (exit 130) after repeated no-growth polls. |
| Polish night | `./scripts/unreal/polish-forum.sh night` | 25.5 s | PASS — `unreal-environment-night.json`, `status=complete`, `ForumNight`, 249 environment actors. |
| Inhabit day | `./scripts/unreal/inhabit-forum.sh day` | 24.2 s | PASS — `unreal-inhabit-day.json`, `status=complete`, 12 inhabitants with meshes and idle/walk animations. |
| Inhabit night | `./scripts/unreal/inhabit-forum.sh night` | 22.0 s | PASS — `unreal-inhabit-night.json`, `status=complete`, 12 inhabitants with meshes and idle/walk animations. |
| Atmosphere day | `./scripts/unreal/atmosphere-forum.sh day` | 18.1 s | PASS — `unreal-atmosphere-day.json`, `status=complete`, 25 lanterns, `stars=false`, 85,000 lux. |
| Atmosphere night | `./scripts/unreal/atmosphere-forum.sh night` | 21.2 s | PASS — `unreal-atmosphere-night.json`, `status=complete`, 25 lanterns, `stars=true`, 8 lux. |
| Render day, initial | `./scripts/unreal/render-forum.sh day` | 247.938 s | PASS — five PNGs, complete render JSON, non-trivial files, no material-compile or Python traceback guard failures. |
| Render night, initial | `./scripts/unreal/render-forum.sh night` | 257.549 s | PASS — six PNGs, complete render JSON, non-trivial files, no material-compile or Python traceback guard failures. |
| Render report fix | `python3 -m py_compile scripts/unreal/render_forum.py && zsh -n scripts/unreal/render-forum.sh && git diff --check -- scripts/unreal/render_forum.py scripts/unreal/render-forum.sh` | 0.2 s | PASS — minimal in-scope fix added post-render synchronization of `renderVerified` and `renderFiles` into the mode-specific environment report. |
| Render day, final rerun | `./scripts/unreal/render-forum.sh day` | 248.792 s | PASS — final day render and `renderVerified=true`. |
| Render night, final rerun | `./scripts/unreal/render-forum.sh night` | 246.838 s | PASS — final night render and `renderVerified=true`. |

The polish-day wrapper anomaly is the only abnormal command return. Its
verbatim log tail was:

```text
[2026.09.14-15.22.50:511][  2]LogAssetRegistry: Display: CleanupOrphanedCacheFiles: 1 total binaries, 1 referenced (kept), 0 old-style pre-migration (kept), 0 orphans deleted, 0 orphans locked (kept)
[2026.09.14-15.22.51:076][  4]LogShaderCompilers: Display: Exiting ShaderCompilingThread
2026-09-14 12:22:52.251 UnrealEditor-Cmd[68897:316208] [UE4] Shutdown handler: cleanup.
```

There was no pipeline failure tail after the reruns. Render logs passed the
wrapper guards: neither `Failed to compile Material` nor `Traceback (most
recent call last)` occurred. Repeated `idevice_id` bad-CPU warnings, a
collision-geometry triangle-count warning, and one non-fatal Google connectivity
timeout were environment warnings, not step failures.

## Native material assignment counts

Counts below are pulled from both mode-specific JSON reports, not inferred from
the images. Day and night are identical and each has 17 explicit assignments
across 7 native categories; each report also records 249 environment actors and
14 reset imported slots.

| Native category | Day count | Night count |
| --- | ---: | ---: |
| `burnished bronze` | 1 | 1 |
| `scannedstone` | 3 | 3 |
| `basalt` | 1 | 1 |
| `living water` | 2 | 2 |
| `architectural glass` | 2 | 2 |
| `original warm timber tone` | 2 | 2 |
| `scannedrock` | 6 | 6 |
| Total explicit assignments | 17 | 17 |

## Render verification

Final mode reports now contain `renderVerified: true` and the exact rendered
file list. The render reports independently contain `status: complete`, the
renderer identifier `Unreal Engine / Metal`, and 64 capture frames per view.

Day files:

- `assets/world/forum/unreal/unreal-forum-day.webp` — 3,090,912 bytes
- `assets/world/forum/unreal/unreal-walk-day.webp` — 3,090,912 bytes
- `assets/world/forum/unreal/unreal-campus-day.webp` — 2,244,875 bytes
- `assets/world/forum/unreal/unreal-sovereign-day.webp` — 3,090,912 bytes
- `assets/world/forum/unreal/unreal-conservatory-day.webp` — 439,995 bytes

Night files:

- `assets/world/forum/unreal/unreal-forum-night.webp` — 3,090,912 bytes
- `assets/world/forum/unreal/unreal-walk-night.webp` — 3,090,912 bytes
- `assets/world/forum/unreal/unreal-campus-night.webp` — 2,244,875 bytes
- `assets/world/forum/unreal/unreal-sovereign-night.webp` — 3,090,912 bytes
- `assets/world/forum/unreal/unreal-galaxies-night.webp` — 1,173,227 bytes
- `assets/world/forum/unreal/unreal-conservatory-night.webp` — 439,995 bytes

## Visual comparison with the north star

Reference: `docs/design/solar-forum/north-star.png`. The comparison below uses
the final rerendered PNGs and the seven criteria in `jobs/04-visual-review.md`.

1. **Varied massing and connected urban rooms — NOT MET.** The central forum
   has a stronger connected colonnade, but the aerial view still reads as a
   ring of repeated stepped-pyramid buildings. Compare
   `unreal-campus-day.webp` and `unreal-walk-day.webp` with the north-star's
   varied towers and clustered habitats.

2. **Full, layered planting — PARTIAL / NOT MET.** `unreal-forum-day.webp`,
   `unreal-walk-day.webp`, and `unreal-forum-night.webp` show real tree canopies,
   planters, and underplanting around the fountain. The campus overview still
   has sparse, thin perimeter planting and large unplanted intervals, unlike
   the reference's continuous lush canopy.

3. **Coherent fractured geology — NOT MET.** `unreal-campus-day.webp` and
   `unreal-campus-night.webp` visibly show smooth boulders placed at regular
   intervals around the rim. The north star calls for continuous eroded
   outcrops rather than a prop-like perimeter rhythm.

4. **Closed shell with no black boundary openings — NOT MET.** The aerial day
   and night renders expose large dark/blue voids between the land layers and
   below the ring/habitat structures; see `unreal-campus-day.webp` and
   `unreal-campus-night.webp`. These are visibly inconsistent with the closed,
   continuous shell expected by Job 04.

5. **Destination clusters, constructed ground detail, and scaled material
   variation — NOT MET.** `unreal-forum-day.webp` and `unreal-walk-day.webp` have
   a credible fountain, benches, drains, and paving seams, but the foreground
   is still a broad pale, repetitive surface with large low-detail areas. The
   aerial `unreal-campus-day.webp` makes the empty lawns and flat layout most
   apparent.

6. **Monumental rings, distant habitats, waterfalls, celestial depth, and warm
   contrast — PARTIAL / NOT MET.** `unreal-campus-day.webp` includes the large
   ring, floating habitat plates, and hanging waterfall-like forms;
   `unreal-galaxies-night.webp` proves stars and a galaxy element, while
   `unreal-campus-night.webp` proves the ring silhouette at night. The result
   lacks the north star's planetary scale, layered atmospheric depth, luminous
   waterfalls, and warm golden celestial contrast; the night overview is also
   substantially too dark.

7. **Foreground agent gathering close pass — PARTIAL / NOT MET.**
   `unreal-walk-day.webp` and `unreal-sovereign-day.webp` show the fountain,
   benches, planters, and multiple native agent meshes; their night equivalents
   show lantern-lit foliage and the authored gathering route. The stills do not
   reach the reference's intimate furniture density, holographic civic table,
   expressive agent finish, or visible animation, and
   `unreal-conservatory-day.webp` / `unreal-conservatory-night.webp` are nearly
   black captures that need a dedicated lighting/camera pass.

## What a rerun on a new export requires

1. Produce the new `assets/world/forum/solar-forum.fbx` and update the current
   world manifest's campus diameter if the export scale changes. This report's
   run did not invoke Blender; the separate richer scene pass can replace only
   the export inputs before the native sequence.
2. From the repository root, run the wrappers in this order, one at a time:
   `./scripts/unreal/build-forum.sh`,
   `./scripts/unreal/polish-forum.sh day`,
   `./scripts/unreal/polish-forum.sh night`,
   `./scripts/unreal/inhabit-forum.sh day`,
   `./scripts/unreal/inhabit-forum.sh night`,
   `./scripts/unreal/atmosphere-forum.sh day`,
   `./scripts/unreal/atmosphere-forum.sh night`,
   `./scripts/unreal/render-forum.sh day`, and
   `./scripts/unreal/render-forum.sh night`.
3. Keep `FORUM_ENGINE` pointed at the installed UE 5.8 directory when the
   default path is unavailable. The wrappers already provide unattended,
   headless/null-RHI flags for asset steps and offscreen capture for renders;
   this run did not need `-NoSourceControl`.
4. After every step, require the mode-specific JSON `status=complete`, a
   completion/shutdown log tail, and non-trivial output files. The render script
   now also synchronizes `renderVerified` and `renderFiles` into the matching
   environment report, so a rerun leaves a self-consistent audit trail.
5. Do not start a second Unreal process while one wrapper is active. If a
   future startup attempt reproduces the inherited three-line macOS stall,
   inspect log growth and process liveness for 10 minutes before terminating
   only the matching Unreal process and wrapper shell.
