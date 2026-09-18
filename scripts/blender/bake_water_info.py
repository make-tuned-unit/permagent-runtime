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
import struct
import zlib
from pathlib import Path

import bpy
from mathutils import Vector


def decode_png(blob):
    """Decode an 8-bit RGBA PNG to a flat bytearray. Used to verify our own output."""
    pos, idat, width, height = 8, b'', 0, 0
    while pos < len(blob):
        length = struct.unpack('>I', blob[pos:pos + 4])[0]
        kind = blob[pos + 4:pos + 8]
        data = blob[pos + 8:pos + 8 + length]
        if kind == b'IHDR':
            width, height = struct.unpack('>II', data[:8])
        elif kind == b'IDAT':
            idat += data
        pos += 12 + length
    raw = zlib.decompress(idat)
    stride = width * 4
    out = bytearray()
    prev = bytearray(stride)
    at = 0
    for _ in range(height):
        f = raw[at]
        at += 1
        line = bytearray(raw[at:at + stride])
        at += stride
        for x in range(stride):
            a = line[x - 4] if x >= 4 else 0
            b = prev[x]
            c = prev[x - 4] if x >= 4 else 0
            if f == 1:
                line[x] = (line[x] + a) & 255
            elif f == 2:
                line[x] = (line[x] + b) & 255
            elif f == 3:
                line[x] = (line[x] + (a + b) // 2) & 255
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[x] = (line[x] + (a if pa <= pb and pa <= pc else (b if pb <= pc else c))) & 255
        out += line
        prev = line
    return out

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

# The rays, cached against the blend that produced them.
#
# Tracing 512x512 texels takes about twenty minutes, and the first two bakes
# were lost to a WRITE bug with rays that had been correct all along. Paying
# the trace again to fix an encoder is the kind of cost that stops you from
# checking your work. The cache is keyed on the blend's mtime and the
# resolution, so it is reused only when it describes this exact island.
CACHE = ROOT / 'assets/world/forum/.water-rays.bin'
CACHE_KEY = (int(BLEND.stat().st_mtime), RES)
surface = [[None] * RES for _ in range(RES)]
bed = [[None] * RES for _ in range(RES)]
cached = False
if CACHE.exists() and os.environ.get('FORUM_WATER_RETRACE') != '1':
    try:
        blob = zlib.decompress(CACHE.read_bytes())
        key_mtime, key_res, count = struct.unpack('<qii', blob[:16])
        if (key_mtime, key_res) == CACHE_KEY:
            at = 16
            for _ in range(count):
                j, i, s, b = struct.unpack('<iiff', blob[at:at + 16])
                surface[j][i], bed[j][i] = s, b
                at += 16
            cached = True
            print(f'reusing {count} traced water texels from {CACHE.name}')
    except Exception as exc:  # a corrupt cache is not a reason to fail
        print(f'ignoring unreadable ray cache ({exc})')

if not cached:
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
    hits = [(j, i) for j in range(RES) for i in range(RES) if surface[j][i] is not None]
    packed = struct.pack('<qii', CACHE_KEY[0], CACHE_KEY[1], len(hits))
    packed += b''.join(struct.pack('<iiff', j, i, surface[j][i], bed[j][i]) for j, i in hits)
    CACHE.write_bytes(zlib.compress(packed, 6))
    print(f'traced and cached {len(hits)} water texels')

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

water_texels = sum(1 for j in range(RES) for i in range(RES) if surface[j][i] is not None)

# The rows, as the browser will read them.
#
# Row order is the one part of this that is easy to get silently backwards.
# The manifest maps three.z = -blender.y, so v = 0 is blender y = max_y, which
# is `j = RES - 1`; the loader sets `flipY = false`, so v = 0 is the FIRST row
# in the file. Hence rows are written from the top of the world down. This is
# the same order Blender's own writer produced (its pixel buffer is bottom-up
# and it saves bottom-last), so the shader's sampling is unchanged.
rows = []
for j in range(RES - 1, -1, -1):
    row = bytearray(RES * 4)
    for i in range(RES):
        if surface[j][i] is None:
            continue  # land: all four channels stay zero, mask included
        depth = max(0.0, surface[j][i] - bed[j][i])
        shore = min(1.0, dist[j][i] * metres_per_texel / SHORE_RANGE)
        floor = max(0.0, min(1.0, (bed[j][i] - lo.z) / max(1e-6, hi.z - lo.z)))
        at = i * 4
        row[at] = int(min(1.0, depth / DEPTH_RANGE) * 255 + 0.5)
        row[at + 1] = int(shore * 255 + 0.5)
        row[at + 2] = int(floor * 255 + 0.5)
        row[at + 3] = 255
    rows.append(bytes(row))


def encode_png(rows_, width, height):
    """A minimal 8-bit RGBA PNG, written here rather than by Blender.

    Blender's image API is not used for this file any more, and that is the
    fix. `images.new` makes a GENERATED image; assigning `.pixels`, and then
    `foreach_set` + `update()`, both left `save()` writing the buffer that
    image was generated with — black with alpha 1 in every texel. The rays had
    always been right, so it failed silently and the lagoon rendered as a flat
    pale sheet: zero depth is no absorption, and a full mask ran the shore foam
    across the whole world.

    These channels are numbers, not colour. Going straight to zlib means no
    colour management, no alpha premultiplication and no generated-image
    semantics can touch them between here and the shader.
    """
    raw = b''.join(b'\x00' + r for r in rows_)  # filter type 0 per row

    def chunk(kind, payload):
        return (struct.pack('>I', len(payload)) + kind + payload
                + struct.pack('>I', zlib.crc32(kind + payload) & 0xFFFFFFFF))

    return (b'\x89PNG\r\n\x1a\n'
            + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(raw, 9))
            + chunk(b'IEND', b''))


WEB.mkdir(parents=True, exist_ok=True)
target = WEB / 'water-info.png'
target.write_bytes(encode_png(rows, RES, RES))

# Read the file back and check it against the rays. This is what caught the
# blank texture, and it stays: a bake that silently produces one must fail the
# build rather than ship.
saved_water = 0
saved_depth = 0
for row in rows:
    for i in range(RES):
        if row[i * 4 + 3] > 127:
            saved_water += 1
        if row[i * 4] > 0:
            saved_depth += 1
decoded = decode_png(target.read_bytes())
file_water = sum(1 for k in range(RES * RES) if decoded[k * 4 + 3] > 127)
if file_water != saved_water or saved_water != water_texels:
    raise SystemExit(
        f'water-info.png mask does not match the bake: {file_water} in the file, '
        f'{saved_water} encoded, {water_texels} traced.')
if water_texels and saved_depth == 0:
    raise SystemExit(
        'water-info.png carries no depth at all. Every texel would render as '
        'zero-depth water: no absorption, and foam across the whole lagoon.')
print(f'verified: {file_water} masked texels, {saved_depth} with depth')
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
