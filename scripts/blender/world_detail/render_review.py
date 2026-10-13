"""Neutral-lit diagnostic renders of the *exported* W3 GLBs (re-imported, so the
renders double as an export round-trip check). Eevee, <= 960x540, a couple of
seconds per frame. Writes into <out>/review/ and review/review.json.

  /Applications/Blender.app/Contents/MacOS/Blender -b -noaudio \
      --python scripts/blender/world_detail/render_review.py -- [--out DIR] [--legacy GLB] [--portrait PNG]

A Blender render can reject an asset; it cannot accept the shipping World
(W6 does native arrival acceptance).
"""
import sys
import json
import math
from pathlib import Path

import bpy
import numpy as np
from mathutils import Vector

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent.parent
W, H = 960, 540


def args():
    argv = sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else []
    o = {'out': REPO / 'assets/world/detail-v4/hero',
         'legacy': REPO / 'ui/command-center/public/world/garden-hero/hero-kit-v3b.glb',
         'portrait': None}
    for i, a in enumerate(argv):
        if a in ('--out', '--legacy', '--portrait'):
            o[a[2:]] = Path(argv[i + 1])
    return o


def gl2b(v):
    return Vector((v[0], -v[2], v[1]))


def reset():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    sc = bpy.context.scene
    sc.render.engine = 'BLENDER_EEVEE'
    sc.render.resolution_x, sc.render.resolution_y = W, H
    sc.render.film_transparent = False
    try:
        sc.eevee.taa_render_samples = 32
    except AttributeError:
        pass
    sc.view_settings.view_transform = 'AgX'
    sc.view_settings.look = 'None'
    world = bpy.data.worlds.new('w')
    sc.world = world
    world.use_nodes = True
    return sc


def world_color(rgb, strength=1.0):
    bg = bpy.context.scene.world.node_tree.nodes['Background']
    bg.inputs['Color'].default_value = (*rgb, 1)
    bg.inputs['Strength'].default_value = strength


def import_glb(path):
    before = set(bpy.data.objects)
    bpy.ops.import_scene.gltf(filepath=str(path))
    new = [o for o in bpy.data.objects if o not in before]
    by = {}
    for o in new:
        base = o.name.split('.0')[0] if o.name.rsplit('.', 1)[-1].isdigit() and len(o.name.rsplit('.', 1)[-1]) == 3 else o.name
        by.setdefault(base, o)
    roots = [o for o in new if o.parent is None]
    return roots[0], by, new


def place(root, loc, yaw=0.0):
    root.location = loc
    root.rotation_mode = 'XYZ'
    root.rotation_euler = (0, 0, yaw)


def pose(nodes, rotation_x):
    for name, ang in rotation_x.items():
        n = nodes.get(name)
        if n is None:
            continue
        n.rotation_mode = 'XYZ'
        n.rotation_euler.x = ang


def floor(size=12, color=(0.36, 0.34, 0.31)):
    bpy.ops.mesh.primitive_plane_add(size=size)
    p = bpy.context.object
    m = bpy.data.materials.new('floor')
    m.use_nodes = True
    b = m.node_tree.nodes['Principled BSDF']
    b.inputs['Base Color'].default_value = (*color, 1)
    b.inputs['Roughness'].default_value = 0.8
    p.data.materials.append(m)
    return p


def light(kind, loc, target=(0, 0, 0.8), energy=500, color=(1, 1, 1), size=1.0):
    ld = bpy.data.lights.new(kind + str(len(bpy.data.lights)), kind)
    ld.energy = energy
    ld.color = color
    if kind == 'AREA':
        ld.size = size
    if kind == 'SUN':
        ld.angle = math.radians(2)
    o = bpy.data.objects.new(ld.name, ld)
    bpy.context.scene.collection.objects.link(o)
    o.location = loc
    o.rotation_euler = (Vector(target) - Vector(loc)).to_track_quat('-Z', 'Y').to_euler()
    return o


def camera(loc, target, lens=50, ortho=None):
    cd = bpy.data.cameras.new('cam')
    if ortho:
        cd.type = 'ORTHO'
        cd.ortho_scale = ortho
    else:
        cd.lens = lens
    cd.clip_start = 0.02
    o = bpy.data.objects.new('cam', cd)
    bpy.context.scene.collection.objects.link(o)
    o.location = loc
    o.rotation_euler = (Vector(target) - Vector(loc)).to_track_quat('-Z', 'Y').to_euler()
    bpy.context.scene.camera = o
    return o


def neutral_rig(center=(0, 0, 0.9), radius=4.0):
    world_color((0.42, 0.42, 0.43), 0.9)
    c = Vector(center)
    light('AREA', c + Vector((-radius * 0.6, -radius * 0.8, radius * 0.7)), center, 900, (1, 1, 1), 3)
    light('AREA', c + Vector((radius * 0.8, -radius * 0.3, radius * 0.3)), center, 300, (1, 1, 1), 3)
    light('AREA', c + Vector((0.2, radius * 0.9, radius * 0.8)), center, 400, (1, 1, 1), 3)


def render(path):
    bpy.context.scene.render.filepath = str(path)
    bpy.ops.render.render(write_still=True)
    return path


def read_png(path):
    img = bpy.data.images.load(str(path), check_existing=False)
    w, h = img.size
    a = np.array(img.pixels[:], np.float32).reshape(h, w, 4)
    bpy.data.images.remove(img)
    return a


def write_png(path, arr):
    h, w = arr.shape[:2]
    img = bpy.data.images.new('compose', w, h, alpha=True)
    img.pixels.foreach_set(np.clip(arr, 0, 1).astype(np.float32).ravel())
    img.filepath_raw = str(path)
    img.file_format = 'PNG'
    img.save()
    bpy.data.images.remove(img)


def write_jpg(path, arr, quality=90):
    h, w = arr.shape[:2]
    img = bpy.data.images.new('compose_jpg', w, h, alpha=False)
    img.pixels.foreach_set(np.clip(arr, 0, 1).astype(np.float32).ravel())
    img.filepath_raw = str(path)
    img.file_format = 'JPEG'
    bpy.context.scene.render.image_settings.quality = quality
    try:
        img.save(quality=quality)
    except TypeError:
        img.save()
    bpy.data.images.remove(img)


def panels(rv, name, renders, desc, shots):
    """Render each (setup_fn) at half width and join side by side (960x540)."""
    sc = bpy.context.scene
    sc.render.resolution_x = W // len(renders)
    tiles = []
    for i, setup in enumerate(renders):
        setup()
        tmp = rv / f'_panel_{i}.png'
        render(tmp)
        tiles.append(read_png(tmp))
        tmp.unlink()
    sc.render.resolution_x = W
    write_jpg(rv / f'{name}.jpg', np.concatenate(tiles, 1))
    shots.append({'file': f'review/{name}.jpg', 'what': desc})
    print('[W3 render]', name, flush=True)


def resize_nearest(a, w, h):
    ys = (np.arange(h) * a.shape[0] / h).astype(int)
    xs = (np.arange(w) * a.shape[1] / w).astype(int)
    return a[ys][:, xs]


def main():
    o = args()
    out = Path(o['out'])
    rv = out / 'review'
    rv.mkdir(parents=True, exist_ok=True)
    for old in list(rv.glob('*.png')) + list(rv.glob('*.jpg')):
        old.unlink()     # idempotent: the directory holds exactly this run's shots
    man = json.loads((out / 'manifest.json').read_text())
    assets = {a['name']: a for a in man['assets']}
    seated = man['poses']['seated_at_arrival_table']
    shots = []

    def shot(name, desc):
        st = bpy.context.scene.render.image_settings
        st.file_format, st.quality = 'JPEG', 90
        p = render(rv / f'{name}.jpg')
        st.file_format = 'PNG'
        shots.append({'file': f'review/{name}.jpg', 'what': desc})
        print('[W3 render]', name, flush=True)

    robot_glb = out / 'hero-robot.lod0.glb'

    # ---------------------------------------------------------------- robot turnaround (neutral)
    reset(); floor(); neutral_rig()
    root, nodes, _ = import_glb(robot_glb)
    cam = camera((0, -6, 0.9), (0, 0, 0.9), ortho=3.5)
    for name, yaw, desc in (('robot-front', 0, 'front'), ('robot-side', math.pi / 2, 'side (robot left)'),
                            ('robot-rear', math.pi, 'rear'), ('robot-three-quarter', -math.pi / 4, 'three-quarter')):
        root.rotation_mode = 'XYZ'
        root.rotation_euler = (0, 0, yaw)
        shot(name, f'hero robot LOD0, neutral grey light, orthographic {desc}')
    root.rotation_euler = (0, 0, -math.pi / 6)
    bpy.data.objects.remove(cam)
    camera((0.25, -1.3, 1.5), (0, 0, 1.42), lens=85)
    shot('robot-face-closeup', 'face close-up (visor, eyes, ear hubs, shell seams)')
    bpy.data.objects.remove(bpy.context.scene.camera)
    root.rotation_euler = (0, 0, 0)
    camera((0.78, -0.62, 0.66), (0.37, -0.11, 0.55), lens=85)
    shot('robot-hand-closeup', 'hand close-up at rest (articulated fingers, wrist coupling)')
    bpy.data.objects.remove(bpy.context.scene.camera)
    digits = man['rig_contract']['digits']['poses']

    def hand_pose(name):
        def setup():
            for cam_o in [x for x in bpy.data.objects if x.type == 'CAMERA']:
                bpy.data.objects.remove(cam_o)
            for dg, c in digits[name]['curl'].items():
                for sd in ('L', 'R'):
                    n = nodes.get(f'{dg}.{sd}')
                    if n is not None:
                        n.rotation_mode = 'XYZ'
                        n.rotation_euler.x = c
            camera((0.50, -0.78, 0.60), (0.345, -0.11, 0.535), lens=100)
        return setup
    panels(rv, 'robot-hand-closeup-v2', [hand_pose('relaxed'), hand_pose('point')],
           'round 2 hands, front three-quarter, LOD0: left = relaxed rest pose (per-digit pivots at identity); '
           'right = the manifest "point" pose applied to the digit pivots (index extended, others folded, thumb '
           'tucked) - per-finger articulation', shots)
    hand_pose('relaxed')()
    for cam_o in [x for x in bpy.data.objects if x.type == 'CAMERA']:
        bpy.data.objects.remove(cam_o)
    root.rotation_euler = (0, 0, -math.pi / 9)
    camera((0.16, -0.95, 1.52), (0.02, -0.25, 1.48), lens=85)
    shot('robot-face-closeup-v2', 'round 2 face close-up at eye distance: smooth-shaded 40-segment eye domes '
                                  'against the visor lens and bezel')
    bpy.data.objects.remove(bpy.context.scene.camera)
    root.rotation_euler = (0, 0, 0)
    camera((0.78, -0.62, 0.66), (0.37, -0.11, 0.55), lens=85)
    camera((0.5, -0.8, 0.25), (0.12, -0.05, 0.1), lens=70)
    shot('robot-feet-closeup', 'feet: sole, toe cap, heel support, ground contact')

    # relights
    for l in [x for x in bpy.data.objects if x.type == 'LIGHT']:
        bpy.data.objects.remove(l)
    bpy.data.objects.remove(bpy.context.scene.camera)
    camera((2.0, -4.6, 1.15), (0, 0, 0.9), lens=50)
    root.rotation_euler = (0, 0, 0)
    world_color((0.05, 0.06, 0.09), 1.0)
    light('SUN', (4, -1.5, 1.6), (0, 0, 0.9), 6.0, (1.0, 0.70, 0.42))
    light('AREA', (-3, -2, 2), (0, 0, 1), 120, (0.55, 0.7, 1.0), 4)
    shot('robot-relight-warm', 'relight: low warm sun from the right + cool sky fill')
    for l in [x for x in bpy.data.objects if x.type == 'LIGHT']:
        bpy.data.objects.remove(l)
    world_color((0.02, 0.03, 0.06), 1.0)
    light('SUN', (-2, 3, 4), (0, 0, 0.9), 1.2, (0.6, 0.72, 1.0))
    light('POINT', (0.7, -0.9, 0.9), (0, 0, 1), 60, (1.0, 0.6, 0.3))
    shot('robot-relight-night', 'relight: night, cool moon rim + warm lantern-like key')

    # LOD compare
    reset(); floor(); neutral_rig()
    r0, _, _ = import_glb(robot_glb)
    r1, _, _ = import_glb(out / 'hero-robot.lod1.glb')
    place(r0, (-0.5, 0, 0)); place(r1, (0.5, 0, 0))
    camera((0, -6, 0.9), (0, 0, 0.9), ortho=3.5)
    shot('robot-lod0-vs-lod1', f"LOD0 left ({assets['hero-robot']['lods']['lod0']['triangles']} tris) vs "
                               f"LOD1 right ({assets['hero-robot']['lods']['lod1']['triangles']} tris)")
    reset(); floor(); neutral_rig((0, 0, 0.7), 5)
    lods = assets['hero-robot']['lods']
    for i, (n, x) in enumerate((('hero-robot.lod0', -1.6), ('hero-robot.lod1', -0.55), ('hero-robot.lod2', 0.5))):
        r, _, _ = import_glb(out / f'{n}.glb')
        place(r, (x, 0, 0))
    for n, x in (('arrival-chair.lod0', 1.35), ('arrival-chair.lod2', 2.05)):
        r, _, _ = import_glb(out / f'{n}.glb')
        place(r, (x, 0, 0))
    r, _, _ = import_glb(out / 'arrival-table.lod2.glb')
    place(r, (3.55, 0, 0))
    camera((0.95, -9, 1.6), (0.95, 0, 0.75), ortho=6.2)
    ch, tb = assets['arrival-chair']['lods'], assets['arrival-table']['lods']
    shot('robot-lod0-vs-lod1-vs-lod2',
         f"robot LOD0 / LOD1 / LOD2 ({lods['lod0']['triangles']} / {lods['lod1']['triangles']} / "
         f"{lods['lod2']['triangles']} tris), chair LOD0 / LOD2 ({ch['lod0']['triangles']} / {ch['lod2']['triangles']}), "
         f"table LOD2 ({tb['lod2']['triangles']}); orthographic, same scale")

    # ---------------------------------------------------------------- portrait parity
    reset(); neutral_rig((0, 0, 1.3), 3)
    world_color((0.10, 0.12, 0.14), 1.0)
    parity = []
    legacy = Path(o['legacy'])
    if legacy.exists():
        before = set(bpy.data.objects)
        bpy.ops.import_scene.gltf(filepath=str(legacy))
        new = [x for x in bpy.data.objects if x not in before]
        keep = set()
        rob = next(x for x in new if x.name == 'robot')
        stack = [rob]
        while stack:
            x = stack.pop(); keep.add(x); stack += list(x.children)
        for x in new:
            if x not in keep:
                bpy.data.objects.remove(x)
        place(rob, (0, 0, 0))
        camera((0, -2.6, 1.25), (0, 0, 1.2), lens=60)
        render(rv / '_parity_legacy.png')
        parity.append(rv / '_parity_legacy.png')
        for x in list(keep):
            bpy.data.objects.remove(x)
    rv4, _, _ = import_glb(robot_glb)
    place(rv4, (0, 0, 0))
    render(rv / '_parity_v4.png')
    parity.append(rv / '_parity_v4.png')
    tiles = []
    if o['portrait'] and Path(o['portrait']).exists():
        shotimg = read_png(Path(o['portrait']))
        hh, ww = shotimg.shape[:2]
        # 10-desk.png portrait tile (1280x800 capture): x 931..1043, y 232..368 from the top
        top, bot, left, right = 232, 368, 931, 1043
        crop = shotimg[hh - bot:hh - top, left:right]
        tiles.append(resize_nearest(crop, 320, 540))
    for p in parity:
        a = read_png(p)
        tiles.append(resize_nearest(a[:, 240:720], 320, 540))
    if tiles:
        comp = np.concatenate(tiles, 1)
        write_png(rv / 'portrait-parity.png', comp)
        shots.append({'file': 'review/portrait-parity.png',
                      'what': 'left: shipped desk portrait (native 10-desk.png crop, nearest-scaled); middle: legacy '
                              'hero-kit v3b robot; right: v4 robot; same camera for the two renders'})
    for p in parity:
        p.unlink()

    # ---------------------------------------------------------------- ensemble
    def ensemble(night=False):
        reset(); floor(16)
        t = assets['arrival-table']
        anchors = {a['name']: a for a in t['anchors']}
        table, _, _ = import_glb(out / 'arrival-table.lod0.glb')
        for seat in ('host', 'guest'):
            a = anchors[f'anchor_seat_{seat}']
            ch, _, _ = import_glb(out / 'arrival-chair.lod0.glb')
            place(ch, gl2b(a['gltf_position_in_root']), a['gltf_yaw_rad'])
        a = anchors['anchor_seat_host']
        rb, rn, _ = import_glb(robot_glb)
        place(rb, gl2b(a['gltf_position_in_root']) + Vector((0, 0, seated['root_offset_y'])), a['gltf_yaw_rad'])
        pose(rn, seated['rotation_x'])
        for prop, an in (('arrival-folio', 'anchor_folio_slot'), ('arrival-cup', 'anchor_cup_host'),
                         ('arrival-lantern', 'anchor_lantern'), ('arrival-instrument', 'anchor_instrument')):
            pr, _, _ = import_glb(out / f'{prop}.lod0.glb')
            place(pr, gl2b(anchors[an]['gltf_position_in_root']), anchors[an]['gltf_yaw_rad'])
        if night:
            world_color((0.015, 0.02, 0.045), 1.0)
            light('SUN', (-3, 4, 5), (0, 0, 0.5), 0.6, (0.55, 0.68, 1.0))
            lp = gl2b(anchors['anchor_lantern']['gltf_position_in_root']) + Vector((0, 0, 0.10))
            ld = bpy.data.lights.new('lantern', 'POINT'); ld.energy = 25; ld.color = (1.0, 0.62, 0.3)
            ld.shadow_soft_size = 0.02
            lo = bpy.data.objects.new('lantern', ld); bpy.context.scene.collection.objects.link(lo); lo.location = lp
        else:
            neutral_rig((0, 0, 0.6), 5)
        return anchors

    anchors = ensemble()
    camera((2.6, -3.0, 1.65), (0, 0.45, 0.75), lens=35)
    shot('ensemble-arrival', 'arrival approach: host seated, open guest chair, folio, cup, lantern, instrument')
    bpy.data.objects.remove(bpy.context.scene.camera)
    camera((-0.6, 3.6, 1.6), (0.3, -0.2, 0.7), lens=35)
    shot('ensemble-reverse', 'reverse arrival: from behind the host across the table to the open guest chair')
    bpy.data.objects.remove(bpy.context.scene.camera)
    camera((-3.6, 0.6, 1.2), (0, 0.4, 0.7), lens=40)
    shot('ensemble-side', 'side elevation: seated contact (soles, cushion, palms), table and chair heights')
    bpy.data.objects.remove(bpy.context.scene.camera)
    g = gl2b(anchors['anchor_eye_guest']['gltf_position_in_root'])
    hf = gl2b(anchors['anchor_eye_host']['gltf_position_in_root'])
    camera(tuple(g + (g - hf).normalized() * 0.35 + Vector((0, 0, 0.05))), tuple(hf + Vector((0, 0, -0.35))), lens=40)
    shot('seated-face-hands-table', 'matched view: seated guest eye height looking at host face, hands and table')
    bpy.data.objects.remove(bpy.context.scene.camera)
    hc = gl2b(anchors['anchor_hand_host_L']['gltf_position_in_root'])
    hr_ = gl2b(anchors['anchor_hand_host_R']['gltf_position_in_root'])
    mid = (hc + hr_) / 2
    toward = (g - hf).normalized()
    camera(tuple(mid + toward * 1.75 + Vector((0, 0, 0.50))), tuple(mid + (hf - mid) * 0.45), lens=42)
    shot('seated-face-hands-table-v2', 'round 2 matched view, closer: host face, per-digit hands resting palm-down '
                                       '(relaxed pose; fingertips meet the walnut v2 top), seat weave behind')
    bpy.data.objects.remove(bpy.context.scene.camera)
    hl = gl2b(anchors['anchor_hand_host_R']['gltf_position_in_root'])
    camera((hl.x + 0.65, hl.y - 0.55, 0.85), (hl.x - 0.05, hl.y + 0.05, 0.62), lens=60)
    shot('contact-closeup', 'contact close-up: palm on table, cup on saucer, folio on table')
    anchors = ensemble(night=True)
    camera((2.6, -3.0, 1.65), (0, 0.45, 0.75), lens=35)
    shot('ensemble-relight-night', 'relight: night, moonlight + lantern point light (runtime adds the light)')

    # ---------------------------------------------------------------- furniture + props alone
    reset(); floor(); neutral_rig((0, 0, 0.4), 3)
    for i, (yaw, x) in enumerate(((0, -1.0), (math.pi / 2, 0.0), (math.pi, 1.0))):
        c, _, _ = import_glb(out / 'arrival-chair.lod0.glb')
        place(c, (x, 0, 0), yaw)
    camera((0, -6, 0.45), (0, 0, 0.45), ortho=3.4)
    shot('chair-front-side-rear', 'arrival chair: front, side, rear (orthographic)')
    reset(); floor(); neutral_rig((0, 0, 0.1), 2)
    for i, n in enumerate(('arrival-folio', 'arrival-cup', 'arrival-lantern', 'arrival-instrument')):
        p, _, _ = import_glb(out / f'{n}.lod0.glb')
        place(p, (-0.48 + i * 0.32, 0, 0), 0)
    camera((0.0, -1.55, 0.6), (0, 0, 0.09), lens=45)
    shot('props-lineup', 'folio, cup + saucer, lantern, table instrument')
    reset(); floor(); neutral_rig((0, 0, 0.3), 4)
    tb, _, _ = import_glb(out / 'arrival-table.lod0.glb')
    camera((2.2, -2.4, 1.6), (0, 0, 0.35), lens=35)
    shot('table-three-quarter', 'arrival table alone: plank field, breadboard ring, brass inlay, pedestal')
    bpy.data.objects.remove(bpy.context.scene.camera)
    camera((0, -4.5, 0.32), (0, 0, 0.32), ortho=2.4)
    shot('table-side-elevation', 'table elevation: top thickness, apron, splayed legs on brass shoes')

    (rv / 'review.json').write_text(json.dumps({'renderer': f'Blender {bpy.app.version_string} Eevee, {W}x{H}',
                                                'source': 'exported LOD GLBs re-imported (round trip)',
                                                'shots': shots}, indent=2) + '\n')


main()
