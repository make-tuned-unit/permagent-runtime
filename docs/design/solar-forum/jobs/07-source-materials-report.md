# Source PBR material report

Implemented [`forum_source_materials.py`](../../../../scripts/blender/forum_source_materials.py)
for execution in the builder globals after `forum_celestial.py` and before the
source save/export section. The module uses only the already downloaded CC0
Poly Haven files recorded in `docs/design/solar-forum/environment-sources.json`.
It does not download files, generate bitmap assets, edit the main builder, or
change geometry.

The module creates deterministic object-space UVs where one UV unit represents
the configured metre tile size after object scale. The named `ForumSurfaceUV`
layer is marked active/render and each material uses an explicit `ShaderNodeUVMap`
reference, so the intended channel survives GLTF export. It uses data API mesh
and node operations, with no per-object `bpy.ops` calls. New materials are added to
the builder's shared `materials` list so the existing GLB/FBX batching loop
exports them.

## Assignments

- `Forum White Sandstone Masonry`, with white sandstone diffuse and roughness
  maps, tile size 2.8 m: objects containing `Workshop coursed stone`,
  `Arrival tower`, or one of the four habitat foundation names (`Archive
  gardens`, `Research studios`, `Civic residences`, `Learning ateliers`).
  Finely modeled paving, terraces, civic slabs and solar accents keep their
  original pale materials.
- `Forum Mossy Coastal Rock`, with mossy-rock diffuse and roughness maps, tile
  size 3.6 m: objects containing `Eroded coastal outcrop`.
- `Forum Basalt stratum 0` through `Forum Basalt stratum 3`, with the
  rock_09 diffuse map and authored per-stratum roughness value, tile size 5.5
  m: objects whose original material is the matching basalt stratum. This
  preserves the four authored geological colour bands while adding readable
  mineral variation.
- `Forum Oiled Timber Grain`, with the original Oiled structural timber base
  tone and roughness plus a fine directional Wave/Noise grain and restrained
  source-only bump, tile size 1.35 m: `Harbor boardwalk`, `Theatre timber
  canopy`, `Workbench top`, `Commons bench timber slat`, `Solar shading
  louvre`, and `Overlook seat slat`. Small branches and foliage stems retain
  the authored bark material so the structural beams do not become a wall of
  bark.
- `Forum Architectural Glass`, using Principled transmission weight 0.55,
  IOR 1.45, low roughness, and a restrained dielectric tint, tile size 2.4 m:
  `Glazed vault panel` and `Recessed atelier glazing`. The photovoltaic jade
  material remains untouched as an authored solar accent.

All diffuse maps use sRGB and all roughness maps use Non-Color. Loaded masonry
and rock images are downscaled in memory to a 1024-pixel maximum; source files
remain unchanged. Timber grain is procedural and source-only, so it adds no
bitmap payload. The final GLB byte size must be measured by the root export
because Blender's image encoding and deduplication determine the actual
payload.

The supplied sandstone and mossy-rock normal files are DirectX normals. They
are intentionally omitted rather than connected with an incorrect green
channel convention. Existing source-only procedural bump nodes remain on the
original stone materials that are not reassigned. The new timber Wave/Noise
grain and bump are source-render detail; GLTF retains the authored timber
base-colour and roughness factors, while the browser runtime helper supplies
its own deterministic fine variation.

## Check

Ran:

```text
python3 -m py_compile scripts/blender/forum_source_materials.py
git diff --check -- scripts/blender/forum_source_materials.py
```

Both checks passed. Blender was not launched for this bounded job. The root
integration should execute the module in the documented builder position, run
the export, inspect the `forum_source_materials_manifest` printed by the
module, measure the GLB size/triangle budget, and render close and overview
views to confirm that the textured masonry and geology remain selective and
that timber and glass read at human height. This source pass improves material
readability; it does not establish photorealism by itself.

## Integration corrections

Actual Blender execution found mismatched return unpacking (helper returns two values, four callers expected four) and a rock texture path missing its `textures/` directory. Root corrected both and verified every referenced file exists. Actual browser execution also required preserving `PHYSICAL` when copying a standard material into a physical one. Source syntax checks alone did not catch these integration errors.
