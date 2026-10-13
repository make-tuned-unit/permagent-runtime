"""Arrival-table social furniture (lane W3): table, chair, folio, cup, lantern
and table instrument, plus the ensemble layout (seats, hand contacts, prop
slots, gaze targets) recorded as named anchors on the table.

Dimensions are frozen around a *properly seated* hero robot (hero_robot.py):
the robot root drops 0.30 m, thighs rest on a 0.30 m compressed cushion, shins
are vertical with soles on the floor, and flat palms rest 0.13 m inside the
table edge. Table top 0.60 m, radius 1.00 m, seat roots 1.25 m from centre.
Blender space: +Z up, a sitter faces -Y; the exporter converts to glTF +Y up.
"""
import math
import bpy
from mathutils import Vector
from detail_lib import (Kit, Palette, pbr, make_image, tex_walnut, tex_weave, tex_leather, rbox, ellipsoid,
                        lathe, tube, torus, prism, annulus_sector, superquadric, path_tube, rounded_rect_path,
                        box_uv, transform, displace, seg, TAU)

TABLE_R = 1.00
TABLE_TOP = 0.60
SEAT_RADIUS = 1.25          # chair origin (= seated robot root xy) distance from table centre
SEAT_TOP = 0.30             # compressed cushion surface under a seated hero robot
SEAT_DROP = 0.30            # robot root drop when seated (see hero_robot.solve_seated)
HAND_REACH = 0.38           # palm contact distance in front of the robot root
SEATS = {'host': 0.0, 'guest': -110.0, 'companion': 110.0}   # degrees around the table from +Y

FURNITURE_CELLS = {
    'brass': ((0.56, 0.36, 0.15), 0.34, 1.0),
    'bronze': ((0.28, 0.17, 0.085), 0.45, 1.0),
    'graphite': ((0.03, 0.035, 0.037), 0.7, 0.0),
    'oak': ((0.46, 0.30, 0.15), 0.55, 0.0),
    'paper': ((0.86, 0.83, 0.74), 0.86, 0.0),
    'paper_edge': ((0.74, 0.70, 0.61), 0.9, 0.0),
    'coffee': ((0.05, 0.025, 0.012), 0.08, 0.0),
    'strap': ((0.07, 0.09, 0.08), 0.75, 0.0),
    'walnut': ((0.17, 0.09, 0.045), 0.48, 0.0),
}

_SHARED = {}


def shared_materials():
    """Authored procedural maps (numpy), cached per Blender session."""
    if _SHARED:
        return _SHARED
    wc, wn = tex_walnut(1024)
    fc, fn = tex_weave(512)
    lc, ln = tex_leather(512)
    walnut = pbr('Timber • oiled walnut', (1, 1, 1), 0.46, base_img=make_image('walnut_basecolor', wc),
                 normal_img=make_image('walnut_normal', wn[::2, ::2], 'Non-Color'), normal_strength=0.6)
    fabric = pbr('Fabric • moss weave', (1, 1, 1), 0.88, base_img=make_image('weave_basecolor', fc),
                 normal_img=make_image('weave_normal', fn, 'Non-Color'), normal_strength=0.8)
    leather = pbr('Leather • tobacco', (1, 1, 1), 0.55, base_img=make_image('leather_basecolor', lc),
                  normal_img=make_image('leather_normal', ln, 'Non-Color'), normal_strength=0.5)
    stoneware = pbr('Stoneware • oat glaze', (0.80, 0.74, 0.62), 0.2, coat=0.5, coat_rough=0.1)
    glass = pbr('Glass • lantern', (0.86, 0.92, 0.90), 0.05)
    glass.surface_render_method = 'BLENDED'
    glass.node_tree.nodes['Principled BSDF'].inputs['Alpha'].default_value = 0.22
    flame = pbr('Lantern • warm emission', (1.0, 0.62, 0.28), 0.5, emission=(1.0, 0.62, 0.28), emit=3.0)
    display = pbr('Display • jade phosphor', (0.10, 0.45, 0.36), 0.3, emission=(0.14, 0.62, 0.48), emit=0.8)
    _SHARED.update(walnut=walnut, fabric=fabric, leather=leather, stoneware=stoneware, glass=glass,
                   flame=flame, display=display)
    return _SHARED


def _palette(k):
    k.palette = _SHARED.get('palette') or Palette('Trim • brass and bronze', FURNITURE_CELLS)
    _SHARED['palette'] = k.palette


def _seat_frame(name):
    ang = math.radians(SEATS[name])
    pos = Vector((-SEAT_RADIUS * math.sin(ang), SEAT_RADIUS * math.cos(ang), 0))
    # chair front (-Y local) must face the table centre
    return pos, ang


def _local(pos, ang, v):
    c, s = math.cos(ang), math.sin(ang)
    return (pos.x + v[0] * c - v[1] * s, pos.y + v[0] * s + v[1] * c, pos.z + v[2])


# =========================================================================== table
def build_table(detail=1.0):
    d = detail
    hi = d >= 0.8
    m = shared_materials()
    k = Kit('arrival_table', d)
    _palette(k)
    P = 'pal:'
    W = m['walnut']
    z0, z1 = TABLE_TOP - 0.032, TABLE_TOP
    field_r = 0.78
    ring_in = field_r + 0.006
    gap = 0.0035
    if hi:
        # centre field: five planks, grain along X, each clipped to the circle
        n = 5
        for i in range(n):
            a = -field_r + 2 * field_r * i / n + gap / 2
            b = -field_r + 2 * field_r * (i + 1) / n - gap / 2
            arc = seg(10, d, 4)
            out = []
            a0, a1 = math.asin(a / field_r), math.asin(b / field_r)
            for j in range(arc + 1):
                t = a0 + (a1 - a0) * j / arc
                out.append((field_r * math.cos(t), field_r * math.sin(t)))
            for j in range(arc + 1):
                t = (math.pi - a1) + ((math.pi - a0) - (math.pi - a1)) * j / arc
                out.append((field_r * math.cos(t), field_r * math.sin(t)))
            plank = prism(out, z0, z1, bevel=0.003, segments=1)
            box_uv(plank, 0.9, 'x', offset=(0.13 * i, 0.37 * i))
            k.add('arrival_table', W, plank)
        # breadboard ring: eight segments with butt joints
        for i in range(8):
            a0 = TAU * i / 8 + gap / 2 / TABLE_R
            a1 = TAU * (i + 1) / 8 - gap / 2 / TABLE_R
            sec = annulus_sector(ring_in, TABLE_R, a0, a1, z0, z1, segments=seg(8, d, 3), bevel=0.012,
                                 bevel_segments=2)
            mid = (a0 + a1) / 2
            box_uv(sec, 0.9, 'x', offset=(0.21 * i, 0.11 * i), rot=mid + math.pi / 2)
            k.add('arrival_table', W, sec)
        # brass inlay between field and ring, flush with the top
        k.add('arrival_table', P + 'brass', lathe([(field_r - 0.0005, z1 - 0.004), (ring_in + 0.0005, z1 - 0.004),
                                                   (ring_in + 0.0005, z1 - 0.0004), (field_r - 0.0005, z1 - 0.0004)],
                                                  seg(96, d, 32)))
        # dark sub-top: makes plank seams read as real gaps
        k.add('arrival_table', P + 'graphite', lathe([(0, z0 - 0.012), (0.975, z0 - 0.012), (0.975, z0 + 0.0005),
                                                      (0, z0 + 0.0005)], seg(48, d, 16)))
        # brass edge band on the outer face, below the rounded rim
        k.add('arrival_table', P + 'brass', lathe([(TABLE_R - 0.001, z0 + 0.002), (TABLE_R + 0.0025, z0 + 0.002),
                                                   (TABLE_R + 0.0025, z0 + 0.011), (TABLE_R - 0.001, z0 + 0.011)],
                                                  seg(96, d, 32)))
    else:
        top = lathe([(0, z0), (TABLE_R, z0), (TABLE_R, z1 - 0.008), (TABLE_R - 0.01, z1), (0, z1)], seg(40, d, 24))
        box_uv(top, 0.9, 'x')
        k.add('arrival_table', W, top)
        k.add('arrival_table', P + 'brass', lathe([(field_r - 0.002, z1 + 0.0002), (ring_in, z1 + 0.0002),
                                                   (ring_in, z1 + 0.0006), (field_r - 0.002, z1 + 0.0006)], seg(40, d, 24)))
    # apron ring with pegs
    apron = lathe([(0.58, 0.49), (0.62, 0.49), (0.62, z0 - 0.012), (0.58, z0 - 0.012)], seg(48, d, 16))
    box_uv(apron, 1.4, 'z')
    k.add('arrival_table', W, apron)
    if hi:
        for i in range(8):
            a = TAU * (i + 0.5) / 8
            k.add('arrival_table', P + 'oak', tube((0.615 * math.cos(a), 0.615 * math.sin(a), 0.523),
                                                   (0.626 * math.cos(a), 0.626 * math.sin(a), 0.523), 0.009, 10))
    # radial cleats under the top (45 deg from the legs)
    for i in range(4):
        a = TAU * i / 4 + math.pi / 4
        cl = rbox((0.33, 0, z0 - 0.012 - 0.02), (0.50, 0.05, 0.04), 0.006, 1)
        transform(cl, (0, 0, 0), (0, 0, a))
        box_uv(cl, 1.2, 'x', rot=a)
        k.add('arrival_table', W, cl)
    # turned pedestal column
    col = lathe([(0, 0.12), (0.105, 0.12), (0.105, 0.15), (0.088, 0.19), (0.072, 0.27), (0.066, 0.36),
                 (0.074, 0.43), (0.09, 0.47), (0.11, 0.49), (0.11, z0 - 0.012), (0, z0 - 0.012)], seg(28, d, 12))
    box_uv(col, 1.3, 'z')
    k.add('arrival_table', W, col)
    for z, r in ((0.19, 0.089), (0.455, 0.083)):
        k.add('arrival_table', P + 'brass', torus((0, 0, z), r, 0.008, seg(28, d, 12), 4))
    # cross-halved hub and four splayed legs with brass shoes on the floor
    hub = rbox((0, 0, 0.17), (0.24, 0.24, 0.12), 0.022, seg(2, d, 1))
    box_uv(hub, 1.3, 'x')
    k.add('arrival_table', W, hub)
    for i in range(4):
        a = TAU * i / 4
        c, s = math.cos(a), math.sin(a)
        start = Vector((0.07 * c, 0.07 * s, 0.17))
        end = Vector((0.60 * c, 0.60 * s, 0.045))
        dvec = end - start
        leg = rbox((0, 0, 0), (dvec.length + 0.04, 0.075, 0.09), 0.014, seg(2, d, 1))
        # taper: shrink the far end
        for v in leg.verts:
            t = (v.co.x + dvec.length / 2) / dvec.length
            v.co.y *= 1 - 0.3 * max(0, t)
            v.co.z *= 1 - 0.35 * max(0, t)
        pitch = math.atan2(-dvec.z, math.hypot(dvec.x, dvec.y))
        transform(leg, tuple((start + end) / 2), (0, pitch, a))
        box_uv(leg, 1.2, 'x')
        k.add('arrival_table', W, leg)
        sh = rbox((0, 0, 0.016), (0.11, 0.075, 0.032), 0.008, seg(2, d, 1))
        transform(sh, (0.605 * c, 0.605 * s, 0), (0, 0, a))
        k.add('arrival_table', P + 'brass', sh)
        k.add('arrival_table', P + 'brass', tube((0.60 * c, 0.60 * s, 0.032), (0.60 * c, 0.60 * s, 0.07), 0.03,
                                                 seg(16, d, 8)))

    # ---- ensemble anchors (positions in table space; consumer = W6 placements, W7 motion)
    k.anchor('anchor_table_surface', 'arrival_table', (0, 0, TABLE_TOP), kind='surface',
             note=f'table top centre; walkable-blocking cylinder r {TABLE_R} h {TABLE_TOP}')
    for name in SEATS:
        pos, ang = _seat_frame(name)
        k.anchor(f'anchor_seat_{name}', 'arrival_table', tuple(pos), (0, 0, ang), kind='seat',
                 note='place arrival_chair origin here; its -Y (glTF +Z) faces the table centre')
        k.anchor(f'anchor_eye_{name}', 'arrival_table', _local(pos, ang, (0, -0.32, 1.49 - SEAT_DROP)),
                 kind='gaze', note='seated hero-robot eye centre; eye-contact height')
        for side, sgn in (('L', -1), ('R', 1)):
            k.anchor(f'anchor_hand_{name}_{side}', 'arrival_table',
                     _local(pos, ang, (sgn * 0.36, -HAND_REACH, TABLE_TOP)), (0, 0, ang), kind='contact',
                     note='flat-palm contact point on the table top for a seated hero robot')
    host, hang = _seat_frame('host')
    guest, gang = _seat_frame('guest')
    folio = (0.36, 0.40, TABLE_TOP)
    k.anchor('anchor_folio_slot', 'arrival_table', folio, (0, 0, math.radians(-25)), kind='attachment',
             note='work folio rests here (interaction object), within the host right hand reach, read by the guest')
    k.anchor('anchor_gaze_shared', 'arrival_table', (folio[0], folio[1], TABLE_TOP + 0.05), kind='gaze',
             note='joint-attention target above the folio')
    k.anchor('anchor_cup_host', 'arrival_table', (-0.58, 0.70, TABLE_TOP), kind='attachment', note='cup + saucer')
    k.anchor('anchor_lantern', 'arrival_table', (-0.40, -0.34, TABLE_TOP), kind='light',
             note='table lantern; keep clear of host-guest sightline')
    k.anchor('anchor_instrument', 'arrival_table', (0.04, 0.02, TABLE_TOP), (0, 0, math.radians(-35)),
             kind='attachment', note='restrained table instrument, display faces the guest')
    k.finish()
    return k


def table_collision():
    k = Kit('arrival_table_collision', 1.0)
    k.add('arrival_table_collision', pbr('collision', (1, 0, 1), 1), lathe(
        [(0, 0), (TABLE_R, 0), (TABLE_R, TABLE_TOP), (0, TABLE_TOP)], 16))
    k.finish()
    return k, {'type': 'cylinder', 'radius': TABLE_R, 'height': TABLE_TOP, 'center': [0, TABLE_TOP / 2, 0],
               'blocking': True}


# =========================================================================== chair
def build_chair(detail=1.0):
    d = detail
    hi = d >= 0.8
    m = shared_materials()
    k = Kit('arrival_chair', d)
    _palette(k)
    P = 'pal:'
    W, F = m['walnut'], m['fabric']
    fx, bx = 0.27, 0.27
    fy, by = -0.24, 0.215
    rake = math.radians(9)
    # legs (front) and back legs rising into raked posts
    for sgn in (-1, 1):
        leg = rbox((sgn * fx, fy, 0.11), (0.042, 0.042, 0.22), 0.008, seg(2, d, 1))
        for v in leg.verts:
            if v.co.z < 0.05:
                v.co.x = sgn * fx + (v.co.x - sgn * fx) * 0.8
                v.co.y = fy + (v.co.y - fy) * 0.8
        box_uv(leg, 1.4, 'z')
        k.add('arrival_chair', W, leg)
        post_len = 0.82
        post = rbox((0, 0, post_len / 2), (0.044, 0.046, post_len), 0.008, seg(2, d, 1))
        transform(post, (sgn * bx, by, 0))
        for v in post.verts:    # rake only above the seat rail
            h = max(0.0, v.co.z - 0.22)
            v.co.y += h * math.tan(rake)
        box_uv(post, 1.4, 'z')
        k.add('arrival_chair', W, post)
        for (x, y) in ((sgn * fx, fy), (sgn * bx, by)):
            k.add('arrival_chair', P + 'brass', rbox((x, y, 0.009), (0.05, 0.05, 0.018), 0.004, 1))
    # rails (seat frame) with oak pegs at the mortise joints
    rails = [((0, fy, 0.19), (2 * fx - 0.03, 0.03, 0.06), 'x'),
             ((0, by, 0.19), (2 * bx - 0.03, 0.03, 0.06), 'x'),
             ((-fx, (fy + by) / 2, 0.19), (0.03, by - fy - 0.03, 0.06), 'y'),
             ((fx, (fy + by) / 2, 0.19), (0.03, by - fy - 0.03, 0.06), 'y')]
    for (c, sz, g) in rails:
        r = rbox(c, sz, 0.006, 1)
        box_uv(r, 1.4, g)
        k.add('arrival_chair', W, r)
    # stretchers low between legs (sidewise) for construction credibility
    for sgn in (-1, 1):
        r = rbox((sgn * fx, (fy + by) / 2, 0.07), (0.024, by - fy, 0.028), 0.006, 1)
        box_uv(r, 1.4, 'y')
        k.add('arrival_chair', W, r)
    if hi:
        for sgn in (-1, 1):
            for (x, y, nrm) in ((sgn * fx, fy - 0.022, (0, -1, 0)), (sgn * bx, by + 0.022, (0, 1, 0)),
                                (sgn * (fx + 0.022), fy + 0.03, (sgn, 0, 0)), (sgn * (bx + 0.022), by - 0.03, (sgn, 0, 0))):
                a = Vector((x, y, 0.19))
                k.add('arrival_chair', P + 'oak', tube(tuple(a - Vector(nrm) * 0.004), tuple(a + Vector(nrm) * 0.002),
                                                       0.0065, 8))
    # top rail across the posts
    top_y = by + (0.80 - 0.22) * math.tan(rake)
    tr = rbox((0, top_y, 0.79), (2 * bx + 0.05, 0.035, 0.07), 0.012, seg(2, d, 1))
    for v in tr.verts:   # gentle curve that follows a sitter's back
        v.co.y += 0.03 * (1 - (v.co.x / (bx + 0.03)) ** 2)
    box_uv(tr, 1.4, 'x')
    k.add('arrival_chair', W, tr)
    # seat cushion: pillow with compression under the sitter, piping seams top and bottom
    cw, cd, ch = 0.60, 0.50, 0.11
    ccy, cz = -0.015, 0.215 + ch / 2
    cush = superquadric((0, ccy, cz), (cw / 2, cd / 2, ch / 2), 0.22, 0.32, seg(40, d, 16), seg(20, d, 8))
    top_z = cz + ch / 2

    def compress(co):
        x, y, z = co
        if z > cz:
            w = math.exp(-((x / 0.20) ** 2 + ((y - 0.0) / 0.17) ** 2))
            z -= (top_z - SEAT_TOP) * w * min(1.0, (z - cz) / (ch / 2)) * 1.0
            # displaced filling bulges the side walls slightly
            r = math.hypot(x / (cw / 2), (y - ccy) / (cd / 2))
            if 0.8 < r:
                sc = 1 + 0.012 * w
                x *= sc
        return (x, y, z)
    displace(cush, compress)
    box_uv(cush, 2.2, 'x')
    k.add('arrival_chair', F, cush)
    if hi:
        for zz in (top_z - 0.012, cz - ch / 2 + 0.014):
            k.add('arrival_chair', F, path_tube(rounded_rect_path(0, ccy, zz, cw - 0.012, cd - 0.012, 0.05, 4),
                                                0.0055, 5))
        # tufting buttons
        for (x, y) in ((-0.15, -0.12), (0.15, -0.12), (-0.15, 0.1), (0.15, 0.1)):
            k.add('arrival_chair', P + 'bronze', ellipsoid((x, y, top_z - 0.012), (0.009, 0.009, 0.005), 8, 4))
    # back cushion following the rake
    bcz = 0.60
    bcy = by + (bcz - 0.22) * math.tan(rake) - 0.045
    back = superquadric((0, 0, 0), (0.27, 0.045, 0.19), 0.22, 0.3, seg(36, d, 14), seg(16, d, 8))
    displace(back, lambda co: (co[0], co[1] - 0.012 * (1 - (co[0] / 0.27) ** 2) * (1 - (co[2] / 0.19) ** 2), co[2]))
    transform(back, (0, bcy, bcz), (-rake, 0, 0))
    box_uv(back, 2.2, 'x')
    k.add('arrival_chair', F, back)
    if hi:
        pipe = path_tube([(x, 0, z) for (x, _, z) in rounded_rect_path(0, 0, 0, 0.52, 0.36, 0.05, 4)], 0.005, 5)
        # rounded_rect_path is in XY; re-map Y->Z for the back cushion's front face
        for v in pipe.verts:
            v.co.y, v.co.z = v.co.z, v.co.y
        transform(pipe, (0, bcy - 0.04, bcz), (-rake, 0, 0))
        k.add('arrival_chair', F, pipe)
    # back cushion straps to the top rail
    for sgn in (-1, 1):
        k.add('arrival_chair', P + 'strap', rbox((sgn * 0.17, top_y + 0.005, 0.765), (0.03, 0.045, 0.05), 0.004, 1))

    k.anchor('anchor_seat', 'arrival_chair', (0, 0.0, SEAT_TOP), kind='seat',
             note='compressed cushion surface; align hero robot anchor_seat_contact here (root drops 0.30 m)')
    k.anchor('anchor_back', 'arrival_chair', (0, bcy - 0.05, bcz), kind='contact', note='back cushion front face')
    k.anchor('anchor_feet', 'arrival_chair', (0, -0.36, 0), kind='contact', note='seated soles land here')
    k.anchor('anchor_stand', 'arrival_chair', (0, -0.62, 0), kind='approach',
             note='where a sitter stands, facing away from the table, before sitting / after rising')
    k.anchor('anchor_eye_seated', 'arrival_chair', (0, -0.32, 1.49 - SEAT_DROP), kind='gaze',
             note='seated hero-robot eye centre')
    k.finish()
    return k


def chair_collision():
    k = Kit('arrival_chair_collision', 1.0)
    k.add('arrival_chair_collision', pbr('collision', (1, 0, 1), 1), rbox((0, 0.02, 0.43), (0.62, 0.56, 0.86), 0, 0))
    k.finish()
    return k, {'type': 'box', 'size': [0.62, 0.86, 0.56], 'center': [0, 0.43, -0.02], 'blocking': True}


# =========================================================================== small props
def build_folio(detail=1.0):
    d = detail
    hi = d >= 0.8
    m = shared_materials()
    k = Kit('arrival_folio', d)
    _palette(k)
    P = 'pal:'
    Lm = m['leather']
    L, D = 0.32, 0.235
    back = rbox((0, 0, 0.002), (L, D, 0.004), 0.0015, seg(2, d, 1))
    box_uv(back, 3.0, 'x')
    k.add('arrival_folio', Lm, back)
    k.add('arrival_folio', P + 'paper_edge', rbox((0.004, 0, 0.0075), (L - 0.016, D - 0.014, 0.007), 0.0015, 1))
    k.add('arrival_folio', P + 'paper', rbox((0.004, 0, 0.0138), (L - 0.018, D - 0.016, 0.0056), 0.0012, 1))
    if hi:
        k.add('arrival_folio', P + 'paper', rbox((0.03, 0.004, 0.0170), (L - 0.07, D - 0.03, 0.0008), 0.0003, 1))
    hinge = (-L / 2, 0, 0.019)
    k.pivot('Folio_Cover', 'arrival_folio', hinge)
    cover = rbox((0, 0, 0.0195), (L, D, 0.004), 0.0015, seg(2, d, 1))
    box_uv(cover, 3.0, 'x', offset=(0.3, 0.1))
    k.add('Folio_Cover', Lm, cover)
    spine = tube((-L / 2, -D / 2, 0.0105), (-L / 2, D / 2, 0.0105), 0.0105, seg(12, d, 6))
    box_uv(spine, 3.0, 'y')
    k.add('arrival_folio', Lm, spine)
    if hi:
        for sy in (-1, 1):   # brass corner guards on the free edge
            k.add('Folio_Cover', P + 'brass', rbox((L / 2 - 0.012, sy * (D / 2 - 0.012), 0.0198), (0.03, 0.03, 0.0052), 0.002, 1))
        # deep-pressed border line (authored mark, no lettering)
        k.add('Folio_Cover', P + 'bronze', path_tube(rounded_rect_path(0.008, 0, 0.0218, L - 0.05, D - 0.04, 0.01, 2),
                                                      0.0009, 4))
    # elastic strap around the free end (closes the folio)
    sx = L / 2 - 0.045
    k.add('Folio_Cover', P + 'strap', rbox((sx, 0, 0.0222), (0.012, D + 0.002, 0.0012), 0.0004, 1))
    k.add('arrival_folio', P + 'strap', rbox((sx, 0, -0.0004), (0.012, D + 0.002, 0.0008), 0.0003, 1))
    for sy in (-1, 1):
        k.add('arrival_folio', P + 'strap', rbox((sx, sy * (D / 2 + 0.0012), 0.011), (0.012, 0.0012, 0.0236), 0.0004, 1))
    k.anchor('anchor_folio_grip', 'arrival_folio', (L / 2 - 0.02, -D / 2 + 0.05, 0.012), kind='attachment',
             note='hand grip on the free edge')
    k.anchor('anchor_folio_reading', 'arrival_folio', (0, 0, 0.03), kind='gaze', note='reading / camera target')
    k.anchor('anchor_folio_contact', 'arrival_folio', (0, 0, 0), kind='contact', note='rests on table top')
    k.finish()
    return k


def build_cup(detail=1.0):
    d = detail
    hi = d >= 0.8
    m = shared_materials()
    k = Kit('arrival_cup', d)
    _palette(k)
    S = m['stoneware']
    sau = lathe([(0, 0.0015), (0.032, 0.0015), (0.035, 0.0), (0.039, 0.0), (0.05, 0.004), (0.072, 0.009),
                 (0.077, 0.0125), (0.0745, 0.0135), (0.05, 0.0085), (0.036, 0.0072), (0, 0.0068)], 28 if hi else 10)
    k.add('arrival_cup', S, sau)
    zb = 0.0068
    cup = lathe([(0, zb + 0.003), (0.023, zb + 0.003), (0.025, zb), (0.03, zb), (0.033, zb + 0.006),
                 (0.038, zb + 0.03), (0.0405, zb + 0.072), (0.041, zb + 0.077), (0.0395, zb + 0.0785),
                 (0.0375, zb + 0.076), (0.0355, zb + 0.03), (0.029, zb + 0.012), (0, zb + 0.011)], 28 if hi else 10)
    k.add('arrival_cup', S, cup)
    k.add('arrival_cup', 'pal:coffee', lathe([(0, zb + 0.058), (0.0362, zb + 0.058), (0.0362, zb + 0.060),
                                              (0, zb + 0.060)], 24 if hi else 8))
    h = torus((0, 0, 0), 0.019, 0.0058, seg(16, d, 6), seg(7, d, 4), rot=(math.pi / 2, 0, 0),
              arc=math.radians(230), phase=math.radians(-115))
    transform(h, (0.047, 0, zb + 0.044))
    k.add('arrival_cup', S, h)
    k.anchor('anchor_cup_handle', 'arrival_cup', (0.064, 0, zb + 0.044), kind='attachment', note='handle grip')
    k.anchor('anchor_cup_lip', 'arrival_cup', (-0.04, 0, zb + 0.078), kind='contact', note='drinking edge')
    k.anchor('anchor_cup_contact', 'arrival_cup', (0, 0, 0), kind='contact', note='saucer foot on the table')
    k.finish()
    return k


def build_lantern(detail=1.0):
    d = detail
    hi = d >= 0.8
    m = shared_materials()
    k = Kit('arrival_lantern', d)
    _palette(k)
    P = 'pal:'
    k.add('arrival_lantern', P + 'brass', lathe([(0, 0), (0.068, 0), (0.072, 0.006), (0.07, 0.018), (0.06, 0.024),
                                                 (0, 0.024)], seg(32, d, 12)))
    k.add('arrival_lantern', m['glass'], lathe([(0.049, 0.024), (0.053, 0.024), (0.053, 0.196), (0.049, 0.196)],
                                                seg(32, d, 12)))
    for i in range(4):
        a = TAU * i / 4 + math.pi / 4
        k.add('arrival_lantern', P + 'brass', tube((0.059 * math.cos(a), 0.059 * math.sin(a), 0.02),
                                                   (0.059 * math.cos(a), 0.059 * math.sin(a), 0.205), 0.0055, seg(8, d, 6)))
    k.add('arrival_lantern', P + 'brass', lathe([(0, 0.196), (0.07, 0.196), (0.072, 0.204), (0.05, 0.226),
                                                 (0.022, 0.244), (0.016, 0.25), (0, 0.25)], seg(32, d, 12)))
    k.add('arrival_lantern', P + 'brass', torus((0, 0, 0.268), 0.019, 0.0042, seg(20, d, 10), 5, rot=(math.pi / 2, 0, 0)))
    # warm core on a bronze stem
    k.add('arrival_lantern', P + 'bronze', tube((0, 0, 0.024), (0, 0, 0.07), 0.008, 10))
    k.add('arrival_lantern', m['flame'], ellipsoid((0, 0, 0.098), (0.016, 0.016, 0.03), seg(16, d, 8), seg(10, d, 6)))
    if hi:
        for i in range(8):
            a = TAU * i / 8
            k.add('arrival_lantern', P + 'bronze', ellipsoid((0.062 * math.cos(a), 0.062 * math.sin(a), 0.21),
                                                             (0.005, 0.005, 0.004), 6, 4))
    k.anchor('anchor_light', 'arrival_lantern', (0, 0, 0.10), kind='light',
             note='point light origin (warm 2700K, small radius); emissive core is in the mesh')
    k.anchor('anchor_lantern_handle', 'arrival_lantern', (0, 0, 0.285), kind='attachment', note='carry ring')
    k.anchor('anchor_lantern_contact', 'arrival_lantern', (0, 0, 0), kind='contact', note='base on table')
    k.finish()
    return k


def build_instrument(detail=1.0):
    """Restrained evidence instrument: a walnut base, a brass gimbal ring with
    authored tick marks, and a tilted dark-glass display with three phosphor
    bars (reads as a measurement without any text)."""
    d = detail
    hi = d >= 0.8
    m = shared_materials()
    k = Kit('arrival_instrument', d)
    _palette(k)
    P = 'pal:'
    base = lathe([(0, 0), (0.085, 0), (0.088, 0.006), (0.084, 0.024), (0.07, 0.03), (0, 0.03)], seg(36, d, 14))
    k.add('arrival_instrument', P + 'walnut', base)
    k.add('arrival_instrument', P + 'brass', lathe([(0, 0.03), (0.03, 0.03), (0.026, 0.05), (0, 0.05)], seg(20, d, 10)))
    for sgn in (-1, 1):
        k.add('arrival_instrument', P + 'brass', tube((sgn * 0.02, 0, 0.045), (sgn * 0.082, 0, 0.115), 0.006, 8))
    ringc = (0, 0, 0.125)
    k.add('arrival_instrument', P + 'brass', torus(ringc, 0.082, 0.0065, seg(40, d, 16), seg(6, d, 4),
                                                    rot=(math.pi / 2, 0, 0)))
    tilt = math.radians(-20)
    disc = lathe([(0, -0.004), (0.066, -0.004), (0.068, 0), (0.066, 0.004), (0, 0.004)], seg(32, d, 12))
    transform(disc, ringc, (math.pi / 2 + tilt, 0, 0))
    k.add('arrival_instrument', P + 'graphite', disc)
    for i, (w, h) in enumerate(((0.016, 0.05), (0.016, 0.075), (0.016, 0.034))):
        bar = rbox((-0.03 + i * 0.03, -0.0048, -0.035 + h / 2), (w, 0.0016, h), 0.001, 1)
        transform(bar, ringc, (tilt, 0, 0))
        k.add('arrival_instrument', m['display'], bar)
    if hi:
        for i in range(24):
            a = TAU * i / 24
            ln = 0.012 if i % 6 == 0 else 0.006
            t = rbox((0, -0.0075, 0), (0.0022, 0.003, ln), 0.0005, 1)
            transform(t, (0, 0, 0), (0, a, 0))
            transform(t, (ringc[0] + 0.0745 * math.sin(a), 0, ringc[2] + 0.0745 * math.cos(a)))
            k.add('arrival_instrument', P + 'bronze', t)
    k.anchor('anchor_instrument_face', 'arrival_instrument', (0, -0.01, 0.125), (tilt, 0, 0), kind='gaze',
             note='display centre; the display faces -Y (glTF +Z)')
    k.anchor('anchor_instrument_touch', 'arrival_instrument', (0, 0, 0.215), kind='contact', note='top of the ring')
    k.anchor('anchor_instrument_contact', 'arrival_instrument', (0, 0, 0), kind='contact', note='base on table')
    k.finish()
    return k


def box_collision(root, size, center, blocking=False):
    k = Kit(root + '_collision', 1.0)
    k.add(root + '_collision', pbr('collision', (1, 0, 1), 1), rbox(center, size, 0, 0))
    k.finish()
    # manifest reports glTF axes: (x, z, -y)
    return k, {'type': 'box', 'size': [size[0], size[2], size[1]], 'center': [center[0], center[2], -center[1]],
               'blocking': blocking}
