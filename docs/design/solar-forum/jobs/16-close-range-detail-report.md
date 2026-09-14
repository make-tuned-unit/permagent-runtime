# Job 16 — Close-range detail and light

Date: 2026-09-14  
Scope: owned Blender source modules plus this report; no Blender, cargo, or git state operation performed.

## Changes by file

- `scripts/blender/forum_landform.py`: added deterministic two-octave per-vertex displacement to the ridge strata, moss ledge, and foothill surface; added five linked 16-block scree fans, seven linked half-buried fractured boulders, dark plunge pools, three tilted mist sheets per fall, spray tufts, a 25 m mist bank at each shore, and route-checked island quay bollards, rope rings, crates, and lanterns.
- `scripts/blender/forum_landscape.py`: added route-checked linked harbour bollards, rope coils, crates, amber lanterns, four additional boats with masts, promenade lanterns at roughly 12 m intervals, and Maker/Reading island kiosks with fabric awnings.
- `scripts/blender/forum_towers.py`: added linked warm emissive window strips with alternating floor patterns to the three spires, four habitats, and market hall.
- `scripts/blender/forum_terrace.py`: added three books, two cups, a potted seedling, bronze tablet, two folded sand blankets, faint emissive info-panel line patterns, and the north-gap amber brazier.
- `scripts/blender/forum_meshgate.py`: added horizonBlue chevron insets, two cyan display panels with line patterns, a blue dais inlay ring, four dormant bronze braziers, and route-checked horizonBlue spur lanterns.
- `scripts/blender/forum_source_materials.py`: added the Job 16 material registry so the final source-material manifest records the new close-range palette.

The other allowed modules (`forum_habitat.py`, `forum_celestial.py`, and `forum_civic_planting.py`) were reviewed and did not require edits for this pass.

## Triangle budget estimate

| Art-director item | Estimated delta |
|---|---:|
| 1. Ridge displacement, scree, fractured foothill boulders | ~15,000 |
| 2. Pools, layered mist, spray, shore mist banks | ~3,000 |
| 3. Harbour/island props and additional boats | ~5,000 |
| 4. Emissive windows and promenade/gate lanterns | ~2,000 |
| 5. Terrace human-scale detail | ~500 |
| 6. MESH gate polish | ~3,000 |
| **Estimated total** | **~28,500** |

Using the reported ~761,000-triangle starting point, the expected export is approximately **789,500 triangles**, below the 1,100,000 cap. The estimate counts raw mesh triangles conservatively; linked duplicates reuse mesh datablocks.

## Materials

New Job 16 material names registered by the source-material pass are:

`Forum Harbour Amber`, `Forum Harbour Rope`, `Forum Plunge Pool Dark Water`, `Forum Island Quay Amber`, `Forum Warm Window Emission`, `Forum Info Display Line`, `Forum Brazier Amber Embers`, and `Mesh gate cyan display glass`.

The existing `Celestial waterfall mist` and `Forum Sand Fabric` materials were reused. The waterfall mist alpha is set to 0.27 for the layered sheets and shore banks; gate braziers remain dormant with no emission.

## Completeness and verification

All six numbered art-director items are implemented in the owned source modules. The requested dais floor inlay necessarily lies on the existing MESH walk-through surface axis; it is surface infrastructure, not a freestanding obstruction. All freestanding gate props and spur lanterns remain outside the route corridor and use the shared route guard.

`python3 -m py_compile` passed for all nine allowed Blender Python modules.
