# Job 15 — Planting, foothills, islands, and balcony readability

Date: 2026-09-14  
Branch: `feat/solarpunk-forum`  
Scope: owned Blender source modules only; no Blender run, build, render, or git operation performed.

## Changes

- `forum_landscape.py` now creates three deterministic shared tree meshes,
  each with a trunk and three broad crown layers, plus one shared understory
  shrub mesh. Fourteen dense groves (8–16 linked trees each) are distributed
  across the inner and outer meadow bands. New grove trees, shrubs, hedgerows,
  and garden-bed planting use the segment-aware `_point_close_to_route` guard
  at 4.5 m.
- `forum_landform.py` adds a smooth rolling 118–136 m foothill mesh with
  deterministic low-frequency rolling variation, route-safe foothill grove
  clusters, island shallow-water halos, and a route guard for boats and
  fractured outcrop blocks. Island understory and grove trees reuse the shared
  landscape meshes when available. Terrace planting is also route guarded.
- `forum_habitat.py` assigns the four existing basalt strata the requested
  warm ochre, grey basalt, dark umber, and pale limestone colors.
- `forum_towers.py` changes the NE spire balcony units to 0.35 m slabs with
  alternating 2.0/2.8 m cantilever depth, adds linked planter parapet
  sections, and uses linked trailing understory instances every third balcony.
  The existing spire sky bridges remain intact.

## Budget expectation

The shared tree variant is approximately 84 triangles and the shared shrub is
10 triangles. New linked planting, foothill faces, island halos, and balcony
parapets are expected to add approximately **25,000–35,000 triangles net**
after replacing the prior sparse inner-grove canopy. Starting from the latest
reported 756,060-triangle export, the expected result is approximately
781,000–791,000 triangles, comfortably below the 1,100,000-triangle cap.
Linked duplicates do not multiply mesh datablocks in the exporter.

## Verification

Ran successfully:

```text
python3 -m py_compile scripts/blender/forum_landform.py \
  scripts/blender/forum_landscape.py scripts/blender/forum_towers.py \
  scripts/blender/forum_habitat.py scripts/blender/forum_celestial.py \
  scripts/blender/forum_civic_planting.py
```

No helper signature is known to be unresolved. The only integration assumption
is that `forum_landscape.py` executes before `forum_landform.py` and
`forum_towers.py`, so their linked-tree helpers are available; both consumers
retain a `leaf_spray` fallback for standalone execution.
