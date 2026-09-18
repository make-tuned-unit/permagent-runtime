# Celestial habitat implementation report

Revised the bounded celestial scenery module in
[`scripts/blender/forum_celestial.py`](../../../../scripts/blender/forum_celestial.py).
The module is intended to run in the existing `build_solar_forum.py` globals
after `forum_landscape.py` and before the source materials/save/export section.

The rendered `celestial-habitat.webp` exposed three concrete failures in the
first pass: the floating habitats read as white saucers, the waterfall sheets
collapsed into turquoise poles, and the ring read as a bare checkerboard hoop.
This revision addresses those scene-reading problems while keeping the
landmarks near their existing coordinates and preserving the original
geodisc.

The scene addition contains:

- A monumental, closed orbital garden ring north of the playable geodisc at
  `(0, 260, 106)` metres. Its 86 m inner radius and 101 m outer radius remain
  modeled in the X/Z plane with 6 m structural depth. A continuous inner
  service gallery, outer maintenance gallery, overlapping stone fascia panels,
  thicker radial ribs, tangential gallery rails and three tapered support
  pylons replace the harsh alternating blank-panel rhythm. Fifteen upper soil
  bays now carry paired 4–8 m articulated trees with lower and upper crowns.
- Two distant floating garden habitats at `(-148, 204, 92)` and
  `(148, 211, 98)` metres. Each has a closed geological underside, uneven
  planted mounds, five separated terrace slices, seven varied-height garden
  buildings with recessed glazing, an asymmetric stepped observatory, paired
  rooted canopy trees, hanging edge groves, and waterfall source lips.
- Each habitat now uses three broad staggered water sheets plus narrower edge
  sheets and lightweight mist strands per main fall. Width sway and broken
  edges are deterministic, with sheet lengths staggered so the waterfalls read
  as layered cascades rather than straight rectangular posts.
- `celestial_manifest`, published in the builder globals for root integration.
  It reports exact module mesh-object count, triangle count, bounds, landmark
  positions, playability metadata, and camera/star-field integration hints when
  Blender executes the module.

The authored scenery envelope stays approximately `x=-189..189 m`,
`y=176..264 m`, `z=4..214 m`; the runtime manifest calculates exact bounds
after transforms. The farthest visible architectural point remains roughly
335–340 m from the world origin. A camera far clip of at least 430 m and a star
sphere radius of at least 380 m leave useful margins. Root has a much larger
runtime far clip available; the source render should still confirm that the
upper ring, tree crowns and waterfall tails are not culled.

## Checks

Ran:

```text
python3 -m py_compile scripts/blender/forum_celestial.py
git diff --check -- scripts/blender/forum_celestial.py
```

Both checks passed. Blender/Unreal were not launched, and no generated binary
asset was changed. The module continues to assert `<100,000` added triangles
when executed inside Blender; the root render/export pass should inspect the
resulting `celestial_manifest`, uneven habitat silhouette, ring tree scale,
waterfall layering and camera clipping. This is a targeted geometry revision,
not a claim that the north-star image has been fully achieved.

## Integration

Add this one execution line immediately after the existing landscape module:

```python
exec(compile((ROOT / 'scripts/blender/forum_celestial.py').read_text(),
             str(ROOT / 'scripts/blender/forum_celestial.py'), 'exec'))
```

Keep it before source save, GLB/FBX export and any material batching. The
distant terraces are authored as scenery and carry `playable=False`; they do
not add routes, collision promises or agent destinations.
