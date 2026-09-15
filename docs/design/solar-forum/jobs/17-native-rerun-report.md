# Job 17 — native Unreal rerun report

Date: 2026-09-14

Branch: `feat/solarpunk-forum`, head `1af950d656e492fb8418642c7bcd9c7cfc9818a9` (short `1af950d6`)

Export: `assets/world/forum/solar-forum.fbx` (17 MB, regenerated 2026-09-14)

Engine: Unreal Engine 5.8.2 / Metal

## Result

**Blocked at import/build.** The new FBX was not opened by Unreal: both bounded launches stalled before normal Unreal initialization, Python execution, or the completion JSON write. No downstream day/night polish, inhabit, atmosphere, or render step was started. Existing WebP files remain prior-run evidence and are not relabeled as renders of this export.

The required `pgrep -fl UnrealEditor-Cmd` check was run before each attempted editor launch and before the evidence-shrink step. On this managed macOS host it consistently returned `sysmon request failed: sysmond service not found` and `pgrep: Cannot get process list`. No competing process was observable through that required check, and each timed-out editor process was absent afterward. Free disk was 14 GB before the first attempt and 30 GB afterward, never below the 8 GB stop threshold.

## Commands and outcomes

| Step | Exact command | Duration | Result |
| --- | --- | ---: | --- |
| Import/build attempt 1 | `./scripts/unreal/build-forum.sh` under `perl -e 'alarm 300; exec @ARGV'` | 300.00 s | FAIL/TIMEOUT; `unreal-import.json` remains `{"status": "pending"}` |
| Import/build attempt 2 | `UnrealEditor-Cmd ... -ExecutePythonScript=.../import_forum.py -NoSourceControl -unattended -NullRHI -nosplash -nosound -stdout` under `perl -e 'alarm 180; exec @ARGV'` | 180.01 s | FAIL/TIMEOUT; completion report still pending |
| Polish day/night | Not run | — | BLOCKED by import |
| Inhabit day/night | Not run | — | BLOCKED by import |
| Atmosphere day/night | Not run | — | BLOCKED by import |
| Render day/night | Not run | — | BLOCKED by import |
| PNG → WebP shrink | `python3 scripts/blender/shrink-evidence.py` | 0.30 s | PASS/no-op; 0 PNGs converted, 0 docs rewritten |

The verbatim tail of both Unreal launch logs was:

```text
2026-09-14 20:47:38.662 UnrealEditor-Cmd[67696:2473283] Failure on line 688 in function id scheduleApplicationNotification(LSNotificationCode, NSWorkspaceNotificationCenter *): noErr == _LSModifyNotification(notificationID, 1, &code, 0, NULL, NULL, NULL)
2026-09-14 20:47:38.908 UnrealEditor-Cmd[67696:2473491] Connection Invalid error for service com.apple.hiservices-xpcservice.
2026-09-14 20:47:38.908 UnrealEditor-Cmd[67696:2473283] Error received in message reply handler: Connection invalid
```

The second attempt has the same three-line tail with timestamp `20:53:00` and PID `75862` in `unreal-import-manual.log`. There is no Unreal Python traceback, mesh validation, or import completion line in either log.

## Material mapping audit

The partial `scripts/unreal/polish_forum.py` edits were retained and extended to cover the new names. The matcher now routes `Forum Hologram*`, hologram light cones, horizon-blue/cyan gate displays, and unlit channels to `M_EmissiveCyan`; emissive window strips plus amber lantern/core/head variants to `M_EmissiveAmber`; and all explicit glass slots, including mesh-gate display glass, to translucent `M_ArchitecturalGlass`. Oiled timber and basalt-strata names retain native mappings.

Because import never loaded the new mesh, **fresh assignment counts are not observable**:

| Native category | New rerun count |
| --- | ---: |
| `architectural glass` | not observed |
| `emissive amber` | not observed |
| `emissive cyan` | not observed |
| all other native categories | not observed |
| Total explicit assignments | not observed |

The 17 assignments recorded in the existing day/night environment JSON files are from the previous export and must not be used as Job 17 evidence.

## Render verification and paths

Fresh `renderVerified`: **not reached**. Fresh PNG count: **0**. The existing prior-run paths are:

- `assets/world/forum/unreal/unreal-forum-day.webp`
- `assets/world/forum/unreal/unreal-walk-day.webp`
- `assets/world/forum/unreal/unreal-campus-day.webp`
- `assets/world/forum/unreal/unreal-sovereign-day.webp`
- `assets/world/forum/unreal/unreal-conservatory-day.webp`
- `assets/world/forum/unreal/unreal-forum-night.webp`
- `assets/world/forum/unreal/unreal-walk-night.webp`
- `assets/world/forum/unreal/unreal-campus-night.webp`
- `assets/world/forum/unreal/unreal-sovereign-night.webp`
- `assets/world/forum/unreal/unreal-galaxies-night.webp`
- `assets/world/forum/unreal/unreal-conservatory-night.webp`

They are unchanged and remain comparable only to the previous export, not to the new FBX or `docs/design/solar-forum/north-star.png`.

## Top remaining native gaps

After the launch issue is repaired, compare new day/night captures against the north star for: varied connected massing and towers; continuous layered planting; fractured continuous geology; a closed shell without boundary voids; constructed ground detail and material variation; stronger monumental scale, waterfall/celestial depth, and warm night contrast; and a denser foreground agent-gathering pass. The conservatory camera/lighting and the new hologram, window-strip, lantern, and mesh-gate presentation also require fresh visual QA.

## Recovery condition

Rerun from `./scripts/unreal/build-forum.sh` once the managed Unreal launch stall is resolved. Do not treat the current mode JSON files as new-export evidence until import, both polish modes, both inhabit modes, both atmosphere modes, and both renders have overwritten them successfully.

## Completed run (root session, outside the Codex sandbox)

The launch stall above is specific to the sandboxed worker: the same wrappers
ran to completion when launched directly by the root session on 2026-09-14
evening, with other sessions' builds paused. Every editor commandlet logs its
shutdown handler and then hangs on exit; a watchdog (`/tmp/forum-unreal-hang-watchdog.sh`)
kills an editor once its mode JSON reports `complete` and the log shows the
shutdown handler, which is why the polish steps report a non-zero wrapper exit
despite a complete result.

| Step | Wrapper exit | Duration | Mode JSON |
| --- | --- | --- | --- |
| import / build | 0 | ~3 min | `unreal-import.json` complete, 58,229 cm diameter, blueprint up to date |
| polish day | 1 (hung editor killed) | 41 min incl. hang | `unreal-environment-day.json` complete |
| polish night | 1 (hung editor killed) | 2 min | `unreal-environment-night.json` complete |
| inhabit day / night | 0 / 0 | 23 s / 15 s | complete |
| atmosphere day / night | 0 / 0 | 17 s / 12 s | complete |
| render day / night | 0 / 0 | 267 s / 247 s | 11 + 6 captures, shrunk to WebP |

Renders: `assets/world/forum/unreal/unreal-*-{day,night}.webp` (new export,
revision bf06eb363adbe7d7). Native gaps seen in the day forum capture: the
sand-fabric sofas map to a dark material, the terrace glass balustrade renders
as opaque cyan, and the displaced ridge behind the commons reads as a black
wall with emissive cracks; all three are `polish_forum.py` slot-mapping work.

## Slot mapping fixes (pending rerun)

`polish_forum.py` now creates deterministic, idempotent native materials and
resolves slots by normalized material name rather than slot index:

- `M_SandFabric` — `Forum Sand Fabric`; `Forum Oiled Timber` remains on
  `M_OriginalWarmTone`.
- `M_HologramGlass` — `Forum Hologram Glass` and mesh-gate cyan display glass;
  translucent cyan-tinted glass at opacity 0.35 with faint cyan emission.
- `M_Hologram`, `M_HologramContinents`, and `M_HologramLightCone` — their
  corresponding hologram slots at authored translucent opacities and cyan
  emission strengths.
- `M_EmissiveAmber` and `M_EmissiveAmberStrong` — Job16 warm window strips,
  amber lantern/core/head variants, strand/brazier/quay amber, and lantern
  heads; strengths retain the authored 3.5–8 range.
- `M_EmissiveHorizonBlue` — MESH gate `horizonBlue` inlay/channel and unlit
  channels, colour (0.33, 0.60, 1.0), emission strength 1.5.
- `M_BasaltStratum0` through `M_BasaltStratum3` — the four basalt stratum
  slots with authored ochre, grey basalt, umber, and limestone colours;
  `M_MossyRock` remains on `Ridge mossy ledge` and mossy coastal rock.
- `M_CelestialWaterfallMist` — `Celestial waterfall mist`, translucent
  white-cyan at opacity 0.30. The ridge cracks are authored moss vertex-colour
  variation, not an emissive rock mapping.

The mode-specific JSON report now records both all defaulted slots and
`unmappedForumSlots` so the next completed rerun can verify zero unmapped forum
slots.
