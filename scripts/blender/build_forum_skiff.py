"""The sovereign's skiff — a small solar flyer the agent boards to cross the world.

Jesse: "I want a spaceship that I can get into a fly around to the different
parts of the map and land on the floating islands."

So this is a LANDER, not a fighter: a wide, flat-bottomed hull with three legs
and a low centre of mass, because the thing it has to do most convincingly is
set down on a floating terrace and sit there while you walk out of it. It reads
from two distances that matter — a silhouette against the sky at 200m, and a
vehicle you are standing next to at 3m — so the shapes are large and the detail
is in the proportions rather than in panel lines nothing will resolve.

Named parts the runtime drives (`ForumFlight.tsx`):
  Hull, CanopyGlass, ThrusterL/ThrusterR/ThrusterAft (glow with throttle),
  LegFore/LegPortAft/LegStarAft (extend on approach), Beacon (pulses when idle).

Forward is +Y and the resting waterline of the legs is z=0, matching the bird
and the ship, so the runtime never has to guess an offset.

Build:  blender --background --factory-startup --python-exit-code 1 \
          --python scripts/blender/build_forum_skiff.py
"""
import bpy
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / 'assets/world/forum'
WEB = ROOT / 'ui/command-center/public/world'

bpy.ops.object.select_all(action='SELECT')
bpy.ops.object.delete(use_global=False)
bpy.context.scene.unit_settings.system = 'METRIC'


def material(name, rgba, roughness=0.5, metallic=0.0, emission=None):
    m = bpy.data.materials.new(name)
    m.diffuse_color = rgba
    m.roughness = roughness
    m.metallic = metallic
    if emission is not None:
        # The web pass reads emissive strength off the material name family, so
        # the name matters as much as the value here.
        m.diffuse_color = emission
    return m


SHELL = material('Skiff shell', (0.86, 0.87, 0.88, 1.0), 0.34, 0.15)
TRIM = material('Skiff bronze trim', (0.48, 0.33, 0.17, 1.0), 0.3, 0.6)
GLASS = material('Skiff canopy glass', (0.36, 0.55, 0.62, 0.35), 0.08)
GLOW = material('Skiff thruster glow', (0.25, 0.85, 0.95, 1.0), 0.9)
LEGS = material('Skiff leg', (0.32, 0.34, 0.36, 1.0), 0.45, 0.4)


def mesh(name, verts, faces, mat, smooth=False):
    me = bpy.data.meshes.new(name)
    me.from_pydata(verts, [], faces)
    me.update()
    ob = bpy.data.objects.new(name, me)
    bpy.context.collection.objects.link(ob)
    ob.data.materials.append(mat)
    if smooth:
        for p in ob.data.polygons:
            p.use_smooth = True
    return ob


# ── Hull ────────────────────────────────────────────────────────────────
# A lifting body: wide across the beam, tapered to a blunt nose, flat
# underneath so it sits on a terrace instead of rocking on a keel.
SECTIONS = [
    # (y fore/aft, half-width, floor z, deck z)
    (-2.60, 0.50, 0.30, 0.78),
    (-1.60, 1.35, 0.20, 1.05),
    (-0.20, 1.72, 0.16, 1.22),
    (1.30, 1.55, 0.18, 1.12),
    (2.45, 0.92, 0.24, 0.86),
    (3.15, 0.26, 0.34, 0.62),
]

verts, faces = [], []
for y, half, floor, deck in SECTIONS:
    base = len(verts)
    verts.extend([
        (-half, y, floor), (half, y, floor),   # underside
        (half, y, deck), (-half, y, deck),     # deck
    ])
    if base:
        p = base - 4
        for a, b in ((0, 1), (1, 2), (2, 3), (3, 0)):
            faces.append((p + a, p + b, base + b, base + a))
faces.append((0, 1, 2, 3))
last = len(verts) - 4
faces.append((last + 3, last + 2, last + 1, last))
mesh('Hull', verts, faces, SHELL, smooth=True)

# ── Canopy ──────────────────────────────────────────────────────────────
# Forward and low, so a pilot's eye sits where the runtime puts the camera.
mesh('CanopyGlass', [
    (-0.86, 0.35, 1.22), (0.86, 0.35, 1.22),
    (0.62, 2.05, 1.06), (-0.62, 2.05, 1.06),
    (-0.72, 1.15, 1.72), (0.72, 1.15, 1.72),
], [(0, 1, 2, 3), (0, 1, 5, 4), (3, 2, 5, 4)], GLASS, smooth=True)

# ── Thrusters ───────────────────────────────────────────────────────────
for name, x, y in (('ThrusterL', -1.42, -1.95), ('ThrusterR', 1.42, -1.95), ('ThrusterAft', 0.0, -2.62)):
    bpy.ops.mesh.primitive_cylinder_add(vertices=12, radius=0.42, depth=0.9, rotation=(1.5708, 0, 0), location=(x, y, 0.62))
    ob = bpy.context.object
    ob.name = name
    ob.data.materials.append(GLOW)
    for p in ob.data.polygons:
        p.use_smooth = True

# ── Landing legs ────────────────────────────────────────────────────────
# Three-point stance: one forward, two aft, wide enough that a terrace edge
# does not tip it. The runtime extends these on approach.
for name, x, y in (('LegFore', 0.0, 1.70), ('LegPortAft', -1.25, -1.45), ('LegStarAft', 1.25, -1.45)):
    bpy.ops.mesh.primitive_cylinder_add(vertices=6, radius=0.11, depth=1.1, location=(x, y, -0.35))
    leg = bpy.context.object
    leg.name = name
    leg.data.materials.append(LEGS)
    bpy.ops.mesh.primitive_cylinder_add(vertices=8, radius=0.34, depth=0.1, location=(x, y, -0.88))
    pad = bpy.context.object
    pad.name = f'{name}Pad'
    pad.data.materials.append(LEGS)

# ── Trim and beacon ─────────────────────────────────────────────────────
bpy.ops.mesh.primitive_cube_add(size=1, location=(0, -0.2, 1.26))
spine = bpy.context.object
spine.name = 'Spine'
spine.scale = (0.30, 2.30, 0.06)
bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
spine.data.materials.append(TRIM)

bpy.ops.mesh.primitive_uv_sphere_add(segments=10, ring_count=6, radius=0.13, location=(0, -1.15, 1.42))
beacon = bpy.context.object
beacon.name = 'Beacon'
beacon.data.materials.append(GLOW)
for p in beacon.data.polygons:
    p.use_smooth = True

bpy.ops.object.select_all(action='SELECT')
OUT.mkdir(parents=True, exist_ok=True)
WEB.mkdir(parents=True, exist_ok=True)
bpy.ops.wm.save_as_mainfile(filepath=str(OUT / 'forum-skiff.blend'))
bpy.ops.export_scene.gltf(
    filepath=str(WEB / 'forum-skiff.glb'),
    export_format='GLB',
    export_yup=True,
    export_animations=False,
    export_cameras=False,
    export_lights=False,
)
bpy.ops.export_scene.fbx(
    filepath=str(OUT / 'forum-skiff.fbx'),
    object_types={'MESH'},
    apply_unit_scale=True,
    bake_anim=False,
    axis_forward='-Y',
    axis_up='Z',
)
print('skiff written to', WEB / 'forum-skiff.glb')
