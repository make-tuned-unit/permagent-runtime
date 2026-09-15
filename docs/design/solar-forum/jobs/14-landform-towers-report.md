# Job 14 — Landform and towers report

## Final iteration — 2026-09-14

The art-director iteration replaced the stadium-like ridge with nine distinct
sloped skyline peaks (55–75 m), stepped foothill strata, colored basalt bands,
and moss ledges. Waterfalls now have rocky notch spurs, three sub-stream sheets,
splash/mist banks, and channels into the lagoon. The NE spires gained a planted
plaza; west habitats gained 1.5 m floor setbacks, recessed dark loggias,
masonry-tagged shells, columns, and planted green setbacks. Lagoon islands now
have tree/understory planting, pavilions, mooring posts, and four boats. Meadow
planting was increased to 12 groves with hedgerows and terraced beds. A filled,
top-wound lagoon surface plus shallow shore band fixes the former black-void
read in overview renders.

Final scratch verification:

| Measure | Result |
|---|---:|
| FORUM_VERIFIED triangles | 756,060 |
| FORUM_VERIFIED GLB bytes | 54,044,068 |
| Blend bytes | 8,123,376 |
| FBX bytes | 18,361,276 |
| Exported meshes | 36 |
| Celestial triangles | 95,744 |
| Prior route audit | 258 waypoints, 0 bad hits, 0.0429 m max error |

Render paths produced:

- `/tmp/forum-scratch-landform/assets/campus-expanded.webp`
- `/tmp/forum-scratch-landform/assets/geodisc-profile.webp`
- `/tmp/forum-scratch-landform/assets/celestial-habitat.webp`
- `/tmp/forum-scratch-landform/assets/garden-district.webp`

The mandated factory-startup invocations crash in Blender 5.2.1 Metal before
Python (exit 139), so verification and rendering used the established
no-factory startup workaround. `forum_meshgate.py` was skipped in-memory only;
it is outside this assignment and has the known pre-existing line-245 vector
bug. The final standalone route raycast command also hit the host startup crash;
the report carries forward the prior verified 258-waypoint result (0.0429 m).

Date: 2026-09-14  
Branch: `feat/solarpunk-forum`  
Scope: P1 vertical eco-towers/urban rooms, P2 lagoon and landform, P3 celestial layer.

## Follow-up art-direction pass — source update, 2026-09-14

The pass-2 source review exposed that the prior report overstated the visual
result: the authored ridge was still nine cone-like peak meshes on stepped
bands, the NE spires were clustered too tightly, and the waterfall sheets did
not follow a terrain path into the lagoon. The owned source modules were
updated to address those findings:

- `forum_landform.py` now generates one continuous 41×116 polar terrain grid
  over 100–215° and r=128–180, with seven varied gaussian crest bumps,
  smoothstep foothills, fractal/fine noise, notch saddles and sampled rocky
  waterfall spurs. The mesh is split into `Basalt stratum 0..3 ridge` height
  bands plus `Ridge mossy ledge`, with smooth shading, a 40° edge split and
  moss/scree vertex colors. Waterfall sheets use eight terrain-following
  segments and terminate at 9 m splash discs / 12 m mist banks on the lagoon.
- `forum_towers.py` moves the three NE spires to (74,58), (104,86), and
  (122,52), preserving 72/96/118 m heights and adding separation for a
  readable cluster. West habitat floors retain their full street-facing mass,
  while setbacks are applied only to the lagoon-facing edge every third floor
  and roof gardens are explicitly named as planted setbacks.

The required factory-startup build was retried after polling for idle Blender
and exited 139 before Python during Blender 5.2.1 Metal GPU initialization.
The same absolute build without `--factory-startup` also exited 139 at the same
pre-Python stage. Therefore this follow-up has no new `FORUM_VERIFIED` counts,
no new Blender renders, and no new route raycast result. Existing render paths
remain the prior scratch outputs listed above; they must not be treated as
validation of this source update until Blender can launch.

## Built

- Replaced the legacy repeated stepped-pyramid placement with three distinct rooms:
  the NE 72/96/118 m spire cluster with planted balcony tiers, glass fins, crown beacons and two bridge spans; the W four-habitat market cluster with stepped green roofs and two bridges; and the SE 28 m geodesic conservatory dome with linked bronze lattice and eight interior trees. The existing conservatory, maker hall, theatre and harbour remain.
- Final west-habitat tops measure 44.6/38.7/35.7/29.8 m including roof slabs, matching the requested 28–44 m mid-rise range.
- Cut the inner meadow to a 62–118 m lagoon with a water annulus, three destination islands, timber quay segments for the four radial approaches, three planted stepped terraces, and zero-datum route pads.
- Added a grounded fractured N/NW ridge spanning the 140–178 m rim, 55–70 m crest intent, eight fractured crest pinnacles, two vertical waterfalls, and mist beds. Replaced the old evenly spaced boulders with six linked fractured outcrop groups using three shared block meshes.
- Added diagonal inner meadow groves with layered canopy/understory and preserved the shared meadow/strata boundary. Added the second 28° orbital ring, linked tower groups and terrace/soil units on both rings, plus four non-walkable floating habitats (two large, two compact; the compact pair have lagoons, towers, falls and mist).

## Visual-review status

Against `jobs/04-visual-review.md`:

1. Varied massing / connected urban rooms — **Pass.** Legacy pyramid count in the final scratch blend: `0`; three authored clusters are visible in the campus overview.
2. Planting density / layered canopy — **Pass for this geometry pass.** Inner groves, terrace planting, balcony vegetation, dome forest and floating-island planting are authored as layered beds/canopies. The close commons view retains dense planted furniture-scale foreground.
3. Fractured outcrops / coherent geology — **Pass.** Six outcrop groups replace the old smooth perimeter placement; the ridge adds fractured crest pinnacles.
4. Meadow/strata shell gaps and winding — **Pass.** Shared boundary maximum distance is `0.0 m`; `160/160` checked first-stratum faces have positive outward radial winding. The final geodisc profile shows no open black join gaps.
5. Empty lawns / flat surfacing — **Pass with stylized source materials.** The lagoon, destination courts, three terraces, inner grove beds, planted islands and clustered rooms break the former empty disc. The campus render reads as a lagoon city rather than eight isolated copies.
6. Rings / distant habitats / waterfalls / celestial depth — **Pass for Blender geometry.** Two rings are present; the second is `28°`; both have linked tower groups and planted terraces; four floating habitats are present; the campus ridge and floating habitats carry waterfall/mist geometry. Browser gas-giant/moon lighting remains separate worker scope.

## Verification

Final scratch manifest:

| Measure | Result |
|---|---:|
| Exported meshes | 34 |
| Triangles | 713,860 |
| GLB bytes | 51,419,788 |
| Geodisc diameter | 358.79 m |
| Celestial triangles | 95,744 |
| Route waypoints | 258 |
| Route bad hits | 0 |
| Max route surface error | 0.0429 m |

The builder printed `FORUM_VERIFIED` and passed `triangles < 1,200,000` and `bytes < 95,000,000`. Linked-data checks in the final blend passed for spire balcony units, first/second orbital tower groups, first/second orbital terrace panels, conservatory lattice tubes and fractured outcrop blocks (`unique_data=1` within each repeated group). The shared meadow/strata boundary and outward winding checks are listed above.

Scratch artifacts:

- Blend: `/tmp/forum-scratch-landform/assets/solar-forum.blend`
- GLB: `/tmp/forum-scratch-landform/web/solar-forum.glb`
- Manifest: `/tmp/forum-scratch-landform/web/solar-forum.manifest.json`
- Campus: `/tmp/forum-scratch-landform/campus-expanded.webp`
- Profile: `/tmp/forum-scratch-landform/geodisc-profile.webp`
- Celestial: `/tmp/forum-scratch-landform/celestial-habitat.webp`
- Commons acceptance view: `/tmp/forum-scratch-landform/commons-detail.webp`
- Additional dome QA: `/tmp/forum-scratch-landform/garden-district.webp`, `/tmp/forum-scratch-landform/conservatory-walk.webp`

## Commands

Syntax check:

```text
python3 -m py_compile scripts/blender/forum_towers.py scripts/blender/forum_landform.py scripts/blender/forum_landscape.py scripts/blender/forum_habitat.py scripts/blender/forum_celestial.py scripts/blender/forum_civic_planting.py
```

The requested factory-startup command was attempted with the absolute paths. This host's Blender 5.2.1 build crashes in Metal initialization before Python (`GPU_backend_type_selection_detect`); the same happens when any shell environment assignment is injected. The successful dry run used the same absolute builder code, set `FORUM_SCRATCH=/tmp/forum-scratch-landform` inside Blender Python, and skipped only the optional `forum_meshgate.py` via an in-memory `Path.exists` override. The terrace module remained enabled. No shipped asset was written by these runs.

The final dry run used:

```text
/Applications/Blender.app/Contents/MacOS/Blender --background --python-expr "import os,sys; from pathlib import Path; os.environ['FORUM_SCRATCH']='/tmp/forum-scratch-landform'; _exists=Path.exists; Path.exists=lambda self: False if self.name=='forum_meshgate.py' else _exists(self); sys.argv.append('--no-render'); _p='/Users/j/Documents/dev/permagent-runtime/recovery/solar-forum/scripts/blender/build_solar_forum.py'; globals()['__file__']=_p; exec(compile(open(_p).read(),_p,'exec'))"
```

Final source renders were run from the scratch blend with the camera definitions from `scripts/blender/render_forum_source.py`, using host-compatible `BLENDER_EEVEE` and scratch-root output paths. A separate Blender raycast script checked all `patrolRoutes.json` waypoints at their authored elevations; all `258` passed.

## Deviation / follow-up

`forum_meshgate.py` exists but is outside this assignment and currently fails at line 245 by adding its 3D `gate_normal` to a 2D `gate_center_xy`. It was suppressed only for scratch P1–P3 verification; the gate worker should fix that module before a full builder run including P4. The factory-startup/Metal and shell-environment workarounds are host execution constraints, not scene changes.
