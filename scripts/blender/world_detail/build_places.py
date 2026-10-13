"""World detail-v4, lane W4: place construction + equipment.

  /Applications/Blender.app/Contents/MacOS/Blender -b -noaudio \
      --python scripts/blender/world_detail/build_places.py -- [--out DIR] [--assets a,b] [--no-blend]

Writes assets/world/detail-v4/environment/places/:
  <asset>.lod0.glb / .lod1.glb / .collision.glb, <asset>.blend, textures/,
  manifest.json (anchors, bounds, collision envelopes, suggested placements
  keyed to the six place ids), census.json, envelopes.json, route-check.json
then refreshes environment/manifest.json + census.json.
The route check bundles the runtime habitatLayout.ts read-only (node +
esbuild from ui/command-center/node_modules) and floods walkability with the
proposed blocking envelopes added. Refuses to write outside detail-v4/environment.
"""
import sys
import json
import math
import shutil
import datetime
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
REPO = HERE.parent.parent.parent

import bpy  # noqa: E402
import detail_lib as L  # noqa: E402
import places_assets as A  # noqa: E402
import places_export as X  # noqa: E402

TEXTURE_MAX_PX = 1024
PI = math.pi
B_HERO_TABLE = dict(source='ruling: hero table / bookcase', lod0=12000, lod1=4000, lod2=1000)
B_REPEAT = dict(source='ruling: repeated desk / bench / planter', lod0=6000, lod1=2000, lod2=500)
B_INSTR = dict(source='ruling: telescope or hero instrument', lod0=12000, lod1=4000, lod2=1000)


def lane(l0, l1):
    return dict(source='lane: architectural kit module (ruling: <= 250k LOD0 across unique modules)', lod0=l0, lod1=l1)


# collision boxes: Blender-local (center, size, blocking)
ASSETS = {
    'archive-bookcase': dict(build=A.build_bookcase, places=['gallery'], budget=B_HERO_TABLE,
                             boxes=[((0, 0, 1.40), (2.6, 0.54, 2.8), True)]),
    'archive-folio-stand': dict(build=A.build_folio_stand, places=['gallery', 'observatory_garden'], budget=B_REPEAT,
                                boxes=[((0, 0, 0.6), (0.7, 0.55, 1.2), True)]),
    'bench-long': dict(build=lambda d: A.build_bench(d, 3.2), places=['observatory_garden'], budget=B_REPEAT,
                       boxes=[((0, 0.03, 0.45), (3.24, 0.62, 0.9), True)]),
    'bench-short': dict(build=lambda d: A.build_bench(d, 1.6), places=['gallery'], budget=B_REPEAT,
                        boxes=[((0, 0.03, 0.45), (1.64, 0.62, 0.9), True)]),
    'maker-workbench': dict(build=A.build_workbench, places=['maker'], budget=B_HERO_TABLE,
                            boxes=[((0, 0.0, 0.61), (1.9, 0.78, 1.22), True)]),
    'maker-result-shelf': dict(build=A.build_result_shelf, places=['maker'], budget=B_HERO_TABLE,
                               boxes=[((0, 0, 0.8), (1.4, 0.42, 1.6), True)]),
    'maker-progress-stand': dict(build=A.build_progress_stand, places=['maker'], budget=B_REPEAT,
                                 boxes=[((0, 0, 0.78), (0.74, 0.66, 1.56), True)]),
    'observatory-forecast-plinth': dict(build=A.build_forecast_plinth, places=['council_garden'], budget=B_INSTR,
                                        boxes=[((0, 0, 0.6), (0.95, 0.62, 1.2), True)]),
    'observatory-telescope': dict(build=A.build_telescope, places=['observatory_garden'], budget=B_INSTR,
                                  boxes=[((0, 0, 0.95), (1.72, 1.72, 1.9), True), ((0, -0.05, 1.75), (0.7, 2.3, 1.5), True)]),
    'observatory-plinth-steps': dict(build=A.build_plinth_steps, places=['observatory_garden'], budget=lane(1500, 500),
                                     boxes=[((0, 0.30, 0.17), (1.84, 0.60, 0.34), False)]),
    'conservatory-measurement-panel': dict(build=A.build_measurement_panel, places=['conservatory'], budget=B_REPEAT,
                                           boxes=[((0, 0, 0.88), (1.24, 0.5, 1.76), True)]),
    'conservatory-growing-bed': dict(build=A.build_growing_bed, places=['conservatory'], budget=B_REPEAT,
                                     boxes=[((-0.02, 0, 0.45), (2.48, 0.92, 0.9), True)]),
    'conservatory-equipment-cart': dict(build=A.build_equipment_cart, places=['conservatory'], budget=B_REPEAT,
                                        boxes=[((-0.04, 0, 0.5), (1.14, 0.6, 1.04), True)]),
    'bridge-abutment-garden': dict(build=lambda d: A.build_abutment(d, 'garden'), places=['route'], budget=lane(4000, 1500),
                                   boxes=[((-1.18, 0, 0.5), (0.28, 0.28, 1.95), True), ((1.18, 0, 0.5), (0.28, 0.28, 1.95), True),
                                          ((0, 0.3, 0.0), (2.56, 0.6, 0.9), False)]),
    'bridge-abutment-observatory': dict(build=lambda d: A.build_abutment(d, 'observatory'), places=['route'], budget=lane(4000, 1500),
                                        boxes=[((-2.92, 0, 0.7), (0.42, 0.42, 1.4), True), ((2.92, 0, 0.7), (0.42, 0.42, 1.4), True)]),
    'water-coping-2m': dict(build=lambda d: A.build_coping(d, False), places=['route'], budget=lane(1200, 400),
                            boxes=[((0, -0.155, -0.04), (2.0, 0.41, 0.08), False)]),
    'water-coping-corner': dict(build=lambda d: A.build_coping(d, True), places=['route'], budget=lane(700, 250),
                                boxes=[((0, -0.155, -0.04), (0.41, 0.41, 0.08), False)]),
    'rail-stanchion': dict(build=A.build_stanchion, places=['route'], budget=lane(600, 200),
                           boxes=[((0, 0, 0.48), (0.14, 0.14, 0.96), False)]),
    'column-base-stone': dict(build=A.build_column_base, places=['gallery'], cls='construction', budget=lane(1500, 500),
                              boxes=[((0, 0, 0.165), (0.72, 0.72, 0.33), True)]),
    'post-shoe-bronze': dict(build=A.build_post_shoe, places=['maker', 'council_garden'], cls='construction', budget=lane(800, 300),
                             boxes=[((0, 0, 0.11), (0.30, 0.30, 0.22), True)]),
    'terrace-drain-grate': dict(build=A.build_drain, places=['route'], budget=lane(800, 300),
                                boxes=[((0, 0, -0.04), (1.0, 0.16, 0.08), False)]),
}

# Suggested runtime placements (three.js world, metres; heading = rotation.y, asset front = local +Z).
P = []


def put(asset, place, pos, heading=0.0, note='', scale=None):
    e = {'asset': asset, 'place': place, 'position': [round(v, 3) for v in pos], 'heading': round(heading, 4), 'note': note}
    if scale:
        e['scale'] = scale
    P.append(e)


for x in (-13, -10, -7):
    put('archive-bookcase', 'gallery', (x, 3.4, -22.3), 0, 'replaces garden-crafted bookcase-v1 (ARCHIVE_DECOR), same solid 2.6x2.8x0.54')
put('archive-folio-stand', 'gallery', (-7.6, 3.4, -19.3), 0, 'record stand: reader faces north toward the shelves')
put('bench-short', 'gallery', (-12, 3.4, -18), PI, 'reading seat; replaces HABITAT_DECOR bench at the same point')
for x in (-14, -6):
    put('column-base-stone', 'gallery', (x, 3.4, -16), 0, 'base for the archive pillar (0.54 m)')
put('maker-workbench', 'maker', (10, 3.4, -17.9), 0, 'replaces the steward station desk (same position; worker at z -17.2)')
put('maker-result-shelf', 'maker', (12.4, 3.4, -19.4), 0, 'result shelf beside the bench')
put('maker-progress-stand', 'maker', (7.9, 3.4, -18.6), 0.5, 'in-progress stand, turned toward the bench')
for x in (7, 14):
    put('post-shoe-bronze', 'maker', (x, 3.4, -16), 0, 'workshop pillar shoe (0.2 m post)')
put('observatory-forecast-plinth', 'council_garden', (1.4, 3.4, -23.4), 0.35, 'forecast panel plinth west of the forecaster desk, facing the terrace')
for (x, z) in ((-0.2, -21.7), (8.8, -21.7), (-0.2, -26.3), (8.8, -26.3)):
    put('post-shoe-bronze', 'council_garden', (x, 3.4, z), 0, 'observatory pillar shoe (0.16 m post)')
put('observatory-telescope', 'observatory_garden', (4, 3.72, -43.2), 0, 'on OBSERVATORY_PLINTH top (+0.32); barrel points north, eyepiece south')
put('observatory-plinth-steps', 'observatory_garden', (4, 3.4, -41.05 + 0.6), 0, 'south steps onto the plinth (needs the plinth top walkable, W6)')
for x in (-2, 10):
    put('bench-long', 'observatory_garden', (x, 3.4, -48.8), 0, 'sky-court bench facing the telescope; replaces OBSERVATORY_BENCHES solid 3.4x0.9x0.75')
put('archive-folio-stand', 'observatory_garden', (7.9, 3.4, -44.4), -PI / 2, 'field-notebook stand facing the instrument plinth')
put('conservatory-measurement-panel', 'conservatory', (8.3, 0, -9.6), 0.3, 'measurement panel west of the growth station desk')
put('conservatory-growing-bed', 'conservatory', (13.0, 0, -6.6), -PI / 2, 'growing bed along the east side, tended from the west')
put('conservatory-equipment-cart', 'conservatory', (12.2, 0, -9.6), -0.5, 'equipment cart')
put('bridge-abutment-garden', 'route', (2.05, 0, -2.2), -PI / 2, 'water-garden bridge, west water edge (deck 0.45 here)')
put('bridge-abutment-garden', 'route', (5.95, 0, -2.2), PI / 2, 'water-garden bridge, east water edge')
put('bridge-abutment-observatory', 'route', (4, 3.4, -27.6), 0, 'north bridge, terrace end')
put('bridge-abutment-observatory', 'route', (4, 3.4, -33.0), PI, 'north bridge, observatory-pad end')
for z in (-4.2, 0.0):
    put('water-coping-2m', 'route', (2.1, 0.04, z), -PI / 2, 'water garden west edge')
    put('water-coping-2m', 'route', (5.9, 0.04, z), PI / 2, 'water garden east edge')
for x in (3.05, 4.95):
    put('water-coping-2m', 'route', (x, 0.04, 1.0), 0, 'water garden south edge (scale 0.95 along local X)', scale=[0.95, 1, 1])
put('water-coping-corner', 'route', (2.1, 0.04, 1.0), 0, 'south-west water corner')
put('water-coping-corner', 'route', (5.9, 0.04, 1.0), PI / 2, 'south-east water corner')
for x in (2.5, 3.5, 4.5, 5.5):
    for z in (-3.15, -1.25):
        put('rail-stanchion', 'route', (x, 0.45, z), PI / 2, 'garden bridge rail post, base plate inside the 2.0 m deck '
            '(runtime rail solids at z -3.23/-1.17 keep their footprint)')
for x in (1.08, 6.92):
    for z in (-28.6, -30.3, -32.0):
        put('rail-stanchion', 'route', (x, 3.4, z), 0, 'north bridge guard post')
put('terrace-drain-grate', 'route', (4, 3.4, -26.9), 0, 'channel drain across the north bridge terrace landing')
put('terrace-drain-grate', 'route', (4, 3.4, -33.7), 0, 'channel drain across the north bridge pad landing')
put('terrace-drain-grate', 'route', (0.7, 0, -2.2), PI / 2, 'channel drain at the garden bridge west landing')
put('terrace-drain-grate', 'route', (7.3, 0, -2.2), PI / 2, 'channel drain at the garden bridge east landing')


def b2g(v):
    return (v[0], v[2], -v[1])


def to_world(local_gl, pos, heading, scale=(1, 1, 1)):
    x, y, z = local_gl[0] * scale[0], local_gl[1] * scale[1], local_gl[2] * scale[2]
    c, s = math.cos(heading), math.sin(heading)
    return (pos[0] + x * c + z * s, pos[1] + y, pos[2] - x * s + z * c)


def envelopes(records):
    out, dests = [], []
    for i, p in enumerate(P):
        spec = ASSETS[p['asset']]
        sc = p.get('scale', [1, 1, 1])
        c, s = abs(math.cos(p['heading'])), abs(math.sin(p['heading']))
        for j, (ctr, size, blocking) in enumerate(spec['boxes']):
            g = b2g(ctr)
            gs = (size[0] * sc[0], size[2] * sc[1], size[1] * sc[2])
            w = to_world(g, p['position'], p['heading'], sc)
            out.append({'name': f"{p['asset']}#{i}.{j}", 'asset': p['asset'], 'place': p['place'], 'blocking': blocking,
                        'at': [round(v, 3) for v in w],
                        'size': [round(gs[0] * c + gs[2] * s, 3), round(gs[1], 3), round(gs[0] * s + gs[2] * c, 3)]})
        rec = records.get(p['asset'])
        if rec:
            for a in rec['anchors']:
                if a['kind'] in ('approach', 'interact') and p['place'] != 'route':
                    w = to_world(a['gltf_position_in_root'], p['position'], p['heading'], sc)
                    dests.append({'name': f"{p['asset']}#{i} {a['name']}", 'at': [round(v, 3) for v in w]})
    sc_path = REPO / 'assets/world/detail-v4/environment/botany/scatter.json'
    if sc_path.exists():   # blocking plants from the botany scatter join the same check
        sc = json.loads(sc_path.read_text())
        for i, it in enumerate(sc['instances']):
            spec = sc['species'][it['species']]
            if not spec.get('blocking'):
                continue
            r = {'canopy-olive': 0.30, 'shrub-laurel': 0.55}[it['species']] * it['scale']
            out.append({'name': f"{it['species']}@{i}", 'asset': it['species'], 'place': it['place'], 'blocking': True,
                        'at': it['position'], 'size': [round(2 * r, 3), 1.0, round(2 * r, 3)]})
    return {'note': 'axis-aligned world footprints of each collision box after placement yaw (conservative for '
                    'rotated boxes); blocking=false boxes are placement/contact envelopes only',
            'envelopes': out, 'destinations': dests}


def build(name, spec, out, write_blend):
    rec = {'name': name, 'class': spec.get('cls', 'place' if spec['places'] != ['route'] else 'construction'), 'places': spec['places'],
           'budget': spec['budget'], 'lods': {}}
    for lod, detail in ((0, 1.0), (1, 0.45)):
        L.reset_scene()
        kit = spec['build'](detail)
        objs = kit.finish()
        glb = out / f'{name}.lod{lod}.glb'
        L.export_glb_fmt(glb, objs, 'JPEG')
        info = X.inspect_glb(glb)
        info.update({'file': glb.name, 'detail': detail, 'blender_triangles': L.triangles(objs)})
        rec['lods'][f'lod{lod}'] = info
        if lod == 0:
            rec['bounds'] = X.world_bounds(objs)
            rec['pivot'] = {'origin': 'floor contact point under the asset centre (coping: lip line at the coping top; '
                                      'vine/telescope noted in anchors)', 'gltf': [0, 0, 0]}
            rec['nodes'] = [{'name': o.name, 'kind': 'pivot' if o.empty_display_type == 'SPHERE' else
                             ('anchor' if 'anchor' in o else 'root'), 'parent': o.parent.name if o.parent else None,
                             'gltf_position_in_root': X.to_gltf(o.matrix_world.translation)}
                            for o in objs if o.type == 'EMPTY']
            rec['anchors'] = X.anchors_of(objs)
            ck = L.Kit(f"{name.replace('-', '_')}_collision")
            cm = L.pbr('collision', (1, 0, 1), 1)
            prims = []
            for (ctr, size, blocking) in spec['boxes']:
                ck.add(ck.root_name, cm, L.rbox(ctr, size, 0, 0))
                prims.append({'type': 'box', 'center_gltf': list(b2g(ctr)), 'size_gltf': [size[0], size[2], size[1]],
                              'blocking': blocking})
            ck.finish()
            cglb = out / f'{name}.collision.glb'
            L.export_glb_fmt(cglb, ck.objects, 'AUTO')
            ci = X.inspect_glb(cglb)
            rec['collision'] = {'file': cglb.name, 'bytes': ci['bytes'], 'sha256': ci['sha256'], 'primitives': prims,
                                'blocking': any(b for (_, _, b) in spec['boxes'])}
            L.reset_scene()
            kit = spec['build'](detail)
            kit.finish()
            if write_blend:
                bpy.context.preferences.filepaths.save_version = 0
                blend = out / f'{name}.blend'
                bpy.ops.wm.save_as_mainfile(filepath=str(blend), relative_remap=True, compress=True)
                rec['source_blend'] = {'file': blend.name, 'sha256': L.sha256(blend), 'bytes': blend.stat().st_size,
                                       'contains': 'LOD0 (LOD1 and the collision proxy are regenerated by the script)'}
    rec['suggested_placements'] = [p for p in P if p['asset'] == name]
    return rec


def census_entry(rec):
    b = rec['budget']
    l0, l1 = rec['lods']['lod0'], rec['lods']['lod1']
    max_px = max([max(i['width'], i['height']) for i in l0['images']] or [0])
    kinds = {a['kind'] for a in rec['anchors']}
    checks = {
        'lod0_triangles': {'value': l0['triangles'], 'ceiling': b['lod0'], 'pass': l0['triangles'] <= b['lod0']},
        'lod1_triangles': {'value': l1['triangles'], 'ceiling': b['lod1'], 'pass': l1['triangles'] <= b['lod1']},
        'lod1_reduces': {'value': round(l1['triangles'] / max(l0['triangles'], 1), 3), 'pass': l1['triangles'] < l0['triangles']},
        'max_texture_px': {'value': max_px, 'ceiling': TEXTURE_MAX_PX, 'pass': max_px <= TEXTURE_MAX_PX},
        'primitives_lod0': {'value': l0['primitives_draw_calls'], 'ceiling': 6, 'pass': l0['primitives_draw_calls'] <= 6},
        'collision_glb': {'value': rec['collision']['file'], 'pass': bool(rec['collision']['file'])},
        'anchors': {'value': sorted(kinds), 'pass': bool(kinds)},
    }
    if rec['class'] == 'place':
        checks['approach_anchor'] = {'value': 'anchor_approach' in {a['name'] for a in rec['anchors']},
                                     'pass': 'anchor_approach' in {a['name'] for a in rec['anchors']}}
    return {'asset': rec['name'], 'class': rec['class'], 'budget_source': b['source'],
            'lod0': {k: l0[k] for k in ('file', 'triangles', 'primitives_draw_calls', 'bytes', 'materials', 'texture_decoded_bytes')},
            'lod1': {k: l1[k] for k in ('file', 'triangles', 'primitives_draw_calls', 'bytes', 'materials', 'texture_decoded_bytes')},
            'textures': [{k: i[k] for k in ('name', 'mime', 'width', 'height', 'colorspace', 'bytes')} for i in l0['images']],
            'collision_bytes': rec['collision']['bytes'],
            'placements': len(rec['suggested_placements']),
            'checks': checks, 'pass': all(c['pass'] for c in checks.values())}


def route_check(out):
    node = shutil.which('node') or '/opt/homebrew/bin/node'
    layout = out.parent / 'layout' / 'layout-snapshot.json'
    try:
        r = subprocess.run([node, str(HERE / 'places_layout_probe.mjs'), 'check', str(layout), str(out / 'envelopes.json'),
                            str(out / 'route-check.json')], capture_output=True, text=True, timeout=300)
        print('[W4 places]', r.stdout.strip(), r.stderr.strip()[:400])
        return json.loads((out / 'route-check.json').read_text())
    except Exception as e:   # reported, not hidden
        return {'pass': None, 'error': repr(e)}


def main():
    argv = sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else []
    out = REPO / 'assets/world/detail-v4/environment/places'
    only, write_blend = None, True
    i = 0
    while i < len(argv):
        if argv[i] == '--out':
            out = Path(argv[i + 1]).resolve(); i += 1
        elif argv[i] == '--assets':
            only = argv[i + 1].split(','); i += 1
        elif argv[i] == '--no-blend':
            write_blend = False
        i += 1
    X.guard_out(out, 'places')
    out.mkdir(parents=True, exist_ok=True)
    (out / 'textures').mkdir(exist_ok=True)
    L.TEXTURE_DIR = out / 'textures'
    man_path = out / 'manifest.json'
    records = {}
    if only and man_path.exists():
        records = {a['name']: a for a in json.loads(man_path.read_text())['assets']}
    for n, spec in ASSETS.items():
        if only and n not in only:
            continue
        print(f'[W4 places] {n}', flush=True)
        records[n] = build(n, spec, out, write_blend)
    ordered = [records[n] for n in ASSETS if n in records]
    env = envelopes(records)
    (out / 'envelopes.json').write_text(json.dumps(env, indent=1) + '\n')
    rc = route_check(out)
    now = datetime.datetime.now(datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')
    clearances = {
        'body_margin_m': 0.47,
        'designed_minimums': {
            'primary_route_clear_width_m': 1.6, 'secondary_route_clear_width_m': 1.2,
            'interaction_standoff_m': '0.55-0.75 (anchor_interact) in front of every place asset',
            'approach_standoff_m': '1.1-1.35 (anchor_approach)',
            'headroom_over_routes_m': 2.3, 'tree_crown_base_min_m': 2.3,
            'bridge_garden_clear_between_newels_m': 2.14, 'bridge_north_clear_between_newels_m': 5.42,
            'step_rise_m': 0.16, 'coping_curb_above_paving_m': 0.04,
        },
        'check': {k: rc.get(k) for k in ('pass', 'reachable_cells_before', 'reachable_cells_after', 'area_lost_m2',
                                         'regressions', 'error')},
    }
    manifest = {
        'schema': 'permagent.world.detail-v4.places.v1', 'lane': 'W4 (Astra ruling 2026-10-12)',
        'status': 'candidate — not published; W6 owns placements/colliders in habitatLayout.ts', 'generated': now,
        'blender': bpy.app.version_string, 'metersPerUnit': 1, 'runtimeUp': 'Y', 'forward': 'glTF +Z (Blender -Y)',
        'source_scripts': {f'scripts/blender/world_detail/{p.name}': L.sha256(p) for p in sorted(HERE.glob('*'))
                           if p.name.startswith(('places_', 'build_places', 'detail_lib', 'botany_lib'))},
        'provenance': {'geometry': 'authored procedurally in Blender by these scripts; no third-party or generated meshes',
                       'textures': 'walnut = W3 detail_lib.tex_walnut (identical pixels/names, de-duplicable by hash); '
                                   'limestone + trim palette authored with numpy in places_lib.py; no AI imagery',
                       'license': 'project-authored'},
        'material_family': 'W3: Timber • oiled walnut, Ceramic • warm ivory, brass/bronze/graphite trim palette, '
                           'Display • jade phosphor; W4 adds Stone • warm limestone and Panel • dark glass slot',
        'panel_slot_contract': 'meshes named *_panel_slot use material "Panel • dark glass slot" with UV 0..1 across the '
                               'front face; anchor_panel_slot gives centre, glTF normal and size_w_m/size_h_m',
        'route_clearance': clearances,
        'suggested_placements': P,
        'assets': ordered,
    }
    man_path.write_text(json.dumps(manifest, indent=2) + '\n')
    entries = [census_entry(r) for r in ordered]
    placed0 = sum(records[p['asset']]['lods']['lod0']['triangles'] for p in P if p['asset'] in records)
    placed_calls = sum(records[p['asset']]['lods']['lod0']['primitives_draw_calls'] for p in P if p['asset'] in records)
    census = {'schema': 'permagent.world.detail-v4.places.census.v1', 'generated': now, 'assets': entries,
              'route_check': clearances['check']}
    census['totals'] = {
        'lod0_bytes': sum(r['lods']['lod0']['bytes'] for r in ordered),
        'lod1_bytes': sum(r['lods']['lod1']['bytes'] for r in ordered),
        'collision_bytes': sum(e['collision_bytes'] for e in entries),
        'unique_modules_lod0_triangles': sum(r['lods']['lod0']['triangles'] for r in ordered),
        'unique_modules_ceiling': 250000,
        'suggested_placements': len(P),
        'placed_lod0_triangles_all_visible': placed0,
        'placed_lod0_draw_calls_all_visible_unbatched': placed_calls,
        'all_pass': all(e['pass'] for e in entries) and bool(rc.get('pass')),
    }
    (out / 'census.json').write_text(json.dumps(census, indent=2) + '\n')
    X.write_top_level(out.parent)
    for e in entries:
        print(f"[W4 places] {e['asset']:31s} lod0 {e['lod0']['triangles']:6d} lod1 {e['lod1']['triangles']:5d} "
              f"prims {e['lod0']['primitives_draw_calls']} bytes {e['lod0']['bytes']:8d} pass={e['pass']}"
              + ('' if e['pass'] else f" FAIL {[k for k, c in e['checks'].items() if not c['pass']]}"))
    print('[W4 places] totals', json.dumps(census['totals']))
    print('[W4 places] route', json.dumps(clearances['check']))


if __name__ == '__main__':
    main()
