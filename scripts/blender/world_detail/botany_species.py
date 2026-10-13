"""Lane W4 species kit: seven readable plant forms, each built at LOD0/LOD1.

Forms (silhouette first, no claim of scientific species identity):
  canopy-olive      umbrella canopy tree: flared trunk, 3 primaries, 9 secondaries,
                    18 twigs, alpha-tested sprig cards (LOD1: larger cluster cards)
  shrub-laurel      rounded broadleaf shrub: 8 stems + side shoots, geometry leaves
                    (LOD1: sprig cards)
  fern-sword        vase of arching fronds with tapering geometry pinnae + fiddleheads
  flower-coneflower basal rosette + 7 daisy heads with drooping petals
  grass-fountain    fountain tussock of arching blades with straw tips + seed plumes
  iris-water        vertical sword-leaf fans with three flowers (water-edge form)
  vine-pendant      hanging vine strand with heart leaves (arbor / rail form)
"""
import math
import random
from mathutils import Vector, Matrix
import detail_lib as L
import botany_lib as B

TAU = math.tau
Z = Vector((0, 0, 1))


def _rot(v, axis, ang):
    return Matrix.Rotation(ang, 3, Vector(axis)) @ Vector(v)


def _curve(p0, d0, length, segs, bend_up=0.0, droop=0.0, wobble=0.0, rng=None):
    """Polyline from p0 heading d0 that bends toward +Z (bend_up>0) or down."""
    pts = [Vector(p0)]
    d = Vector(d0).normalized()
    step = length / segs
    for i in range(segs):
        d = (d + Z * (bend_up - droop) * (1.0 / segs) * 2.0).normalized()
        if wobble and rng:
            d = (d + Vector((rng.uniform(-1, 1), rng.uniform(-1, 1), rng.uniform(-1, 1))) * wobble).normalized()
        pts.append(pts[-1] + d * step)
    return pts


def _at(pts, t):
    """Point and tangent at fraction t along a polyline."""
    seglens = [(pts[i + 1] - pts[i]).length for i in range(len(pts) - 1)]
    total = sum(seglens)
    target = t * total
    acc = 0
    for i, sl in enumerate(seglens):
        if acc + sl >= target or i == len(seglens) - 1:
            f = 0 if sl == 0 else (target - acc) / sl
            return pts[i].lerp(pts[i + 1], min(max(f, 0), 1)), (pts[i + 1] - pts[i]).normalized()
        acc += sl
    return pts[-1], (pts[-1] - pts[-2]).normalized()


def _wind(kit, root, sway_z, tip_z, profile, amp, freq, stiff):
    rule = 'weight = smoothstep(0, 1, (y - wind_root.y) / (wind_tip.y - wind_root.y)); offset = amp * weight^2'
    for name, z, note in (('wind_root', 0.0, 'zero-motion pivot (soil contact / attachment)'),
                          ('wind_sway', sway_z, 'primary sway pivot (trunk top / clump centre)'),
                          ('wind_tip', tip_z, 'height of full-amplitude motion')):
        a = kit.anchor(name, root, (0, 0, z), kind='wind', note=note)
        a['wind_profile'] = profile
        a['wind_amplitude_m'] = amp
        a['wind_frequency_hz'] = freq
        a['wind_stiffness'] = stiff
        a['wind_weight_rule'] = rule


def leaflet(base, direction, up, length, width, uv):
    """2-triangle diamond leaflet (fern pinna, petal)."""
    import bmesh
    d = Vector(direction).normalized()
    side = d.cross(Vector(up))
    side = side.normalized() if side.length > 1e-6 else Vector((1, 0, 0))
    b = Vector(base)
    bm = bmesh.new()
    uvl = bm.loops.layers.uv.verify()
    pts = (b, b + d * length * 0.38 - side * width, b + d * length, b + d * length * 0.38 + side * width)
    vs = [bm.verts.new(p) for p in pts]
    u, v0, v1 = uv
    tv = (0.0, 0.4, 1.0, 0.4)
    for tri in ((0, 3, 2), (0, 2, 1)):
        f = bm.faces.new([vs[k] for k in tri])
        for loop, k in zip(f.loops, tri):
            loop[uvl].uv = (u, v0 + (v1 - v0) * tv[k])
    return bm


def _cell(m, name):
    u, (v0, v1) = m.pal_uv[name]
    return (u, v0, v1)


# ------------------------------------------------------------------ canopy tree
def build_canopy(m, lod):
    rng = random.Random(7001)
    hi = lod == 0
    k = L.Kit('canopy_olive')
    root = 'canopy_olive'
    TOP = 2.30
    n = 12 if hi else 6
    trunk = [Vector((0.07 * math.sin(z * 1.3), 0.05 * math.sin(z * 0.9 + 1.0), z))
             for z in [-0.10 + (TOP + 0.10) * i / n for i in range(n + 1)]]
    radii = [0.13 + 0.13 * math.exp(-max(p.z, 0) / 0.22) - 0.015 * p.z / TOP for p in trunk]
    k.add(root, m.bark, B.sweep(trunk, radii, 10 if hi else 6, uv_v_scale=0.9, cap_tip=False, cap_base=True))
    if hi:   # surface roots so the flare reads as rooted on any slope
        for i in range(4):
            a = TAU * i / 4 + rng.uniform(-0.3, 0.3)
            d = Vector((math.cos(a), math.sin(a), 0))
            pts = [Vector((0, 0, 0.18)) + d * 0.12, d * 0.32 + Vector((0, 0, 0.03)), d * 0.52 + Vector((0, 0, -0.04)),
                   d * 0.66 + Vector((0, 0, -0.08))]
            k.add(root, m.bark, B.sweep(pts, [0.08, 0.05, 0.03, 0.015], 5, uv_v_scale=0.9))
    top = trunk[-1]
    clumps = []          # (centre, radius, n_cards)
    for pi in range(4):
        az = TAU * pi / 4 + rng.uniform(-0.3, 0.3)
        out = Vector((math.cos(az), math.sin(az), 0))
        tilt = math.radians(rng.uniform(50, 58))
        d0 = (out * math.sin(tilt) + Z * math.cos(tilt)).normalized()
        plen = rng.uniform(1.5, 1.8)
        prim = _curve(top - Z * 0.05, d0, plen, 7 if hi else 4, bend_up=0.12, wobble=0.04, rng=rng)
        k.add(root, m.bark, B.sweep(prim, [0.10 - 0.055 * i / (len(prim) - 1) for i in range(len(prim))],
                                    8 if hi else 5, uv_v_scale=0.9))
        for si, t in enumerate((0.55, 0.8, 1.0)):
            p, tan = _at(prim, t)
            yaw = (-1 if si % 2 else 1) * rng.uniform(0.5, 0.8) if si < 2 else rng.uniform(-0.2, 0.2)
            sd = (_rot(tan, Z, yaw) + out * 0.5 + Z * 0.15).normalized()
            sec = _curve(p, sd, rng.uniform(0.9, 1.2), 4 if hi else 2, bend_up=0.15, wobble=0.06, rng=rng)
            k.add(root, m.bark, B.sweep(sec, [0.048 - 0.03 * i / (len(sec) - 1) for i in range(len(sec))],
                                        6 if hi else 4, uv_v_scale=1.4))
            tips = []
            if hi:
                for tw_i in range(2):
                    q, qt = _at(sec, 0.6 if tw_i == 0 else 1.0)
                    td = (_rot(qt, Z, rng.uniform(-0.8, 0.8)) + Z * 0.35).normalized()
                    tw = _curve(q, td, rng.uniform(0.35, 0.5), 2, bend_up=0.05, rng=rng)
                    k.add(root, m.bark, B.sweep(tw, [0.016, 0.011, 0.007], 4, uv_v_scale=2.0))
                    tips.append(tw[-1])
            else:
                tips.append(sec[-1])
            for tp in tips:
                clumps.append((tp + Z * 0.12, rng.uniform(0.55, 0.75)))
            clumps.append((_at(sec, 0.45)[0] + Z * 0.1, 0.45))
    for (c, r) in clumps:
        cards = 17 if hi else 4
        for i in range(cards):
            # points on an upper-weighted puff around the branch tip
            u = rng.uniform(-0.45, 1.0)
            az = rng.uniform(0, TAU)
            rr = r * rng.uniform(0.35, 1.0) * math.sqrt(max(0.0, 1 - u * u * 0.6))
            pnt = c + Vector((math.cos(az) * rr, math.sin(az) * rr, u * r * 0.65))
            outward = (pnt - c)
            outward = outward.normalized() if outward.length > 1e-3 else Z
            facing = (outward + Vector((rng.uniform(-0.4, 0.4), rng.uniform(-0.4, 0.4), rng.uniform(0.0, 0.5)))).normalized()
            upv = (Vector((rng.uniform(-1, 1), rng.uniform(-1, 1), rng.uniform(0.2, 1.2)))).normalized()
            if hi:
                w = rng.uniform(0.62, 0.86)
                tile = 'canopy_cluster' if i % 3 else 'canopy_sprig'
                bm = B.card(pnt - upv * w * 0.5, facing, upv, w, w, tile, fold=0.15, u_flip=rng.random() < 0.5)
            else:
                w = rng.uniform(1.1, 1.35)
                bm = B.card(pnt - upv * w * 0.5, facing, upv, w, w, 'canopy_cluster', fold=0.12, u_flip=rng.random() < 0.5)
            k.add(root, m.foliage, bm)
    return k, dict(crown_center=(0, 0, 3.7), crown_scale=(2.3, 2.3, 1.2), wind=(TOP, 5.0, 'tree', 0.14, 0.35, 0.8))


# ------------------------------------------------------------------ broadleaf shrub
def build_shrub(m, lod):
    rng = random.Random(7002)
    hi = lod == 0
    k = L.Kit('shrub_laurel')
    root = 'shrub_laurel'
    stem_c = _cell(m, 'stem')
    leaf_c = _cell(m, 'laurel')
    leaves = []
    for s in range(7):
        a = TAU * s / 7 + rng.uniform(-0.25, 0.25)
        out = Vector((math.cos(a), math.sin(a), 0))
        d0 = (out * rng.uniform(0.38, 0.62) + Z).normalized()
        ln = rng.uniform(0.85, 1.05)
        st = _curve(out * 0.05 + Vector((0, 0, -0.04)), d0, ln, 4 if hi else 2, droop=0.10, wobble=0.06, rng=rng)
        k.add(root, m.palette, B.sweep(st, [0.020 - 0.013 * i / (len(st) - 1) for i in range(len(st))],
                                       4 if hi else 3, uv_cell=stem_c))
        shoots = [st]
        if hi:
            for t in (0.45, 0.7):
                p, tan = _at(st, t)
                sd = (_rot(tan, Z, rng.choice((-1, 1)) * rng.uniform(0.8, 1.2)) + out * 0.6).normalized()
                sh = _curve(p, sd, rng.uniform(0.3, 0.42), 2, droop=0.05, rng=rng)
                k.add(root, m.palette, B.sweep(sh, [0.009, 0.007, 0.005], 3, uv_cell=stem_c))
                shoots.append(sh)
        for sh in shoots:
            cnt = (16 if sh is st else 8) if hi else 0
            for i in range(cnt):
                t = 0.35 + 0.65 * (i / max(cnt - 1, 1)) ** 0.8
                p, tan = _at(sh, t)
                leaves.append((p, tan, out))
    for (p, tan, out) in leaves:
        side = tan.cross(Z)
        side = side.normalized() if side.length > 1e-4 else out
        d = (tan * 0.5 + side * rng.choice((-1, 1)) * rng.uniform(0.5, 1.0) + out * 0.35 + Z * rng.uniform(-0.1, 0.35)).normalized()
        k.add(root, m.palette, B.leaf(p, d, Z, rng.uniform(0.15, 0.20), rng.uniform(0.045, 0.06), leaf_c,
                                      fold=0.28, droop=rng.uniform(0.05, 0.25), shape='broad'))
    if not hi:   # cards on the dome the LOD0 leaves fill
        for i in range(30):
            u = rng.uniform(-0.2, 1.0)
            az = rng.uniform(0, TAU)
            rr = math.sqrt(max(0.0, 1 - u * u))
            pnt = Vector((math.cos(az) * rr * 0.55, math.sin(az) * rr * 0.55, 0.45 + u * 0.4))
            facing = (Vector((pnt.x, pnt.y, pnt.z - 0.4)).normalized() + Vector((0, 0, 0.3))).normalized()
            upv = Vector((rng.uniform(-1, 1), rng.uniform(-1, 1), 1.0)).normalized()
            k.add(root, m.foliage, B.card(pnt - upv * 0.24, facing, upv, 0.48, 0.48, 'laurel_sprig',
                                          u_flip=rng.random() < 0.5))
    return k, dict(crown_center=(0, 0, 0.45), crown_scale=(0.65, 0.65, 0.5), wind=(0.3, 1.0, 'shrub', 0.05, 0.6, 0.6))


# ------------------------------------------------------------------ fern
def build_fern(m, lod):
    rng = random.Random(7003)
    hi = lod == 0
    k = L.Kit('fern_sword')
    root = 'fern_sword'
    rach_c, pin_c = _cell(m, 'stem'), _cell(m, 'fern')
    fronds = 8 if hi else 7
    pairs = 18 if hi else 8
    for f in range(fronds):
        a = TAU * f / fronds + rng.uniform(-0.2, 0.2)
        out = Vector((math.cos(a), math.sin(a), 0))
        ln = rng.uniform(0.70, 0.92)
        rise = rng.uniform(0.45, 0.62)
        p0 = out * 0.04 + Vector((0, 0, 0.0))
        pts = []
        segs = 8 if hi else 4
        for i in range(segs + 1):
            t = i / segs
            # quadratic arch: up first, then out and over
            pts.append(p0 + out * (ln * t ** 0.9) + Z * (rise * math.sin(math.pi * t * 0.78) * (1 - 0.25 * t)))
        k.add(root, m.palette, B.sweep(pts, [0.006 - 0.004 * i / segs for i in range(segs + 1)], 3, uv_cell=rach_c))
        side = out.cross(Z).normalized()
        for j in range(pairs):
            t = 0.08 + 0.9 * j / (pairs - 1)
            p, tan = _at(pts, t)
            plen = (0.15 if hi else 0.19) * math.sin(math.pi * (0.12 + 0.86 * t)) ** 0.7 * (1 - 0.35 * t)
            nrm = side.cross(tan).normalized()
            for sgn in (-1, 1):
                d = (side * sgn + tan * 0.45 - nrm * 0.15).normalized()
                k.add(root, m.palette, leaflet(p, d, nrm, plen, plen * (0.19 if hi else 0.24), pin_c))
    if hi:   # two fiddleheads unrolling in the crown
        for f in range(2):
            a = TAU * f / 2 + 0.7
            c = Vector((math.cos(a) * 0.05, math.sin(a) * 0.05, 0.0))
            pts = [c + Vector((0, 0, 0.18 * i / 4)) for i in range(4)]
            o = Vector((math.cos(a), math.sin(a), 0))
            for i in range(9):
                th = i / 8 * TAU * 0.85
                pts.append(c + Vector((0, 0, 0.20)) + o * (0.035 * math.sin(th)) + Z * (0.035 * (1 - math.cos(th))) * 0.8)
            k.add(root, m.palette, B.sweep(pts, [0.007] * len(pts), 3, uv_cell=_cell(m, 'fiddle')))
    return k, dict(crown_center=(0, 0, 0.25), crown_scale=(0.8, 0.8, 0.4), wind=(0.2, 0.6, 'groundcover', 0.04, 0.9, 0.5))


# ------------------------------------------------------------------ flowering accent
def build_coneflower(m, lod):
    rng = random.Random(7004)
    hi = lod == 0
    k = L.Kit('flower_coneflower')
    root = 'flower_coneflower'
    for i in range(14 if hi else 8):
        a = TAU * i / (14 if hi else 8) + rng.uniform(-0.2, 0.2)
        d = Vector((math.cos(a), math.sin(a), rng.uniform(0.4, 0.8))).normalized()
        if hi:
            k.add(root, m.palette, B.leaf((0, 0, 0.01), d, Z, rng.uniform(0.18, 0.25), 0.032, _cell(m, 'leaf_basal'),
                                          fold=0.25, droop=0.35))
        else:
            k.add(root, m.palette, leaflet((0, 0, 0.01), d, Z, 0.22, 0.04, _cell(m, 'leaf_basal')))
    for s in range(8):
        a = TAU * s / 8 + rng.uniform(-0.3, 0.3)
        out = Vector((math.cos(a), math.sin(a), 0))
        h = rng.uniform(0.5, 0.8)
        st = _curve(out * 0.03, (out * 0.18 + Z).normalized(), h, 5 if hi else 2, droop=0.02, wobble=0.03, rng=rng)
        k.add(root, m.palette, B.sweep(st, [0.006] * len(st), 4 if hi else 3, uv_cell=_cell(m, 'stem')))
        top, tan = st[-1], (st[-1] - st[-2]).normalized()
        if hi:
            p, t2 = _at(st, 0.4)
            k.add(root, m.palette, B.leaf(p, (out + Z * 0.4).normalized(), Z, 0.09, 0.016, _cell(m, 'leaf_basal'), droop=0.2))
        cone_prof = ([(0.0, -0.006), (0.017, 0.0), (0.016, 0.013), (0.009, 0.024), (0.0, 0.027)] if hi
                     else [(0.0, -0.004), (0.020, 0.004), (0.0, 0.030)])
        cone = L.lathe(cone_prof, 8 if hi else 5)
        L.transform(cone, loc=tuple(top))
        L.flat_uv(cone, *m.pal('cone', 0.5))
        k.add(root, m.palette, cone)
        npet = 11 if hi else 6
        for pp in range(npet):
            pa = TAU * pp / npet + rng.uniform(-0.1, 0.1)
            pd = Vector((math.cos(pa), math.sin(pa), -rng.uniform(0.1, 0.4))).normalized()
            k.add(root, m.palette, leaflet(top + Vector((0, 0, 0.004)) + Vector((math.cos(pa), math.sin(pa), 0)) * 0.014,
                                           pd, Z, rng.uniform(0.07, 0.085), 0.016, _cell(m, 'petal')))
    return k, dict(crown_center=(0, 0, 0.4), crown_scale=(0.4, 0.4, 0.4), wind=(0.25, 0.82, 'groundcover', 0.05, 1.0, 0.4))


# ------------------------------------------------------------------ ornamental grass
def build_grass(m, lod):
    rng = random.Random(7005)
    hi = lod == 0
    k = L.Kit('grass_fountain')
    root = 'grass_fountain'
    blades = 52 if hi else 20
    for b in range(blades):
        a = rng.uniform(0, TAU)
        r = rng.uniform(0, 0.10)
        out = Vector((math.cos(a), math.sin(a), 0))
        lean = rng.uniform(0.15, 0.55)
        ln = rng.uniform(0.55, 0.80)
        segs = 4 if hi else 2
        base = out * r
        pts = []
        for i in range(segs + 1):
            t = i / segs
            pts.append(base + out * (ln * (lean * t + 0.55 * t * t)) + Z * (ln * (1.0 * t - 0.45 * t * t)))
        widths = [0.013 * (1 - 0.35 * i / segs) for i in range(segs)] + [0.0]
        k.add(root, m.palette, B.ribbon(pts, widths, out.cross(Z), _cell(m, 'grass'), fold=0.35))
    if hi:
        for s in range(5):
            a = rng.uniform(0, TAU)
            out = Vector((math.cos(a), math.sin(a), 0))
            st = _curve(out * 0.03, (out * 0.35 + Z).normalized(), rng.uniform(0.65, 0.85), 3, droop=0.08, rng=rng)
            k.add(root, m.palette, B.sweep(st, [0.003] * len(st), 3, uv_cell=_cell(m, 'stem')))
            p, tan = st[-1], (st[-1] - st[-2]).normalized()
            plume = L.ellipsoid((0, 0, 0), (0.014, 0.014, 0.065), 4, 3)
            L.transform(plume, loc=tuple(p + tan * 0.05), rot=tan.to_track_quat('Z', 'Y').to_euler())
            L.flat_uv(plume, *m.pal('seed', 0.7))
            k.add(root, m.palette, plume)
    return k, dict(crown_center=(0, 0, 0.35), crown_scale=(0.6, 0.6, 0.4), wind=(0.15, 0.75, 'grass', 0.10, 0.8, 0.3))


# ------------------------------------------------------------------ water-edge iris
def build_iris(m, lod):
    rng = random.Random(7006)
    hi = lod == 0
    k = L.Kit('iris_water')
    root = 'iris_water'
    fans = 4 if hi else 3
    for f in range(fans):
        a = TAU * f / fans + rng.uniform(-0.3, 0.3)
        c = Vector((math.cos(a), math.sin(a), 0)) * rng.uniform(0.05, 0.12)
        fan_dir = Vector((math.cos(a + 1.3), math.sin(a + 1.3), 0))
        for l in range(4 if hi else 3):
            spread = (l - 1.5) * 0.16
            h = rng.uniform(0.55, 0.85) * (1 - abs(spread) * 0.6)
            segs = 4 if hi else 2
            pts = [c + fan_dir * (math.sin(spread) * h * (i / segs) ** 1.4 + spread * 0.02) + Z * (h * i / segs)
                   + fan_dir.cross(Z) * (0.04 * (i / segs) ** 2) for i in range(segs + 1)]
            widths = [0.020 * (1 - 0.25 * i / segs) for i in range(segs)] + [0.0]
            k.add(root, m.palette, B.ribbon(pts, widths, fan_dir, _cell(m, 'iris_leaf'), fold=0.15))
    for s in range(3):
        a = TAU * s / 3 + 0.4
        out = Vector((math.cos(a), math.sin(a), 0))
        st = _curve(out * 0.06, (out * 0.08 + Z).normalized(), rng.uniform(0.82, 0.98), 4 if hi else 2, wobble=0.02, rng=rng)
        k.add(root, m.palette, B.sweep(st, [0.006] * len(st), 4 if hi else 3, uv_cell=_cell(m, 'stem')))
        top = st[-1]
        for pp in range(3):
            pa = TAU * pp / 3 + a
            od = Vector((math.cos(pa), math.sin(pa), 0))
            if hi:
                k.add(root, m.palette, B.leaf(top, (od + Z * 0.25).normalized(), Z, 0.075, 0.030,
                                              _cell(m, 'iris_fall'), fold=0.2, droop=0.9, shape='broad'))
                k.add(root, m.palette, B.leaf(top + Z * 0.01, (od * 0.35 + Z).normalized(), od.cross(Z), 0.06, 0.022,
                                              _cell(m, 'iris_fall'), fold=0.3, droop=-0.2, shape='broad'))
                k.add(root, m.palette, leaflet(top + Z * 0.004, (od + Z * 0.3).normalized(), Z, 0.028, 0.006,
                                               _cell(m, 'iris_beard')))
            else:
                k.add(root, m.palette, leaflet(top, (od + Z * 0.1).normalized(), Z, 0.075, 0.028, _cell(m, 'iris_fall')))
    return k, dict(crown_center=(0, 0, 0.45), crown_scale=(0.3, 0.3, 0.5), wind=(0.2, 0.95, 'grass', 0.05, 0.8, 0.5))


# ------------------------------------------------------------------ hanging vine
def build_vine(m, lod):
    rng = random.Random(7007)
    hi = lod == 0
    k = L.Kit('vine_pendant')
    root = 'vine_pendant'
    segs = 16 if hi else 6
    pts = []
    for i in range(segs + 1):
        t = i / segs
        pts.append(Vector((0.10 * math.sin(t * 7.0) * t, 0.07 * math.sin(t * 4.0 + 1) * t, -1.4 * t)))
    k.add(root, m.palette, B.sweep(pts, [0.010 - 0.006 * i / segs for i in range(segs + 1)], 4 if hi else 3,
                                   uv_cell=_cell(m, 'stem')))
    n = 34 if hi else 9
    for i in range(n):
        t = 0.04 + 0.94 * i / (n - 1)
        p, tan = _at(pts, t)
        a = i * 2.4
        out = Vector((math.cos(a), math.sin(a), 0))
        if hi:
            d = (out + Z * rng.uniform(-0.6, 0.1)).normalized()
            k.add(root, m.palette, B.leaf(p, d, Z, rng.uniform(0.07, 0.10) * (1 - 0.3 * t), rng.uniform(0.035, 0.05),
                                          _cell(m, 'vine'), fold=0.25, droop=0.45, shape='heart'))
        else:
            k.add(root, m.foliage, B.card(p + Z * 0.15, out, -Z, 0.34, 0.36, 'vine_sprig', u_flip=i % 2 == 0))
    return k, dict(crown_center=(0, 0, -0.7), crown_scale=(0.2, 0.2, 0.7), wind=(-0.5, -1.4, 'hanging', 0.12, 0.5, 0.3))


SPECIES = {
    'canopy-olive': dict(build=build_canopy, form='canopy tree', alpha='LOD0 + LOD1 alpha-tested sprig/cluster cards '
                         '(MASK 0.5, double-sided, colour-dilated atlas); trunk and branches are geometry',
                         budget=dict(source='ruling: canopy tree', lod0=6000, lod1=2000, lod2=500),
                         collision=dict(type='cylinder', radius=0.30, height=2.4, blocking=True,
                                        note='trunk only; crown base is above head height (see crown_base_m)'),
                         shadow=dict(lod0=True, lod1=True, note='alpha-tested depth (customDepthMaterial with alphaTest)')),
    'shrub-laurel': dict(build=build_shrub, form='broadleaf shrub', alpha='LOD0 geometry leaves (opaque palette); '
                         'LOD1 alpha-tested sprig cards',
                         budget=dict(source='ruling: shrub / groundcover patch', lod0=1500, lod1=500, lod2=120),
                         collision=dict(type='cylinder', radius=0.55, height=1.0, blocking=True,
                                        note='shrub mass; planted inside beds'),
                         shadow=dict(lod0=True, lod1=False, note='LOD1 shadow off')),
    'fern-sword': dict(build=build_fern, form='fern groundcover', alpha='geometry only (no alpha; avoids the rejected '
                       'solid-disk fern card)',
                       budget=dict(source='ruling: shrub/groundcover patch, fern uses smaller', lod0=1200, lod1=450),
                       collision=dict(type='cylinder', radius=0.45, height=0.6, blocking=False, note='soft; walk-through'),
                       shadow=dict(lod0=False, lod1=False, note='receives only')),
    'flower-coneflower': dict(build=build_coneflower, form='flowering accent', alpha='geometry only',
                              budget=dict(source='ruling: flower clusters use smaller budgets', lod0=1000, lod1=400),
                              collision=dict(type='cylinder', radius=0.30, height=0.8, blocking=False, note='soft'),
                              shadow=dict(lod0=False, lod1=False, note='small flowers never cast')),
    'grass-fountain': dict(build=build_grass, form='ornamental grass tussock', alpha='geometry blades only',
                           budget=dict(source='ruling: groundcover patch', lod0=1000, lod1=300),
                           collision=dict(type='cylinder', radius=0.40, height=0.75, blocking=False, note='soft'),
                           shadow=dict(lod0=False, lod1=False, note='receives only')),
    'iris-water': dict(build=build_iris, form='water-edge sword-leaf clump', alpha='geometry only',
                       budget=dict(source='ruling: groundcover patch', lod0=800, lod1=300),
                       collision=dict(type='cylinder', radius=0.25, height=0.95, blocking=False,
                                      note='soft; may stand 0.05 m into shallow water at the coping'),
                       shadow=dict(lod0=False, lod1=False, note='receives only')),
    'vine-pendant': dict(build=build_vine, form='hanging climbing vine', alpha='LOD0 geometry leaves; LOD1 sprig cards',
                         budget=dict(source='lane: hanging strand', lod0=600, lod1=200),
                         collision=dict(type='none', blocking=False,
                                        note='hangs from an arbor/rail; origin is the attachment point; bottom at '
                                             '-1.4 m, so mount at >= 3.7 m above a route for 2.3 m headroom'),
                         shadow=dict(lod0=False, lod1=False, note='receives only')),
}
