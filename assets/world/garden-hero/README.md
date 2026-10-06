# World hero kit — 2026-10-06

Asset-only lane. Integration runtime lives separately in `/Users/j/Documents/dev/permagent-worktrees/world-production-20261006`; this lane does not edit it.

`exports/hero-kit.glb` contains ten top-level roots, all at their own floor origin: `robot`, `canopy`, `shrub`, `fern`, `vine`, `flowers`, `table`, `bench`, `planter`, `instrument`. Load the scene as a library and clone individual roots; displaying the whole scene at one origin overlaps assets. Individual GLBs are also provided. Do not apply the complete scene as an environment replacement.

Metre scale, glTF +Y up; robot and reader face +Z. Blender source uses +Z up and faces −Y. Robot height approximately 1.76 m. Table radius 1.12 m, top height .82 m. Bench width 1.62 m. Planter lip height .67 m, soil height .575 m. Vine attachment is its top at 1.60 m above the origin; floor origin is retained for consistency. Canopy spans roughly 3–4 m.

Robot's `Head`, `Arm.L`, `Arm.R`, `Leg.L`, `Leg.R` are independent rigid pivots. Parent these into the existing animation/state binding. No skeleton or baked clips; geometry does not invent work status. Limbs are intentionally separated at dark joints. Variant tinting can target ceramic and eye materials without recoloring dark faceplate/brass.

Static geometry is merged by material within each root. Robot geometry is merged within each pivot/material. Leaf geometry is opaque, curved, two sided visually without alpha sorting; mesh triangles have no thickness. Use double-sided leaf materials in the runtime if necessary for under-canopy views. Roots can be instanced in runtime; the GLB does not prescribe scene placement or collision.

Collision recommendations: robot capsule radius .32 m and height1.76m; canopy trunk capsule radius .16m (crowns nonblocking); shrubs/fern/vine/flowers nonblocking placement excluded from walk corridors; table cylinder radius1.12m/height.82m; bench box1.62m×.55m×1.02m; planter cylinder radius.55m/height.67m; reader nonblocking tabletop prop. These are recommended runtime proxies, not collision meshes embedded in GLB. Keep all inhabited landscape traversable.

Source and provenance: authored locally with Blender scripts using `docs/design/solar-forum/north-star.png` as art direction. No third-party mesh, purchased content, cloud GPU, or external texture dependency. Material maps are locally authored procedural patterns without baked lighting. Packed albedo, roughness and tangent normal PNG maps have 256px resolution; exact decoded budget and export hashes are in `manifest.json`. Fine leaf/bark surface detail is supplementary; shape and lighting must remain readable at distance.

Original base generation (follow the current geometry-only reproduction below to recreate v2):

```sh
KIT_EXPORT_ONLY=1 /Applications/Blender.app/Contents/MacOS/Blender --background --threads 4 --python source/build_kit.py
```

`source/hero-kit.blend` is the editable **v1** unassembled asset library; it retains the original robot. `source/robot-v2.blend` is the authoritative editable revised robot, superseding the robot in that library. `source/robot-v1.glb` preserves the input for reproducible revision. Current mono GLB is assembled from the individual exports and includes robot v2. `enrich_review.py` is prepared but unfinished/unexecuted to completion due the resource pause; current GLBs have no texture maps. Do not run it as part of current accepted reproduction.

Current geometry-only reproduction is `build_kit.py` with `KIT_EXPORT_ONLY=1`, then `batch_glb.py`, then `revise_robot.py`, then `assemble_kit.py`, then `validate_glb.py`. `revise_robot.py` writes its source/GLB and runs one small robot diagnostic. Heavy steps require the parent's resource clearance. `run_diagnostic_bounded.py revise_robot.py` supervises a 90-second / 1-GiB RSS / 3-GiB free-disk cap. The separate foliage diagnostic additionally requires over 5 GiB free before starting. No full-kit Blender import is required for assembly.

`previews/robot-v2-export-diagnostic-512.png` and `previews/foliage-v1-export-diagnostic-512.png` are versioned CPU Cycles four-sample, 512px diagnostics of the actual exported individual GLBs. The original v1 robot preview was overwritten before versioned naming was adopted and is not retained as before/after evidence. Workbench headless aborted before image output; CPU Cycles diagnostic succeeded. These images verify shape/transforms only at draft material quality, not runtime performance or final hero acceptance. Runtime acceptance requires parent review in the actual scene.

## Repository checkpoint

This directory preserves the editable library, revised robot, source input and
reproduction scripts. Run from this directory with the installed Blender path;
`source/build_kit.py` recreates the initial kit, `source/batch_glb.py` batches it,
`source/revise_robot.py` rebuilds the revised robot from `source/robot-v1.glb`,
and `source/assemble_kit.py` makes the mono kit. `source/validate_glb.py` performs
bounded binary checks. Outputs go to `exports/` and remain generated assets.
Do not run generation until the coordinated memory/disk slot is available.

The browser's staged mono output is
`ui/command-center/public/world/garden-hero/hero-kit.glb` in the repository.
Its normalized public manifest marks this a draft, with visual acceptance and
hero texture enrichment pending. Geometry validation and offline diagnostics do
not establish runtime quality, FPS, full walkability or packaged delivery.
