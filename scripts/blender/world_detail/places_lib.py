"""Lane W4 place-construction helpers: the W3 material family (oiled walnut
with the identical procedural maps so W6 can de-duplicate by hash, warm ivory
ceramic, brass/bronze/graphite trim) plus a warm limestone, a places trim
palette, the jade phosphor display and a named panel-slot glass.

Blender space, metres, +Z up, assets face -Y (the approach / reader side);
the exporter writes glTF +Y up facing +Z. Origin = floor contact point.
"""
import math
import bmesh
import numpy as np
from mathutils import Vector, Matrix
import detail_lib as L

TAU = math.tau

PLACES_CELLS = {
    'brass': ((0.56, 0.36, 0.15), 0.34, 1.0), 'bronze': ((0.28, 0.17, 0.085), 0.45, 1.0),
    'graphite': ((0.03, 0.035, 0.037), 0.7, 0.0), 'steel': ((0.10, 0.11, 0.115), 0.38, 1.0),
    'glass_dark': ((0.012, 0.035, 0.042), 0.12, 0.0), 'paper': ((0.86, 0.83, 0.74), 0.86, 0.0),
    'paper_edge': ((0.74, 0.70, 0.61), 0.9, 0.0), 'leather': ((0.24, 0.12, 0.06), 0.55, 0.0),
    'book_oxblood': ((0.33, 0.09, 0.07), 0.6, 0.0), 'book_navy': ((0.08, 0.12, 0.22), 0.6, 0.0),
    'book_moss': ((0.20, 0.27, 0.15), 0.6, 0.0), 'book_ochre': ((0.62, 0.45, 0.18), 0.6, 0.0),
    'book_slate': ((0.30, 0.33, 0.35), 0.6, 0.0), 'book_umber': ((0.30, 0.20, 0.12), 0.6, 0.0),
    'book_teal': ((0.10, 0.30, 0.30), 0.6, 0.0), 'book_cream': ((0.80, 0.74, 0.60), 0.7, 0.0),
    'linen': ((0.70, 0.66, 0.56), 0.85, 0.0), 'soil': ((0.13, 0.09, 0.06), 0.95, 0.0),
    'terracotta': ((0.62, 0.32, 0.20), 0.8, 0.0), 'wet_stone': ((0.36, 0.33, 0.28), 0.35, 0.0),
    'moss': ((0.22, 0.30, 0.12), 0.9, 0.0), 'rubber': ((0.018, 0.022, 0.024), 0.85, 0.0),
    'oak': ((0.46, 0.30, 0.15), 0.55, 0.0), 'leaf': ((0.16, 0.30, 0.11), 0.6, 0.0),
    'leaf_light': ((0.30, 0.46, 0.18), 0.6, 0.0), 'stake': ((0.80, 0.76, 0.64), 0.6, 0.0),
    'copper': ((0.55, 0.27, 0.16), 0.4, 1.0), 'channel': ((0.02, 0.02, 0.02), 0.95, 0.0),
}
_ARR = {}


def tex_limestone(size=512, seed=61):
    base = L.fbm(seed, size, base=6, octaves=5)
    fine = L.fbm(seed + 1, size, base=64, octaves=2)
    pits = (L.fbm(seed + 2, size, base=128, octaves=1) > 0.72).astype(np.float32)
    beds = np.sin((np.linspace(0, 1, size, endpoint=False)[:, None] * 9 + base * 0.8) * TAU) * 0.5 + 0.5
    t = np.clip(0.55 * base + 0.25 * fine + 0.1 * beds, 0, 1)
    col = L.lerp_color(t, (0.66, 0.60, 0.50), (0.84, 0.79, 0.69))
    col *= (1 - 0.12 * pits)[..., None]
    return col, L.height_to_normal(0.5 * base + 0.3 * fine - 0.4 * pits, 1.2)


class PM:
    """Place materials (rebound after each scene reset; pixels cached)."""

    def __init__(self):
        if not _ARR:
            _ARR['walnut'] = L.tex_walnut(1024)
            _ARR['stone'] = tex_limestone(512)
        wc, wn = _ARR['walnut']
        sc, sn = _ARR['stone']
        # identical names/pixels to W3 arrival_furniture.shared_materials() -> identical bytes
        self.walnut = L.pbr('Timber • oiled walnut', (1, 1, 1), 0.46, base_img=L.make_image('walnut_basecolor', wc),
                            normal_img=L.make_image('walnut_normal', wn[::2, ::2], 'Non-Color'), normal_strength=0.6)
        self.stone = L.pbr('Stone • warm limestone', (1, 1, 1), 0.82, base_img=L.make_image('limestone_basecolor', sc),
                           normal_img=L.make_image('limestone_normal', sn[::2, ::2], 'Non-Color'), normal_strength=0.7)
        self.ceramic = L.pbr('Ceramic • warm ivory', (0.83, 0.79, 0.67), 0.30, 0.0, coat=0.34, coat_rough=0.22)
        self.display = L.pbr('Display • jade phosphor', (0.10, 0.45, 0.36), 0.3, emission=(0.14, 0.62, 0.48), emit=0.8)
        self.slot = L.pbr('Panel • dark glass slot', (0.012, 0.035, 0.042), 0.10, coat=0.6, coat_rough=0.05)
        self.collision = L.pbr('collision', (1, 0, 1), 1)

    def kit(self, root, detail):
        k = L.Kit(root, detail)
        k.palette = L.Palette('Trim • places palette', PLACES_CELLS, cell_px=16, grid=6)
        return k


def wood(k, M, node, center, size, bevel=0.006, grain='x', rot=(0, 0, 0), seg=1):
    bm = L.rbox(center, size, bevel, seg if k.detail > 0.7 else 1, rot)
    L.box_uv(bm, 0.9, grain)
    k.add(node, M.walnut, bm)


def stone(k, M, node, center, size, bevel=0.012, rot=(0, 0, 0)):
    bm = L.rbox(center, size, bevel, 2 if k.detail > 0.7 else 1, rot)
    L.box_uv(bm, 1.0, 'x')
    k.add(node, M.stone, bm)


def stone_bm(k, M, node, bm):
    L.box_uv(bm, 1.0, 'x')
    k.add(node, M.stone, bm)


def ceramic(k, M, node, bm):
    k.add(node, M.ceramic, bm)


def pal(k, node, cell, bm):
    k.add(node, f'pal:{cell}', bm)


def bolt(k, node, loc, normal=(0, 0, 1), r=0.009, h=0.006, cell='bronze'):
    """Hex bolt head (LOD0 only: LOD1 relies on a future normal bake)."""
    if k.detail <= 0.7:
        return
    bm = L.lathe([(0, 0), (r, 0), (r, h * 0.7), (r * 0.7, h), (0, h)], 6)
    q = Vector(normal).to_track_quat('Z', 'Y')
    bmesh.ops.transform(bm, matrix=Matrix.Translation(Vector(loc)) @ q.to_matrix().to_4x4(), verts=bm.verts)
    pal(k, node, cell, bm)


def slot_quad(center, w, h, tilt_x=0.0, thick=0.008):
    """Panel slot: a thin box whose FRONT face (-Y) carries UV 0..1 across the
    slot so the runtime can map a screen/DOM texture onto it."""
    bm = bmesh.new()
    uvl = bm.loops.layers.uv.verify()
    hw, hh = w / 2, h / 2
    front = [bm.verts.new((x, -thick / 2, z)) for (x, z) in ((-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh))]
    back = [bm.verts.new((x, thick / 2, z)) for (x, z) in ((-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh))]
    f = bm.faces.new(front)
    for loop, uv in zip(f.loops, ((0, 0), (1, 0), (1, 1), (0, 1))):
        loop[uvl].uv = uv
    bm.faces.new(list(reversed(back)))
    for i in range(4):
        j = (i + 1) % 4
        bm.faces.new((front[j], front[i], back[i], back[j]))
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    bmesh.ops.transform(bm, matrix=Matrix.Translation(Vector(center)) @ Matrix.Rotation(tilt_x, 4, 'X'), verts=bm.verts)
    for loop in [l for fc in bm.faces for l in fc.loops if fc is not f]:
        loop[uvl].uv = (0.5, 0.5)
    return bm


def remap(bm, m3):
    """Apply a 3x3 axis remap (e.g. build in (u, v, w) then map to XYZ)."""
    bmesh.ops.transform(bm, matrix=Matrix(m3).to_4x4(), verts=bm.verts)
    return bm


def tilt_matrix(center, tilt_x):
    return Matrix.Translation(Vector(center)) @ Matrix.Rotation(tilt_x, 4, 'X')


def xf(bm, m4):
    bmesh.ops.transform(bm, matrix=m4, verts=bm.verts)
    return bm
