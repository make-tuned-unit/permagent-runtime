"""Lane W4 place assets (archive, workshop, observatory, conservatory) and
construction details (bridge ends, water coping, rails, column bases, drains).
Every builder takes `detail` (1.0 = LOD0, 0.45 = LOD1) and returns a Kit.
LOD1 drops fasteners and small props and merges repeated parts, keeping the
silhouette and every contact surface.
"""
import math
import random
import bmesh
from mathutils import Vector, Matrix
import detail_lib as L
import botany_lib as B
from places_lib import PM, wood, stone, stone_bm, ceramic, pal, bolt, slot_quad, xf, tilt_matrix

TAU = math.tau
BOOKS = ['book_oxblood', 'book_navy', 'book_moss', 'book_ochre', 'book_slate', 'book_umber', 'book_teal', 'book_cream']


def _anchor(k, name, loc, kind, note='', yaw=0.0, normal=None, size=None):
    a = k.anchor(name, k.root_name, loc, (0, 0, yaw), kind=kind, note=note)
    if normal is not None:
        n = Vector(normal).normalized()
        a['normal'] = [round(n.x, 4), round(n.z, 4), round(-n.y, 4)]   # glTF
    if size is not None:
        a['size_w_m'], a['size_h_m'] = size
    return a


def _standard_anchors(k, approach=1.2, interact=0.6, note=''):
    # interact = half collision depth + 0.48 m: the runtime nav keeps body centres 0.47 m off every
    # solid (habitatWalkable margin), so a closer standing point would be unreachable.
    _anchor(k, 'anchor_approach', (0, -approach, 0), 'approach',
            'floor point where a walker stops before engaging; asset front faces glTF +Z ' + note, yaw=0)
    _anchor(k, 'anchor_interact', (0, -interact, 0), 'interact', 'feet of the agent/user while interacting, facing the asset')


# ============================================================ GARDEN ARCHIVE
def build_bookcase(detail):
    M = PM(); k = M.kit('archive_bookcase', detail); r = k.root_name
    hi = detail > 0.7
    W, D, H = 2.6, 0.54, 2.8
    wood(k, M, r, (0, 0.03, 0.05), (W - 0.06, D - 0.08, 0.10), 0.004)                     # recessed plinth
    for sx in (-1, 1):
        x = sx * (W / 2 - 0.025)
        wood(k, M, r, (x, 0, 1.41), (0.05, D, 2.62), 0.004, grain='z')                      # side panel
        if hi:   # frame-and-panel on the outer face (reads from the side and the return path)
            for (zc, h) in ((0.16, 0.06), (1.41, 0.06), (2.66, 0.06)):
                wood(k, M, r, (x + sx * 0.031, 0, zc), (0.012, D - 0.04, h), 0.003)
            for yy in (-D / 2 + 0.04, D / 2 - 0.04):
                wood(k, M, r, (x + sx * 0.031, yy, 1.41), (0.012, 0.06, 2.5), 0.003, grain='z')
    wood(k, M, r, (0, 0, 1.41), (0.04, D - 0.04, 2.56), 0.003, grain='z')                    # centre divider
    wood(k, M, r, (0, -0.005, 2.735), (W + 0.06, D + 0.06, 0.05), 0.008)                      # cornice
    wood(k, M, r, (0, -0.01, 2.78), (W + 0.12, D + 0.10, 0.04), 0.012)
    if hi:
        wood(k, M, r, (0, -0.01, 2.70), (W + 0.02, D + 0.02, 0.025), 0.01)                   # cove step
    wood(k, M, r, (0, D / 2 - 0.012, 1.41), (W - 0.06, 0.02, 2.6), 0.002, grain='z')         # closed back
    for xx in ((-1.27, 0.0, 1.27) if hi else (-1.27, 1.27)):                                  # rear frame (reverse view)
        wood(k, M, r, (xx, D / 2 + 0.006, 1.41), (0.07, 0.016, 2.62), 0.003, grain='z')
    for zz in ((0.14, 1.40, 2.66) if hi else (0.14, 2.66)):
        wood(k, M, r, (0, D / 2 + 0.006, zz), (W - 0.04, 0.016, 0.07), 0.003)
    levels = [0.13, 0.61, 1.09, 1.57, 2.05]
    rng = random.Random(4101)
    for z in levels + [2.62]:
        wood(k, M, r, (0, 0, z), (W - 0.1, D - 0.06, 0.03), 0.003)
        if z < 2.6:
            wood(k, M, r, (0, -D / 2 + 0.035, z - 0.004), (W - 0.1, 0.022, 0.045), 0.004)    # lipping
            for bx in (-0.64, 0.64):
                pal(k, r, 'brass', L.rbox((bx, -D / 2 + 0.022, z - 0.004), (0.07, 0.004, 0.028), 0.002, 1))  # blank label holder
                if hi:
                    for px in (bx - 0.6, bx + 0.6):
                        pal(k, r, 'brass', L.tube((px, -0.15, z - 0.03), (px, -0.15, z - 0.015), 0.004, 6))
    for li, z in enumerate(levels):
        clear = 0.44 if li < 4 else 0.52
        for bay, (x0, x1) in enumerate(((-1.24, -0.04), (0.04, 1.24))):
            if not hi:   # LOD1: one merged block per bay with a few spine colour bands
                x = x0 + 0.04
                while x < x1 - 0.1:
                    w = min(rng.uniform(0.18, 0.34), x1 - 0.04 - x)
                    h = rng.uniform(0.24, 0.31)
                    pal(k, r, rng.choice(BOOKS), L.rbox((x + w / 2, -0.02, z + 0.015 + h / 2), (w, 0.22, h), 0, 0))
                    x += w + rng.uniform(0.0, 0.06)
                continue
            x = x0 + rng.uniform(0.02, 0.08)
            run = 0
            while x < x1 - 0.05:
                kind = rng.random()
                if kind < 0.07 and x < x1 - 0.32:      # lying stack
                    for s in range(3):
                        pal(k, r, rng.choice(BOOKS), L.rbox((x + 0.13, -0.03, z + 0.015 + 0.025 + s * 0.045),
                                                            (0.24 - s * 0.02, 0.19, 0.042), 0.003, 1))
                    x += 0.30
                elif kind < 0.095 and x < x1 - 0.2:    # linen archival box with brass pull
                    pal(k, r, 'linen', L.rbox((x + 0.09, -0.01, z + 0.015 + 0.13), (0.17, 0.26, 0.26), 0.006, 1))
                    pal(k, r, 'brass', L.tube((x + 0.09, -0.141, z + 0.16), (x + 0.09, -0.152, z + 0.16), 0.012, 8))
                    x += 0.2
                elif kind < 0.16:                      # gap
                    x += rng.uniform(0.04, 0.12)
                    run = 0
                else:
                    w, h, d = rng.uniform(0.025, 0.058), rng.uniform(0.21, min(0.33, clear - 0.04)), rng.uniform(0.17, 0.23)
                    lean = 0.0
                    if run > 4 and rng.random() < 0.12:
                        lean = rng.uniform(0.12, 0.22)
                    col = rng.choice(BOOKS)
                    cz = z + 0.015 + h / 2
                    bm = L.rbox((0, 0, 0), (w, d, h), 0.002, 1)
                    xf(bm, Matrix.Translation(Vector((x + w / 2 + math.sin(lean) * h / 2, -0.03 + (0.23 - d) / 2 * -1, cz)))
                       @ Matrix.Rotation(-lean, 4, 'Y'))
                    pal(k, r, col, bm)
                    if rng.random() < 0.35:   # spine band (authored mark, no lettering)
                        pal(k, r, 'brass', L.rbox((x + w / 2, -0.03 - d / 2 - 0.001 + (0.23 - d) / 2 * -1, cz + h * 0.32),
                                                  (w * 0.9, 0.002, 0.008), 0, 0))
                    x += w + (math.sin(lean) * h if lean else 0.002)
                    run += 1
    _standard_anchors(k, 1.3, 0.77)
    for li, z in enumerate(levels):
        for bay, bx in enumerate((-0.64, 0.64)):
            _anchor(k, f'anchor_browse_{bay}_{li}', (bx, -0.34, z + 0.18), 'reach', 'hand reach point in front of a shelf bay')
    _anchor(k, 'anchor_record_slot', (0.64, -0.12, 1.09 + 0.015), 'slot', 'where a highlighted record box/folio sits')
    _anchor(k, 'anchor_gaze_shelves', (0, -0.3, 1.45), 'gaze', 'joint-attention point for the archive conversation')
    return k


def build_folio_stand(detail):
    M = PM(); k = M.kit('archive_folio_stand', detail); r = k.root_name
    hi = detail > 0.7
    for a in (math.radians(45), math.radians(-45)):                                            # crossed feet
        bm = L.rbox((0, 0, 0.035), (0.62, 0.075, 0.07), 0.012, 2 if hi else 1, (0, 0, a))
        L.box_uv(bm, 0.9, 'x'); k.add(r, M.walnut, bm)
        for s in (-1, 1):
            p = (s * 0.29 * math.cos(a), s * 0.29 * math.sin(a), 0.012)
            pal(k, r, 'brass', L.rbox(p, (0.06, 0.08, 0.024), 0.006, 1, (0, 0, a)))
    col = [(0, 0.06), (0.075, 0.06), (0.075, 0.10), (0.055, 0.14), (0.048, 0.40), (0.058, 0.44), (0.058, 0.48),
           (0.045, 0.52), (0.042, 0.86), (0.06, 0.92), (0.06, 0.97), (0, 0.97)]
    bm = L.lathe(col, L.seg(16, detail, 6)); L.box_uv(bm, 0.9, 'z'); k.add(r, M.walnut, bm)
    wood(k, M, r, (0, 0.02, 0.995), (0.30, 0.24, 0.05), 0.006)                                  # head block
    tilt = math.radians(20)
    T = tilt_matrix((0, 0.0, 1.085), tilt)
    desk = L.rbox((0, 0, 0), (0.66, 0.48, 0.032), 0.006, 2 if hi else 1); L.box_uv(desk, 0.9, 'x')
    k.add(r, M.walnut, xf(desk, T))
    lip = L.rbox((0, -0.235, 0.03), (0.66, 0.025, 0.045), 0.006, 1); L.box_uv(lip, 0.9, 'x')
    k.add(r, M.walnut, xf(lip, T))
    for sx in (-0.11, 0.11):                                                                    # braces (reverse view)
        br = L.rbox((0, 0, 0), (0.035, 0.035, 0.16), 0.004, 1, (math.radians(-35), 0, 0)); L.box_uv(br, 0.9, 'z')
        k.add(r, M.walnut, xf(br, Matrix.Translation((sx, 0.08, 1.03))))
    ap = L.rbox((0, 0.215, -0.04), (0.56, 0.025, 0.06), 0.004, 1); L.box_uv(ap, 0.9, 'x')
    k.add(r, M.walnut, xf(ap, T))                                                                # rear apron under the desk
    # the open folio: leather cover, two page blocks with a gentle V, ribbon, brass corners
    for s in (-1, 1):
        cov = L.rbox((s * 0.142, 0.0, 0.019), (0.285, 0.39, 0.006), 0.003, 1, (0, s * math.radians(-3), 0))
        pal(k, r, 'leather', xf(cov, T))
        pg = L.rbox((s * 0.135, 0.0, 0.032), (0.255, 0.355, 0.018), 0.004, 1, (0, s * math.radians(-4), 0))
        pal(k, r, 'paper', xf(pg, T))
        if hi:
            for yy in (-0.19, 0.19):
                pal(k, r, 'brass', xf(L.rbox((s * 0.275, yy, 0.02), (0.02, 0.02, 0.008), 0.002, 1), T))
            for li in range(5):   # authored ruled lines (no lettering)
                pal(k, r, 'paper_edge', xf(L.rbox((s * 0.135, -0.11 + li * 0.05, 0.0418), (0.20, 0.003, 0.0008), 0, 0), T))
    rib = L.rbox((0.01, -0.23, 0.0), (0.012, 0.004, 0.13), 0, 0)
    pal(k, r, 'book_oxblood', xf(rib, T @ Matrix.Translation((0, 0.0, -0.035))))
    _standard_anchors(k, 1.3, 0.78)
    page = T @ Vector((0, 0, 0.042))
    n = (Matrix.Rotation(tilt, 3, 'X') @ Vector((0, 0, 1)))
    _anchor(k, 'anchor_folio_page', tuple(page), 'panel_slot', 'centre of the open pages; normal = page normal',
            normal=n, size=(0.51, 0.355))
    _anchor(k, 'anchor_panel_slot', (0, -0.05, 1.48), 'panel_slot', 'where an evidence/memory panel may float above the folio',
            normal=(0, -1, 0.25), size=(0.6, 0.38))
    for s, nm in ((-1, 'L'), (1, 'R')):
        _anchor(k, f'anchor_hand_{nm}', tuple(T @ Vector((s * 0.2, -0.12, 0.045))), 'contact', 'palm on page edge')
    return k


def build_bench(detail, length=3.2):
    tag = 'long' if length > 2 else 'short'
    M = PM(); k = M.kit(f'bench_{tag}', detail); r = k.root_name
    hi = detail > 0.7
    supports = [-length / 2 + 0.18, length / 2 - 0.18] + ([0.0] if length > 2 else [])
    for x in supports:
        stone(k, M, r, (x, 0.0, 0.025), (0.17, 0.58, 0.05), 0.01)                                 # foot plinth
        stone(k, M, r, (x, 0.0, 0.235), (0.12, 0.50, 0.37), 0.018)                                # support
        pal(k, r, 'bronze', L.rbox((x, 0.0, 0.415), (0.13, 0.46, 0.012), 0.003, 1))              # bearer plate
        # back standard: bronze bar rising from the rear of the support, raked 12 deg
        st = L.rbox((0, 0, 0), (0.035, 0.022, 0.50), 0.006, 1, (math.radians(-12), 0, 0))
        pal(k, r, 'bronze', xf(st, Matrix.Translation((x, 0.255, 0.66))))
        pal(k, r, 'bronze', L.rbox((x, 0.225, 0.43), (0.035, 0.08, 0.03), 0.005, 1))
    n_sl = 5 if hi else 3
    span = 0.52
    w = (span - 0.012 * (n_sl - 1)) / n_sl
    for i in range(n_sl):
        y = -span / 2 + w / 2 + i * (w + 0.012)
        wood(k, M, r, (0, y, 0.44), (length, w, 0.035), 0.006, 'x')
        for x in supports:
            for dx in (-0.03, 0.03):
                bolt(k, r, (x + dx, y, 0.4575), r=0.007, h=0.004, cell='brass')
    for j, (z, yy) in enumerate(((0.62, 0.288), (0.73, 0.312), (0.84, 0.336))):
        if not hi and j == 1:
            continue
        wood(k, M, r, (0, yy, z), (length - 0.06, 0.03, 0.085 if hi else 0.13), 0.006, 'x',
             rot=(math.radians(-12), 0, 0))
    if hi:   # rear rail ties the standards (finished from behind)
        wood(k, M, r, (0, 0.27, 0.53), (length - 0.1, 0.022, 0.04), 0.004, 'x')
    seats = [-length / 2 + 0.5, 0.0, length / 2 - 0.5] if length > 2 else [-0.38, 0.38]
    for i, x in enumerate(seats):
        _anchor(k, f'anchor_seat_{i}', (x, 0.02, 0.4575), 'seat', 'seat surface for a sitter facing glTF +Z')
        _anchor(k, f'anchor_stand_{i}', (x, -0.62, 0), 'approach', 'sit/rise point in front of the seat')
    _anchor(k, 'anchor_approach', (0, -1.1, 0), 'approach', 'floor point in front of the bench')
    return k


# ============================================================ MAKER WORKSHOP
def build_workbench(detail):
    M = PM(); k = M.kit('maker_workbench', detail); r = k.root_name
    hi = detail > 0.7
    W, D, H = 1.9, 0.74, 0.9
    wood(k, M, r, (0, 0, H - 0.035), (W - 0.12, D, 0.07), 0.006)                               # laminated top
    for s in (-1, 1):
        wood(k, M, r, (s * (W / 2 - 0.03), 0, H - 0.035), (0.06, D + 0.02, 0.075), 0.008, 'y')    # breadboard ends
        for yy in (-0.24, 0.24):
            if hi:
                pal(k, r, 'oak', L.tube((s * (W / 2 - 0.03), yy, H + 0.001), (s * (W / 2 - 0.03), yy, H - 0.004), 0.009, 8))
    for sx in (-1, 1):
        for sy in (-1, 1):
            wood(k, M, r, (sx * 0.78, sy * 0.28, (H - 0.07) / 2), (0.08, 0.08, H - 0.07), 0.006, 'z')
        wood(k, M, r, (sx * 0.78, 0, 0.16), (0.07, 0.64, 0.07), 0.006, 'y')                    # side stretcher
        wood(k, M, r, (sx * 0.78, 0, H - 0.12), (0.06, 0.60, 0.06), 0.005, 'y')                # top rail
    for sy in (-1, 1):
        wood(k, M, r, (0, sy * 0.28, 0.16), (1.66, 0.05, 0.07), 0.006, 'x')                     # long stretchers
        for sx in (-1, 1):   # through tenons + wedges proud of the legs
            wood(k, M, r, (sx * 0.845, sy * 0.28, 0.16), (0.05, 0.036, 0.05), 0.004, 'x')
            if hi:
                pal(k, r, 'oak', L.rbox((sx * 0.872, sy * 0.28, 0.16), (0.006, 0.01, 0.055), 0.001, 1))
    for i in range(6 if hi else 2):                                                             # lower shelf slats
        n = 6 if hi else 2
        wood(k, M, r, (0, -0.25 + i * 0.5 / (n - 1), 0.205), (1.5, 0.5 / n - 0.012, 0.02), 0.003)
    # crates on the shelf
    for cx, cw in ((-0.42, 0.42), (0.18, 0.36)):
        if hi:
            for sx in (-1, 1):
                wood(k, M, r, (cx + sx * (cw / 2 - 0.01), 0, 0.30), (0.02, 0.34, 0.17), 0.002, 'y')
            for sy in (-1, 1):
                for zz in (0.25, 0.34):
                    wood(k, M, r, (cx, sy * 0.16, zz), (cw, 0.02, 0.06), 0.002)
        else:
            wood(k, M, r, (cx, 0, 0.30), (cw, 0.34, 0.17), 0.002)
    # front vise: chop, screw, handle
    wood(k, M, r, (-0.62, -D / 2 - 0.035, H - 0.1), (0.36, 0.06, 0.2), 0.008)
    pal(k, r, 'steel', L.tube((-0.62, -D / 2 - 0.07, H - 0.11), (-0.62, -D / 2 + 0.12, H - 0.11), 0.016, L.seg(10, detail, 6)))
    pal(k, r, 'brass', L.tube((-0.78, -D / 2 - 0.09, H - 0.11), (-0.46, -D / 2 - 0.09, H - 0.11), 0.011, L.seg(8, detail, 5)))
    if hi:
        for x in (-0.79, -0.45):
            pal(k, r, 'brass', L.ellipsoid((x, -D / 2 - 0.09, H - 0.11), (0.018, 0.018, 0.018), 8, 4))
        for i in range(6):   # bench-dog holes as dark inlays
            pal(k, r, 'channel', L.rbox((-0.2 + i * 0.16, -D / 2 + 0.08, H + 0.0005), (0.02, 0.02, 0.001), 0, 0))
    # tool rail backboard on the rear edge, finished on both faces
    wood(k, M, r, (0, D / 2 - 0.02, H + 0.17), (1.7, 0.03, 0.34), 0.006)
    if hi:
        for zz in (H + 0.015, H + 0.325):
            wood(k, M, r, (0, D / 2 + 0.002, zz), (1.7, 0.012, 0.03), 0.003)
    pal(k, r, 'brass', L.tube((-0.8, D / 2 - 0.05, H + 0.27), (0.8, D / 2 - 0.05, H + 0.27), 0.008, L.seg(8, detail, 4)))
    tools = [(-0.55, 'square'), (-0.3, 'caliper'), (0.0, 'driver'), (0.12, 'driver'), (0.24, 'driver'), (0.5, 'mallet')]
    for x, kind in tools:
        y = D / 2 - 0.065
        if kind == 'driver':
            pal(k, r, 'steel', L.tube((x, y, H + 0.27), (x, y, H + 0.12), 0.004, 6))
            pal(k, r, 'oak' if x < 0.1 else 'book_oxblood', L.tube((x, y, H + 0.30), (x, y, H + 0.22), 0.012, L.seg(8, detail, 5)))
        elif kind == 'square':
            pal(k, r, 'steel', L.rbox((x, y, H + 0.17), (0.012, 0.004, 0.2), 0, 0))
            pal(k, r, 'brass', L.rbox((x + 0.07, y, H + 0.08), (0.15, 0.012, 0.02), 0.002, 1))
        elif kind == 'caliper':
            pal(k, r, 'brass', L.rbox((x, y, H + 0.18), (0.012, 0.004, 0.18), 0, 0))
            for zz in (H + 0.10, H + 0.14):
                pal(k, r, 'brass', L.rbox((x + 0.03, y, zz), (0.06, 0.004, 0.01), 0, 0))
        else:
            pal(k, r, 'oak', L.tube((x, y, H + 0.27), (x, y, H + 0.10), 0.01, 6))
            wood(k, M, r, (x, y, H + 0.09), (0.12, 0.06, 0.06), 0.008)
    _standard_anchors(k, 1.4, 0.88)
    _anchor(k, 'anchor_work_surface', (0, -0.05, H), 'contact', 'work surface centre (folio / task object rests here)')
    _anchor(k, 'anchor_tool_rail', (-0.3, D / 2 - 0.08, H + 0.22), 'reach', 'hand reach to the tool rail')
    _anchor(k, 'anchor_vise', (-0.62, -D / 2 - 0.09, H - 0.11), 'reach', 'vise handle')
    for s, nm in ((-1, 'L'), (1, 'R')):
        _anchor(k, f'anchor_hand_{nm}', (s * 0.22, -0.18, H), 'contact', 'palm on the bench top')
    return k


def build_result_shelf(detail):
    M = PM(); k = M.kit('maker_result_shelf', detail); r = k.root_name
    hi = detail > 0.7
    W, D, H = 1.4, 0.42, 1.6
    for s in (-1, 1):
        wood(k, M, r, (s * (W / 2 - 0.02), -D / 2 + 0.025, H / 2), (0.04, 0.05, H), 0.006, 'z')
        wood(k, M, r, (s * (W / 2 - 0.02), D / 2 - 0.025, H / 2), (0.04, 0.05, H), 0.006, 'z')
        for zz in (0.06, 0.6, 1.1, 1.56):
            wood(k, M, r, (s * (W / 2 - 0.02), 0, zz), (0.035, D - 0.06, 0.035), 0.004, 'y')
    shelves = [0.08, 0.55, 1.05]
    for z in shelves + [1.55]:
        wood(k, M, r, (0, 0, z), (W - 0.08, D - 0.02, 0.03), 0.004)
        if z < 1.5:
            pal(k, r, 'brass', L.rbox((0, -D / 2 + 0.006, z + 0.03), (W - 0.12, 0.004, 0.018), 0.001, 1))   # blank label rail
    for zz in ((0.3, 0.8, 1.3) if hi else (0.8,)):                                               # rear slats
        wood(k, M, r, (0, D / 2 - 0.01, zz), (W - 0.08, 0.018, 0.09), 0.003)
    rng = random.Random(4202)
    slot = 0
    for si, z in enumerate(shelves):
        for tx in (-0.32, 0.32):
            slot += 1
            top = z + 0.015
            if hi:
                tray = L.rbox((tx, 0, top + 0.02), (0.54, 0.32, 0.04), 0.012, 2)
                ceramic(k, M, r, tray)
                ceramic(k, M, r, L.rbox((tx, 0, top + 0.036), (0.50, 0.28, 0.012), 0.004, 1))
            else:
                ceramic(k, M, r, L.rbox((tx, 0, top + 0.02), (0.54, 0.32, 0.04), 0.01, 1))
            item = (si * 2 + (0 if tx < 0 else 1)) % 4
            ty = top + 0.04
            if item == 0 and hi:      # finished ceramic module
                ceramic(k, M, r, L.rbox((tx, 0, ty + 0.06), (0.16, 0.12, 0.12), 0.02, 2))
                pal(k, r, 'brass', L.rbox((tx, -0.062, ty + 0.06), (0.1, 0.004, 0.02), 0.002, 1))
            elif item == 1 and hi:    # rolled drawing tube
                pal(k, r, 'linen', L.tube((tx - 0.18, 0, ty + 0.03), (tx + 0.18, 0, ty + 0.03), 0.028, 10))
                for ex in (-0.185, 0.185):
                    pal(k, r, 'brass', L.tube((tx + ex - 0.01, 0, ty + 0.03), (tx + ex + 0.01, 0, ty + 0.03), 0.031, 10))
            elif item == 2 and hi:    # brass gear
                teeth = 12
                out = []
                for i in range(teeth * 2):
                    a = TAU * i / (teeth * 2)
                    rr = 0.07 if i % 2 == 0 else 0.058
                    out.append((tx + rr * math.cos(a), rr * math.sin(a)))
                pal(k, r, 'brass', L.prism(out, ty, ty + 0.018))
                pal(k, r, 'steel', L.tube((tx, 0, ty), (tx, 0, ty + 0.03), 0.012, 8))
    _standard_anchors(k, 1.25, 0.70)
    slot = 0
    for si, z in enumerate(shelves):
        for tx in (-0.32, 0.32):
            slot += 1
            _anchor(k, f'anchor_result_slot_{slot}', (tx, 0, z + 0.015 + 0.042), 'slot',
                    'tray surface for one verified result (bottom shelf = oldest)')
    return k


def build_progress_stand(detail):
    M = PM(); k = M.kit('maker_progress_stand', detail); r = k.root_name
    hi = detail > 0.7
    hub = Vector((0, 0, 0.62))
    for i in range(3):
        a = TAU * i / 3 + math.pi / 2
        foot = Vector((0.36 * math.cos(a), 0.36 * math.sin(a), 0.02))
        bm = L.tube(hub, foot, 0.022, L.seg(8, detail, 5)); L.box_uv(bm, 0.9, 'z'); k.add(r, M.walnut, bm)
        pal(k, r, 'brass', L.ellipsoid(tuple(foot + Vector((0, 0, -0.005))), (0.03, 0.03, 0.02), 8, 4))
    pal(k, r, 'brass', L.lathe([(0, 0.58), (0.045, 0.58), (0.05, 0.62), (0.045, 0.68), (0, 0.68)], L.seg(14, detail, 6)))
    pal(k, r, 'steel', L.tube((0, 0, 0.66), (0, 0, 1.1), 0.018, L.seg(10, detail, 6)))
    pal(k, r, 'brass', L.lathe([(0, 1.06), (0.032, 1.06), (0.034, 1.12), (0, 1.12)], L.seg(12, detail, 6)))
    if hi:
        pal(k, r, 'graphite', L.tube((0.03, 0, 1.09), (0.09, 0, 1.09), 0.006, 6))
        pal(k, r, 'graphite', L.ellipsoid((0.1, 0, 1.09), (0.018, 0.018, 0.018), 8, 4))
    tilt = math.radians(-15)
    T = tilt_matrix((0, 0, 1.33), tilt)
    pal(k, r, 'brass', L.rbox((0, 0.04, 1.14), (0.1, 0.04, 0.06), 0.008, 1))                    # yoke
    for s in (-1, 1):
        pal(k, r, 'brass', xf(L.rbox((s * 0.33, 0.03, -0.12), (0.02, 0.02, 0.1), 0.003, 1), T))
    frame = [((0, 0, 0.22), (0.62, 0.035, 0.03)), ((0, 0, -0.22), (0.62, 0.035, 0.03)),
             ((-0.3, 0, 0), (0.03, 0.035, 0.46)), ((0.3, 0, 0), (0.03, 0.035, 0.46))]
    for c, sz in frame:
        bm = L.rbox(c, sz, 0.004, 1); L.box_uv(bm, 0.9, 'x'); k.add(r, M.walnut, xf(bm, T))
    ceramic(k, M, r, xf(L.rbox((0, 0.008, 0), (0.57, 0.012, 0.41), 0.003, 1), T))                 # ivory panel inset
    # in-progress assembly on the board: half-built module, exposed gear, loose fasteners
    ceramic(k, M, r, xf(L.rbox((-0.1, -0.05, 0.02), (0.18, 0.08, 0.14), 0.015, 2 if hi else 1), T))
    if hi:
        ceramic(k, M, r, xf(L.shell_patch((0, 0, 0), (0.09, 0.05, 0.07), 0.2, 1.4, 0.3, 2.6, 4, 6, 0.008), T @ Matrix.Translation((0.13, -0.06, 0.02))))
        out = [(0.02 * math.cos(TAU * i / 20) * (1.0 if i % 2 else 0.8) / 0.02 * 0.045,
                0.02 * math.sin(TAU * i / 20) * (1.0 if i % 2 else 0.8) / 0.02 * 0.045) for i in range(20)]
        g = L.prism(out, 0, 0.012)
        pal(k, r, 'brass', xf(g, T @ Matrix.Translation((0.05, -0.02, 0.12)) @ Matrix.Rotation(math.pi / 2, 4, 'X')))
        for i in range(4):
            bolt(k, r, tuple(T @ Vector((-0.2 + i * 0.03, -0.022, -0.16))), (0, -1, 0.3), 0.007, 0.01, 'steel')
        # side arm with parts tray
        pal(k, r, 'steel', L.tube((0, 0, 0.92), (0.26, 0, 0.92), 0.008, 6))
        pal(k, r, 'brass', L.rbox((0.32, 0, 0.93), (0.14, 0.1, 0.02), 0.004, 1))
    _standard_anchors(k, 1.35, 0.82)
    n = Matrix.Rotation(tilt, 3, 'X') @ Vector((0, -1, 0))
    _anchor(k, 'anchor_work_piece', tuple(T @ Vector((0, -0.06, 0.02))), 'contact', 'the in-progress piece on the board')
    _anchor(k, 'anchor_panel_slot', tuple(T @ Vector((0, -0.01, 0))), 'panel_slot', 'board face (in-progress state)',
            normal=n, size=(0.57, 0.41))
    _anchor(k, 'anchor_parts_tray', (0.32, 0, 0.94), 'slot', 'loose parts tray')
    return k


# ============================================================ OBSERVATORY
def build_forecast_plinth(detail):
    M = PM(); k = M.kit('observatory_forecast_plinth', detail); r = k.root_name
    hi = detail > 0.7
    stone(k, M, r, (0, 0, 0.04), (0.95, 0.62, 0.08), 0.012)
    stone(k, M, r, (0, 0.01, 0.50), (0.78, 0.48, 0.84), 0.03)
    # sloped cap: prism in the YZ plane extruded along X
    prof = [(-0.30, 0.92), (0.30, 0.92), (0.30, 1.13), (-0.30, 0.99)]
    bm = bmesh.new()
    vs0 = [bm.verts.new((-0.44, y, z)) for (y, z) in prof]
    vs1 = [bm.verts.new((0.44, y, z)) for (y, z) in prof]
    bm.faces.new(list(reversed(vs0))); bm.faces.new(vs1)
    for i in range(4):
        j = (i + 1) % 4
        bm.faces.new((vs0[i], vs0[j], vs1[j], vs1[i]))
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    stone_bm(k, M, r, bm)
    slope = math.atan2(0.14, 0.60)
    T = tilt_matrix((0, 0.0, 1.06 + 0.004), slope)
    for c, sz in (((0, -0.235, 0), (0.72, 0.03, 0.022)), ((0, 0.235, 0), (0.72, 0.03, 0.022)),
                  ((-0.345, 0, 0), (0.03, 0.5, 0.022)), ((0.345, 0, 0), (0.03, 0.5, 0.022))):
        pal(k, r, 'brass', xf(L.rbox(c, sz, 0.004, 1), T))                                        # bezel
    slot = slot_quad((0, 0, 0), 0.62, 0.42, 0.0, 0.01)
    xf(slot, T @ Matrix.Rotation(-math.pi / 2, 4, 'X'))                                              # front face up the slope
    k.add(r, M.slot, slot, obj='observatory_forecast_plinth_panel_slot')
    if hi:
        for (x, y) in ((-0.345, -0.235), (0.345, -0.235), (-0.345, 0.235), (0.345, 0.235)):
            bolt(k, r, tuple(T @ Vector((x, y, 0.012))), tuple(Matrix.Rotation(slope, 3, 'X') @ Vector((0, 0, 1))), 0.008, 0.005, 'bronze')
    # armillary on the rear-right corner
    ax, ay, az = 0.33, 0.2, 1.13
    az = 1.10   # cap surface at the armillary corner is ~1.107; the foot sinks 7 mm into it
    post = L.lathe([(0, az), (0.04, az), (0.03, az + 0.03), (0.012, az + 0.03), (0.012, az + 0.16), (0, az + 0.16)], L.seg(10, detail, 6))
    pal(k, r, 'bronze', L.transform(post, loc=(ax, ay, 0)))
    rc = (ax, ay, az + 0.26)
    for rot in ((0, 0, 0), (math.pi / 2, 0, 0), (math.pi / 2, 0, math.pi / 2), (math.radians(66), 0, 0.4)):
        if not hi and rot[2] == 0.4:
            continue
        pal(k, r, 'brass', L.torus(rc, 0.1, 0.005, L.seg(28, detail, 12), 4 if hi else 3, rot))
    pal(k, r, 'brass', L.ellipsoid(rc, (0.018, 0.018, 0.018), 8, 4))
    # fix the armillary over the cap corner
    k.buckets  # noqa
    # rear: recessed access plate with bolts, drain notch
    pal(k, r, 'bronze', L.rbox((0, 0.252, 0.5), (0.36, 0.008, 0.42), 0.004, 1))
    for (x, z) in ((-0.15, 0.32), (0.15, 0.32), (-0.15, 0.68), (0.15, 0.68)):
        bolt(k, r, (x, 0.256, z), (0, 1, 0), 0.01, 0.006)
    pal(k, r, 'channel', L.rbox((0, 0.31, 0.02), (0.12, 0.02, 0.03), 0, 0))
    _standard_anchors(k, 1.4, 0.80)
    n = Matrix.Rotation(slope, 3, 'X') @ Vector((0, 0, 1))
    _anchor(k, 'anchor_panel_slot', tuple(T @ Vector((0, 0, 0.01))), 'panel_slot',
            'forecast panel: mesh "observatory_forecast_plinth_panel_slot" (material "Panel • dark glass slot", UV 0..1)',
            normal=n, size=(0.62, 0.42))
    _anchor(k, 'anchor_armillary', rc, 'gaze', 'small brass armillary (sky reference, not data)')
    return k


def build_telescope(detail):
    M = PM(); k = M.kit('observatory_telescope', detail); r = k.root_name
    hi = detail > 0.7
    S = L.seg(24, detail, 10)
    ped = [(0, 0), (0.86, 0), (0.86, 0.10), (0.78, 0.16), (0.78, 0.23), (0.57, 0.23), (0.57, 0.32), (0.48, 0.36),
           (0.46, 0.60), (0.48, 0.81), (0.57, 0.85), (0.57, 0.91), (0.66, 0.91), (0.66, 1.02), (0, 1.02)]
    bm = L.lathe(ped, S); L.box_uv(bm, 1.0, 'z'); k.add(r, M.stone, bm)
    if hi:
        for i in range(8):
            a = TAU * i / 8
            bolt(k, r, (0.6 * math.cos(a), 0.6 * math.sin(a), 1.02), (0, 0, 1), 0.016, 0.01)
        pal(k, r, 'bronze', L.rbox((0, -0.48, 0.55), (0.24, 0.02, 0.3), 0.004, 1))                # pedestal service plate
        for (x, z) in ((-0.09, 0.43), (0.09, 0.43), (-0.09, 0.67), (0.09, 0.67)):
            bolt(k, r, (x, -0.49, z), (0, -1, 0), 0.01, 0.006)
    az = k.pivot('Telescope_Azimuth', r, (0, 0, 1.02))
    an = 'Telescope_Azimuth'
    pal(k, an, 'brass', L.lathe([(0.34, 1.02), (0.46, 1.02), (0.47, 1.045), (0.36, 1.045)], S))       # lower race
    pal(k, an, 'graphite', L.lathe([(0.37, 1.045), (0.45, 1.045), (0.45, 1.055), (0.37, 1.055)], S))  # bearing gap
    pal(k, an, 'brass', L.lathe([(0, 1.055), (0.46, 1.055), (0.46, 1.09), (0, 1.09)], S))             # turntable
    if hi:
        for i in range(12):
            a = TAU * i / 12 + 0.13
            bolt(k, an, (0.41 * math.cos(a), 0.41 * math.sin(a), 1.09), (0, 0, 1), 0.012, 0.008, 'brass')
    # fork arms: tapered prisms in the (Y,Z) plane, extruded along X
    PIV = 1.78
    for s in (-1, 1):
        prof = [(-0.17, 1.09), (0.17, 1.09), (0.10, PIV + 0.02), (0.07, PIV + 0.1), (-0.07, PIV + 0.1), (-0.10, PIV + 0.02)]
        bm = bmesh.new()
        x0, x1 = s * 0.27, s * 0.38
        a0 = [bm.verts.new((x0, y, z)) for (y, z) in prof]
        a1 = [bm.verts.new((x1, y, z)) for (y, z) in prof]
        bm.faces.new(list(reversed(a0))); bm.faces.new(a1)
        for i in range(len(prof)):
            j = (i + 1) % len(prof)
            bm.faces.new((a0[i], a0[j], a1[j], a1[i]))
        bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
        if hi:
            bmesh.ops.bevel(bm, geom=list(bm.edges), offset=0.012, segments=1, affect='EDGES', clamp_overlap=True)
        pal(k, an, 'brass', bm)
        # trunnion housing + cap + bolts
        hx = s * 0.39
        pal(k, an, 'brass', L.tube((hx, 0, PIV), (hx + s * 0.06, 0, PIV), 0.11, L.seg(20, detail, 10)))
        pal(k, an, 'bronze', L.tube((hx + s * 0.06, 0, PIV), (hx + s * 0.075, 0, PIV), 0.075, L.seg(16, detail, 8)))
        if hi:
            for i in range(4):
                a = TAU * i / 4 + math.pi / 4
                bolt(k, an, (hx + s * 0.06, 0.085 * math.cos(a), PIV + 0.085 * math.sin(a)), (s, 0, 0), 0.01, 0.007)
        if hi:   # fork-root gussets
            pal(k, an, 'brass', L.rbox((s * 0.22, 0, 1.13), (0.1, 0.24, 0.08), 0.01, 1))
    pal(k, an, 'brass', L.rbox((0, 0, 1.115), (0.62, 0.30, 0.05), 0.012, 2 if hi else 1))               # yoke plate
    # altitude axis: barrel points toward +Y (away from the observer) at 30 deg elevation
    el = math.radians(30)
    alt = k.pivot('Telescope_Altitude', an, (0, 0, PIV))
    aname = 'Telescope_Altitude'
    R = Matrix.Translation((0, 0, PIV)) @ Matrix.Rotation(-(math.pi / 2 - el), 4, 'X')   # local +Z -> barrel axis
    def along(bm):
        return xf(bm, R)
    pal(k, aname, 'steel', L.tube((-0.39, 0, PIV), (0.39, 0, PIV), 0.045, L.seg(12, detail, 6)))      # trunnion axle
    prof = [(0.14, -1.2), (0.18, -1.08), (0.23, -0.82), (0.235, -0.48), (0.235, 0.08), (0.27, 1.06), (0.27, 1.2)]
    shell = [(0, -1.2)] + prof + [(0.25, 1.2), (0, 1.2)]
    ceramic(k, M, aname, along(L.lathe(shell, L.seg(32, detail, 12))))
    ceramic(k, M, aname, along(L.lathe([(0.27, 1.06), (0.30, 1.12), (0.295, 1.34), (0.275, 1.34), (0.275, 1.12)], L.seg(28, detail, 12))))
    pal(k, aname, 'glass_dark', along(L.lathe([(0, 1.215), (0.245, 1.215), (0.245, 1.225), (0, 1.235)], L.seg(24, detail, 10))))
    pal(k, aname, 'brass', along(L.lathe([(0.272, 1.03), (0.29, 1.045), (0.29, 1.08), (0.272, 1.09)], L.seg(28, detail, 12))))
    for zc in (-0.3, 0.3):                                                                       # tube rings + clamp blocks
        pal(k, aname, 'brass', along(L.lathe([(0.236, zc - 0.035), (0.262, zc - 0.035), (0.262, zc + 0.035), (0.236, zc + 0.035)], L.seg(28, detail, 12))))
        pal(k, aname, 'brass', along(L.rbox((0.0, 0.27, zc), (0.06, 0.05, 0.07), 0.006, 1)))
        if hi:
            bolt(k, aname, tuple(R @ Vector((0.0, 0.296, zc))), tuple(R.to_3x3() @ Vector((0, 1, 0))), 0.009, 0.008, 'steel')
    # rear: focuser, diagonal, eyepiece
    pal(k, aname, 'brass', along(L.lathe([(0, -1.32), (0.06, -1.32), (0.06, -1.2), (0, -1.2)], L.seg(16, detail, 8))))
    if hi:
        for s in (-1, 1):
            pal(k, aname, 'graphite', along(L.tube((s * 0.06, 0, -1.26), (s * 0.11, 0, -1.26), 0.022, 12)))
    eye_base = R @ Vector((0, 0, -1.34))
    pal(k, aname, 'brass', L.tube(tuple(eye_base + Vector((0, 0.02, -0.02))), tuple(eye_base + Vector((0, -0.02, 0.02))), 0.035, L.seg(12, detail, 6)))
    up = Vector((0, -0.25, 1)).normalized()
    pal(k, aname, 'brass', L.tube(tuple(eye_base), tuple(eye_base + up * 0.12), 0.026, L.seg(12, detail, 6)))
    pal(k, aname, 'rubber', L.tube(tuple(eye_base + up * 0.12), tuple(eye_base + up * 0.17), 0.03, L.seg(12, detail, 6), r2=0.034))
    eyepiece = eye_base + up * 0.17
    # finder scope on two small rings
    fo = Vector((-0.2, 0.0, 0.0))
    if hi:
        for zc in (-0.55, -0.2):
            pal(k, aname, 'brass', along(L.tube(tuple(Vector((-0.15, -0.17, zc))), tuple(Vector((-0.2, -0.2, zc))), 0.008, 6)))
        ceramic(k, M, aname, along(L.tube((-0.23, -0.22, -0.7), (-0.23, -0.22, 0.0), 0.032, 12)))
    # slow-motion handle hanging from the rear
    hs = R @ Vector((0.08, 0.0, -0.9))
    pal(k, aname, 'brass', L.tube(tuple(hs), tuple(hs + Vector((0.05, -0.1, -0.32))), 0.007, 6))
    pal(k, aname, 'graphite', L.ellipsoid(tuple(hs + Vector((0.05, -0.1, -0.34))), (0.02, 0.02, 0.03), 8, 4))
    _anchor(k, 'anchor_approach', (0, -1.95, 0), 'approach', 'on the plinth top, behind the eyepiece; asset front faces glTF +Z')
    _anchor(k, 'anchor_observer', (0, -1.55, 0), 'interact', 'observer feet; eyepiece at about 1.35 m (robot stoops / sits on a stool)')
    _anchor(k, 'anchor_eyepiece', tuple(eyepiece), 'gaze', 'eyepiece cup centre', normal=tuple(up))
    _anchor(k, 'anchor_objective', tuple(R @ Vector((0, 0, 1.3))), 'gaze', 'objective end (points north at heading 0)')
    _anchor(k, 'anchor_handle', tuple(hs + Vector((0.05, -0.1, -0.34))), 'reach', 'slow-motion handle knob')
    return k


# ============================================================ CONSERVATORY
def _seedling(k, node, base, scale=1.0, n=5, rng=None):
    pal(k, node, 'leaf', L.tube(base, (base[0], base[1], base[2] + 0.06 * scale), 0.003, 4))
    top = Vector((base[0], base[1], base[2] + 0.06 * scale))
    uv_u, uv_v = k.palette.uv['leaf_light']
    for i in range(n):
        a = TAU * i / n + (rng.uniform(-0.3, 0.3) if rng else 0)
        d = Vector((math.cos(a), math.sin(a), 0.5)).normalized()
        bm = B.leaf(top - Vector((0, 0, 0.02 * scale * (i % 2))), d, (0, 0, 1), 0.05 * scale, 0.02 * scale,
                    (uv_u, uv_v, uv_v), fold=0.3, droop=0.2, shape='broad')
        k.add(node, k.palette.material, bm)


def build_measurement_panel(detail):
    M = PM(); k = M.kit('conservatory_measurement_panel', detail); r = k.root_name
    hi = detail > 0.7
    rng = random.Random(4301)
    for s in (-1, 1):
        x = s * 0.56
        stone(k, M, r, (x, 0, 0.04), (0.22, 0.30, 0.08), 0.01)
        ceramic(k, M, r, L.rbox((x, 0, 0.9), (0.08, 0.08, 1.64), 0.018, 2 if hi else 1))
        pal(k, r, 'brass', L.lathe([(0, 1.72), (0.05, 1.72), (0.045, 1.75), (0, 1.76)], L.seg(12, detail, 6)) if False else
            L.rbox((x, 0, 1.73), (0.095, 0.095, 0.02), 0.006, 1))
    tilt = math.radians(-8)
    T = tilt_matrix((0, 0.0, 1.27), tilt)
    for c, sz in (((0, 0, 0.36), (1.04, 0.05, 0.05)), ((0, 0, -0.36), (1.04, 0.05, 0.05)),
                  ((-0.495, 0, 0), (0.05, 0.05, 0.72)), ((0.495, 0, 0), (0.05, 0.05, 0.72))):
        bm = L.rbox(c, sz, 0.006, 1); L.box_uv(bm, 0.9, 'x'); k.add(r, M.walnut, xf(bm, T))
    slot = slot_quad((0, 0, 0), 0.92, 0.62, 0.0, 0.012)
    k.add(r, M.slot, xf(slot, T), obj='conservatory_measurement_panel_panel_slot')
    # authored before/after bar chart in phosphor (no lettering): baseline, divider, bars
    pal(k, r, 'brass', xf(L.rbox((0, -0.008, 0), (0.006, 0.004, 0.56), 0, 0), T))
    for side, heights in ((-1, (0.12, 0.16, 0.14, 0.18)), (1, (0.20, 0.27, 0.31, 0.36))):
        k.add(r, M.display, xf(L.rbox((side * 0.23, -0.008, -0.24), (0.36, 0.003, 0.005), 0, 0), T))
        for i, h in enumerate(heights):
            x = side * 0.23 + (i - 1.5) * 0.075
            k.add(r, M.display, xf(L.rbox((x, -0.008, -0.24 + h / 2), (0.04, 0.003, h), 0, 0), T))
    if hi:
        for gz in (-0.1, 0.05):
            pal(k, r, 'slate' if False else 'steel', xf(L.rbox((0, -0.0075, gz), (0.86, 0.002, 0.002), 0, 0), T))
    # rear cover with stiffener ribs + conduit to the footing (finished reverse view)
    bm = L.rbox((0, 0.03, 0), (0.98, 0.012, 0.68), 0.003, 1); L.box_uv(bm, 0.9, 'x'); k.add(r, M.walnut, xf(bm, T))
    for zz in ((-0.2, 0.2) if hi else (0.0,)):
        bm = L.rbox((0, 0.045, zz), (0.94, 0.02, 0.04), 0.003, 1); L.box_uv(bm, 0.9, 'x'); k.add(r, M.walnut, xf(bm, T))
    pal(k, r, 'graphite', L.tube((0.5, 0.06, 1.0), (0.53, 0.06, 0.08), 0.012, L.seg(8, detail, 5)))
    # sample shelf with three pots, seedlings and measuring stakes
    wood(k, M, r, (0, -0.06, 0.55), (1.0, 0.28, 0.03), 0.004)
    for s in (-1, 1):
        pal(k, r, 'brass', L.rbox((s * 0.46, -0.06, 0.5), (0.03, 0.2, 0.07), 0.004, 1))
    for i, x in enumerate((-0.3, 0.0, 0.3)):
        pot = L.lathe([(0, 0.565), (0.05, 0.565), (0.065, 0.66), (0.07, 0.67), (0.064, 0.67), (0.059, 0.655), (0, 0.655)], L.seg(14, detail, 7))
        L.transform(pot, loc=(x, -0.06, 0))
        pal(k, r, 'terracotta', pot)
        pal(k, r, 'soil', L.lathe([(0, 0.65), (0.058, 0.65), (0.058, 0.655), (0, 0.66)], L.seg(12, detail, 6)) if False else
            L.transform(L.lathe([(0, 0.0), (0.058, 0.0), (0, 0.006)], L.seg(12, detail, 6)), loc=(x, -0.06, 0.65)))
        if hi:
            _seedling(k, r, (x, -0.06, 0.655), 0.8 + 0.35 * i, 5, rng)
            pal(k, r, 'stake', L.rbox((x + 0.04, -0.06 + 0.03, 0.74), (0.008, 0.004, 0.18), 0, 0))
            for t in range(6):
                pal(k, r, 'graphite', L.rbox((x + 0.04, -0.06 + 0.027, 0.66 + t * 0.03), (0.01, 0.002, 0.002), 0, 0))
    _standard_anchors(k, 1.4, 0.74)
    n = Matrix.Rotation(tilt, 3, 'X') @ Vector((0, -1, 0))
    _anchor(k, 'anchor_panel_slot', tuple(T @ Vector((0, -0.007, 0))), 'panel_slot',
            'measurement panel: mesh "conservatory_measurement_panel_panel_slot" (UV 0..1); left half = before, right = after',
            normal=n, size=(0.92, 0.62))
    for i, x in enumerate((-0.3, 0.0, 0.3)):
        _anchor(k, f'anchor_sample_{i + 1}', (x, -0.06, 0.66), 'slot', 'sample pot soil surface')
    return k


def build_growing_bed(detail):
    M = PM(); k = M.kit('conservatory_growing_bed', detail); r = k.root_name
    hi = detail > 0.7
    rng = random.Random(4302)
    W, D = 2.4, 0.9
    z0, z1 = 0.53, 0.85
    for (c, sz) in (((0, -D / 2 + 0.025, (z0 + z1) / 2), (W, 0.05, z1 - z0)), ((0, D / 2 - 0.025, (z0 + z1) / 2), (W, 0.05, z1 - z0)),
                    ((-W / 2 + 0.025, 0, (z0 + z1) / 2), (0.05, D - 0.1, z1 - z0)), ((W / 2 - 0.025, 0, (z0 + z1) / 2), (0.05, D - 0.1, z1 - z0)),
                    ((0, 0, z0 + 0.02), (W - 0.1, D - 0.1, 0.04))):
        ceramic(k, M, r, L.rbox(c, sz, 0.012, 2 if hi else 1))
    for (c, sz, g) in (((0, -D / 2 + 0.03, z1 + 0.012), (W + 0.02, 0.08, 0.024), 'x'), ((0, D / 2 - 0.03, z1 + 0.012), (W + 0.02, 0.08, 0.024), 'x'),
                       ((-W / 2 + 0.03, 0, z1 + 0.012), (0.08, D - 0.14, 0.024), 'y'), ((W / 2 - 0.03, 0, z1 + 0.012), (0.08, D - 0.14, 0.024), 'y')):
        wood(k, M, r, c, sz, 0.006, g)                                                           # walnut rim cap
    pal(k, r, 'soil', L.rbox((0, 0, z1 - 0.06), (W - 0.1, D - 0.1, 0.02), 0.004, 1))
    for x in (-1.05, 0.0, 1.05):
        for y in (-0.36, 0.36):
            wood(k, M, r, (x, y, z0 / 2), (0.07, 0.07, z0), 0.005, 'z')
    wood(k, M, r, (0, 0, 0.15), (W - 0.2, 0.62, 0.025), 0.004)                                   # lower shelf
    pal(k, r, 'graphite', L.rbox((0.7, 0, 0.18), (0.6, 0.4, 0.03), 0.006, 1))                    # drip tray
    pal(k, r, 'brass', L.tube((W / 2 - 0.02, -0.2, z0 + 0.03), (W / 2 + 0.06, -0.2, z0 - 0.02), 0.012, 8))   # drain spout
    rows = (-0.24, 0.0, 0.24)
    for ri, y in enumerate(rows):
        for i in range(7 if hi else 0):
            x = -0.95 + i * 0.317 + rng.uniform(-0.03, 0.03)
            _seedling(k, r, (x, y, z1 - 0.05), rng.uniform(0.9, 1.5), 5 if ri != 1 else 4, rng)
        if not hi:   # LOD1: a low leafy band per row
            pal(k, r, 'leaf', L.rbox((0, y, z1 - 0.01), (W - 0.3, 0.12, 0.06), 0.02, 1))
        pal(k, r, 'stake', L.rbox((-W / 2 + 0.12, y, z1 + 0.04), (0.012, 0.004, 0.14), 0, 0))  # blank row stake
    # drip irrigation along the back rim with emitters, valve handwheel and supply riser
    yb = D / 2 - 0.09
    pal(k, r, 'brass', L.tube((-W / 2 + 0.1, yb, z1 + 0.04), (W / 2 - 0.1, yb, z1 + 0.04), 0.011, L.seg(10, detail, 5)))
    if hi:
        for ri, y in enumerate(rows):
            for i in range(4):
                x = -0.8 + i * 0.53
                pal(k, r, 'rubber', L.tube((x, yb, z1 + 0.04), (x, y + 0.06, z1 - 0.04), 0.003, 4))
    vx = -W / 2 - 0.02
    pal(k, r, 'brass', L.tube((-W / 2 + 0.1, yb, z1 + 0.04), (vx, yb, z1 + 0.04), 0.011, 8))
    pal(k, r, 'brass', L.tube((vx, yb, z1 + 0.04), (vx, yb, 0.0), 0.011, 8))
    pal(k, r, 'bronze', L.torus((vx - 0.03, yb, z1 + 0.04), 0.045, 0.006, L.seg(16, detail, 8), 4, (0, math.pi / 2, 0)))
    if hi:
        for i in range(3):
            a = TAU * i / 3
            pal(k, r, 'bronze', L.tube((vx - 0.03, yb, z1 + 0.04), (vx - 0.03, yb + 0.045 * math.cos(a), z1 + 0.04 + 0.045 * math.sin(a)), 0.004, 4))
    _standard_anchors(k, 1.5, 0.95)
    _anchor(k, 'anchor_valve', (vx - 0.03, yb, z1 + 0.04), 'reach', 'irrigation valve handwheel')
    for ri, y in enumerate(rows):
        _anchor(k, f'anchor_row_{ri + 1}', (0, y, z1 - 0.05), 'slot', 'soil surface at the centre of a planted row')
    return k


def build_equipment_cart(detail):
    M = PM(); k = M.kit('conservatory_equipment_cart', detail); r = k.root_name
    hi = detail > 0.7
    W, D = 1.0, 0.55
    for sx in (-1, 1):
        for sy in (-1, 1):
            wood(k, M, r, (sx * (W / 2 - 0.03), sy * (D / 2 - 0.03), 0.5), (0.04, 0.04, 0.78), 0.004, 'z')
            pal(k, r, 'brass', L.rbox((sx * (W / 2 - 0.03), sy * (D / 2 - 0.03), 0.1), (0.05, 0.04, 0.02), 0.003, 1))
            wheel = L.tube((sx * (W / 2 - 0.03) - 0.018, sy * (D / 2 - 0.03), 0.05), (sx * (W / 2 - 0.03) + 0.018, sy * (D / 2 - 0.03), 0.05), 0.05, L.seg(14, detail, 8))
            pal(k, r, 'rubber', wheel)
    for z in (0.3, 0.86):
        wood(k, M, r, (0, 0, z), (W, D, 0.03), 0.005)
        for s in (-1, 1):
            wood(k, M, r, (0, s * (D / 2 - 0.01), z + 0.03), (W, 0.02, 0.04), 0.003)
    pal(k, r, 'brass', L.tube((-W / 2 - 0.02, -D / 2 + 0.05, 0.88), (-W / 2 - 0.08, -D / 2 + 0.05, 1.02), 0.012, 8))
    pal(k, r, 'brass', L.tube((-W / 2 - 0.02, D / 2 - 0.05, 0.88), (-W / 2 - 0.08, D / 2 - 0.05, 1.02), 0.012, 8))
    pal(k, r, 'brass', L.tube((-W / 2 - 0.08, -D / 2 + 0.05, 1.02), (-W / 2 - 0.08, D / 2 - 0.05, 1.02), 0.016, L.seg(10, detail, 6)))
    top = 0.875
    # watering can
    can = L.lathe([(0, top), (0.08, top), (0.09, top + 0.03), (0.085, top + 0.18), (0.06, top + 0.2), (0, top + 0.2)], L.seg(16, detail, 8))
    L.transform(can, loc=(0.22, -0.02, 0))
    pal(k, r, 'copper', can)
    pal(k, r, 'copper', L.tube((0.3, -0.02, top + 0.05), (0.48, -0.02, top + 0.22), 0.012, 8))
    pal(k, r, 'brass', L.tube((0.48, -0.02, top + 0.22), (0.5, -0.02, top + 0.235), 0.028, 10))
    pal(k, r, 'copper', L.torus((0.18, -0.02, top + 0.2), 0.06, 0.008, L.seg(14, detail, 7), 4, (math.pi / 2, 0, 0), arc=math.pi))
    # hand scale
    if hi:
        pal(k, r, 'brass', L.lathe([(0, top), (0.06, top), (0.05, top + 0.02), (0.012, top + 0.02), (0.012, top + 0.14), (0, top + 0.14)], 12) if False else
            L.transform(L.lathe([(0, 0), (0.06, 0), (0.05, 0.02), (0.012, 0.02), (0.012, 0.14), (0, 0.14)], 12), loc=(-0.22, 0.1, top)))
        pal(k, r, 'brass', L.transform(L.lathe([(0, 0), (0.085, 0.0), (0.09, 0.02), (0.085, 0.022), (0, 0.006)], 16), loc=(-0.22, 0.1, top + 0.14)))
        pal(k, r, 'glass_dark', L.tube((-0.22, 0.04, top + 0.07), (-0.22, 0.03, top + 0.07), 0.04, 14))
        k.add(r, M.display, L.rbox((-0.22, 0.0288, top + 0.08), (0.004, 0.0015, 0.028), 0, 0))
        # notebook + scoop
        pal(k, r, 'leather', L.rbox((-0.05, 0.12, top + 0.012), (0.16, 0.22, 0.022), 0.004, 1))
        pal(k, r, 'paper', L.rbox((-0.05, 0.12, top + 0.012), (0.15, 0.21, 0.018), 0.002, 1))
        pal(k, r, 'brass', L.rbox((0.0, -0.17, top + 0.02), (0.2, 0.06, 0.03), 0.01, 1))
        # lower shelf: nested pots, coiled hose, probe rack
        for i in range(3):
            pal(k, r, 'terracotta', L.transform(L.lathe([(0, 0), (0.06, 0), (0.08, 0.12), (0.075, 0.12), (0.055, 0.01), (0, 0.01)], 14), loc=(0.25, 0.0, 0.315 + i * 0.03)))
        pal(k, r, 'rubber', L.torus((-0.15, 0.0, 0.345), 0.14, 0.02, 20, 6))
        pal(k, r, 'rubber', L.torus((-0.15, 0.0, 0.385), 0.13, 0.02, 20, 6))
        wood(k, M, r, (0.0, 0.18, top + 0.03), (0.2, 0.06, 0.05), 0.006)
        for i in range(3):
            pal(k, r, 'steel', L.tube((-0.07 + i * 0.07, 0.18, top + 0.05), (-0.07 + i * 0.07, 0.18, top + 0.26), 0.004, 6))
            ceramic(k, M, r, L.ellipsoid((-0.07 + i * 0.07, 0.18, top + 0.27), (0.014, 0.014, 0.02), 8, 4))
    _standard_anchors(k, 1.35, 0.80)
    _anchor(k, 'anchor_can_grip', (0.18, -0.02, top + 0.26), 'reach', 'watering-can handle')
    _anchor(k, 'anchor_scale_pan', (-0.22, 0.1, top + 0.16), 'slot', 'scale pan surface')
    _anchor(k, 'anchor_push_handle', (-W / 2 - 0.08, 0, 1.02), 'reach', 'push handle centre')
    return k


# ============================================================ CONSTRUCTION
def build_abutment(detail, kind='garden'):
    M = PM(); k = M.kit(f'bridge_abutment_{kind}', detail); r = k.root_name
    hi = detail > 0.7
    if kind == 'garden':
        half, post, deck, base = 1.18, 0.22, 0.45, -0.45     # deck 2.0 m wide, top 0.45 at the water edge
        stone(k, M, r, (0, 0.30, (base + deck - 0.05) / 2), (2.56, 0.6, deck - 0.05 - base), 0.02)      # cheek block in the water
        pal(k, r, 'wet_stone', L.rbox((0, 0.603, -0.06), (2.5, 0.006, 0.12), 0.0, 0))                  # wet line on the water face
        pal(k, r, 'bronze', L.rbox((0, 0.15, deck - 0.055), (2.0, 0.22, 0.012), 0.003, 1))              # bearing plate
        for x in (-0.8, -0.27, 0.27, 0.8):
            bolt(k, r, (x, 0.15, deck - 0.049), (0, 0, 1), 0.012, 0.008)
        for x in (-0.6, 0.6):                                                                          # weep holes
            pal(k, r, 'channel', L.tube((x, 0.58, 0.03), (x, 0.61, 0.03), 0.025, 8))
        top = deck + 1.0
        for s in (-1, 1):
            x = s * half
            stone(k, M, r, (x, 0.0, base / 2 + top / 2), (post, post, top - base), 0.012)
            stone(k, M, r, (x, 0.0, top + 0.03), (post + 0.06, post + 0.06, 0.06), 0.012)
            stone(k, M, r, (x, 0.0, top + 0.09), (post - 0.02, post - 0.02, 0.06), 0.02)
            pal(k, r, 'bronze', L.rbox((x - s * (post / 2 + 0.006), 0.04, deck + 0.95), (0.012, 0.14, 0.14), 0.003, 1))  # rail end fixing
            for (yy, zz) in ((-0.01, deck + 0.9), (0.09, deck + 0.9), (-0.01, deck + 1.0), (0.09, deck + 1.0)):
                bolt(k, r, (x - s * (post / 2 + 0.012), yy, zz), (-s, 0, 0), 0.008, 0.005)
        _anchor(k, 'anchor_deck_end', (0, 0.0, deck), 'contact', 'deck end bearing (deck surface level at the water edge)')
        _anchor(k, 'anchor_approach', (0, -0.9, 0), 'approach', 'land side of the bridge end; deck runs toward glTF -Z')
        _anchor(k, 'anchor_water_line', (0, 0.62, -0.06), 'contact', 'water surface on the cheek face')
    else:
        half, post = 2.92, 0.34
        stone(k, M, r, (0, 0.0, -1.32), (6.5, 0.7, 2.16), 0.03)                                  # pier wall below the deck
        stone(k, M, r, (0, 0.0, -0.28), (6.7, 0.82, 0.12), 0.02)                                 # corbel course
        for x in (-1.6, 1.6):                                                                       # scupper spouts
            for sy in (-1, 1):
                pal(k, r, 'bronze', L.tube((x, sy * 0.35, -0.42), (x, sy * 0.52, -0.46), 0.03, 8))
        pal(k, r, 'bronze', L.rbox((0, 0.0, 0.004), (5.5, 0.22, 0.008), 0.002, 1))                # threshold strip
        pal(k, r, 'channel', L.rbox((0, 0.13, 0.002), (5.5, 0.012, 0.004), 0, 0))                 # expansion joint
        for s in (-1, 1):
            x = s * half
            stone(k, M, r, (x, 0.0, 0.6), (post, post, 1.2), 0.015)
            stone(k, M, r, (x, 0.0, 1.235), (post + 0.08, post + 0.08, 0.07), 0.015)
            pal(k, r, 'bronze', L.lathe([(0, 1.27), (0.1, 1.27), (0.1, 1.29), (0.05, 1.33), (0, 1.38)], L.seg(12, detail, 6)) if False else
                L.transform(L.lathe([(0, 0), (0.1, 0), (0.1, 0.02), (0.05, 0.06), (0, 0.11)], L.seg(12, detail, 6)), loc=(x, 0, 1.27)))
            pal(k, r, 'bronze', L.rbox((x - s * (post / 2 + 0.006), 0.0, 0.5), (0.012, 0.16, 0.5), 0.003, 1))   # guard end fixing
            for zz in (0.32, 0.68):
                bolt(k, r, (x - s * (post / 2 + 0.012), 0.0, zz), (-s, 0, 0), 0.009, 0.006)
        _anchor(k, 'anchor_deck_end', (0, 0.0, 0), 'contact', 'deck surface at the bridge end (y = 3.4 in the world)')
        _anchor(k, 'anchor_approach', (0, -1.0, 0), 'approach', 'landing side; the bridge runs toward glTF -Z')
    return k


def build_coping(detail, corner=False):
    M = PM(); k = M.kit('water_coping_corner' if corner else 'water_coping_2m', detail); r = k.root_name
    hi = detail > 0.7
    rng = random.Random(4401 + corner)
    stones = [(-0.5, 0.996), (0.5, 0.996)] if not corner else [(0, 0.41)]
    for (cx, ln) in stones:
        w = 0.41 if not corner else 0.41
        top = L.rbox((cx, -0.155, -0.04), (ln, w, 0.08), 0.012, 2 if hi else 1)               # top stone (lip at y +0.05)
        stone_bm(k, M, r, top)
        # batter face toward the water, below the lip, sloping in
        bm = bmesh.new()
        x0, x1 = cx - ln / 2 + 0.003, cx + ln / 2 - 0.003
        prof = [(0.02, -0.08), (0.0, -0.40), (-0.12, -0.40), (-0.12, -0.08)]
        a = [bm.verts.new((x0, y, z)) for (y, z) in prof]
        b = [bm.verts.new((x1, y, z)) for (y, z) in prof]
        bm.faces.new(list(reversed(a))); bm.faces.new(b)
        for i in range(4):
            j = (i + 1) % 4
            bm.faces.new((a[i], a[j], b[j], b[i]))
        bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
        stone_bm(k, M, r, bm)
        pal(k, r, 'wet_stone', L.rbox((cx, 0.021, -0.115), (ln - 0.01, 0.004, 0.07), 0, 0))     # wet line (water at -0.07)
        if hi:
            pal(k, r, 'channel', L.rbox((cx, 0.03, -0.081), (ln - 0.02, 0.012, 0.004), 0, 0))   # drip groove under the lip
    if corner:
        for (c, sz) in (((0.0, -0.155, -0.24), (0.004, 0.41, 0.3)),):
            pass
        bm = bmesh.new()   # second batter face (water on +X too)
        prof = [(0.02, -0.08), (0.0, -0.40), (-0.12, -0.40), (-0.12, -0.08)]
        a = [bm.verts.new((x + 0.205 - 0.003 - 0.0, -0.36, z)) for (x, z) in [(p[0], p[1]) for p in prof]]
        b = [bm.verts.new((x + 0.205 - 0.003, 0.05, z)) for (x, z) in [(p[0], p[1]) for p in prof]]
        bm.faces.new(a); bm.faces.new(list(reversed(b)))
        for i in range(4):
            j = (i + 1) % 4
            bm.faces.new((a[j], a[i], b[i], b[j]))
        bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
        stone_bm(k, M, r, bm)
    if hi:   # moss in the joint and at the waterline
        for i in range(5 if not corner else 2):
            x = (rng.uniform(-0.9, 0.9) if not corner else rng.uniform(-0.15, 0.15))
            pal(k, r, 'moss', L.ellipsoid((x, rng.uniform(-0.02, 0.025), -0.075), (rng.uniform(0.02, 0.05), 0.012, 0.01), 6, 3))
        if not corner:
            pal(k, r, 'moss', L.ellipsoid((0.0, -0.2, 0.0), (0.006, 0.12, 0.004), 4, 3))          # joint moss
    _anchor(k, 'anchor_water_line', (0, 0.06, -0.07), 'contact', 'water surface 0.07 m below the coping top, 0.06 m beyond the lip')
    if not corner:
        for s, nm in ((-1, 'L'), (1, 'R')):
            _anchor(k, f'anchor_join_{nm}', (s * 1.0, 0, 0), 'contact', 'segment join; chain segments end to end')
    return k


def build_stanchion(detail):
    M = PM(); k = M.kit('rail_stanchion', detail); r = k.root_name
    hi = detail > 0.7
    pal(k, r, 'bronze', L.rbox((0, 0, 0.006), (0.14, 0.14, 0.012), 0.003, 1))
    for (x, y) in ((-0.05, -0.05), (0.05, -0.05), (-0.05, 0.05), (0.05, 0.05)):
        bolt(k, r, (x, y, 0.012), (0, 0, 1), 0.008, 0.006)
    pal(k, r, 'bronze', L.lathe([(0, 0.012), (0.035, 0.012), (0.03, 0.04), (0.022, 0.06), (0.022, 0.92), (0.03, 0.93), (0, 0.93)], L.seg(12, detail, 6)))
    pal(k, r, 'bronze', L.tube((0, -0.05, 0.95), (0, 0.05, 0.95), 0.032, L.seg(12, detail, 6)))  # handrail saddle
    pal(k, r, 'bronze', L.torus((0, 0, 0.5), 0.03, 0.007, L.seg(12, detail, 6), 4, (0, math.pi / 2, 0)))  # mid-rail eye
    _anchor(k, 'anchor_rail_top', (0, 0, 0.95), 'contact', 'handrail centreline (rail runs along glTF Z)')
    return k


def build_column_base(detail):
    M = PM(); k = M.kit('column_base_stone', detail); r = k.root_name
    stone(k, M, r, (0, 0, 0.06), (0.72, 0.72, 0.12), 0.012)
    prof = [(0, 0.12), (0.33, 0.12), (0.34, 0.15), (0.325, 0.18), (0.29, 0.19), (0.285, 0.22), (0.30, 0.245), (0.285, 0.27), (0.27, 0.30), (0, 0.30)]
    bm = L.lathe(prof, L.seg(24, detail, 10)); stone_bm(k, M, r, bm)
    pal(k, r, 'bronze', L.lathe([(0.272, 0.30), (0.282, 0.30), (0.282, 0.33), (0.272, 0.33)], L.seg(24, detail, 10)))
    for (x, y) in ((-0.3, -0.3), (0.3, -0.3), (-0.3, 0.3), (0.3, 0.3)):
        bolt(k, r, (x, y, 0.12), (0, 0, 1), 0.014, 0.01)
    _anchor(k, 'anchor_shaft', (0, 0, 0.33), 'contact', 'column shaft seats here (round shaft r 0.27)')
    return k


def build_post_shoe(detail):
    M = PM(); k = M.kit('post_shoe_bronze', detail); r = k.root_name
    pal(k, r, 'bronze', L.rbox((0, 0, 0.008), (0.30, 0.30, 0.016), 0.004, 1))
    for (x, y) in ((-0.11, -0.11), (0.11, -0.11), (-0.11, 0.11), (0.11, 0.11)):
        bolt(k, r, (x, y, 0.016), (0, 0, 1), 0.012, 0.008)
    for s in (-1, 1):
        pal(k, r, 'bronze', L.rbox((s * 0.105, 0, 0.11), (0.012, 0.2, 0.19), 0.004, 1))
        bolt(k, r, (s * 0.112, 0, 0.14), (s, 0, 0), 0.01, 0.006)
    pal(k, r, 'bronze', L.rbox((0, 0, 0.21), (0.23, 0.23, 0.02), 0.005, 1))
    _anchor(k, 'anchor_post', (0, 0, 0.22), 'contact', 'post (0.16-0.20 m) seats inside the shoe')
    return k


def build_drain(detail):
    M = PM(); k = M.kit('terrace_drain_grate', detail); r = k.root_name
    hi = detail > 0.7
    pal(k, r, 'channel', L.rbox((0, 0, -0.05), (1.0, 0.14, 0.09), 0, 0))
    for s in (-1, 1):
        pal(k, r, 'bronze', L.rbox((0, s * 0.075, -0.012), (1.0, 0.012, 0.026), 0.002 if hi else 0, 1 if hi else 0))
    n = 14 if hi else 6
    for i in range(n):
        x = -0.47 + i * 0.94 / (n - 1)
        pal(k, r, 'bronze', L.rbox((x, 0, -0.006), (0.012 if hi else 0.03, 0.14, 0.012), 0.001 if hi else 0, 1 if hi else 0))
    _anchor(k, 'anchor_flush_top', (0, 0, 0), 'contact', 'top is flush with paving (walkable, non-blocking)')
    return k


def build_plinth_steps(detail):
    """Two stone steps (2 x 0.16 m rise) so the 0.32 m instrument plinth can be
    mounted from the south; origin = foot of the bottom riser centre, the plinth
    is toward glTF -Z. Requires W6 to make the plinth top walkable."""
    M = PM(); k = M.kit('observatory_plinth_steps', detail); r = k.root_name
    hi = detail > 0.7
    W = 1.6
    for i, (y0, h) in enumerate(((0.0, 0.16), (0.30, 0.32))):
        stone(k, M, r, (0, y0 + 0.30 / 2 + (0.15 if i == 1 else 0) - (0.15 if i == 1 else 0), h / 2), (W, 0.30 + (0.0 if i == 0 else 0.0), h), 0.01)
        pal(k, r, 'bronze', L.rbox((0, y0 - 0.012 + 0.012, h - 0.006), (W - 0.04, 0.03, 0.012), 0.003, 1))   # nosing
    for s_ in (-1, 1):
        stone(k, M, r, (s_ * (W / 2 + 0.06), 0.30, 0.17), (0.12, 0.60, 0.34), 0.012)               # cheeks
    _anchor(k, 'anchor_approach', (0, -0.6, 0), 'approach', 'foot of the steps; plinth top (+0.32) lies toward glTF -Z')
    _anchor(k, 'anchor_top', (0, 0.75, 0.32), 'contact', 'top landing on the plinth')
    return k
