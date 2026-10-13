"""Lane W4 botany helpers: textures (bark, foliage atlas, gradient palette),
UV'd branch sweeps, geometry leaves, alpha cards and the species builders.

Conventions follow detail_lib (W3): Blender metres, +Z up, origin = soil
contact under the stem/trunk centre; the exporter writes glTF +Y up.
Every species is ONE mesh object (one primitive per material, <= 2 materials)
parented to a root empty, plus wind-anchor empties with identical names on
every species and LOD, so the runtime can build one InstancedMesh per
(species, LOD, material, spatial cell).
All randomness is seeded; rebuilding gives byte-identical GLBs.
No AI imagery: every map here is drawn with numpy.
"""
import math
import random
import bpy
import bmesh
import numpy as np
from pathlib import Path
from mathutils import Vector, Matrix, Quaternion
import detail_lib as L

TAU = math.tau

# ------------------------------------------------------------------ images

def make_rgba(name, rgba, colorspace='sRGB'):
    """rgba: HxWx4 float (row 0 = bottom). Unlike detail_lib.make_image this
    keeps the alpha channel (foliage cards need true alpha)."""
    h, w = rgba.shape[:2]
    img = bpy.data.images.get(name)
    if img:
        bpy.data.images.remove(img)
    img = bpy.data.images.new(name, w, h, alpha=True, float_buffer=False)
    img.colorspace_settings.name = colorspace
    img.pixels.foreach_set(np.clip(rgba, 0, 1).astype(np.float32).ravel())
    img.update()
    img.file_format = 'PNG'
    path = Path(L.TEXTURE_DIR) / f'{name}.png'
    img.filepath_raw = str(path)
    img.save()
    img.source = 'FILE'
    img.filepath = str(path)
    img.reload()
    img.colorspace_settings.name = colorspace
    img.alpha_mode = 'STRAIGHT'
    return img


def _box_down(a, f):
    h, w = a.shape[:2]
    return a.reshape(h // f, f, w // f, f, -1).mean(axis=(1, 3))


def _blur(a):
    return (a + np.roll(a, 1, 0) + np.roll(a, -1, 0) + np.roll(a, 1, 1) + np.roll(a, -1, 1)) / 5.0


def dilate_color(rgba, iters=24):
    """Push opaque colour outward into transparent texels so bilinear/mip
    filtering at the alpha edge never pulls in black (no leaf halos)."""
    rgb, a = rgba[..., :3].copy(), rgba[..., 3]
    known = (a > 0.02).astype(np.float32)
    acc = rgb * known[..., None]
    for _ in range(iters):
        acc_b, k_b = _blur(acc), _blur(known)
        fill = (known < 0.5) & (k_b > 1e-4)
        acc[fill] = acc_b[fill] / k_b[fill][..., None] * 1.0
        known = np.where(fill, 1.0, known)
        acc = np.where(known[..., None] > 0, acc, 0)
    rest = known < 0.5
    if rest.any():
        mean = rgb[a > 0.5].mean(axis=0) if (a > 0.5).any() else np.array([0.2, 0.3, 0.15])
        acc[rest] = mean
    out = rgba.copy()
    soft = a < 0.98
    out[..., :3] = np.where(soft[..., None], acc, rgb)
    return out


# --------------------------------------------------------------- foliage atlas
ATLAS_TILES = {'canopy_sprig': 0, 'laurel_sprig': 1, 'vine_sprig': 2, 'canopy_cluster': 3}


def _leaf_mask(X, Y, base, ang, length, width, shape='lance'):
    ca, sa = math.cos(ang), math.sin(ang)
    s = ((X - base[0]) * ca + (Y - base[1]) * sa) / length
    q = (-(X - base[0]) * sa + (Y - base[1]) * ca) / width
    sc = np.clip(s, 0, 1)
    if shape == 'heart':
        prof = np.sin(np.pi * np.clip(sc * 0.92 + 0.08, 0, 1)) ** 0.6 * (1 - 0.55 * sc)
    elif shape == 'broad':
        prof = np.sin(np.pi * sc) ** 0.7 * (1 - 0.15 * sc)
    else:
        prof = np.sin(np.pi * sc) ** 0.85
    inside = (s >= 0) & (s <= 1) & (np.abs(q) <= prof)
    return inside, s, q


def _draw_sprig(size, rng, n, leaf_len, leaf_w, base_col, tip_col, shape, bend=0.06, twig_col=(0.30, 0.26, 0.18),
                radial=False):
    S = size
    yy, xx = np.mgrid[0:S, 0:S].astype(np.float32) / S
    rgba = np.zeros((S, S, 4), np.float32)

    def paint(mask, col):
        rgba[mask, :3] = col[mask] if col.ndim == 3 else col
        rgba[mask, 3] = 1.0

    stems = []
    if radial:
        for k in range(7):
            a = TAU * k / 7 + rng.uniform(-0.2, 0.2)
            stems.append(((0.5, 0.5), (0.5 + 0.42 * math.cos(a), 0.5 + 0.42 * math.sin(a))))
    else:
        stems.append(((0.5, 0.03), (0.5 + bend, 0.95)))
    for (p0, p1) in stems:
        # twig: distance to segment
        d = np.array(p1) - np.array(p0)
        t = np.clip(((xx - p0[0]) * d[0] + (yy - p0[1]) * d[1]) / (d @ d), 0, 1)
        dist = np.hypot(xx - (p0[0] + t * d[0]), yy - (p0[1] + t * d[1]))
        paint(dist < 0.007 + 0.004 * (1 - t), np.array(twig_col, np.float32))
        per = n // len(stems) if radial else n
        for i in range(per):
            u = 0.12 + 0.86 * (i + rng.uniform(0, 0.6)) / per
            bx, by = p0[0] + d[0] * u, p0[1] + d[1] * u
            side = 1 if i % 2 == 0 else -1
            axis = math.atan2(d[1], d[0])
            ang = axis + side * rng.uniform(0.35, 1.25) * (1 - 0.35 * u) + rng.uniform(-0.15, 0.15)
            ln = leaf_len * (1.0 - 0.45 * u ** 2) * rng.uniform(0.85, 1.12)
            wd = leaf_w * rng.uniform(0.85, 1.1)
            x0 = int(max(0, (min(bx, bx + math.cos(ang) * ln) - wd - 0.02) * S))
            x1 = int(min(S, (max(bx, bx + math.cos(ang) * ln) + wd + 0.02) * S))
            y0 = int(max(0, (min(by, by + math.sin(ang) * ln) - wd - 0.02) * S))
            y1 = int(min(S, (max(by, by + math.sin(ang) * ln) + wd + 0.02) * S))
            if x1 <= x0 or y1 <= y0:
                continue
            X, Y = xx[y0:y1, x0:x1], yy[y0:y1, x0:x1]
            inside, s, q = _leaf_mask(X, Y, (bx, by), ang, ln, wd, shape)
            vary = rng.uniform(-0.06, 0.06, 3) + rng.uniform(-0.05, 0.05)
            col = (np.array(base_col)[None, None] * (1 - s[..., None]) + np.array(tip_col)[None, None] * s[..., None]
                   + vary[None, None])
            col = col * (1 - 0.30 * (np.abs(q) < 0.07)[..., None])          # midrib
            veins = (np.abs(np.sin((s - np.abs(q) * 0.6) * 22)) < 0.12) & (np.abs(q) > 0.1)
            col = col * (1 - 0.07 * veins[..., None])
            col = col * (1 + 0.10 * (np.abs(q) > 0.82)[..., None])         # pale margin
            sub = rgba[y0:y1, x0:x1]
            sub[inside, :3] = np.clip(col[inside], 0, 1)
            sub[inside, 3] = 1.0
    return rgba


def foliage_atlas(seed=401, tile=512):
    """2x2 atlas, one 512 tile per card type. Rendered at 2x and box-filtered
    (antialiased alpha), then colour-dilated so the alpha edge never fringes."""
    rng = np.random.default_rng(seed)
    big = tile * 2
    tiles = [
        _draw_sprig(big, rng, 38, 0.19, 0.034, (0.29, 0.36, 0.20), (0.45, 0.52, 0.33), 'lance', bend=0.12),
        _draw_sprig(big, rng, 12, 0.34, 0.085, (0.12, 0.22, 0.09), (0.23, 0.35, 0.14), 'broad', bend=-0.05),
        _draw_sprig(big, rng, 10, 0.26, 0.11, (0.15, 0.27, 0.10), (0.27, 0.41, 0.16), 'heart', bend=0.10),
        _draw_sprig(big, rng, 42, 0.17, 0.030, (0.27, 0.33, 0.19), (0.43, 0.50, 0.31), 'lance', radial=True),
    ]
    tiles = [dilate_color(_box_down(t, 2)) for t in tiles]
    atlas = np.zeros((tile * 2, tile * 2, 4), np.float32)
    for i, t in enumerate(tiles):
        gx, gy = i % 2, i // 2
        atlas[gy * tile:(gy + 1) * tile, gx * tile:(gx + 1) * tile] = t
    return atlas


def tile_uv(name, u, v):
    """Map a 0..1 tile-local UV into the 2x2 atlas with a 4-texel inset."""
    i = ATLAS_TILES[name]
    gx, gy = i % 2, i // 2
    e = 4 / 1024
    return (gx * 0.5 + e + u * (0.5 - 2 * e), gy * 0.5 + e + v * (0.5 - 2 * e))


# --------------------------------------------------------------- bark + palette

def tex_bark(size=512, seed=411):
    fis = L.fbm(seed, size, base=24, octaves=3)
    # stretch along V (bark fissures run up the trunk; V = length)
    st = sum(np.roll(fis, k, 0) for k in range(-12, 13)) / 25.0
    st = (st - st.min()) / (np.ptp(st) + 1e-6)
    blot = L.fbm(seed + 1, size, base=4, octaves=3)
    ridges = np.clip((st - 0.42) * 3.0, 0, 1)
    col = L.lerp_color(np.clip(0.65 * ridges + 0.35 * blot, 0, 1), (0.17, 0.15, 0.12), (0.42, 0.39, 0.33))
    lich = (L.fbm(seed + 2, size, base=12, octaves=3) > 0.64).astype(np.float32)
    col = col * (1 - 0.25 * lich[..., None]) + np.array([0.36, 0.40, 0.30]) * 0.25 * lich[..., None]
    return col, L.height_to_normal(ridges * 0.8 + blot * 0.2, 2.2)


BOTANY_CELLS = {   # name: (base sRGB, tip sRGB)  gradient runs along V inside the cell
    'stem': ((0.25, 0.27, 0.15), (0.33, 0.36, 0.20)),
    'laurel': ((0.14, 0.25, 0.10), (0.27, 0.40, 0.16)),
    'fern': ((0.13, 0.25, 0.08), (0.30, 0.45, 0.15)),
    'grass': ((0.26, 0.33, 0.14), (0.66, 0.60, 0.38)),
    'iris_leaf': ((0.18, 0.30, 0.18), (0.36, 0.48, 0.30)),
    'iris_fall': ((0.24, 0.20, 0.52), (0.50, 0.46, 0.82)),
    'iris_beard': ((0.85, 0.70, 0.25), (0.92, 0.80, 0.40)),
    'petal': ((0.86, 0.50, 0.32), (0.96, 0.76, 0.55)),
    'cone': ((0.20, 0.11, 0.04), (0.33, 0.19, 0.07)),
    'vine': ((0.14, 0.27, 0.10), (0.28, 0.42, 0.17)),
    'fiddle': ((0.30, 0.36, 0.14), (0.42, 0.46, 0.22)),
    'seed': ((0.55, 0.48, 0.30), (0.78, 0.70, 0.48)),
    'root': ((0.20, 0.16, 0.11), (0.28, 0.22, 0.15)),
    'bud': ((0.30, 0.30, 0.45), (0.45, 0.40, 0.70)),
    'leaf_basal': ((0.12, 0.22, 0.09), (0.25, 0.37, 0.15)),
    'moss': ((0.20, 0.27, 0.10), (0.30, 0.36, 0.15)),
}
PAL_GRID, PAL_PX = 4, 32


def botany_palette():
    size = PAL_GRID * PAL_PX
    rgb = np.zeros((size, size, 3), np.float32)
    uv = {}
    for i, (k, (a, b)) in enumerate(BOTANY_CELLS.items()):
        gx, gy = i % PAL_GRID, i // PAL_GRID
        t = np.linspace(0, 1, PAL_PX, dtype=np.float32)
        col = np.array(a)[None] * (1 - t[:, None]) + np.array(b)[None] * t[:, None]
        rgb[gy * PAL_PX:(gy + 1) * PAL_PX, gx * PAL_PX:(gx + 1) * PAL_PX] = col[:, None, :]
        e = 3 / size
        uv[k] = (((gx + 0.5) / PAL_GRID), (gy / PAL_GRID + e, (gy + 1) / PAL_GRID - e))
    return rgb, uv


_ARRAYS = {}


class Mats:
    """Builds the three botany materials. Pixel arrays are computed once per
    Blender session (deterministic) and re-bound after each scene reset."""

    def __init__(self):
        if not _ARRAYS:
            _ARRAYS['bark'] = tex_bark(512)
            _ARRAYS['palette'] = botany_palette()
            _ARRAYS['atlas'] = foliage_atlas()
        bc, bn = _ARRAYS['bark']
        self.bark = L.pbr('Bark • grey olive', (1, 1, 1), 0.88, base_img=L.make_image('bark_basecolor', bc),
                          normal_img=L.make_image('bark_normal', bn[::2, ::2], 'Non-Color'), normal_strength=0.9)
        prgb, self.pal_uv = _ARRAYS['palette']
        self.palette = L.pbr('Botany • leaf and stem palette', (1, 1, 1), 0.62,
                             base_img=L.make_image('botany_palette_basecolor', prgb), double_sided=True)
        atlas = make_rgba('foliage_atlas_basecolor_alpha', _ARRAYS['atlas'])
        self.foliage = foliage_material(atlas)

    def pal(self, cell, t=0.5):
        u, (v0, v1) = self.pal_uv[cell]
        return u, v0 + (v1 - v0) * t


def foliage_material(img):
    """Alpha-tested (glTF alphaMode MASK, cutoff 0.5), double-sided. The
    exporter recognises Image Alpha -> Math Round -> BSDF Alpha as MASK 0.5."""
    m = bpy.data.materials.new('Foliage • alpha-tested atlas')
    m.use_nodes = True
    nt = m.node_tree
    b = nt.nodes['Principled BSDF']
    b.inputs['Roughness'].default_value = 0.70
    t = nt.nodes.new('ShaderNodeTexImage'); t.image = img; t.location = (-600, 200)
    nt.links.new(t.outputs['Color'], b.inputs['Base Color'])
    r = nt.nodes.new('ShaderNodeMath'); r.operation = 'ROUND'; r.location = (-300, 0)
    nt.links.new(t.outputs['Alpha'], r.inputs[0])
    nt.links.new(r.outputs[0], b.inputs['Alpha'])
    m.surface_render_method = 'DITHERED'
    m.use_backface_culling = False
    return m


# --------------------------------------------------------------- geometry

def _frames(pts):
    """Parallel-transport frames along a polyline (no twisting flips)."""
    n = len(pts)
    tans = []
    for i in range(n):
        a, b = pts[max(i - 1, 0)], pts[min(i + 1, n - 1)]
        tans.append((b - a).normalized())
    ref = Vector((1, 0, 0)) if abs(tans[0].x) < 0.9 else Vector((0, 1, 0))
    nrm = tans[0].cross(ref).normalized()
    out = []
    for i in range(n):
        if i > 0:
            q = tans[i - 1].rotation_difference(tans[i])
            nrm = (q @ nrm).normalized()
        out.append((tans[i], nrm, tans[i].cross(nrm).normalized()))
    return out


def sweep(points, radii, sides, uv_v_scale=1.0, uv_u_scale=1.0, cap_tip=True, cap_base=False, uv_cell=None):
    """Tapered tube along a polyline with real UVs (u around, v along length in
    metres * uv_v_scale). If uv_cell=(u, v0, v1) every loop maps into a
    palette gradient cell instead (base -> tip)."""
    pts = [Vector(p) for p in points]
    fr = _frames(pts)
    bm = bmesh.new()
    uvl = bm.loops.layers.uv.verify()
    rings, vcoord = [], [0.0]
    for i in range(1, len(pts)):
        vcoord.append(vcoord[-1] + (pts[i] - pts[i - 1]).length)
    total = vcoord[-1] or 1.0
    for p, (t, n, b), r in zip(pts, fr, radii):
        rings.append([bm.verts.new(p + r * (math.cos(TAU * j / sides) * n + math.sin(TAU * j / sides) * b))
                      for j in range(sides)])

    def uvof(i, j):
        if uv_cell:
            u, v0, v1 = uv_cell
            return (u, v0 + (v1 - v0) * vcoord[i] / total)
        return (j / sides * uv_u_scale, vcoord[i] * uv_v_scale)

    for i in range(len(rings) - 1):
        for j in range(sides):
            j1 = (j + 1) % sides
            f = bm.faces.new((rings[i][j], rings[i][j1], rings[i + 1][j1], rings[i + 1][j]))
            jj = j + 1  # seam: u reaches 1.0 rather than wrapping to 0
            for loop, (ii, uu) in zip(f.loops, ((i, j), (i, jj), (i + 1, jj), (i + 1, j))):
                loop[uvl].uv = uvof(ii, uu)
    if cap_tip:
        tip = bm.verts.new(pts[-1] + fr[-1][0] * radii[-1] * 0.8)
        for j in range(sides):
            f = bm.faces.new((rings[-1][j], rings[-1][(j + 1) % sides], tip))
            for loop in f.loops:
                loop[uvl].uv = uvof(len(pts) - 1, j)
    if cap_base:
        f = bm.faces.new(list(reversed(rings[0])))
        for loop in f.loops:
            loop[uvl].uv = uvof(0, 0)
    bm.normal_update()
    return bm


def leaf(base, direction, up, length, width, uv, fold=0.25, droop=0.15, shape='lance'):
    """Geometry leaf: base, two shoulders, a raised midrib point and the tip
    (4 triangles). Folded along the midrib and drooping toward the tip.
    uv = (u, v0, v1) palette gradient cell: base -> tip."""
    d = Vector(direction).normalized()
    side = d.cross(Vector(up)).normalized()
    if side.length < 1e-6:
        side = Vector((1, 0, 0))
    nrm = side.cross(d).normalized()
    b = Vector(base)
    sh = 0.42 if shape == 'lance' else 0.35 if shape == 'heart' else 0.48
    mid = b + d * length * 0.5 + nrm * width * fold
    pl = b + d * length * sh - side * width + nrm * (-droop * length * 0.3)
    pr = b + d * length * sh + side * width + nrm * (-droop * length * 0.3)
    tip = b + d * length + nrm * (-droop * length)
    bm = bmesh.new()
    uvl = bm.loops.layers.uv.verify()
    vs = [bm.verts.new(p) for p in (b, pl, pr, mid, tip)]
    u, v0, v1 = uv
    tv = {0: 0.0, 1: sh, 2: sh, 3: 0.5, 4: 1.0}
    for tri in ((0, 3, 1), (0, 2, 3), (1, 3, 4), (3, 2, 4)):
        f = bm.faces.new([vs[k] for k in tri])
        for loop, k in zip(f.loops, tri):
            loop[uvl].uv = (u, v0 + (v1 - v0) * tv[k])
    return bm


def ribbon(points, widths, up_hint, uv, fold=0.0):
    """Blade/frond strip with a centre crease (2 quads per segment). A final
    width of 0 closes to a single tip vertex (pointed blade)."""
    pts = [Vector(p) for p in points]
    bm = bmesh.new()
    uvl = bm.loops.layers.uv.verify()
    n = len(pts)
    u, v0, v1 = uv
    rows = []
    for i, p in enumerate(pts):
        t = (pts[min(i + 1, n - 1)] - pts[max(i - 1, 0)]).normalized()
        side = t.cross(Vector(up_hint))
        side = side.normalized() if side.length > 1e-6 else Vector((1, 0, 0))
        nrm = side.cross(t).normalized()
        w = widths[i]
        if w < 1e-6:
            rows.append([bm.verts.new(p)])
        else:
            rows.append([bm.verts.new(p - side * w), bm.verts.new(p + nrm * w * fold), bm.verts.new(p + side * w)])

    def put(face, vv):
        for loop, x in zip(face.loops, vv):
            loop[uvl].uv = (u, x)

    for i in range(n - 1):
        tv0, tv1 = v0 + (v1 - v0) * i / (n - 1), v0 + (v1 - v0) * (i + 1) / (n - 1)
        a, b = rows[i], rows[i + 1]
        if len(b) == 1:
            put(bm.faces.new((a[0], a[1], b[0])), (tv0, tv0, tv1))
            put(bm.faces.new((a[1], a[2], b[0])), (tv0, tv0, tv1))
        else:
            put(bm.faces.new((a[0], a[1], b[1], b[0])), (tv0, tv0, tv1, tv1))
            put(bm.faces.new((a[1], a[2], b[2], b[1])), (tv0, tv0, tv1, tv1))
    return bm


def card(center, facing, up, w, h, tile, fold=0.18, u_flip=False):
    """Alpha card (foliage atlas tile). 3x2 vertex grid folded along its long
    axis (4 triangles), anchored at its bottom-centre `center`."""
    f = Vector(facing).normalized()
    upv = Vector(up)
    side = upv.cross(f).normalized()
    if side.length < 1e-6:
        side = Vector((1, 0, 0))
    upv = f.cross(side).normalized()
    c = Vector(center)
    bm = bmesh.new()
    uvl = bm.loops.layers.uv.verify()
    grid = []
    for r, vy in enumerate((0.0, 1.0)):
        row = []
        for k, ux in enumerate((-0.5, 0.0, 0.5)):
            off = -abs(ux) * 2 * fold * w * 0.5
            row.append(bm.verts.new(c + side * ux * w + upv * vy * h + f * off))
        grid.append(row)
    for k in range(2):
        q = (grid[0][k], grid[0][k + 1], grid[1][k + 1], grid[1][k])
        face = bm.faces.new(q)
        for loop, (uu, vv) in zip(face.loops, ((k / 2, 0), ((k + 1) / 2, 0), ((k + 1) / 2, 1), (k / 2, 1))):
            loop[uvl].uv = tile_uv(tile, 1 - uu if u_flip else uu, vv)
    return bm


def set_foliage_normals(obj, center, scale=(1, 1, 1), blend=0.7):
    """Bend foliage normals toward a crown ellipsoid (soft volumetric shading,
    no flat-card flicker). `center`/`scale` describe the crown volume."""
    me = obj.data
    c = Vector(center) - obj.matrix_world.translation
    norms = []
    for v in me.vertices:
        d = v.co - c
        d = Vector((d.x / scale[0], d.y / scale[1], d.z / scale[2]))
        n = d.normalized() if d.length > 1e-6 else Vector((0, 0, 1))
        n = (n * blend + v.normal * (1 - blend) + Vector((0, 0, 0.15))).normalized()
        norms.append(n)
    me.normals_split_custom_set_from_vertices(norms)
