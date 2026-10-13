"""Shared, deterministic modelling helpers for the World detail-v4 pass (lane W3).

Conventions (match the existing hero-kit sources):
  * Blender space, metres, +Z up, assets face -Y. The glTF exporter converts to
    +Y up with the asset facing +Z, which is what the runtime expects.
  * Geometry is built as small closed bmesh primitives and accumulated per
    (node, material). `Kit.finish()` merges each bucket into one mesh object
    parented to its pivot empty, so the runtime gets one primitive per
    material per articulated node (draw calls stay bounded).
  * Everything is seeded and order-stable so rebuilding produces the same
    meshes. Textures are authored here procedurally with numpy; no AI images.
"""
import bpy
import bmesh
import math
import hashlib
import numpy as np
from pathlib import Path
from mathutils import Vector, Matrix, Euler

TAU = math.tau
TEXTURE_DIR = None   # set by build_hero.py; None => pack into the .blend


# --------------------------------------------------------------------------- scene

def reset_scene():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    for coll in (bpy.data.meshes, bpy.data.materials, bpy.data.images, bpy.data.objects):
        for item in list(coll):
            coll.remove(item)


def seg(n, detail, minimum=3):
    return max(minimum, int(round(n * detail)))


# --------------------------------------------------------------------------- bmesh primitives
# Every primitive returns a new bmesh in asset space (already transformed).

def _xf(bm, loc=(0, 0, 0), rot=(0, 0, 0), scale=None):
    m = Matrix.Translation(Vector(loc)) @ Euler(rot, 'XYZ').to_matrix().to_4x4()
    if scale is not None:
        m = m @ Matrix.Diagonal((*scale, 1))
    bmesh.ops.transform(bm, matrix=m, verts=bm.verts)
    return bm


def _finish(bm):
    bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=1e-6)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    return bm


def rbox(center, size, bevel=0.0, segments=2, rot=(0, 0, 0)):
    """Rounded box. `bevel` is the edge radius in metres."""
    bm = bmesh.new()
    bmesh.ops.create_cube(bm, size=1.0)
    for v in bm.verts:
        v.co.x *= size[0]; v.co.y *= size[1]; v.co.z *= size[2]
    b = min(bevel, min(size) * 0.49)
    if b > 1e-5 and segments > 0:
        bmesh.ops.bevel(bm, geom=list(bm.edges) + list(bm.verts), offset=b, segments=segments,
                        affect='EDGES', profile=0.5, clamp_overlap=True)
    _xf(bm, center, rot)
    return _finish(bm)


def _unit_sphere(u, v):
    # Built from a lathe profile rather than bmesh.ops.create_uvsphere, whose
    # vertex/face order after remove_doubles is not reproducible run to run.
    prof = [(math.sin(math.pi * i / v), -math.cos(math.pi * i / v)) for i in range(v + 1)]
    prof[0], prof[-1] = (0.0, -1.0), (0.0, 1.0)
    return lathe(prof, u)


def ellipsoid(center, radii, u=24, v=12, rot=(0, 0, 0)):
    bm = _unit_sphere(u, v)
    _xf(bm, center, rot, radii)
    return _finish(bm)


def lathe(profile, segments=24, center=(0, 0, 0), rot=(0, 0, 0), arc=TAU, phase=0.0):
    """Revolve a (radius, z) profile around local Z. Profile points with r == 0
    collapse to a pole, so a profile that starts and ends on the axis is closed."""
    bm = bmesh.new()
    full = abs(arc - TAU) < 1e-6
    cols = segments if full else segments + 1
    rings = []
    for (r, z) in profile:
        if r <= 1e-7:
            rings.append([bm.verts.new((0, 0, z))])
        else:
            ring = []
            for k in range(cols):
                a = phase + arc * k / segments
                ring.append(bm.verts.new((r * math.cos(a), r * math.sin(a), z)))
            rings.append(ring)
    for i in range(len(rings) - 1):
        a, b = rings[i], rings[i + 1]
        n = segments if full else segments
        for k in range(n):
            k1 = (k + 1) % cols if full else k + 1
            if len(a) == 1 and len(b) == 1:
                continue
            if len(a) == 1:
                bm.faces.new((a[0], b[k1], b[k]))
            elif len(b) == 1:
                bm.faces.new((a[k], a[k1], b[0]))
            else:
                bm.faces.new((a[k], a[k1], b[k1], b[k]))
    if not full:  # close the cut ends with fans so the solid stays closed
        for end in (0, cols - 1):
            pts = [r[0] if len(r) == 1 else r[end] for r in rings]
            uniq = []
            for p in pts:
                if not uniq or p is not uniq[-1]:
                    uniq.append(p)
            if len(uniq) >= 3:
                try:
                    bm.faces.new(uniq)
                except ValueError:
                    pass
    _xf(bm, center, rot)
    return _finish(bm)


def tube(a, b, r, segments=12, r2=None, caps=True):
    a, b = Vector(a), Vector(b)
    d = b - a
    r2 = r if r2 is None else r2
    prof = ([(0, 0)] if caps else []) + [(r, 0), (r2, d.length)] + ([(0, d.length)] if caps else [])
    bm = lathe(prof, segments)
    q = d.to_track_quat('Z', 'Y')
    m = Matrix.Translation(a) @ q.to_matrix().to_4x4()
    bmesh.ops.transform(bm, matrix=m, verts=bm.verts)
    return bm


def torus(center, major, minor, useg=32, vseg=8, rot=(0, 0, 0), arc=TAU, phase=0.0, scale_minor=(1, 1)):
    """Torus in local XY plane. Partial arcs are capped."""
    bm = bmesh.new()
    full = abs(arc - TAU) < 1e-6
    cols = useg if full else useg + 1
    grid = []
    for i in range(cols):
        a = phase + arc * i / useg
        ca, sa = math.cos(a), math.sin(a)
        ring = []
        for j in range(vseg):
            t = TAU * j / vseg
            rr = major + minor * scale_minor[0] * math.cos(t)
            ring.append(bm.verts.new((rr * ca, rr * sa, minor * scale_minor[1] * math.sin(t))))
        grid.append(ring)
    for i in range(useg):
        i1 = (i + 1) % cols if full else i + 1
        for j in range(vseg):
            j1 = (j + 1) % vseg
            bm.faces.new((grid[i][j], grid[i1][j], grid[i1][j1], grid[i][j1]))
    if not full:
        bm.faces.new(list(reversed(grid[0])))
        bm.faces.new(grid[-1])
    _xf(bm, center, rot)
    return _finish(bm)


def _se(c, e):
    return math.copysign(abs(c) ** e, c)


def shell_patch(center, radii, t0, t1, p0, p1, rows, cols, thickness, exp=1.0, rot=(0, 0, 0), lip=0.35):
    """A thick panel cut from a (super)ellipsoid between polar angles t0..t1
    (from +Z) and azimuth p0..p1 (from +X towards +Y). The inner skin is offset
    towards the centre by `thickness`, the four borders are closed and the
    outer border is pulled in slightly (`lip`) so each panel reads as a moulded
    ceramic part with a softened edge rather than a paper-thin cut."""
    bm = bmesh.new()
    cx, cy, cz = center
    rx, ry, rz = radii

    def point(theta, phi, inset):
        st, ct = math.sin(theta), math.cos(theta)
        cp, sp = math.cos(phi), math.sin(phi)
        x = rx * _se(st, exp) * _se(cp, exp)
        y = ry * _se(st, exp) * _se(sp, exp)
        z = rz * _se(ct, exp)
        n = Vector((x / rx ** 2, y / ry ** 2, z / rz ** 2)).normalized()
        p = Vector((x, y, z)) - n * inset
        return p

    outer, inner = [], []
    for i in range(rows + 1):
        th = t0 + (t1 - t0) * i / rows
        orow, irow = [], []
        for k in range(cols + 1):
            ph = p0 + (p1 - p0) * k / cols
            border = i in (0, rows) or k in (0, cols)
            orow.append(bm.verts.new(point(th, ph, thickness * lip if border else 0.0)))
            irow.append(bm.verts.new(point(th, ph, thickness)))
        outer.append(orow); inner.append(irow)
    for i in range(rows):
        for k in range(cols):
            bm.faces.new((outer[i][k], outer[i][k + 1], outer[i + 1][k + 1], outer[i + 1][k]))
            bm.faces.new((inner[i][k], inner[i + 1][k], inner[i + 1][k + 1], inner[i][k + 1]))
    border = [(i, 0) for i in range(rows)] + [(rows, k) for k in range(cols)] + \
             [(i, cols) for i in range(rows, 0, -1)] + [(0, k) for k in range(cols, 0, -1)]
    nxt = border[1:] + border[:1]
    for (a, b) in zip(border, nxt):
        try:
            bm.faces.new((outer[a[0]][a[1]], inner[a[0]][a[1]], inner[b[0]][b[1]], outer[b[0]][b[1]]))
        except ValueError:
            pass
    _xf(bm, (0, 0, 0), rot)
    bmesh.ops.translate(bm, vec=Vector((cx, cy, cz)), verts=bm.verts)
    return _finish(bm)


def prism(outline, z0, z1, bevel=0.0, segments=1):
    """Extrude a convex/simple XY outline between z0 and z1, optional edge bevel."""
    bm = bmesh.new()
    bot = [bm.verts.new((x, y, z0)) for (x, y) in outline]
    top = [bm.verts.new((x, y, z1)) for (x, y) in outline]
    bm.faces.new(list(reversed(bot)))
    bm.faces.new(top)
    n = len(outline)
    for i in range(n):
        j = (i + 1) % n
        bm.faces.new((bot[i], bot[j], top[j], top[i]))
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    if bevel > 1e-5:
        top_edges = [e for e in bm.edges if all(abs(v.co.z - z1) < 1e-6 for v in e.verts)]
        bmesh.ops.bevel(bm, geom=top_edges, offset=bevel, segments=segments, affect='EDGES',
                        profile=0.5, clamp_overlap=True)
    return _finish(bm)


def annulus_sector(r0, r1, a0, a1, z0, z1, segments=8, bevel=0.0, bevel_segments=1):
    outline = [(r1 * math.cos(a0 + (a1 - a0) * i / segments), r1 * math.sin(a0 + (a1 - a0) * i / segments))
               for i in range(segments + 1)]
    outline += [(r0 * math.cos(a1 - (a1 - a0) * i / segments), r0 * math.sin(a1 - (a1 - a0) * i / segments))
                for i in range(segments + 1)]
    bm = bmesh.new()
    bot = [bm.verts.new((x, y, z0)) for (x, y) in outline]
    top = [bm.verts.new((x, y, z1)) for (x, y) in outline]
    n = len(outline)
    # strip triangulation (outline is not convex) : pair outer i with inner
    m = segments + 1
    for i in range(segments):
        o0, o1 = i, i + 1
        i0, i1 = n - 1 - i, n - 2 - i
        bm.faces.new((top[o0], top[o1], top[i1], top[i0]))
        bm.faces.new((bot[i0], bot[i1], bot[o1], bot[o0]))
    for i in range(n):
        j = (i + 1) % n
        bm.faces.new((bot[i], bot[j], top[j], top[i]))
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    if bevel > 1e-5:
        outer_top = [e for e in bm.edges if all(abs(v.co.z - z1) < 1e-6 and
                                                 abs(math.hypot(v.co.x, v.co.y) - r1) < 1e-4 for v in e.verts)]
        bmesh.ops.bevel(bm, geom=outer_top, offset=bevel, segments=bevel_segments, affect='EDGES',
                        profile=0.5, clamp_overlap=True)
    return _finish(bm)


def transform(bm, loc=(0, 0, 0), rot=(0, 0, 0), pivot=None):
    """Rotate about `pivot` (default origin) then translate."""
    m = Euler(rot, 'XYZ').to_matrix().to_4x4()
    if pivot is not None:
        p = Vector(pivot)
        m = Matrix.Translation(p) @ m @ Matrix.Translation(-p)
    m = Matrix.Translation(Vector(loc)) @ m
    bmesh.ops.transform(bm, matrix=m, verts=bm.verts)
    return bm


# --------------------------------------------------------------------------- UVs

def box_uv(bm, scale=1.0, grain='x', offset=(0.0, 0.0), rot=0.0):
    """Box projection in metres * scale. `grain` is the world axis mapped to U
    on every face where it lies in the projection plane (wood grain direction)."""
    uv = bm.loops.layers.uv.verify()
    gi = 'xyz'.index(grain)
    cr, sr = math.cos(rot), math.sin(rot)
    for f in bm.faces:
        n = f.normal
        ax = max(range(3), key=lambda i: abs(n[i]))
        plane = [i for i in range(3) if i != ax]
        if gi in plane:
            plane.remove(gi)
            ui, vi = gi, plane[0]
        else:
            ui, vi = plane
        for loop in f.loops:
            co = loop.vert.co
            u, v = co[ui] * scale, co[vi] * scale
            loop[uv].uv = (u * cr - v * sr + offset[0], u * sr + v * cr + offset[1])
    return bm


def flat_uv(bm, u, v):
    uv = bm.loops.layers.uv.verify()
    for f in bm.faces:
        for loop in f.loops:
            loop[uv].uv = (u, v)
    return bm


# --------------------------------------------------------------------------- deformation

def displace(bm, fn):
    for v in bm.verts:
        v.co = Vector(fn(v.co.copy()))
    bm.normal_update()
    return bm


# --------------------------------------------------------------------------- procedural textures (numpy)

def _tile_noise(rng, size, cells):
    g = rng.random((cells, cells)).astype(np.float32)
    idx = np.arange(size, dtype=np.float32) * cells / size
    i0 = np.floor(idx).astype(int) % cells
    i1 = (i0 + 1) % cells
    f = idx - np.floor(idx)
    f = f * f * (3 - 2 * f)
    a = g[i0][:, i0] * (1 - f)[None, :] + g[i0][:, i1] * f[None, :]
    b = g[i1][:, i0] * (1 - f)[None, :] + g[i1][:, i1] * f[None, :]
    return a * (1 - f)[:, None] + b * f[:, None]


def fbm(seed, size, base=4, octaves=5, gain=0.5):
    rng = np.random.default_rng(seed)
    out = np.zeros((size, size), np.float32)
    amp, tot = 1.0, 0.0
    cells = base
    for _ in range(octaves):
        out += amp * _tile_noise(rng, size, min(cells, size))
        tot += amp
        amp *= gain
        cells *= 2
    return out / tot


def height_to_normal(h, strength):
    dx = (np.roll(h, -1, 1) - np.roll(h, 1, 1)) * 0.5 * strength
    dy = (np.roll(h, -1, 0) - np.roll(h, 1, 0)) * 0.5 * strength
    n = np.stack([-dx, -dy, np.ones_like(h)], -1)
    n /= np.linalg.norm(n, axis=-1, keepdims=True)
    return n * 0.5 + 0.5


def make_image(name, rgb, colorspace='sRGB'):
    """rgb: HxWx3 float array 0..1 (row 0 = bottom, Blender convention)."""
    h, w = rgb.shape[:2]
    img = bpy.data.images.get(name)
    if img:
        bpy.data.images.remove(img)
    img = bpy.data.images.new(name, w, h, alpha=False, float_buffer=False)
    rgba = np.concatenate([np.clip(rgb, 0, 1), np.ones((h, w, 1), np.float32)], -1).astype(np.float32)
    img.colorspace_settings.name = colorspace     # must precede the pixel write (it resets the buffer)
    img.pixels.foreach_set(rgba.ravel())
    img.update()
    img.file_format = 'PNG'
    if TEXTURE_DIR is not None:
        # Write the authored map next to the sources and bind the image to that file,
        # so the .blend references it and the exporter reads the real pixels.
        path = Path(TEXTURE_DIR) / f'{name}.png'
        img.filepath_raw = str(path)
        img.save()
        img.source = 'FILE'
        img.filepath = str(path)
        img.reload()
        img.colorspace_settings.name = colorspace
    else:
        img.pack()
    return img


def lerp_color(t, a, b):
    a, b = np.array(a, np.float32), np.array(b, np.float32)
    return a[None, None, :] * (1 - t[..., None]) + b[None, None, :] * t[..., None]


def tex_walnut(size=1024, seed=11):
    """Oiled walnut, straight grain along U, restrained figure. Returns
    (basecolor, normal) arrays. ~1 UV unit = 1.1 m with box_uv(scale=0.9)."""
    y = np.linspace(0, 1, size, endpoint=False, dtype=np.float32)
    warp = fbm(seed, size, base=2, octaves=4)
    fine = fbm(seed + 1, size, base=48, octaves=3)
    # stretch fine noise along the grain (U) so it reads as fibres, not blotches
    streak = sum(np.roll(fine, k, 1) for k in range(-10, 11)) / 21.0
    streak = (streak - streak.min()) / (np.ptp(streak) + 1e-6)
    lines = np.sin((y[:, None] * 64 + warp * 1.5) * TAU) * 0.5 + 0.5
    pores = (fbm(seed + 2, size, base=192, octaves=2) > 0.66).astype(np.float32)
    pores = sum(np.roll(pores, k, 1) for k in range(-2, 3)) / 5.0
    t = np.clip(0.55 * streak + 0.25 * lines ** 3 + 0.2 * warp, 0, 1)
    col = lerp_color(t, (0.085, 0.048, 0.026), (0.19, 0.11, 0.058))
    col *= (1 - 0.18 * pores)[..., None]
    h = 0.4 * streak + 0.2 * lines - 0.6 * pores
    return col, height_to_normal(h, 1.6)


def tex_weave(size=512, seed=21, base=(0.16, 0.22, 0.13), alt=(0.22, 0.29, 0.17)):
    u = np.linspace(0, 1, size, endpoint=False, dtype=np.float32)
    f = 64
    su = np.sin(u[None, :] * f * TAU)
    sv = np.sin(u[:, None] * f * TAU)
    checker = (np.sign(np.sin(u[None, :] * f * TAU / 2) * np.sin(u[:, None] * f * TAU / 2)) * 0.5 + 0.5)
    warp = np.abs(sv) * checker + np.abs(su) * (1 - checker)
    slub = fbm(seed, size, base=16, octaves=3)
    t = np.clip(0.65 * warp + 0.35 * slub, 0, 1)
    col = lerp_color(t, base, alt)
    return col, height_to_normal(warp * 0.6 + slub * 0.2, 1.4)


def tex_leather(size=512, seed=31):
    cells = fbm(seed, size, base=48, octaves=2)
    wide = fbm(seed + 1, size, base=4, octaves=4)
    grain = np.abs(cells - 0.5) * 2
    t = np.clip(0.5 * wide + 0.3 * grain, 0, 1)
    col = lerp_color(t, (0.16, 0.075, 0.035), (0.33, 0.17, 0.08))
    return col, height_to_normal(1 - grain, 1.0)


# --------------------------------------------------------------------------- materials

def _bsdf(mat):
    mat.use_nodes = True
    nt = mat.node_tree
    return nt, nt.nodes['Principled BSDF']


def pbr(name, base, rough, metal=0.0, coat=0.0, coat_rough=0.2, emission=None, emit=0.0,
        transmission=0.0, ior=1.45, base_img=None, normal_img=None, normal_strength=1.0,
        mr_img=None, double_sided=False):
    m = bpy.data.materials.get(name) or bpy.data.materials.new(name)
    nt, b = _bsdf(m)
    b.inputs['Base Color'].default_value = (*base, 1)
    b.inputs['Roughness'].default_value = rough
    b.inputs['Metallic'].default_value = metal
    b.inputs['IOR'].default_value = ior
    if coat:
        b.inputs['Coat Weight'].default_value = coat
        b.inputs['Coat Roughness'].default_value = coat_rough
    if emission is not None:
        b.inputs['Emission Color'].default_value = (*emission, 1)
        b.inputs['Emission Strength'].default_value = emit
    if transmission:
        b.inputs['Transmission Weight'].default_value = transmission
    if base_img is not None:
        t = nt.nodes.new('ShaderNodeTexImage'); t.image = base_img; t.location = (-500, 300)
        nt.links.new(t.outputs['Color'], b.inputs['Base Color'])
    if mr_img is not None:
        t = nt.nodes.new('ShaderNodeTexImage'); t.image = mr_img; t.location = (-700, 0)
        sep = nt.nodes.new('ShaderNodeSeparateColor'); sep.location = (-400, 0)
        nt.links.new(t.outputs['Color'], sep.inputs['Color'])
        nt.links.new(sep.outputs['Green'], b.inputs['Roughness'])
        nt.links.new(sep.outputs['Blue'], b.inputs['Metallic'])
    if normal_img is not None:
        t = nt.nodes.new('ShaderNodeTexImage'); t.image = normal_img; t.location = (-700, -300)
        nm = nt.nodes.new('ShaderNodeNormalMap'); nm.location = (-300, -300)
        nm.inputs['Strength'].default_value = normal_strength
        nt.links.new(t.outputs['Color'], nm.inputs['Color'])
        nt.links.new(nm.outputs['Normal'], b.inputs['Normal'])
    m.use_backface_culling = not double_sided
    return m


class Palette:
    """A small authored texture atlas so several non-tinted finishes (graphite
    rubber, brushed brass, dark steel, ...) share one material and one draw call
    per node. Each finish is a flat 16 px cell; parts get UVs collapsed to the
    cell centre, so no mip bleeding occurs on a part's interior."""

    def __init__(self, name, cells, cell_px=16, grid=4):
        self.name, self.cells, self.grid = name, list(cells.keys()), grid
        size = cell_px * grid
        base = np.zeros((size, size, 3), np.float32)
        mr = np.zeros((size, size, 3), np.float32)
        self.uv = {}
        for i, (key, (col, rough, metal)) in enumerate(cells.items()):
            gx, gy = i % grid, i // grid
            base[gy * cell_px:(gy + 1) * cell_px, gx * cell_px:(gx + 1) * cell_px] = col
            mr[gy * cell_px:(gy + 1) * cell_px, gx * cell_px:(gx + 1) * cell_px] = (1.0, rough, metal)
            self.uv[key] = ((gx + 0.5) / grid, (gy + 0.5) / grid)
        safe = '_'.join(w for w in name.lower().replace('•', ' ').split() if w.isalnum())
        self.base_img = make_image(f'{safe}_palette_basecolor', base, 'sRGB')
        self.mr_img = make_image(f'{safe}_palette_orm', mr, 'Non-Color')
        self.material = pbr(name, (1, 1, 1), 1.0, 1.0, base_img=self.base_img, mr_img=self.mr_img)


# --------------------------------------------------------------------------- kit / export

class Kit:
    def __init__(self, root_name, detail=1.0):
        self.root_name = root_name
        self.detail = detail
        self.objects = []
        self.root = self.empty(root_name, None, (0, 0, 0))
        self.nodes = {root_name: self.root}
        self.buckets = {}
        self.palette = None
        self.anchor_specs = []

    def empty(self, name, parent, loc, rot=(0, 0, 0), display='PLAIN_AXES', size=0.05):
        o = bpy.data.objects.new(name, None)
        o.empty_display_type = display
        o.empty_display_size = size
        bpy.context.scene.collection.objects.link(o)
        if parent is not None:
            o.parent = parent
            pw = parent.matrix_world.copy()
            o.matrix_world = Matrix.Translation(Vector(loc)) @ Euler(rot, 'XYZ').to_matrix().to_4x4()
            o.matrix_parent_inverse = Matrix.Identity(4)
            o.matrix_basis = pw.inverted() @ o.matrix_world
        else:
            o.location = loc
            o.rotation_euler = rot
        bpy.context.view_layer.update()
        self.objects.append(o)
        return o

    def pivot(self, name, parent, loc):
        n = self.empty(name, self.nodes[parent], loc, display='SPHERE', size=0.03)
        self.nodes[name] = n
        return n

    def anchor(self, name, parent, loc, rot=(0, 0, 0), kind='contact', note=''):
        a = self.empty(name, self.nodes[parent], loc, rot, display='ARROWS', size=0.06)
        a['anchor'] = kind
        if note:
            a['note'] = note
        self.anchor_specs.append({'name': name, 'parent': parent, 'kind': kind, 'note': note,
                                  'blender_location': [round(c, 4) for c in loc],
                                  'blender_rotation': [round(c, 4) for c in rot]})
        return a

    def add(self, node, material, bm, obj=None, origin=None):
        """Add a bmesh to the (node, material[, obj]) bucket. `material` is a
        bpy material, or 'pal:<cell>' to use the kit palette."""
        if isinstance(material, str) and material.startswith('pal:'):
            u, v = self.palette.uv[material[4:]]
            flat_uv(bm, u, v)
            material = self.palette.material
        elif not bm.loops.layers.uv:
            flat_uv(bm, 0.5, 0.5)
        key = (node, material.name, obj)
        entry = self.buckets.setdefault(key, {'mat': material, 'bms': [], 'origin': origin})
        entry['bms'].append(bm)

    def finish(self):
        for (node, matname, obj), entry in self.buckets.items():
            merged = bmesh.new()
            for b in entry['bms']:
                me_tmp = bpy.data.meshes.new('tmp')
                b.to_mesh(me_tmp)
                merged.from_mesh(me_tmp)
                bpy.data.meshes.remove(me_tmp)
                b.free()
            parent = self.nodes[node]
            origin = Vector(entry['origin']) if entry['origin'] is not None else parent.matrix_world.translation.copy()
            bmesh.ops.translate(merged, vec=-origin, verts=merged.verts)
            # Triangulate here (deterministic, single-threaded bmesh) so the exported
            # index order is reproducible run to run and tangents can be generated.
            bmesh.ops.triangulate(merged, faces=list(merged.faces), quad_method='FIXED', ngon_method='EAR_CLIP')
            name = obj or f'{node}_{matname}'
            me = bpy.data.meshes.new(name)
            merged.to_mesh(me)
            merged.free()
            me.materials.append(entry['mat'])
            for p in me.polygons:
                p.use_smooth = True
            me.set_sharp_from_angle(angle=math.radians(38))
            o = bpy.data.objects.new(name, me)
            bpy.context.scene.collection.objects.link(o)
            o.parent = parent
            o.matrix_parent_inverse = Matrix.Identity(4)
            o.location = origin - parent.matrix_world.translation
            self.objects.append(o)
        bpy.context.view_layer.update()
        return self.objects


def triangles(objs):
    total = 0
    for o in objs:
        if o.type == 'MESH':
            total += sum(len(p.vertices) - 2 for p in o.data.polygons)
    return total


def export_glb(path, objs):
    bpy.ops.object.select_all(action='DESELECT')
    for o in objs:
        o.select_set(True)
    bpy.ops.export_scene.gltf(
        filepath=str(path), export_format='GLB', use_selection=True, export_apply=True,
        export_yup=True, export_extras=True, export_texcoords=True, export_normals=True,
        export_tangents=False, export_materials='EXPORT', export_animations=False,
        export_cameras=False, export_lights=False, export_image_format='AUTO')


def sha256(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for chunk in iter(lambda: f.read(1 << 20), b''):
            h.update(chunk)
    return h.hexdigest()


def superquadric(center, radii, e_side=0.3, e_up=0.3, u=32, v=16, rot=(0, 0, 0)):
    """Rounded 'pillow' solid: a UV sphere remapped to a superellipsoid. Small
    exponents give boxy cushions with soft, densely sampled edges (good for
    seams and compression displacement)."""
    bm = _unit_sphere(u, v)
    for vert in bm.verts:
        x, y, z = vert.co
        rxy = math.hypot(x, y)
        th = math.atan2(rxy, z)
        ph = math.atan2(y, x)
        st, ct = math.sin(th), math.cos(th)
        vert.co = Vector((_se(st, e_up) * _se(math.cos(ph), e_side),
                          _se(st, e_up) * _se(math.sin(ph), e_side),
                          _se(ct, e_up)))
    _xf(bm, center, rot, radii)
    return _finish(bm)


def path_tube(points, radius, vseg=6, closed=True):
    """Sweep a circle along a polyline (piping, straps, handles)."""
    bm = bmesh.new()
    pts = [Vector(p) for p in points]
    n = len(pts)
    rings = []
    for i, p in enumerate(pts):
        if closed:
            t = (pts[(i + 1) % n] - pts[(i - 1) % n]).normalized()
        else:
            t = (pts[min(i + 1, n - 1)] - pts[max(i - 1, 0)]).normalized()
        ref = Vector((0, 0, 1)) if abs(t.z) < 0.9 else Vector((1, 0, 0))
        a = t.cross(ref).normalized()
        b = t.cross(a).normalized()
        rings.append([bm.verts.new(p + radius * (math.cos(TAU * j / vseg) * a + math.sin(TAU * j / vseg) * b))
                      for j in range(vseg)])
    last = n if closed else n - 1
    for i in range(last):
        r0, r1 = rings[i], rings[(i + 1) % n]
        for j in range(vseg):
            j1 = (j + 1) % vseg
            bm.faces.new((r0[j], r1[j], r1[j1], r0[j1]))
    if not closed:
        bm.faces.new(list(reversed(rings[0])))
        bm.faces.new(rings[-1])
    return _finish(bm)


def rounded_rect_path(cx, cy, z, w, d, r, corner_seg=4):
    pts = []
    corners = [(cx + w / 2 - r, cy + d / 2 - r, 0), (cx - w / 2 + r, cy + d / 2 - r, math.pi / 2),
               (cx - w / 2 + r, cy - d / 2 + r, math.pi), (cx + w / 2 - r, cy - d / 2 + r, 1.5 * math.pi)]
    for (x, y, a0) in corners:
        for i in range(corner_seg + 1):
            a = a0 + (math.pi / 2) * i / corner_seg
            pts.append((x + r * math.cos(a), y + r * math.sin(a), z))
    return pts


def export_glb_fmt(path, objs, image_format='AUTO', jpeg_quality=88):
    # tangents only where a normal map needs them (validator: portable tangent space)
    tangents = any(slot.material and slot.material.node_tree and
                   any(n.type == 'NORMAL_MAP' for n in slot.material.node_tree.nodes)
                   for o in objs if o.type == 'MESH' for slot in o.material_slots)
    bpy.ops.object.select_all(action='DESELECT')
    for o in objs:
        o.select_set(True)
    bpy.ops.export_scene.gltf(
        filepath=str(path), export_format='GLB', use_selection=True, export_apply=True,
        export_yup=True, export_extras=True, export_texcoords=True, export_normals=True,
        export_tangents=tangents, export_materials='EXPORT', export_animations=False,
        export_cameras=False, export_lights=False, export_image_format=image_format,
        export_jpeg_quality=jpeg_quality)


def _surface_point(radii, exp, theta, phi, inset):
    rx, ry, rz = radii
    st, ct = math.sin(theta), math.cos(theta)
    x = rx * _se(st, exp) * _se(math.cos(phi), exp)
    y = ry * _se(st, exp) * _se(math.sin(phi), exp)
    z = rz * _se(ct, exp)
    n = Vector((x / rx ** 2, y / ry ** 2, z / rz ** 2)).normalized()
    return Vector((x, y, z)) - n * inset


def _thick_grid(bm, grid_outer, grid_inner, close_rows=True, wrap=False):
    """Faces for an outer/inner skin pair of (rows x cols) vertex grids, with
    the boundary closed by walls. `wrap` joins the last column to the first."""
    rows = len(grid_outer) - 1
    cols = len(grid_outer[0]) - (0 if wrap else 1)
    nc = len(grid_outer[0])
    for i in range(rows):
        for k in range(cols):
            k1 = (k + 1) % nc
            bm.faces.new((grid_outer[i][k], grid_outer[i][k1], grid_outer[i + 1][k1], grid_outer[i + 1][k]))
            bm.faces.new((grid_inner[i][k], grid_inner[i + 1][k], grid_inner[i + 1][k1], grid_inner[i][k1]))
    if wrap:
        for i in (0, rows):
            for k in range(nc):
                k1 = (k + 1) % nc
                bm.faces.new((grid_outer[i][k], grid_inner[i][k], grid_inner[i][k1], grid_outer[i][k1]))


def shell_oval_patch(center, radii, tc, pc, ht, hp, rings, segs, thickness, exp=1.0, lip=0.4):
    """Thick panel with an elliptical outline in (theta, phi) space centred on
    (tc, pc) with half extents (ht, hp): concentric rings around a centre fan."""
    bm = bmesh.new()
    outer, inner = [], []
    for r in range(1, rings + 1):
        f = r / rings
        orow, irow = [], []
        for s in range(segs):
            a = TAU * s / segs
            th, ph = tc + ht * f * math.sin(a), pc + hp * f * math.cos(a)
            orow.append(bm.verts.new(_surface_point(radii, exp, th, ph, thickness * lip if r == rings else 0)))
            irow.append(bm.verts.new(_surface_point(radii, exp, th, ph, thickness)))
        outer.append(orow); inner.append(irow)
    co = bm.verts.new(_surface_point(radii, exp, tc, pc, 0))
    ci = bm.verts.new(_surface_point(radii, exp, tc, pc, thickness))
    for s in range(segs):
        s1 = (s + 1) % segs
        bm.faces.new((co, outer[0][s], outer[0][s1]))
        bm.faces.new((ci, inner[0][s1], inner[0][s]))
    for i in range(rings - 1):
        for s in range(segs):
            s1 = (s + 1) % segs
            bm.faces.new((outer[i][s], outer[i + 1][s], outer[i + 1][s1], outer[i][s1]))
            bm.faces.new((inner[i][s], inner[i][s1], inner[i + 1][s1], inner[i + 1][s]))
    for s in range(segs):
        s1 = (s + 1) % segs
        bm.faces.new((outer[-1][s], inner[-1][s], inner[-1][s1], outer[-1][s1]))
    bmesh.ops.translate(bm, vec=Vector(center), verts=bm.verts)
    return _finish(bm)


def shell_frame(center, radii, tc, pc, rect_ht, rect_hp, oval_ht, oval_hp, segs, rows, thickness, exp=1.0, lip=0.4):
    """Thick bezel between an outer (theta, phi) rectangle and an inner ellipse,
    both centred on (tc, pc): frames an oval visor inside a rectangular opening."""
    bm = bmesh.new()
    outer, inner = [], []
    for r in range(rows + 1):
        f = r / rows
        orow, irow = [], []
        for s in range(segs):
            a = TAU * s / segs
            ca, sa = math.cos(a), math.sin(a)
            m = max(abs(ca), abs(sa))
            rp, rt = rect_hp * ca / m, rect_ht * sa / m         # ray hits the rectangle
            op, ot = oval_hp * ca, oval_ht * sa                  # ... and the ellipse
            ph = pc + op + (rp - op) * f
            th = tc + ot + (rt - ot) * f
            edge = r in (0, rows)
            orow.append(bm.verts.new(_surface_point(radii, exp, th, ph, thickness * lip if edge else 0)))
            irow.append(bm.verts.new(_surface_point(radii, exp, th, ph, thickness)))
        outer.append(orow); inner.append(irow)
    _thick_grid(bm, outer, inner, wrap=True)
    bmesh.ops.translate(bm, vec=Vector(center), verts=bm.verts)
    return _finish(bm)
