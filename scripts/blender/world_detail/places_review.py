"""Lane W4 diagnostic renders of the *exported* environment GLBs (re-imported,
so every shot doubles as an export round-trip check). Eevee, <= 960x540.

  /Applications/Blender.app/Contents/MacOS/Blender -b -noaudio \
      --python scripts/blender/world_detail/places_review.py -- [--only botany|places]

Writes assets/world/detail-v4/environment/review/*.jpg + review.json.
A Blender render can reject an asset; it cannot accept the shipping World.
"""
import sys
import json
import math
from pathlib import Path

import bpy
from mathutils import Vector

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
REPO = HERE.parent.parent.parent
ENV = REPO / 'assets/world/detail-v4/environment'
W, H = 960, 540
SHOTS = []


def gl2b(v):
    return Vector((v[0], -v[2], v[1]))


def reset(samples=24):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    sc = bpy.context.scene
    sc.render.engine = 'BLENDER_EEVEE'
    sc.render.resolution_x, sc.render.resolution_y = W, H
    try:
        sc.eevee.taa_render_samples = samples
    except AttributeError:
        pass
    sc.view_settings.view_transform = 'AgX'
    world = bpy.data.worlds.new('w')
    sc.world = world
    world.use_nodes = True
    return sc


def world_color(rgb, strength=1.0):
    bg = bpy.context.scene.world.node_tree.nodes['Background']
    bg.inputs['Color'].default_value = (*rgb, 1)
    bg.inputs['Strength'].default_value = strength


def import_glb(path, loc=(0, 0, 0), yaw=0.0, scale=1.0):
    before = set(bpy.data.objects)
    bpy.ops.import_scene.gltf(filepath=str(path))
    new = [o for o in bpy.data.objects if o not in before]
    roots = [o for o in new if o.parent is None]
    for r in roots:
        r.location = Vector(loc)
        r.rotation_mode = 'XYZ'
        r.rotation_euler = (0, 0, yaw)
        r.scale = (scale, scale, scale)
    for o in new:
        if o.type == 'EMPTY':
            o.hide_render = True
    return roots[0], new


def floor(size=30, color=(0.42, 0.38, 0.32), z=0.0, loc=(0, 0)):
    bpy.ops.mesh.primitive_plane_add(size=size, location=(loc[0], loc[1], z))
    p = bpy.context.object
    m = bpy.data.materials.new('floor')
    m.use_nodes = True
    b = m.node_tree.nodes['Principled BSDF']
    b.inputs['Base Color'].default_value = (*color, 1)
    b.inputs['Roughness'].default_value = 0.85
    p.data.materials.append(m)
    return p


def box(center, size, color, rough=0.8, name='box'):
    bpy.ops.mesh.primitive_cube_add(size=1, location=center)
    o = bpy.context.object
    o.scale = size
    o.name = name
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    b = m.node_tree.nodes['Principled BSDF']
    b.inputs['Base Color'].default_value = (*color, 1)
    b.inputs['Roughness'].default_value = rough
    o.data.materials.append(m)
    return o


def water_plane(center, size, z):
    bpy.ops.mesh.primitive_plane_add(size=1, location=(center[0], center[1], z))
    o = bpy.context.object
    o.scale = (size[0], size[1], 1)
    m = bpy.data.materials.new('water')
    m.use_nodes = True
    b = m.node_tree.nodes['Principled BSDF']
    b.inputs['Base Color'].default_value = (0.03, 0.07, 0.08, 1)
    b.inputs['Roughness'].default_value = 0.05
    b.inputs['Alpha'].default_value = 0.85
    m.surface_render_method = 'BLENDED'
    o.data.materials.append(m)
    return o


def light(kind, loc, target, energy, color=(1, 1, 1), size=1.0):
    ld = bpy.data.lights.new(kind + str(len(bpy.data.lights)), kind)
    ld.energy = energy
    ld.color = color
    if kind == 'AREA':
        ld.size = size
    if kind == 'SUN':
        ld.angle = math.radians(2)
    if kind == 'POINT':
        ld.shadow_soft_size = size
    o = bpy.data.objects.new(ld.name, ld)
    bpy.context.scene.collection.objects.link(o)
    o.location = loc
    o.rotation_euler = (Vector(target) - Vector(loc)).to_track_quat('-Z', 'Y').to_euler()
    return o


def camera(loc, target, lens=35, ortho=None):
    for o in [o for o in bpy.data.objects if o.type == 'CAMERA']:
        bpy.data.objects.remove(o)
    cd = bpy.data.cameras.new('cam')
    if ortho:
        cd.type = 'ORTHO'
        cd.ortho_scale = ortho
    else:
        cd.lens = lens
    cd.clip_start = 0.05
    cd.clip_end = 400
    o = bpy.data.objects.new('cam', cd)
    bpy.context.scene.collection.objects.link(o)
    o.location = loc
    o.rotation_euler = (Vector(target) - Vector(loc)).to_track_quat('-Z', 'Y').to_euler()
    bpy.context.scene.camera = o
    return o


def day_rig(center=(0, 0, 1), r=8):
    world_color((0.55, 0.60, 0.66), 0.75)
    c = Vector(center)
    light('SUN', c + Vector((-6, -8, 9)), center, 3.2, (1.0, 0.93, 0.82))
    light('AREA', c + Vector((r, -r * 0.4, r * 0.4)), center, 120 * r, (0.85, 0.9, 1.0), r * 0.6)


def night_rig(center=(0, 0, 1)):
    world_color((0.012, 0.018, 0.04), 1.0)
    light('SUN', Vector(center) + Vector((-8, 10, 12)), center, 0.35, (0.55, 0.68, 1.0))


def shot(name, desc):
    st = bpy.context.scene.render.image_settings
    st.file_format, st.quality = 'JPEG', 88
    bpy.context.scene.render.filepath = str(ENV / 'review' / f'{name}.jpg')
    bpy.ops.render.render(write_still=True)
    SHOTS.append({'file': f'review/{name}.jpg', 'what': desc})
    print('[W4 review]', name, flush=True)


# ------------------------------------------------------------------ botany
def botany():
    bot = ENV / 'botany'
    man = json.loads((bot / 'manifest.json').read_text())
    names = [a['name'] for a in man['assets']]
    xs = {'canopy-olive': -4.2, 'shrub-laurel': -0.6, 'grass-fountain': 1.4, 'fern-sword': 3.2,
          'flower-coneflower': 4.8, 'iris-water': 6.0, 'vine-pendant': 7.3}
    for lod in (0, 1):
        reset(); floor(40); day_rig((1.5, 0, 1.5), 10)
        for n in names:
            loc = (xs[n], 0, 2.2 if n == 'vine-pendant' else 0)
            import_glb(bot / f'{n}.lod{lod}.glb', loc, 0.6)
        box((7.3, 0, 2.25), (0.9, 0.12, 0.1), (0.25, 0.18, 0.12), name='vine beam')
        box((-3.0, 0.0, 1.15), (0.02, 0.02, 2.3), (0.9, 0.2, 0.2), 0.5, 'headroom 2.3 m marker')
        if lod == 0:
            camera((1.6, -13.5, 2.6), (1.6, 0, 2.2), 30)
            shot('botany-lineup-front', 'species lineup LOD0, front (-Y): canopy, shrub, grass, fern, coneflower, '
                                        'iris, vine on a beam; red rod = 2.3 m headroom')
            camera((1.6, 13.5, 2.6), (1.6, 0, 2.2), 30)
            shot('botany-lineup-back', 'species lineup LOD0, back (+Y): reverse silhouettes')
        else:
            camera((1.6, -13.5, 2.6), (1.6, 0, 2.2), 30)
            shot('botany-lineup-lod1', 'species lineup LOD1 (cards / reduced geometry), front')
    reset(); floor(20); day_rig((0, 0, 0.5), 4)
    sm = ['grass-fountain', 'fern-sword', 'flower-coneflower', 'iris-water', 'shrub-laurel']
    for i, n in enumerate(sm):
        import_glb(bot / f'{n}.lod0.glb', (-2.4 + i * 1.2, 0, 0), 0.3)
    camera((0, -4.2, 1.35), (0, 0, 0.42), 40)
    shot('botany-groundcover-closeup', 'close-up LOD0: grass, fern, coneflower, iris, shrub (leaf orientation, '
                                       'pinnae taper, petals, contact with ground)')
    reset(); floor(20); day_rig((0, 0, 3), 6)
    import_glb(bot / 'canopy-olive.lod0.glb')
    camera((1.2, -3.2, 1.6), (0, 0, 3.4), 24)
    shot('botany-canopy-underside', 'canopy LOD0 from beneath at standing eye height: branching, card edges, '
                                    'alpha fringe check')


if __name__ == '__main__':
    argv = sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else []
    only = argv[argv.index('--only') + 1] if '--only' in argv else None
    (ENV / 'review').mkdir(parents=True, exist_ok=True)
    rj = ENV / 'review' / 'review.json'
    prev = json.loads(rj.read_text())['shots'] if rj.exists() else []
    if only in (None, 'botany'):
        botany()
    if only in (None, 'places'):
        import places_review_scenes as PR
        PR.run(sys.modules[__name__])
    done = {s['file'] for s in SHOTS}
    merged = [s for s in prev if s['file'] not in done] + SHOTS
    rj.write_text(json.dumps({'renderer': f'Blender {bpy.app.version_string} Eevee, {W}x{H}',
                              'source': 'exported GLBs re-imported (round trip)', 'shots': merged}, indent=2) + '\n')
