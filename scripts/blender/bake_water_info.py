"""Bake what the water needs to look like water: depth, and distance to shore.

The lagoon currently renders as one flat opaque teal. Real water reads as water
because of two things this texture supplies, both of which are properties of the
GEOMETRY and therefore bakeable once rather than computed every frame:

  R  depth      how far it is from the surface down to the bed, normalised.
                Drives Beer-Lambert absorption: shallow water shows the sand
                through it, deep water goes dark and blue. This single channel
                is most of the difference between "teal plane" and "lagoon".
  G  shore      distance from the nearest shoreline, normalised. Drives foam,
                which gathers where water meets land.
  B  bed        the bed height itself, normalised into the island's range, for
                anything that wants the floor without a second ray.
  A  mask       1 where there is water at all, 0 on land. Sampling outside the
                lagoon must not tint the beach.

Rays are cast straight down per texel from above the island: first hit on a
`Water` material gives the surface, the next hit below it gives the bed. That is
the same fact the walker and the ships already use — the GLB names that material
— so nothing here invents a water level.

Output: ui/command-center/public/world/water-info.png plus a manifest carrying
the world-space bounds, because a texture without its extent is unusable.

Build:  blender --background --factory-startup --python-exit-code 1 \
          --python scripts/blender/bake_water_info.py
Env:    FORUM_WATER_RES  texels across the island (default 1024)
"""
import json
import math
import os
from pathlib import Path

import bpy
from mathutils import Vector

ROOT = Path(__file__).resolve().parents[2]
BLEND = ROOT / 'assets/world/forum/solar-forum.blend'
WEB = ROOT / 'ui/command-center/public/world'
RES = int(os.environ.get('FORUM_WATER_RES', '1024'))

assert BLEND.exists(), f'build the island first: {BLEND}'
bpy.ops.wm.open_mainfile(filepath=str(BLEND))

depsgraph = bpy.context.evaluated_depsgraph_get()
scene = bpy.context.scene


def is_water(obj, face_index):
    """Whether the hit face wears the Water material."""
    mesh = obj.data
    if not mesh.materials:
        return False
    try:
        slot = mesh.polygons[face_index].material_index
    except (IndexError, AttributeError):
        return False
    material = mesh.materials[slot] if slot < len(mesh.materials) else None
    return bool(material and material.name.startswith('Water'))


# Island bounds from every mesh in the file, so the texture covers the world
# rather than a guessed square.
lo = Vector((1e9, 1e9, 1e9))
hi = Vector((-1e9, -1e9, -1e9))
for obj in scene.objects:
    if obj.type != 'MESH':
        continue
    for corner in obj.bound_box:
        world = obj.matrix_world @ Vector(corner)
        lo = Vector((min(lo.x, world.x), min(lo.y, world.y), min(lo.z, world.z)))
        hi = Vector((max(hi.x, world.x), max(hi.y, world.y), max(hi.z, world.z)))

# Square the footprint so the texture has uniform texel size on both axes.
centre = (lo + hi) * 0.5
half = max(hi.x - lo.x, hi.y - lo.y) * 0.5
min_x, max_x = centre.x - half, centre.x + half
min_y, max_y = centre.y - half, centre.y + half
top = hi.z + 50.0
print(f'island footprint {2*half:.1f}m, {2*half/RES*100:.1f}cm per texel')

surface = [[None] * RES for _ in range(RES)]
bed = [[None] * RES for _ in range(RES)]

for j in range(RES):
    y = min_y + (j + 0.5) / RES * (max_y - min_y)
    for i in range(RES):
        x = min_x + (i + 0.5) / RES * (max_x - min_x)
        origin = Vector((x, y, top))
        down = Vector((0, 0, -1))
        # First hit: is the top surface here water?
        hit, location, _, face, obj, _ = scene.ray_cast(depsgraph, origin, down)
        if not hit or not is_water(obj, face):
            continue
        surface[j][i] = location.z
        # Next hit below the water surface is the bed.
        below = Vector((x, y, location.z - 0.05))
        hit2, location2, _, face2, obj2, _ = scene.ray_cast(depsgraph, below, down)
        bed[j][i] = location2.z if hit2 else location.z - 12.0

# Distance to shore, in texels, by a two-pass chamfer over the water mask. A
# real Euclidean transform is not worth the runtime here: foam is a soft band a
# few metres wide and the chamfer's error is well under a texel at that width.
FAR = 10 ** 9
dist = [[0 if surface[j][i] is None else FAR for i in range(RES)] for j in range(RES)]
for j in range(RES):
    for i in range(RES):
        if dist[j][i] == 0:
            continue
        best = dist[j][i]
        if j > 0:
            best = min(best, dist[j - 1][i] + 1, dist[j - 1][i - 1] + 1.41 if i > 0 else FAR)
        if i > 0:
            best = min(best, dist[j][i - 1] + 1)
        dist[j][i] = best
for j in range(RES - 1, -1, -1):
    for i in range(RES - 1, -1, -1):
        if dist[j][i] == 0:
            continue
        best = dist[j][i]
        if j < RES - 1:
            best = min(best, dist[j + 1][i] + 1, dist[j + 1][i + 1] + 1.41 if i < RES - 1 else FAR)
        if i < RES - 1:
            best = min(best, dist[j][i + 1] + 1)
        dist[j][i] = best

metres_per_texel = (max_x - min_x) / RES
DEPTH_RANGE = 14.0   # metres; beyond this the water is fully absorbed anyway
SHORE_RANGE = 12.0   # metres; the widest foam band worth encoding

pixels = [0.0] * (RES * RES * 4)
for j in range(RES):
    for i in range(RES):
        at = (j * RES + i) * 4
        if surface[j][i] is None:
            pixels[at + 3] = 0.0
            continue
        depth = max(0.0, surface[j][i] - bed[j][i])
        shore = min(1.0, dist[j][i] * metres_per_texel / SHORE_RANGE)
        pixels[at] = min(1.0, depth / DEPTH_RANGE)
        pixels[at + 1] = shore
        pixels[at + 2] = max(0.0, min(1.0, (bed[j][i] - lo.z) / max(1e-6, hi.z - lo.z)))
        pixels[at + 3] = 1.0

water_texels = sum(1 for j in range(RES) for i in range(RES) if surface[j][i] is not None)

# Writing the buffer, and then PROVING it was written.
#
# The first bake shipped a texture that was black with alpha 1 in every texel —
# which is exactly the buffer `images.new` generates before anything is put in
# it. The rays had worked (the manifest counted 5.4% water from the same data),
# so nothing looked wrong at the console, and the lagoon rendered as a flat
# pale sheet in the app: depth 0 everywhere means no absorption, and mask 1
# everywhere means the shore foam term ran across the entire world.
#
# `foreach_set` + `update()` is the buffer write that actually reaches the
# saved file; `alpha_mode` stops the writer premultiplying a channel that is a
# number, not opacity. The read-back below is the part that matters most: a
# bake that silently produces a blank texture must fail the build, not ship.
image = bpy.data.images.new('forum_water_info', width=RES, height=RES, alpha=True, float_buffer=False)
image.alpha_mode = 'CHANNEL_PACKED'
image.pixels.foreach_set(pixels)
image.update()
image.file_format = 'PNG'
# Data, not colour: the shader reads these as numbers, so no sRGB transform.
image.colorspace_settings.name = 'Non-Color'
WEB.mkdir(parents=True, exist_ok=True)
target = WEB / 'water-info.png'
image.filepath_raw = str(target)
image.save()

# Read the file back as the browser will and check it against what we meant to
# write. Comparing the saved mask to the ray count catches the blank-buffer
# failure, a premultiplied alpha channel, and a flipped or half-written image.
verify = bpy.data.images.load(str(target), check_existing=False)
readback = [0.0] * (RES * RES * 4)
verify.pixels.foreach_get(readback)
saved_water = sum(1 for k in range(RES * RES) if readback[k * 4 + 3] > 0.5)
saved_depth = sum(1 for k in range(RES * RES) if readback[k * 4] > 0.004)
bpy.data.images.remove(verify)
if water_texels and abs(saved_water - water_texels) > max(64, water_texels * 0.02):
    raise SystemExit(
        f'water-info.png mask does not match the bake: {saved_water} texels saved, '
        f'{water_texels} traced. The buffer did not reach the file.')
if water_texels and saved_depth == 0:
    raise SystemExit(
        'water-info.png carries no depth at all. Every texel would render as '
        'zero-depth water: no absorption, and foam across the whole lagoon.')
print(f'verified: {saved_water} masked texels, {saved_depth} with depth')
manifest = {
    'schema': 'permagent.forum.water.v1',
    'resolution': RES,
    'metresPerTexel': round(metres_per_texel, 3),
    'bounds': {'minX': min_x, 'maxX': max_x, 'minZ': -max_y, 'maxZ': -min_y},
    'depthRange': DEPTH_RANGE,
    'shoreRange': SHORE_RANGE,
    'waterCoverage': round(water_texels / (RES * RES), 4),
    'channels': {
        'r': 'depth / depthRange',
        'g': 'distance to shore / shoreRange',
        'b': 'bed height, normalised to the island z range',
        'a': 'water mask',
    },
    # Blender is Z-up and the web scene is Y-up with z = -y, which is the same
    # conversion the GLB export applies; the bounds above are already converted
    # so the shader can use world x/z directly.
    'axes': 'three.x = blender.x, three.z = -blender.y',
}
(WEB / 'water-info.json').write_text(json.dumps(manifest, indent=2) + '\n')
print('water info written:', water_texels, 'water texels of', RES * RES)
