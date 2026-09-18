"""Small solar-sail cutter for the forum's water — hull, mast, two sails, a wake plane.

The boats already in `solar-forum.glb` are baked into the island geometry
(`forum_landform.py:477`, `forum_landscape.py:415`) and can never move: they are
part of the terrain mesh. Jesse asked for more ships and for water that lives,
so this builds a ship as its OWN small GLB, the same way `build_forum_bird.py`
does for the seabirds, which is what lets the runtime clone it a handful of
times and sail each copy along its own course.

Named parts matter: the runtime finds `SailMain`, `SailJib`, `Hull` and `Wake`
by name (`ForumShips.tsx`) to luff the sails as a ship comes about and to fade
the wake with speed. Keep the names if you change the modelling.

Forward is +Y and the waterline is z=0, matching the bird's convention and the
GLB export's `export_yup=True`, so the runtime never has to guess an offset.

Build:  blender --background --factory-startup --python-exit-code 1 \
          --python scripts/blender/build_forum_ship.py
"""
import bpy
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / 'assets/world/forum'
WEB = ROOT / 'ui/command-center/public/world'

bpy.ops.object.select_all(action='SELECT')
bpy.ops.object.delete(use_global=False)
bpy.context.scene.unit_settings.system = 'METRIC'


def material(name, rgba, roughness=0.6):
    m = bpy.data.materials.new(name)
    m.diffuse_color = rgba
    m.roughness = roughness
    return m


# Palette kept in the forum's family: warm timber, bleached canvas, a bronze
# mast. The names are what the web material patch matches on, so they read like
# the rest of the island rather than like a new asset.
TIMBER = material('Forum ship timber', (0.30, 0.20, 0.13, 1.0), 0.72)
CANVAS = material('Forum ship canvas', (0.88, 0.86, 0.80, 1.0), 0.86)
BRONZE = material('Forum ship bronze', (0.45, 0.31, 0.16, 1.0), 0.42)
FOAM = material('Forum ship wake', (0.90, 0.94, 0.95, 1.0), 0.95)


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
# A chined hull: a keel line, a chine and a sheer, tapering to a point at the
# bow and a small transom at the stern. Low poly on purpose — these are read at
# 60-200 m, where silhouette is the only thing that survives.
SECTIONS = [
    # (y along the hull, half-beam at chine, keel depth, sheer height)
    (-2.10, 0.06, -0.10, 0.30),
    (-1.30, 0.42, -0.34, 0.40),
    (-0.30, 0.58, -0.42, 0.44),
    (0.90, 0.54, -0.38, 0.44),
    (1.85, 0.34, -0.24, 0.42),
    (2.55, 0.05, -0.06, 0.38),
]

verts, faces = [], []
for y, beam, keel, sheer in SECTIONS:
    base = len(verts)
    verts.extend([
        (0.0, y, keel),           # keel
        (-beam, y, keel * 0.35),  # chine port
        (-beam * 0.92, y, sheer), # sheer port
        (beam * 0.92, y, sheer),  # sheer starboard
        (beam, y, keel * 0.35),   # chine starboard
    ])
    if base:
        p = base - 5
        for a, b in ((0, 1), (1, 2), (3, 4), (4, 0)):
            faces.append((p + a, p + b, base + b, base + a))
# Deck and transom close the shell so it never shows a hollow interior.
first, last = 0, len(verts) - 5
faces.append((first + 2, first + 3, last + 3, last + 2))
faces.append((last + 1, last + 2, last + 3, last + 4))
hull = mesh('Hull', verts, faces, TIMBER, smooth=True)

# ── Mast and boom ───────────────────────────────────────────────────────
bpy.ops.mesh.primitive_cylinder_add(vertices=8, radius=0.055, depth=3.6, location=(0, 0.35, 1.75))
mast = bpy.context.object
mast.name = 'Mast'
mast.data.materials.append(BRONZE)
for p in mast.data.polygons:
    p.use_smooth = True

bpy.ops.mesh.primitive_cylinder_add(vertices=6, radius=0.035, depth=1.9, rotation=(1.5708, 0, 0), location=(0, -0.45, 0.72))
boom = bpy.context.object
boom.name = 'Boom'
boom.data.materials.append(BRONZE)

# ── Sails ───────────────────────────────────────────────────────────────
# Each sail is a single quad with a little belly, so the runtime can rock it
# about the mast without the geometry reading as a flat sheet of paper.
mesh('SailMain', [
    (0.0, 0.30, 0.78), (0.0, -1.32, 0.66), (0.0, -0.98, 3.05), (0.0, 0.30, 3.30),
], [(0, 1, 2, 3)], CANVAS)
mesh('SailJib', [
    (0.0, 0.42, 0.70), (0.0, 2.35, 0.52), (0.0, 0.46, 2.55),
], [(0, 1, 2)], CANVAS)

# ── Wake ────────────────────────────────────────────────────────────────
# A flat fan just under the waterline. The runtime fades it with speed, so a
# drifting ship leaves nothing and a running one leaves a widening V.
mesh('Wake', [
    (0.0, 2.10, 0.02), (-1.25, -3.40, 0.02), (0.0, -2.60, 0.02), (1.25, -3.40, 0.02),
], [(0, 1, 2), (0, 2, 3)], FOAM)

bpy.ops.object.select_all(action='SELECT')
OUT.mkdir(parents=True, exist_ok=True)
WEB.mkdir(parents=True, exist_ok=True)
bpy.ops.wm.save_as_mainfile(filepath=str(OUT / 'forum-ship.blend'))
bpy.ops.export_scene.gltf(
    filepath=str(WEB / 'forum-ship.glb'),
    export_format='GLB',
    export_yup=True,
    export_animations=False,
    export_cameras=False,
    export_lights=False,
)
bpy.ops.export_scene.fbx(
    filepath=str(OUT / 'forum-ship.fbx'),
    object_types={'MESH'},
    apply_unit_scale=True,
    bake_anim=False,
    axis_forward='-Y',
    axis_up='Z',
)
print('forum ship written to', WEB / 'forum-ship.glb')
