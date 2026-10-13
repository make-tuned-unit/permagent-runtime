"""W3 round-2 helpers (hero robot + Arrival furniture only).

Kept separate from detail_lib.py on purpose: W4's botany/places builders import
detail_lib and record its hash, so W3 changes live here and never alter W4's
outputs or provenance.

  * HeroKit: a Kit whose buckets can ask for full smooth shading (no
    angle-based sharp edges). Low-segment rounded parts (eyes, finger
    phalanges) otherwise pick up hard edges from the 38 degree default and read
    as facets at close-up.
  * tex_walnut_v2: directional oiled-walnut grain (growth rings with dark
    latewood lines, long fibre streaks, pore dashes) that tiles on both axes.
  * load_png / normal_from_basecolor: bind an accepted external base-colour
    candidate (W5) and derive a matching tangent-space normal map from it.
"""
import math
import numpy as np
import bpy
from detail_lib import Kit, height_to_normal, lerp_color, TAU


class HeroKit(Kit):
    def __init__(self, root_name, detail=1.0):
        super().__init__(root_name, detail)
        self.smooth = {}

    def add(self, node, material, bm, obj=None, origin=None, smooth=None):
        """smooth: None keeps the 38 degree sharp-edge rule; a number (degrees)
        overrides it for the whole bucket; >= 180 means fully smooth."""
        super().add(node, material, bm, obj, origin)
        mname = self.palette.material.name if (isinstance(material, str) and material.startswith('pal:')) \
            else material.name
        if smooth is not None:
            key = (node, mname, obj)
            self.smooth[key] = max(self.smooth.get(key, 0.0), float(smooth))

    def finish(self):
        keys = list(self.buckets.keys())
        before = len(self.objects)
        super().finish()
        meshes = [o for o in self.objects[before:] if o.type == 'MESH']
        assert len(meshes) == len(keys)
        for key, o in zip(keys, meshes):
            ang = self.smooth.get(key)
            if ang is None:
                continue
            me = o.data
            if 'sharp_edge' in me.attributes:
                me.attributes.remove(me.attributes['sharp_edge'])
            if ang < 180:
                me.set_sharp_from_angle(angle=math.radians(ang))
        return self.objects


# ----------------------------------------------------------------------------- textures
def _aniso_noise(rng, size, cu, cv):
    """Tileable value noise with cu cells along U (columns) and cv along V (rows)."""
    g = rng.random((cv, cu)).astype(np.float32)
    def axis(n, cells):
        idx = np.arange(n, dtype=np.float32) * cells / n
        i0 = np.floor(idx).astype(int) % cells
        f = idx - np.floor(idx)
        return i0, (i0 + 1) % cells, f * f * (3 - 2 * f)
    u0, u1, fu = axis(size, cu)
    v0, v1, fv = axis(size, cv)
    a = g[v0][:, u0] * (1 - fu)[None, :] + g[v0][:, u1] * fu[None, :]
    b = g[v1][:, u0] * (1 - fu)[None, :] + g[v1][:, u1] * fu[None, :]
    return a * (1 - fv)[:, None] + b * fv[:, None]


def _aniso_fbm(seed, size, cu, cv, octaves=4, gain=0.5):
    rng = np.random.default_rng(seed)
    out = np.zeros((size, size), np.float32)
    amp = tot = 0.0
    amp = 1.0
    for _ in range(octaves):
        out += amp * _aniso_noise(rng, size, min(cu, size), min(cv, size))
        tot += amp
        amp *= gain
        cu, cv = cu * 2, cv * 2
    return out / tot


def _smoothstep(e0, e1, x):
    t = np.clip((x - e0) / (e1 - e0), 0, 1)
    return t * t * (3 - 2 * t)


def tex_walnut_v2(size=1024, seed=11):
    """Flat-sawn oiled walnut, grain along U (image columns = the long axis of a
    board). Returns (basecolor, normal) float arrays (row 0 = bottom).
    ~1 UV unit = 1.1 m with box_uv(scale=0.9), so 1024 px ~ 1.1 mm/px.

    Structure, coarse to fine:
      board tone   - slow colour drift across the grain (heartwood variation)
      growth rings - 26 rings per tile (uneven widths), warped by noise that is ~20x longer
                     along the grain than across it, so rings run as long
                     gently wandering lines; each ring ends in a dark,
                     sharp-edged latewood line (the 'dark seams' of the figure)
      fibre        - very long thin streaks (cells 6 along x 360 across) plus
                     long colour streaks
      pores        - short dark dashes along the grain
    """
    n = size
    v = (np.arange(n, dtype=np.float32) / n)[:, None] * np.ones((1, n), np.float32)
    warp = _aniso_fbm(seed, n, 2, 4, octaves=3)            # long, gentle wander of the rings
    wobble = _aniso_fbm(seed + 1, n, 5, 30, octaves=2)      # local ripple
    spacing = _aniso_fbm(seed + 5, n, 1, 8, octaves=2)      # uneven ring widths (wet/dry years)
    rings = 26
    r = v * rings + (warp - 0.5) * 1.7 + (wobble - 0.5) * 0.30 + (spacing - 0.5) * 2.2
    f = r - np.floor(r)
    # earlywood (light, open) -> gradual darkening -> abrupt thin latewood line
    late = _smoothstep(0.55, 0.92, f) * (1 - _smoothstep(0.94, 0.995, f))
    line = np.exp(-((f - 0.925) / 0.022) ** 2)
    tone = _aniso_fbm(seed + 2, n, 1, 3, octaves=2)          # across-grain heartwood drift
    streak = _aniso_fbm(seed + 6, n, 2, 48, octaves=3)       # long colour streaks along the grain
    fibre = _aniso_fbm(seed + 3, n, 6, 360, octaves=2)
    fibre = (fibre - fibre.min()) / (np.ptp(fibre) + 1e-6)
    pores = _aniso_fbm(seed + 4, n, 40, 420, octaves=2)
    pores = _smoothstep(0.66, 0.78, pores)
    t = np.clip(0.42 + 0.45 * (tone - 0.5) + 0.55 * (streak - 0.5) + 0.25 * (fibre - 0.5), 0, 1)
    early = lerp_color(t, (0.155, 0.088, 0.048), (0.30, 0.18, 0.098))
    dark = np.array((0.07, 0.038, 0.021), np.float32)
    k = np.clip(0.38 * late + 0.50 * line, 0, 0.85)
    col = early * (1 - k[..., None]) + dark[None, None, :] * k[..., None]
    col *= (1 - 0.28 * pores)[..., None]
    col *= (0.86 + 0.28 * fibre)[..., None]
    h = -0.35 * line - 0.2 * late - 0.8 * pores + 0.35 * fibre
    return np.clip(col, 0, 1).astype(np.float32), height_to_normal(h, 1.1)


def load_png(path, name, colorspace='sRGB'):
    """Load an accepted external PNG, bound to its file (relative in the .blend),
    and return (image, float rgb array row 0 = bottom)."""
    img = bpy.data.images.get(name)
    if img:
        bpy.data.images.remove(img)
    img = bpy.data.images.load(str(path), check_existing=False)
    img.name = name
    img.colorspace_settings.name = colorspace
    w, h = img.size
    a = np.array(img.pixels[:], np.float32).reshape(h, w, 4)[..., :3]
    return img, a


def normal_from_basecolor(rgb, strength=1.6, blur=6):
    """High-passed luminance of a base-colour map as height, then a normal map.
    The box blur wraps, so the result tiles exactly when the input does."""
    lum = rgb @ np.array([0.2126, 0.7152, 0.0722], np.float32)
    low = lum.copy()
    for ax in (0, 1):
        acc = np.zeros_like(low)
        for k in range(-blur, blur + 1):
            acc += np.roll(low, k, ax)
        low = acc / (2 * blur + 1)
    hp = lum - low
    hp = hp / (np.abs(hp).max() + 1e-6)
    return height_to_normal(hp, strength)


def tint_base(mat, rgb):
    """Multiply a material's base-colour texture by a factor (exported as glTF
    baseColorFactor), so an external map is calibrated without editing its pixels."""
    nt = mat.node_tree
    b = nt.nodes['Principled BSDF']
    link = b.inputs['Base Color'].links[0]
    tex = link.from_node
    nt.links.remove(link)
    mix = nt.nodes.new('ShaderNodeMix')
    mix.data_type = 'RGBA'
    mix.blend_type = 'MULTIPLY'
    mix.inputs['Factor'].default_value = 1.0
    mix.inputs[7].default_value = (*rgb, 1.0)          # B (colour)
    nt.links.new(tex.outputs['Color'], mix.inputs[6])  # A (colour)
    nt.links.new(mix.outputs[2], b.inputs['Base Color'])
    return mat
