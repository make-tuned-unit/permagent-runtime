# Civic planting replacement report

Implemented [`scripts/blender/forum_civic_planting.py`](../../../../scripts/blender/forum_civic_planting.py), a bounded module to run after `forum_landscape.py` and `forum_celestial.py`, before source materials/save/export.

The current `browser/solar-forum-day.webp` shows the commons surrounded by
large faceted green canopy balls. The module removes only the authored coarse
proxy groups below, records their exact name, material, vertex count, triangle
count, dimensions and location in `civic_planting_manifest`, then creates
rooted replacements using the existing improved `leaf_spray` and `branch`
helpers.

| Removed authored group | Source geometry evidence | Expected source count | Replacement |
| --- | --- | ---: | --- |
| `Olive canopy` | Icosphere, radius 1, scaled about 2.2 × 1.7 × 1.3 m; `Olive leaves` / `Sunlit foliage` | 42 | Four anchored olive clumps plus understory per planter tree |
| `<court> living canopy` | Icosphere, radius 1.1, about 2.2 m diameter; alternating leaf materials | 84 | Rooted destination canopy and understory clumps |
| `Promenade living edge` | Icosphere, radius .95, about 1.9 m diameter; `Sunlit foliage` | 38 | Compact stem, layered edge clump and planter understory |
| `Gallery foliage` | Icosphere, radius .48, about .96 m diameter; `Sunlit foliage` | 15 | Small anchored gallery clump and understory |

The expected total is 179 removed proxy objects from the current builder loops.
The `.8 m` maximum-dimension gate preserves the smaller `Herb garden`
(`.6 m`), `Hanging vine` (`.28 m`), and all `Pollinator garden` and water
objects. Authored `Garden planter`, `Living soil`, existing trunks, paving,
routes and source environment identity remain in place.

Replacement foliage is built through mesh data only. Each removed civic canopy
gets broad folded leaves with varied orientation and size from `leaf_spray`, a
short connecting twig, and a smaller low clump. Promenade replacements use a
compact `.66 m` edge radius and `.34 m` understory radius inside the existing
`.7 m` planter footprint to avoid walkway encroachment. New geometry is tagged
through the existing `finish()` helper and the module asserts fewer than
100,000 added triangles. The current replacement recipe is approximately
38,000 triangles before runtime measurement.

## Checks

Ran:

```text
python3 -m py_compile scripts/blender/forum_civic_planting.py
```

The module also calls `bpy.context.view_layer.update()` before dimension-based
selection and after replacement, uses no `bpy.ops`, and publishes the exact
runtime removal/replacement evidence for root review. Blender, native tools and
exports were not launched for this bounded job.

## Integration

Execute this module after the existing celestial module and before
`forum_source_materials.py` / source save:

```python
exec(compile((ROOT / 'scripts/blender/forum_civic_planting.py').read_text(),
             str(ROOT / 'scripts/blender/forum_civic_planting.py'), 'exec'))
```

Root should inspect the real browser close view and confirm that faceted
canopies are gone, trunks remain visibly connected, planter bounds and routes
stay clear, and `civic_planting_manifest['added_triangles']` passes its gate.
