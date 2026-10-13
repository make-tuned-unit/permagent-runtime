"""Lane W4 place review scenes, driven by places_review.py. Each scene is
assembled from the exported GLBs at their *suggested runtime placements*,
the layout snapshot's solids (grey context boxes) and the botany scatter, so
the renders show what W6 would integrate, from both directions."""
import json
import math
from pathlib import Path
import bpy
from mathutils import Vector, Matrix

R = None   # places_review module (helpers)
TPL = {}


def rt(p):
    """runtime (x, y, z) -> Blender (x, -z, y)"""
    return Vector((p[0], -p[2], p[1]))


def habitat_height(z):
    return 0.0 if z >= -8 else 3.4 if z <= -14 else (-8 - z) * (3.4 / 6)


def plant(sp, pos, heading, scale):
    if sp not in TPL:
        root, new = R.import_glb(R.ENV / 'botany' / f'{sp}.lod0.glb', (0, 0, -100))
        TPL[sp] = [o for o in new if o.type == 'MESH'][0]
    o = TPL[sp].copy()
    bpy.context.scene.collection.objects.link(o)
    o.parent = None
    o.matrix_world = Matrix.Translation(rt(pos)) @ Matrix.Rotation(heading, 4, 'Z') @ Matrix.Diagonal((scale, scale, scale, 1))


def context(center, radius, layout, scatter, man, skip=(), water=False):
    TPL.clear()
    cx, cz = center
    y = habitat_height(cz) if cz > -33 else 3.4
    if water:
        wx0, wx1, wz0, wz1 = 2.1, 5.9, -5.0, 1.0
        for (x0, x1, z0, z1) in ((-20, wx0, -20, 20), (wx1, 20, -20, 20), (wx0, wx1, -20, wz0), (wx0, wx1, wz1, 20)):
            R.box(rt(((x0 + x1) / 2, -0.05, (z0 + z1) / 2)), (x1 - x0, z1 - z0, 0.1), (0.48, 0.44, 0.37), name='paving')
        R.water_plane(rt((4, -0.03, -2)), (3.8, 6.0), -0.03)
        R.box(rt((4, -0.5, -2)), (3.8, 6.0, 0.05), (0.10, 0.11, 0.09), name='basin floor')
        # walkable bridge deck (0 -> 0.45 over x 1..2, flat 2..6, down 6..7)
        for i in range(60):
            x = 1 + i * 0.1 + 0.05
            h = min(.45, (x - 1) * .45, (7 - x) * .45)
            R.box(rt((x, h - 0.04, -2.2)), (0.1, 2.0, 0.08), (0.34, 0.24, 0.15), name='deck stand-in')
    else:
        R.floor(160, (0.45, 0.41, 0.35), y, (cx, -cz))
        if cz <= -26 and cz > -36:
            pass
    for s in layout['solids'] + [{'name': d['asset'], 'at': [d['position'][0], d['position'][1] + d['size'][1] / 2, d['position'][2]],
                                  'size': d['size']} for d in layout['decorSolids'] if d['asset'] in ('planter',)]:
        if s['name'] == 'observatory plinth':
            if math.hypot(s['at'][0] - cx, s['at'][2] - cz) <= radius:
                bpy.ops.mesh.primitive_cylinder_add(vertices=48, radius=s['size'][0] / 2, depth=s['size'][1],
                                                    location=tuple(rt(s['at'])))
                o = bpy.context.object
                m = bpy.data.materials.new('plinth'); m.use_nodes = True
                m.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value = (0.66, 0.62, 0.55, 1)
                o.data.materials.append(m)
            continue
        if any(k in s['name'] for k in skip):
            continue
        at, sz = s['at'], s['size']
        if math.hypot(at[0] - cx, at[2] - cz) > radius or s['name'] == 'water garden':
            continue
        col = (0.30, 0.22, 0.15) if 'bed' in s['name'] else (0.62, 0.60, 0.56)
        R.box(rt(at), (sz[0], sz[2], sz[1]), col, name='ctx ' + s['name'])
    for it in scatter['instances']:
        p = it['position']
        if math.hypot(p[0] - cx, p[2] - cz) <= radius:
            plant(it['species'], p, it['heading'], it['scale'])
    for pl in man['suggested_placements']:
        p = pl['position']
        if math.hypot(p[0] - cx, p[2] - cz) <= radius:
            root, _ = R.import_glb(R.ENV / 'places' / f"{pl['asset']}.lod0.glb", tuple(rt(p)), pl['heading'])
            if pl.get('scale'):
                root.scale = (pl['scale'][0], pl['scale'][2], pl['scale'][1])
    for t in TPL.values():
        t.hide_render = True


def cam(pos, target, lens=24):
    R.camera(tuple(rt(pos)), tuple(rt(target)), lens)


def run(review):
    global R
    R = review
    env = R.ENV
    layout = json.loads((env / 'layout' / 'layout-snapshot.json').read_text())
    scatter = json.loads((env / 'botany' / 'scatter.json').read_text())
    man = json.loads((env / 'places' / 'manifest.json').read_text())
    shot = R.shot

    def day(c):
        R.day_rig(tuple(rt(c)), 10)

    def night(c, lamps):
        R.night_rig(tuple(rt(c)))
        for (p, e) in lamps:
            R.light('POINT', tuple(rt(p)), tuple(rt(p)), e, (1.0, 0.72, 0.45), 0.15)

    # --- asset lineups (reverse coherence of every asset)
    big = ['archive-bookcase', 'maker-result-shelf', 'maker-workbench', 'observatory-telescope', 'conservatory-growing-bed']
    small = ['archive-folio-stand', 'maker-progress-stand', 'observatory-forecast-plinth', 'conservatory-measurement-panel',
             'conservatory-equipment-cart', 'bench-short']
    cons = ['bridge-abutment-garden', 'water-coping-2m', 'water-coping-corner', 'rail-stanchion', 'column-base-stone',
            'post-shoe-bronze', 'terrace-drain-grate', 'observatory-plinth-steps']
    for tag, names, gap, cz, ortho in (('places-lineup-large', big, 3.2, 1.5, 16.0), ('places-lineup-small', small, 1.7, 0.85, 10.5),
                                       ('construction-lineup', cons, 2.4, 0.6, 19.5)):
        for side, yaw in (('front', 0.0), ('back', math.pi)):
            R.reset(); R.floor(60); R.day_rig((0, 0, 1), 12)
            x0 = -gap * (len(names) - 1) / 2
            xs = [x0 + i * gap for i in range(len(names))]
            if tag == 'construction-lineup':
                xs = [-5.4, -2.5, -0.9, 0.0, 1.0, 2.0, 3.2, 5.0]
            for i, n in enumerate(names):
                R.import_glb(env / 'places' / f'{n}.lod0.glb', (xs[i], 0, 0.3 if n.startswith('water') else 0), yaw)
            if tag == 'construction-lineup':
                R.camera((0, -9.5, 4.4), (0, 0, 0.25), 28)
            else:
                R.camera((0, -30, cz), (0, 0, cz), ortho=ortho)
            shot(f'{tag}-{side}', f'{tag}: LOD0 assets seen from the {side} ({"approach" if side == "front" else "reverse / return path"})')
    # telescope LOD0 vs LOD1 + close-up of mount
    R.reset(); R.floor(30); R.day_rig((0, 0, 1.6), 6)
    R.import_glb(env / 'places' / 'observatory-telescope.lod0.glb', (-1.4, 0, 0), 0.5)
    R.import_glb(env / 'places' / 'observatory-telescope.lod1.glb', (1.4, 0, 0), 0.5)
    R.camera((0, -7.5, 1.9), (0, 0, 1.5), 35)
    shot('telescope-lod0-vs-lod1', 'telescope LOD0 (left) vs LOD1 (right): mount, bearings, fork, rings, finder, eyepiece')
    R.reset(); R.floor(30); R.day_rig((0, 0, 1.6), 4)
    R.import_glb(env / 'places' / 'observatory-telescope.lod0.glb', (0, 0, 0), 0.0)
    R.camera((1.2, -2.2, 1.55), (0, -0.3, 1.45), 35)
    shot('telescope-mount-closeup', 'telescope mount: azimuth bearing race, fork arms, trunnion housings and bolts, eyepiece')

    # --- places: hero (approach) and reverse views, all at runtime placements
    scenes = [
        ('gallery', (-10, -19.5), 9, ['archive shelves', 'archive pillar', 'librarian desk'],
         [((-10, 5.0, -14.8), (-10, 4.5, -21.5), 24, 'gallery-hero', 'Garden archive approach: bookcases, record stand, reading seat'),
          ((-9.0, 5.0, -21.1), (-11.5, 4.4, -16.0), 22, 'gallery-reverse', 'Garden archive reverse: back of the reading seat and stand toward the terrace'),
          ((-7.5, 4.9, -18.1), (-7.6, 4.4, -19.4), 30, 'gallery-folio', 'Archive folio and shelves: reader view onto the open folio')]),
        ('maker', (10, -18.5), 8, ['steward desk', 'workshop pillar'],
         [((10.6, 5.3, -14.2), (10.4, 4.2, -18.6), 26, 'maker-hero', 'Maker workshop approach: workbench, result shelf, in-progress stand'),
          ((10.5, 5.2, -21.6), (9.5, 4.0, -16.5), 24, 'maker-reverse', 'Maker workshop reverse: tool rail back, shelf back, stand back')]),
        ('council_garden', (2, -23.5), 8, ['observatory pillar', 'forecaster desk'],
         [((3.2, 5.0, -19.6), (1.5, 4.4, -23.6), 26, 'observatory-hero', 'Observatory approach: forecast panel plinth beside the desk'),
          ((0.6, 5.0, -26.0), (2.0, 4.2, -21.0), 24, 'observatory-reverse', 'Observatory reverse: plinth back (access plate, drain notch)')]),
        ('observatory_garden', (4, -44), 12, ['observatory plinth', 'observatory bench'],
         [((4.0, 5.2, -34.5), (4, 4.6, -44.5), 26, 'observatory-garden-hero', 'Observatory garden from the north bridge end: telescope on its plinth, steps, benches'),
          ((4.3, 6.4, -58.5), (4, 4.3, -45.0), 22, 'observatory-garden-reverse', 'Observatory garden reverse: bench backs, telescope objective end'),
          ((4.5, 5.35, -40.0), (4, 4.95, -43.6), 28, 'telescope-approach', 'Northern telescope approach: steps, observer position, eyepiece')]),
        ('conservatory', (11, -7.5), 8, ['growth_measurement desk'],
         [((9.5, 1.75, -3.6), (11.2, 0.8, -8.0), 24, 'conservatory-hero', 'Conservatory approach: measurement panel, growing bed, cart'),
          ((11.8, 1.75, -12.0), (10.0, 0.9, -6.0), 24, 'conservatory-reverse', 'Conservatory reverse: panel rear cover/conduit, bed side, cart')]),
    ]
    for pid, c, rad, skip, views in scenes:
        for (cp, tg, lens, name, desc) in views:
            R.reset(16)
            context(c, rad, layout, scatter, man, skip)
            day((c[0], habitat_height(c[1]) + 1, c[1]))
            cam(cp, tg, lens)
            shot(name, desc + ' (runtime placements; grey boxes = existing layout solids)')
    # bridge ends
    R.reset(16)
    context((4, -30.5), 9, layout, scatter, man, ['north rail', 'observatory guard'])
    R.box(rt((4, 3.28, -30.3)), (6.0, 9.0, 0.24), (0.62, 0.58, 0.52), name='bridge deck')
    day((4, 4.4, -30))
    cam((4.3, 5.0, -23.8), (4, 4.0, -30.0), 26)
    shot('bridge-north-terrace-end', 'Northern bridge from the terrace: abutment newels, threshold, drain, guard posts')
    cam((3.7, 5.0, -37.5), (4, 4.0, -29.0), 26)
    shot('bridge-north-pad-end', 'Northern bridge from the observatory pad (return path): far abutment, pier wall')
    cam((10.0, 2.2, -30.5), (4, 2.8, -30.0), 24)
    shot('bridge-north-side', 'Northern bridge side elevation: pier wall, corbel, scupper spouts below the deck')
    R.reset(16)
    context((4, -2), 7, layout, scatter, man, ['water garden', 'bridge north rail', 'bridge south rail', 'cascade'], water=True)
    day((4, 1, -2))
    cam((-0.6, 1.6, -2.4), (4, 0.4, -2.2), 26)
    shot('bridge-garden-west-end', 'Water-garden bridge, west end from land: newels, coping, drain')
    cam((8.6, 1.6, -1.8), (4, 0.4, -2.2), 26)
    shot('bridge-garden-east-end', 'Water-garden bridge, east end (return path)')
    cam((1.0, 0.75, 1.9), (2.6, 0.0, -0.8), 30)
    shot('water-edge-contact', 'Water edge contact: coping lip, batter, wet line, irises standing in the margin')
    # night relight
    R.reset(16)
    context((-10, -19.5), 9, layout, scatter, man, ['archive shelves', 'archive pillar', 'librarian desk'])
    night((-10, 4, -19), [((-10, 5.6, -19), 260), ((-7.6, 4.9, -18.8), 30)])
    cam((-10, 5.0, -14.8), (-10, 4.5, -21.5), 24)
    shot('night-gallery', 'Night relight: Garden archive with the runtime lamp position + a reading lamp')
    R.reset(16)
    context((4, -44), 12, layout, scatter, man, ['observatory plinth', 'observatory bench'])
    night((4, 4, -44), [((4, 7, -43), 900)])
    cam((4.0, 5.2, -34.5), (4, 4.6, -44.5), 26)
    shot('night-observatory-garden', 'Night relight: observatory garden under the runtime court lamp')
    R.reset(16)
    context((2, -23.5), 8, layout, scatter, man, ['observatory pillar', 'forecaster desk'])
    night((2, 4, -23.5), [((1.0, 5.6, -22.0), 120)])
    cam((3.2, 5.0, -19.6), (1.5, 4.4, -23.6), 26)
    shot('night-observatory', 'Night relight: forecast plinth, armillary and panel slot glass')
