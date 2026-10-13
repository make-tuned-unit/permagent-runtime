"""Hero robot v4 (lane W3): the same character as hero-kit v2/v3b, rebuilt for
close-up and inside the character budget.

Runtime contract preserved (heroRobot.ts / heroRig.ts / agentWardrobe.ts):
  * root node `robot`, floor origin, faces glTF +Z (Blender -Y).
  * rigid pivots `Head` (z 1.45), `Arm.L`/`Arm.R` (x -/+0.27, z 1.07),
    `Leg.L`/`Leg.R` (x -/+0.125, z 0.68) at their legacy positions, so the
    runtime's rotation.x/z animation and the wardrobe offsets still fit.
  * material name prefixes `Ceramic` (per-agent tint, shadow casting),
    `Faceplate` (forced dark glass) and `Eyes` (emissive tint, blink).
  * head-local mouth slot (glTF 0, -0.085, 0.307) lies on the faceplate and is
    left empty: the runtime adds its voice-synchronised mouth there.
  * no skin, no clips: articulation is nested rigid pivots.

Additive (backwards compatible, rest pose = identity):
  * `Forearm.L/R` (elbow), `Hand.L/R` (wrist), `Shin.L/R` (knee), `Foot.L/R`
    (ankle) child pivots so seated / hand-contact poses are real joints.
  * eyes are one mesh whose origin is the eye centre line, so the runtime's
    `eye.scale.y` blink closes the eyes in place instead of sliding them.
  * named anchors (gaze, mouth, palms, fingertips, soles, seat contact).
"""
import math
from mathutils import Vector, Matrix
from hero_lib import HeroKit
from detail_lib import (Kit, Palette, pbr, rbox, ellipsoid, lathe, tube, torus, shell_patch, shell_frame,
                        shell_oval_patch, superquadric,
                        transform, seg, TAU)

# Legacy pivot positions (Blender space). Do not move: runtime + wardrobe depend on them.
HEAD = (0.0, 0.0, 1.45)
SHOULDER = {'L': (-0.27, 0.0, 1.07), 'R': (0.27, 0.0, 1.07)}
HIP = {'L': (-0.125, 0.0, 0.68), 'R': (0.125, 0.0, 0.68)}
ELBOW_REL = (0.08, -0.02, -0.25)     # x is mirrored per side
WRIST_REL = (0.09, -0.09, -0.445)
KNEE_REL = (0.0, -0.009, -0.335)
ANKLE_REL = (0.0, -0.03, -0.58)

PALETTE_CELLS = {
    'graphite': ((0.026, 0.038, 0.041), 0.62, 0.0),   # joint rubber
    'brass': ((0.56, 0.36, 0.15), 0.34, 1.0),         # brushed hardware
    'bronze': ((0.30, 0.18, 0.09), 0.42, 1.0),        # darker fasteners
    'steel': ((0.10, 0.11, 0.115), 0.38, 1.0),        # dark bearing steel
    'glass': ((0.012, 0.035, 0.042), 0.12, 0.0),      # chest reader glass
    'tread': ((0.018, 0.022, 0.024), 0.85, 0.0),      # sole rubber
}


def materials():
    ceramic = pbr('Ceramic • warm ivory', (0.83, 0.79, 0.67), 0.30, 0.0, coat=0.34, coat_rough=0.22)
    face = pbr('Faceplate • deep teal glass', (0.006, 0.018, 0.022), 0.14, 0.0, coat=0.3, coat_rough=0.06)
    eyes = pbr('Eyes • jade phosphor', (0.14, 0.62, 0.48), 0.24, 0.0, emission=(0.14, 0.62, 0.48), emit=0.6)
    return ceramic, face, eyes


def sx(side):
    return -1.0 if side == 'L' else 1.0


def add(p, a):
    return tuple(x + y for x, y in zip(p, a))


# ---------------------------------------------------------------------- digits
# Per-digit rigid pivots (round 2). Index is the digit nearest the thumb; the
# thumb sits on the medial (body-centre) side, which is anatomically right for
# both the pronated rest pose and the palm-down seated pose. Segment tuples are
# (length, width, depth, built-in relaxed curl in rad). Lengths run knuckle to
# knuckle; a ~2 mm crease is left between phalanges.
FINGER_PITCH = 0.0235
DIGITS = [
    dict(name='Index', i=0, segs=[(0.037, 0.0175, 0.0195, 0.10), (0.027, 0.0165, 0.018, 0.30), (0.022, 0.0155, 0.0165, 0.22)]),
    dict(name='Middle', i=1, segs=[(0.040, 0.0180, 0.0200, 0.12), (0.029, 0.0170, 0.0185, 0.33), (0.023, 0.0158, 0.0168, 0.24)]),
    dict(name='Ring', i=2, segs=[(0.038, 0.0172, 0.0193, 0.14), (0.027, 0.0162, 0.0178, 0.36), (0.022, 0.0152, 0.0162, 0.26)]),
    dict(name='Little', i=3, segs=[(0.031, 0.0160, 0.0180, 0.16), (0.022, 0.0150, 0.0165, 0.40), (0.019, 0.0140, 0.0150, 0.28)]),
    dict(name='Thumb', i=None, segs=[(0.032, 0.0215, 0.0215, 0.0), (0.028, 0.0190, 0.0190, 0.25), (0.023, 0.0170, 0.0170, 0.25)]),
]
HAND_POSES = {
    'relaxed': {'curl': {'Thumb': 0.0, 'Index': 0.0, 'Middle': 0.0, 'Ring': 0.0, 'Little': 0.0},
                'note': 'rest pose (identity); also the seated palms-on-table pose (fingertips meet the table plane)'},
    'flat': {'curl': {'Thumb': -0.10, 'Index': -0.12, 'Middle': -0.12, 'Ring': -0.12, 'Little': -0.12},
             'note': 'fingers opened slightly for a flat-hand gesture'},
    'point': {'curl': {'Thumb': 0.45, 'Index': -0.10, 'Middle': 1.15, 'Ring': 1.20, 'Little': 1.25},
              'note': 'index extended, others folded; point at the instrument / a guest'},
    'grip': {'curl': {'Thumb': 0.45, 'Index': 0.95, 'Middle': 1.0, 'Ring': 1.05, 'Little': 1.1},
             'note': 'closed around anchor_grip (cup handle, folio edge)'},
}
DIGIT_LIMITS = {'curl_rotation_x': [-0.25, 1.40], 'spread_rotation_z': [-0.20, 0.20]}


def digit_frame(side, dg):
    """(base point, base rotation matrix) of a digit's knuckle pivot, Blender space."""
    s = sx(side)
    if dg['name'] == 'Thumb':
        base = Vector((s * 0.36 - s * 0.050, -0.097, 0.574))
        rot = Matrix.Rotation(s * 0.62, 3, 'Y') @ Matrix.Rotation(0.30, 3, 'X')
    else:
        i = dg['i']
        base = Vector((s * 0.36 - s * (1.5 - i) * FINGER_PITCH, -0.106, 0.549))
        rot = Matrix.Rotation(s * (1.3 - i) * 0.05, 3, 'Y')
    return base, rot


def build_digit(k, side, dg, hand_node, ceramic, hi):
    """Adds the digit pivot, its phalanges and its fingertip anchor. LOD0 gives
    each digit its own mesh (one ceramic primitive per digit); lower LODs fuse
    the digit into the Hand mesh but keep the pivot and anchor, so a pose never
    references a missing node."""
    s = sx(side)
    pname = f"{dg['name']}.{side}"
    base, rot = digit_frame(side, dg)
    k.pivot(pname, hand_node, tuple(base))
    node = pname if hi else hand_node
    segs = dg['segs']
    if not hi:   # two cheap segments: proximal, then middle+distal fused at the mean curl
        (l0, w0, d0, c0), (l1, w1, d1, c1), (l2, w2, d2, c2) = segs
        segs = [(l0, w0, d0, c0), (l1 + l2, w1, d1, c1 + c2 * 0.5)]
    u, v = (10, 6) if hi else (6, 4)
    p = base.copy()
    cum = 0.0
    last = len(segs) - 1
    for j, (ln, w, dp, curl) in enumerate(segs):
        cum += curl
        m = rot @ Matrix.Rotation(cum, 3, 'X')
        direction = m @ Vector((0, 0, -1))
        body = ln - 0.0018
        tip = j == last
        part = superquadric((0, 0, 0), (w / 2, dp / 2, body / 2), 0.62, 0.66 if tip else 0.5, u, v)
        # taper towards the distal end (the knuckle end stays broad)
        for vert in part.verts:
            t = 0.5 - vert.co.z / body          # 0 proximal (+z) .. 1 distal (-z)
            sc = 1.0 - 0.07 * t
            vert.co.x *= sc
            vert.co.y *= sc
        centre = p + direction * (ln / 2)
        transform(part, tuple(centre), m.to_euler('XYZ'))
        k.add(node, ceramic, part, smooth=180)
        p = p + direction * ln
    tip_pt = p - direction * 0.004
    k.anchor(f"anchor_tip_{dg['name'].lower()}_{side}", pname, tuple(tip_pt), kind='contact',
             note=f"{dg['name'].lower()} fingertip pad; follows the {pname} pivot")
    if dg['name'] == 'Middle':
        k.anchor(f'anchor_fingertips_{side}', pname, tuple(tip_pt), kind='contact',
                 note='middle fingertip pad (legacy name, now parented to the Middle pivot); touch / point / page-turn')


def build_robot(detail=1.0):
    """detail 1.0 = LOD0 (close-up), ~0.45 = LOD1."""
    d = detail
    hi = d >= 0.8
    k = HeroKit('robot', d)
    k.palette = Palette('Trim • graphite and brass', PALETTE_CELLS)
    ceramic, face, eyes = materials()
    P = 'pal:'

    # ------------------------------------------------------------------ torso (root)
    T = (0.0, 0.0, 0.90)
    k.add('robot', P + 'graphite', ellipsoid(T, (0.235, 0.173, 0.292), seg(14, d, 10), seg(8, d, 6)))
    rows, cols = seg(6, d, 3), seg(7, d, 3)
    tr = (0.255, 0.19, 0.303)
    gap = 0.035
    front = (-math.pi / 2 - 0.95, -math.pi / 2 + 0.95)
    back = (math.pi / 2 - 0.95, math.pi / 2 + 0.95)
    sectors = [front, (front[1] + gap, back[0] - gap), back, (back[1] + gap, front[0] + TAU - gap)]
    for band in [(0.24, 1.38), (1.42, 2.56)]:
        for (p0, p1) in sectors:
            k.add('robot', ceramic, shell_patch(T, tr, band[0], band[1], p0 + gap / 2, p1 - gap / 2,
                                                rows, cols, 0.013, exp=0.92))
    # chest evidence instrument + glass + phosphor lines (Eyes => runtime tints it with the agent)
    k.add('robot', P + 'brass', rbox((0, -0.184, 1.04), (0.15, 0.034, 0.105), 0.016, seg(2, d, 1)))
    k.add('robot', P + 'glass', rbox((0, -0.203, 1.04), (0.116, 0.012, 0.072), 0.008, seg(2, d, 1)))
    for i in range(3):
        k.add('robot', eyes, rbox((-0.008, -0.2105, 1.058 - i * 0.018), (0.066 - i * 0.012, 0.004, 0.005), 0.0015, 1))
    # collar, neck, waist
    k.add('robot', ceramic, torus((0, 0, 1.195), 0.146, 0.02, seg(32, d, 12), seg(8, d, 5)))
    k.add('robot', P + 'brass', tube((0, 0, 1.13), (0, 0, 1.33), 0.066, seg(16, d, 8)))
    for z in ([1.215, 1.245, 1.275] if hi else [1.245]):
        k.add('robot', P + 'graphite', torus((0, 0, z), 0.073, 0.012, seg(16, d, 10), 4))
    k.add('robot', P + 'brass', tube((0, 0, 0.60), (0, 0, 0.72), 0.078, seg(16, d, 8)))
    for z in ([0.628, 0.658, 0.688] if hi else [0.658]):
        k.add('robot', P + 'graphite', torus((0, 0, z), 0.125, 0.013, seg(20, d, 10), 4))
    k.add('robot', P + 'brass', torus((0, 0, 0.70), 0.13, 0.011, seg(28, d, 10), 4))
    # hip yoke (connects the hip bearings so the legs read as attached)
    k.add('robot', P + 'graphite', rbox((0, 0, 0.655), (0.30, 0.13, 0.07), 0.03, seg(2, d, 1)))
    k.add('robot', P + 'steel', rbox((0, -0.066, 0.655), (0.12, 0.012, 0.04), 0.006, 1))
    if hi:
        for s in (-1, 1):
            for z in (0.84, 0.92, 1.11):
                k.add('robot', P + 'bronze', ellipsoid((s * 0.158, -0.142, z), (0.008, 0.006, 0.008), 6, 4))
        for z in (0.90, 0.93, 0.96):
            k.add('robot', P + 'graphite', rbox((0, 0.176, z), (0.13, 0.016, 0.008), 0.003, 1))
        k.add('robot', P + 'brass', rbox((-0.205, -0.05, 1.105), (0.06, 0.07, 0.055), 0.012, 2))
        # backpack service hatch
        k.add('robot', P + 'steel', rbox((0, 0.19, 1.03), (0.16, 0.02, 0.10), 0.01, 2))

    # ------------------------------------------------------------------ head
    k.pivot('Head', 'robot', HEAD)
    H = (0.0, 0.0, 1.47)
    k.add('Head', P + 'graphite', ellipsoid(H, (0.305, 0.262, 0.286), seg(16, d, 12), seg(9, d, 7)))
    hr = (0.315, 0.27, 0.295)
    hg = 0.024
    bands = [(0.03, 0.80), (0.80 + hg, 2.20), (2.20 + hg, math.pi - 0.06)]
    r_band = [seg(4, d, 2), seg(8, d, 3), seg(4, d, 2)]
    c_sec = seg(6, d, 3)
    for b, (t0, t1) in enumerate(bands):
        if b == 1:
            # face opening is centred on -Y (azimuth -90 deg); three side/back panels
            spans = [(-math.radians(40), math.radians(40)), (math.radians(40), math.radians(140)),
                     (math.radians(140), math.radians(220))]
        else:
            spans = [(i * TAU / 6, (i + 1) * TAU / 6) for i in range(6)]
            if b == 2:
                spans = [(s + TAU / 12, e + TAU / 12) for (s, e) in spans]
        for (p0, p1) in spans:
            k.add('Head', ceramic, shell_patch(H, hr, t0, t1, p0 + hg / 2, p1 - hg / 2, r_band[b], c_sec,
                                               0.012, exp=0.97))
    # visor: an oval moulded lens (legacy outline, portrait parity) set in a
    # ceramic bezel that fills the rectangular face opening; the lens bulges
    # ~5 cm proud of the shell and its thick rim sits on the bezel
    tc, pc = (0.80 + hg + 2.20) / 2, -math.pi / 2
    rect_ht, rect_hp = (2.20 - 0.80 - hg) / 2, (math.radians(100) - hg) / 2
    oval_ht, oval_hp = rect_ht * 0.88, rect_hp * 0.90
    k.add('Head', ceramic, shell_frame(H, hr, tc, pc, rect_ht, rect_hp, oval_ht, oval_hp, seg(32, d, 16),
                                       seg(2, d, 1), 0.012, exp=0.97, lip=0.5))
    k.add('Head', face, shell_oval_patch(H, (0.300, 0.325, 0.285), tc, pc, oval_ht, oval_hp, seg(6, d, 3),
                                         seg(32, d, 16), 0.03, exp=0.97, lip=0.5))
    eye_c = (0.0, -0.300, 1.49)
    for s in (-1, 1):
        k.add('Head', eyes, ellipsoid((s * 0.10, -0.300, 1.49), (0.036, 0.016, 0.066), seg(14, d, 8), seg(8, d, 6)),
              obj='Head_Eyes • jade phosphor', origin=eye_c)
    for s in (-1, 1):
        # ear hub: brass turned disc with a dark recessed centre and a bronze cap
        ear = lathe([(0, -0.03), (0.105, -0.03), (0.122, -0.012), (0.122, 0.012), (0.105, 0.03), (0, 0.03)],
                    seg(28, d, 12))
        transform(ear, (s * 0.31, 0.0, 1.48), (0, math.pi / 2, 0))
        k.add('Head', P + 'brass', ear)
        inset = lathe([(0, -0.012), (0.078, -0.012), (0.078, 0.012), (0, 0.012)], seg(24, d, 10))
        transform(inset, (s * 0.336, -0.012, 1.48), (0, math.pi / 2, 0))
        k.add('Head', P + 'graphite', inset)
        if hi:
            cap = lathe([(0, 0), (0.026, 0), (0.022, 0.012), (0, 0.016)], 16)
            transform(cap, (s * 0.346, -0.012, 1.48), (0, s * math.pi / 2, 0))
            k.add('Head', P + 'bronze', cap)
            for z in (1.58, 1.39):
                k.add('Head', P + 'bronze', ellipsoid((s * 0.292, -0.122, z), (0.009, 0.009, 0.009), 8, 5))
    # crown sensor (tiny, central; keeps hats from intersecting a shell seam)
    if hi:
        k.add('Head', P + 'steel', lathe([(0, 0), (0.04, 0), (0.034, 0.014), (0, 0.016)], 16,
                                         center=(0, 0.03, 1.758)))

    k.anchor('anchor_gaze', 'Head', (0, -0.32, 1.49), kind='gaze',
             note='eye centre line, in front of the visor; aim this at the conversation partner')
    k.anchor('anchor_mouth', 'Head', (0, -0.307, 1.365), kind='voice',
             note='runtime voice mouth slot (glTF head-local 0,-0.085,0.307); kept empty in the mesh')
    k.anchor('anchor_headwear', 'Head', (0, 0, 1.765), kind='attachment', note='crown; wardrobe headwear origin')

    # ------------------------------------------------------------------ arms
    for side in ('L', 'R'):
        s = sx(side)
        A = f'Arm.{side}'
        S = SHOULDER[side]
        k.pivot(A, 'robot', S)
        E = add(S, (s * ELBOW_REL[0], ELBOW_REL[1], ELBOW_REL[2]))
        W = add(S, (s * WRIST_REL[0], WRIST_REL[1], WRIST_REL[2]))
        k.pivot(f'Forearm.{side}', A, E)
        k.pivot(f'Hand.{side}', f'Forearm.{side}', W)
        F, Hn = f'Forearm.{side}', f'Hand.{side}'
        k.add(A, P + 'graphite', ellipsoid((s * 0.282, 0, 1.07), (0.088, 0.087, 0.09), seg(14, d, 10), seg(8, d, 6)))
        # pauldron: thick ceramic cap over the shoulder bearing
        k.add(A, ceramic, shell_patch((s * 0.30, 0, 1.075), (0.112, 0.108, 0.105), 0.0, 1.55,
                                      0, TAU, seg(5, d, 3), seg(16, d, 8), 0.012, exp=0.95))
        k.add(A, ceramic, rbox((s * 0.33, -0.01, 0.95), (0.125, 0.135, 0.235), 0.055, seg(3, d, 1)))
        k.add(A, P + 'graphite', tube((s * 0.33, -0.01, 0.862), (s * 0.35, -0.02, 0.80), 0.047, seg(14, d, 8)))
        # forearm (elbow pivot)
        k.add(F, P + 'brass', torus(E, 0.05, 0.012, seg(16, d, 10), 4, rot=(0, math.pi / 2, 0)))
        k.add(F, P + 'graphite', ellipsoid(E, (0.052, 0.058, 0.058), seg(16, d, 8), seg(8, d, 5)))
        k.add(F, ceramic, rbox((s * 0.36, -0.07, 0.715), (0.128, 0.132, 0.19), 0.05, seg(3, d, 1)))
        k.add(F, P + 'brass', torus((s * 0.36, -0.088, 0.618), 0.044, 0.009, seg(16, d, 10), 4))
        # hand (wrist pivot): palm block and dorsal plate. Each digit is its own
        # pivot at the knuckle (MCP / thumb CMC) so W7 can curl and spread digits
        # individually; phalanges are tapered rounded capsules separated by a
        # small crease, so knuckles read as joints instead of a bar.
        k.add(Hn, P + 'graphite', rbox((s * 0.36, -0.10, 0.583), (0.104, 0.062, 0.078), 0.022, seg(2, d, 1)))
        k.add(Hn, ceramic, rbox((s * 0.36, -0.133, 0.594), (0.09, 0.022, 0.06), 0.01, seg(2, d, 1)))
        for dg in DIGITS:
            build_digit(k, side, dg, Hn, ceramic, hi)
        k.anchor(f'anchor_palm_{side}', Hn, (s * 0.36, -0.068, 0.583), kind='contact',
                 note='palm surface centre; flat-hand table contact (palm faces the body at rest)')
        k.anchor(f'anchor_grip_{side}', Hn, (s * 0.36, -0.125, 0.53), kind='attachment',
                 note='held-object attachment (folio, cup) between fingers and thumb')

    # ------------------------------------------------------------------ legs
    for side in ('L', 'R'):
        s = sx(side)
        L = f'Leg.{side}'
        Hp = HIP[side]
        Kn = add(Hp, KNEE_REL)
        An = add(Hp, ANKLE_REL)
        k.pivot(L, 'robot', Hp)
        k.pivot(f'Shin.{side}', L, Kn)
        k.pivot(f'Foot.{side}', f'Shin.{side}', An)
        Sh, Ft = f'Shin.{side}', f'Foot.{side}'
        x = s * 0.125
        k.add(L, P + 'graphite', ellipsoid((x, 0, 0.64), (0.087, 0.085, 0.08), seg(16, d, 10), seg(8, d, 6)))
        k.add(L, ceramic, rbox((x, 0, 0.49), (0.164, 0.164, 0.25), 0.062, seg(3, d, 1)))
        k.add(L, P + 'graphite', tube((x, 0, 0.37), (x, -0.009, 0.335), 0.06, seg(14, d, 8)))
        # shin (knee pivot)
        k.add(Sh, P + 'graphite', ellipsoid((x, -0.009, 0.345), (0.07, 0.072, 0.062), seg(16, d, 8), seg(8, d, 5)))
        kc = lathe([(0, 0), (0.05, 0), (0.056, 0.012), (0.044, 0.026), (0, 0.03)], seg(20, d, 10))
        transform(kc, (x, -0.06, 0.345), (math.pi / 2, 0, 0))
        k.add(Sh, P + 'brass', kc)
        k.add(Sh, ceramic, rbox((x, -0.005, 0.215), (0.15, 0.152, 0.215), 0.054, seg(3, d, 1)))
        # foot (ankle pivot)
        k.add(Ft, P + 'graphite', ellipsoid((x, -0.03, 0.10), (0.052, 0.055, 0.04), seg(14, d, 8), seg(8, d, 5)))
        k.add(Ft, P + 'tread', rbox((x, -0.051, 0.0175), (0.185, 0.284, 0.035), 0.014, 1))
        k.add(Ft, P + 'brass', rbox((x, 0.03, 0.062), (0.162, 0.115, 0.07), 0.024, seg(2, d, 1)))
        # one moulded shoe shell (toe + instep) instead of two intersecting blocks
        k.add(Ft, ceramic, superquadric((x, -0.072, 0.072), (0.088, 0.128, 0.058), 0.45, 0.55,
                                        seg(20, d, 10), seg(10, d, 6)))
        k.add(Ft, ceramic, superquadric((x, -0.02, 0.105), (0.072, 0.06, 0.045), 0.6, 0.7,
                                        seg(16, d, 8), seg(8, d, 5)))
        if hi:
            k.add(Ft, P + 'graphite', rbox((x, -0.14, 0.118), (0.008, 0.07, 0.006), 0.002, 1))
            for i in range(3):   # tread lugs: visible from low/rear camera, grounded contact
                k.add(Ft, P + 'tread', rbox((x, -0.15 + i * 0.1, 0.003), (0.17, 0.05, 0.006), 0.002, 1))
        k.anchor(f'anchor_sole_{side}', Ft, (x, -0.051, 0.0), kind='contact', note='sole centre; ground contact')

    k.anchor('anchor_seat_contact', 'robot', (0, 0.0, 0.60), kind='contact',
             note='seated pelvis/thigh underside at rest-pose height; align with a chair anchor_seat '
                  'after applying poses.seated (root drops by 0.30 m)')
    k.anchor('anchor_chest_reader', 'robot', (0, -0.21, 1.04), kind='attachment', note='chest instrument glass')
    k.finish()
    return k


# ---------------------------------------------------------------------- poses
def _rot(v, th):
    """Rotation about +X in the (y, z) plane. Same sign as runtime rotation.x."""
    y, z = v
    return (y * math.cos(th) - z * math.sin(th), y * math.sin(th) + z * math.cos(th))


def solve_seated(table_reach=0.38, table_top=0.60, seat_drop=0.30):
    """Joint angles (radians, rotation about the node's local X, glTF/three.js
    sign: negative swings forward) for sitting with flat palms on a table whose
    edge is 0.25 m in front of the robot root. Returns dict of node -> angle."""
    # legs: thigh about horizontal so the shin is vertical with the sole on the floor
    hip = (0.0, 0.68)
    knee0 = (KNEE_REL[1], 0.68 + KNEE_REL[2])
    rel = (knee0[0] - hip[0], knee0[1] - hip[1])
    target_knee_z = seat_drop + 0.345 - 0.68   # knee height so a vertical shin reaches floor
    lo, hi = -math.pi / 2, 0.0
    for _ in range(60):
        mid = (lo + hi) / 2
        if _rot(rel, mid)[1] > target_knee_z:
            lo = mid
        else:
            hi = mid
    leg = (lo + hi) / 2
    shin = -leg
    # arms: 2-link IK in the sagittal plane, hand global angle -pi/2 (palm down, fingers forward)
    S = (0.0, 1.07)
    E0 = (ELBOW_REL[1], 1.07 + ELBOW_REL[2])
    W0 = (WRIST_REL[1], 1.07 + WRIST_REL[2])
    U0 = (-0.068, 0.583)                     # palm surface centre (body side) at rest
    u_rel = (U0[0] - W0[0], U0[1] - W0[1])
    u_rot = _rot(u_rel, -math.pi / 2)
    U = (-table_reach, table_top + seat_drop)
    Wt = (U[0] - u_rot[0], U[1] - u_rot[1] + 0.004)
    l1 = math.dist(S, E0)
    l2 = math.dist(E0, W0)
    dx, dz = Wt[0] - S[0], Wt[1] - S[1]
    dist = math.hypot(dx, dz)
    cos_e = max(-1, min(1, (l1 * l1 + l2 * l2 - dist * dist) / (2 * l1 * l2)))
    elbow_int = math.acos(cos_e)
    base = math.atan2(dz, dx)
    cos_a = max(-1, min(1, (l1 * l1 + dist * dist - l2 * l2) / (2 * l1 * dist)))
    # elbow below the shoulder-wrist line (bent backwards/down, natural)
    a_dir = base + math.acos(cos_a)
    E = (S[0] + l1 * math.cos(a_dir), S[1] + l1 * math.sin(a_dir))
    rest_upper = math.atan2(E0[1] - S[1], E0[0] - S[0])
    arm = a_dir - rest_upper
    f_dir = math.atan2(Wt[1] - E[1], Wt[0] - E[0])
    rest_fore = math.atan2(W0[1] - E0[1], W0[0] - E0[0])
    fore = (f_dir - rest_fore) - arm
    hand = -math.pi / 2 - arm - fore
    # Blender-space angle = atan2 in (y,z); our y axis is Blender Y (front = -Y). A rotation
    # about +X by th maps (y,z) as _rot. atan2 differences above are in the same plane/sign.
    wrap = lambda a: (a + math.pi) % TAU - math.pi
    arm, fore, hand = wrap(arm), wrap(fore), wrap(hand)
    return {
        'robot.y_offset': -seat_drop,
        'Leg.L': leg, 'Leg.R': leg, 'Shin.L': shin, 'Shin.R': shin, 'Foot.L': 0.0, 'Foot.R': 0.0,
        'Arm.L': arm, 'Arm.R': arm, 'Forearm.L': fore, 'Forearm.R': fore, 'Hand.L': hand, 'Hand.R': hand,
        '_check': {'elbow_interior_deg': round(math.degrees(elbow_int), 1), 'reach_m': round(dist, 3),
                   'max_reach_m': round(l1 + l2, 3), 'elbow_yz': [round(E[0], 3), round(E[1] - seat_drop, 3)]},
    }
