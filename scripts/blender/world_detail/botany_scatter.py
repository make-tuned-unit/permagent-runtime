"""Lane W4 scatter spec: deterministic species placements for the integrator,
derived from a read-only snapshot of the runtime layout (layout-snapshot.json
written by places_layout_probe.mjs). Pure Python (no bpy).

Rules (numbers are the designed clearances reported in the receipt):
  * Plants go only where the runtime already forbids walking: inside raised
    beds and water, or on existing runtime planting positions (terrace edges,
    station planters, water-front beds). Species are clustered in drifts.
  * Foliage core (0.75 x spread radius x scale) may not cover any cell that
    is reachable from HABITAT_START on the 0.25 m grid (body margin 0.47 m);
    a plant that would spill is shrunk (down to 0.55x) or dropped.
  * Canopy trees keep crown base >= 2.3 m (scale >= 2.3 / crown_base_m) and
    their trunk envelope (r 0.30 x scale) must not touch a reachable cell.
  * Instances are grouped into 10 m spatial cells for InstancedMesh batching.
"""
import base64
import math

CELL = 10.0
PLANT_BASE_Y = {'canopy': -.011091665364801884, 'shrub': .017311472445726395, 'fern': .032241806387901306, 'flowers': 0}
DECOR_TO_SPECIES = {'canopy': 'canopy-olive', 'shrub': 'shrub-laurel', 'fern': 'fern-sword', 'flowers': 'flower-coneflower'}
ARRIVAL = (0.0, 1.6, 6.0)


def habitat_height(z):
    return 0.0 if z >= -8 else 3.4 if z <= -14 else (-8 - z) * (3.4 / 6)


class Grid:
    def __init__(self, layout):
        g = layout['grid']
        self.nx, self.nz = g['nx'], g['nz']
        self.reach = base64.b64decode(g['reachable'])
        self.step = layout['step']
        self.x0, self.z0 = layout['bounds']['x0'], layout['bounds']['z0']

    def hits(self, x, z, r):
        """Reachable cell centres within r of (x, z)."""
        n = int(math.ceil(r / self.step)) + 1
        ci, ck = int((x - self.x0) // self.step), int((z - self.z0) // self.step)
        c = 0
        for i in range(ci - n, ci + n + 1):
            for k in range(ck - n, ck + n + 1):
                if 0 <= i < self.nx and 0 <= k < self.nz and self.reach[i * self.nz + k]:
                    px, pz = self.x0 + (i + .5) * self.step, self.z0 + (k + .5) * self.step
                    if math.hypot(px - x, pz - z) < r:
                        c += 1
        return c


class Rand:
    def __init__(self, seed):
        self.s = seed

    def __call__(self):
        self.s = (self.s * 1664525 + 1013904223) % 4294967296
        return self.s / 4294967296


def in_bed(b, x, z, f=1.0):
    return ((x - b['x']) / (b['rx'] * f)) ** 2 + ((z - b['z']) / (b['rz'] * f)) ** 2 <= 1.0


def build_scatter(layout, species):
    grid = Grid(layout)
    rnd = Rand(90210)
    spread = {k: v['measure']['spread_radius_m'] for k, v in species.items()}
    crown_base = species['canopy-olive']['measure']['crown_base_m']
    tree_min_scale = round(max(2.3 / crown_base, 0.8), 3)
    places = layout['places']
    inst, rejected, shrunk, nudged = [], [], 0, []

    def place_of(x, z):
        best = min(places, key=lambda p: math.hypot(p['position'][0] - x, p['position'][2] - z))
        d = math.hypot(best['position'][0] - x, best['position'][2] - z)
        return best['id'] if d <= 9.0 else ('observatory_garden' if z < -33 else 'route-edge')

    def add(sp, x, y, z, scale, source, soft=True):
        nonlocal shrunk
        if sp == 'canopy-olive':
            scale = max(scale, tree_min_scale)
            if grid.hits(x, z, 0.30 * scale + 0.05):
                rejected.append({'species': sp, 'at': [round(x, 2), round(z, 2)], 'why': 'trunk touches reachable cell'})
                return
        elif sp != 'vine-pendant':
            s = scale
            while s >= scale * 0.55 and grid.hits(x, z, 0.75 * spread[sp] * s):
                s *= 0.9
            if s < scale * 0.55:
                # nudge away from the route (8 directions, 0.3 / 0.6 m) before giving up
                moved = None
                for dist in (0.3, 0.6):
                    for k in range(8):
                        a = math.tau * k / 8
                        nx, nz = x + math.cos(a) * dist, z + math.sin(a) * dist
                        if not grid.hits(nx, nz, 0.75 * spread[sp] * scale * 0.75):
                            moved = (nx, nz)
                            break
                    if moved:
                        break
                if not moved:
                    rejected.append({'species': sp, 'at': [round(x, 2), round(z, 2)], 'why': 'foliage core over reachable route'})
                    return
                x, z = moved
                s = scale * 0.75
                nudged.append(1)
            if s < scale:
                shrunk += 1
            scale = s
        cx, cz = math.floor(x / CELL), math.floor(z / CELL)
        inst.append({'species': sp, 'position': [round(x, 3), round(y, 3), round(z, 3)],
                     'heading': round(rnd() * math.tau, 3), 'scale': round(scale, 3), 'cell': f'{cx},{cz}',
                     'place': place_of(x, z), 'source': source})

    beds = layout['beds']
    # 1. runtime decor plants outside beds keep their audited positions (terrace edges, planters, water-front)
    for d in layout['decor']:
        if d['asset'] not in DECOR_TO_SPECIES:
            continue
        x, y, z = d['position']
        if d['asset'] != 'canopy' and any(in_bed(b, x, z, 1.05) for b in beds):
            continue   # bed interiors are re-planted as clustered drifts below
        support = y + PLANT_BASE_Y[d['asset']] * d['scale']
        sp = DECOR_TO_SPECIES[d['asset']]
        fac = {'canopy-olive': 0.68, 'shrub-laurel': 0.85, 'fern-sword': 0.85, 'flower-coneflower': 1.0}[sp]
        add(sp, x, support, z, d['scale'] * fac, f"replaces HABITAT_DECOR {d['asset']} @ {round(x, 2)},{round(z, 2)}")
    # 2. clustered bed plantings (species drifts, empty bed margin toward routes enforced by the spill rule)
    for bi, b in enumerate(beds):
        y = habitat_height(b['z']) + 0.45
        src = f'HABITAT_BEDS[{bi}]'
        if not b.get('groundcover'):
            n_sh = 2 if b['rx'] * b['rz'] > 3.4 else 1
            for s in range(n_sh):
                ox = (s - (n_sh - 1) / 2) * b['rx'] * 0.7
                add('shrub-laurel', b['x'] + ox, y, b['z'] - b['rz'] * 0.12, 0.9 + 0.1 * s, src)
            a0 = rnd() * math.tau
            for i in range(7):    # fern ring
                a = a0 + math.tau * i / 7
                add('fern-sword', b['x'] + math.cos(a) * b['rx'] * 0.62, y, b['z'] + math.sin(a) * b['rz'] * 0.62, 0.62, src)
            for i in range(5):    # coneflower drift on one side
                a = a0 + math.pi + (i - 2) * 0.28
                r = 0.35 + 0.12 * (i % 2)
                add('flower-coneflower', b['x'] + math.cos(a) * b['rx'] * r, y, b['z'] + math.sin(a) * b['rz'] * r, 0.9, src)
            for i in range(2):
                a = a0 + math.pi * 0.5 + i * math.pi
                add('grass-fountain', b['x'] + math.cos(a) * b['rx'] * 0.38, y, b['z'] + math.sin(a) * b['rz'] * 0.38, 0.8, src)
        else:
            long_x = b['rx'] >= b['rz']
            L_ = (b['rx'] if long_x else b['rz']) * 0.86
            W_ = (b['rz'] if long_x else b['rx']) * 0.45
            drifts = ['grass-fountain', 'flower-coneflower', 'fern-sword', 'grass-fountain', 'flower-coneflower', 'fern-sword']
            n = max(3, int(2 * L_ / 1.25))
            for i in range(n):
                t = -L_ + (i + 0.5) * 2 * L_ / n
                sp = drifts[(i // 2 + bi) % len(drifts)]
                for j in range(2):
                    u = t + (j - 0.5) * 0.45
                    w = (j - 0.5) * W_ * (0.9 if j else 1.1)
                    x, z = (b['x'] + u, b['z'] + w) if long_x else (b['x'] + w, b['z'] + u)
                    add(sp, x, y, z, 0.62 if sp == 'fern-sword' else 0.75, src)
    # 3. water edge: iris clumps standing in the margin of the water garden, clear of both bridge landings
    water = next(s for s in layout['solids'] if s['name'] == 'water garden')
    wx, wz = water['at'][0], water['at'][2]
    hx, hz = water['size'][0] / 2, water['size'][2] / 2
    gb = layout['gardenBridge']
    for side in (-1, 1):
        for i in range(9):
            z = wz - hz + 0.35 + i * (2 * hz - 0.7) / 8
            if gb['z0'] - 0.45 <= z <= gb['z1'] + 0.45:
                continue
            add('iris-water', wx + side * (hx - 0.22), 0.0, z + (rnd() - .5) * 0.15, 0.9 + 0.2 * rnd(), 'water garden margin')
    for i in range(4):
        add('iris-water', wx - hx + 0.6 + i * (2 * hx - 1.2) / 3, 0.0, wz + hz - 0.22, 0.9, 'water garden south margin')
    # 4. hanging vines from the commons arbor beams (edges only; 2.32 m above the floor at the tip)
    for i in range(14):
        x = -7.05 + 0.25 * rnd() if i < 7 else -0.85 - 0.25 * rnd()
        z = 1.3 + (i % 7) * 0.8
        add('vine-pendant', x, 3.72, z, 1.0, 'commons arbor edge beam')

    # census
    counts = {}
    for it in inst:
        counts[it['species']] = counts.get(it['species'], 0) + 1
    tri0 = {k: v['lods']['lod0']['triangles'] for k, v in species.items()}
    tri1 = {k: v['lods']['lod1']['triangles'] for k, v in species.items()}
    prims0 = {k: v['lods']['lod0']['primitives_draw_calls'] for k, v in species.items()}
    switch = {'canopy-olive': 30, 'shrub-laurel': 16, 'fern-sword': 11, 'flower-coneflower': 11,
              'grass-fountain': 11, 'iris-water': 12, 'vine-pendant': 14}
    worst = sum(tri0[it['species']] for it in inst)
    arrival = 0
    for it in inst:
        d = math.dist((it['position'][0], it['position'][1], it['position'][2]), ARRIVAL)
        arrival += tri0[it['species']] if d < switch[it['species']] * it['scale'] else tri1[it['species']]
    groups = {}
    for it in inst:
        groups.setdefault((it['cell'], it['species']), 0)
        groups[(it['cell'], it['species'])] += 1
    draw_upper = sum(prims0[s] for (_, s) in groups)
    # arrival frustum estimate: HABITAT_ARRIVAL looks from (0,1.6,6) toward (-1.1,1.3,-1.5), vertical fov 72,
    # 16:10 -> horizontal half-angle ~49 deg; a cell counts if its centre (+ half diagonal) is in the wedge.
    fx, fz = -1.1 - 0.0, -1.5 - 6.0
    fl = math.hypot(fx, fz)
    fx, fz = fx / fl, fz / fl
    def in_view(cell):
        cx, cz = [int(v) for v in cell.split(',')]
        px, pz = (cx + .5) * CELL - ARRIVAL[0], (cz + .5) * CELL - ARRIVAL[2]
        d = math.hypot(px, pz)
        if d < CELL * 0.75:
            return True
        ang = math.degrees(math.acos(max(-1, min(1, (px * fx + pz * fz) / d))))
        return ang <= 49 + math.degrees(math.atan2(CELL * 0.71, d)) and d < 90
    draw_arrival = sum(prims0[s] for (c, s) in groups if in_view(c))
    batched = {}
    for (c, s) in groups:
        for mname in species[s]['lods']['lod0']['materials']:
            batched.setdefault((c, mname), 1)
    draw_batched = len(batched)
    draw_batched_arrival = sum(1 for (c, _) in batched if in_view(c))
    shadow_lod0 = sum(1 for it in inst if species[it['species']]['shadow']['lod0'])
    census = {
        'instances': len(inst), 'per_species': counts, 'rejected': len(rejected), 'shrunk_to_fit': shrunk,
        'tree_min_scale_for_2.3m_headroom': tree_min_scale,
        'triangles_all_lod0_worst_case': worst,
        'triangles_from_arrival_camera_lod_by_distance': arrival,
        'instanced_groups_cell_x_species': len(groups),
        'draw_calls_upper_bound_all_cells_visible': draw_upper,
        'draw_calls_arrival_frustum_instanced': draw_arrival,
        'draw_calls_if_batchedmesh_per_cell_material': draw_batched,
        'draw_calls_arrival_frustum_batchedmesh': draw_batched_arrival,
        'nudged_off_route': len(nudged),
        'shadow_casting_instances_lod0': shadow_lod0,
        'lane_targets': {'arrival_triangles': 400000, 'arrival_frustum_draw_calls_instanced': 80,
                         'note': 'W4 botany share of the 1M main-view / 250 draw-call frame budgets; the all-cells '
                                 'upper bound is reported, not gated (cells behind the camera are culled)'},
    }
    census['pass'] = arrival <= 400000 and draw_arrival <= 80
    return {
        'schema': 'permagent.world.detail-v4.scatter.v1',
        'coordinates': 'runtime world space (three.js): metres, +Y up, position = soil-contact origin; '
                       'heading = rotation about +Y (rad); uniform scale',
        'layout_source': layout['source'], 'cell_size_m': CELL,
        'rules': __doc__.split('Rules (numbers are the designed clearances reported in the receipt):')[1].strip(),
        'species': {k: {'lod0': f"{k}.lod0.glb", 'lod1': f"{k}.lod1.glb",
                        'collision': species[k]['collision'].get('file'),
                        'blocking': species[k]['collision'].get('blocking', False),
                        'cast_shadow': species[k]['shadow'],
                        'lod0_to_lod1_distance_m': switch[k]} for k in species},
        'instances': inst,
        'groups': [{'cell': c, 'species': s, 'count': n} for (c, s), n in sorted(groups.items())],
        'rejected': rejected,
        'census': census,
    }
