"""World detail-v4, lane W4: botany species kit + scatter spec.

  /Applications/Blender.app/Contents/MacOS/Blender -b -noaudio \
      --python scripts/blender/world_detail/build_botany.py -- [--out DIR] [--species a,b] [--no-blend]

Writes assets/world/detail-v4/environment/botany/:
  <species>.lod0.glb / .lod1.glb / .collision.glb, <species>.blend, textures/,
  manifest.json, census.json, scatter.json (instancing spec keyed to the six
  place ids), then refreshes environment/manifest.json + census.json.
Refuses any directory outside detail-v4/environment (never public/world).
Deterministic: seeded geometry, numpy-authored textures, no AI imagery.
"""
import sys
import json
import math
import datetime
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
REPO = HERE.parent.parent.parent

import bpy  # noqa: E402
from mathutils import Vector  # noqa: E402
import detail_lib as L  # noqa: E402
import botany_lib as B  # noqa: E402
import botany_species as S  # noqa: E402
import botany_scatter as SC  # noqa: E402
import places_export as X  # noqa: E402

TEXTURE_MAX_PX = 1024
LOD_SWITCH = {   # projected-size proxies as camera distance (m) at fov 58; hysteresis = +/- 10 %
    'canopy-olive': 30, 'shrub-laurel': 16, 'fern-sword': 11, 'flower-coneflower': 11,
    'grass-fountain': 11, 'iris-water': 12, 'vine-pendant': 14,
}


def alive(objs):
    out = []
    for o in objs:
        try:
            o.name
        except ReferenceError:
            continue
        out.append(o)
    return out


def finish_species(kit, info, m, name):
    objs = kit.finish()
    meshes = [o for o in objs if o.type == 'MESH']
    for o in meshes:
        mat = o.data.materials[0]
        if mat == m.foliage:
            B.set_foliage_normals(o, info['crown_center'], info['crown_scale'], 0.7)
        elif name in ('shrub-laurel', 'vine-pendant'):
            B.set_foliage_normals(o, info['crown_center'], info['crown_scale'], 0.4)
    bpy.ops.object.select_all(action='DESELECT')
    for o in meshes:
        o.select_set(True)
    bpy.context.view_layer.objects.active = meshes[0]
    if len(meshes) > 1:
        bpy.ops.object.join()
    mesh = bpy.context.view_layer.objects.active
    mesh.name = mesh.data.name = f'{kit.root_name}_mesh'
    sway, tip, profile, amp, freq, stiff = info['wind']
    S._wind(kit, kit.root_name, sway, tip, profile, amp, freq, stiff)
    kit.anchor('anchor_soil_contact', kit.root_name, (0, 0, 0), kind='contact',
               note='origin; set on the soil/bed surface. Root flare / stems sink up to 0.10 m below it')
    kit.objects = alive(kit.objects)
    return kit.objects, mesh


def measure(mesh, name):
    me = mesh.data
    zs = [v.co.z for v in me.vertices]
    rad = max(math.hypot(v.co.x, v.co.y) for v in me.vertices)
    out = {'height_m': round(max(zs), 3), 'lowest_m': round(min(zs), 3), 'spread_radius_m': round(rad, 3)}
    if name == 'canopy-olive':
        crown = [v.co.z for v in me.vertices if math.hypot(v.co.x, v.co.y) > 0.45]
        foliage_idx = [i for i, s in enumerate(me.materials) if s and s.name.startswith('Foliage')]
        fz = [me.vertices[vi].co.z for p in me.polygons if p.material_index in foliage_idx for vi in p.vertices]
        out['crown_base_m'] = round(min(fz), 3)
        out['branch_min_z_outside_0.45m_m'] = round(min(crown), 3)
    return out


def collision_kit(name, spec):
    c = spec['collision']
    if c['type'] == 'none':
        return None, dict(c)
    k = L.Kit(f"{name.replace('-', '_')}_collision")
    r, h = c['radius'], c['height']
    k.add(k.root_name, L.pbr('collision', (1, 0, 1), 1), L.lathe([(0, 0), (r, 0), (r, h), (0, h)], 12))
    k.finish()
    return k, dict(c, center=[0, round(h / 2, 3), 0])


def build(name, spec, out, write_blend):
    rec = {'name': name, 'class': 'botany', 'form': spec['form'], 'alpha_strategy': spec['alpha'],
           'budget': spec['budget'], 'shadow': spec['shadow'], 'lods': {}}
    for lod in (0, 1):
        L.reset_scene()
        m = B.Mats()
        kit, info = spec['build'](m, lod)
        objs, mesh = finish_species(kit, info, m, name)
        if lod == 0:
            meas = measure(mesh, name)
            if 'crown_base_m' in meas:
                kit.anchor('anchor_crown_base', kit.root_name, (0, 0, meas['crown_base_m']), kind='clearance',
                           note='lowest foliage at scale 1.0; scale >= 2.3 / this keeps 2.3 m headroom at the drip line')
                objs = alive(kit.objects)
        glb = out / f'{name}.lod{lod}.glb'
        L.export_glb_fmt(glb, objs, 'WEBP')
        info_glb = X.inspect_glb(glb)
        info_glb.update({'file': glb.name, 'blender_triangles': L.triangles(objs), 'mesh_node': mesh.name})
        rec['lods'][f'lod{lod}'] = info_glb
        if lod == 0:
            rec['measure'] = meas
            rec['bounds'] = X.world_bounds(objs)
            rec['pivot'] = {'origin': 'soil contact under the stem/trunk centre (vine: top attachment point)',
                            'gltf': [0, 0, 0]}
            rec['anchors'] = X.anchors_of(objs)
            ck, cspec = collision_kit(name, spec)
            if ck:
                cglb = out / f'{name}.collision.glb'
                L.export_glb_fmt(cglb, ck.objects, 'AUTO')
                ci = X.inspect_glb(cglb)
                rec['collision'] = dict(cspec, file=cglb.name, bytes=ci['bytes'], sha256=ci['sha256'],
                                        triangles=ci['triangles'])
            else:
                rec['collision'] = dict(cspec, file=None)
            if write_blend:
                bpy.context.preferences.filepaths.save_version = 0
                blend = out / f'{name}.blend'
                bpy.ops.wm.save_as_mainfile(filepath=str(blend), relative_remap=True, compress=True)
                rec['source_blend'] = {'file': blend.name, 'sha256': L.sha256(blend), 'bytes': blend.stat().st_size,
                                       'contains': 'LOD0 + collision proxy; LOD1 is regenerated by the script'}
    rec['lod_switch'] = {'lod0_to_lod1_distance_m': LOD_SWITCH[name], 'hysteresis': 0.10,
                         'note': 'distance at fov 58 for scale 1; scale the threshold by instance scale'}
    return rec


def census_entry(rec):
    b = rec['budget']
    l0, l1 = rec['lods']['lod0'], rec['lods']['lod1']
    max_px = max([max(i['width'], i['height']) for i in l0['images'] + l1['images']] or [0])
    checks = {
        'lod0_triangles': {'value': l0['triangles'], 'ceiling': b['lod0'], 'pass': l0['triangles'] <= b['lod0']},
        'lod1_triangles': {'value': l1['triangles'], 'ceiling': b['lod1'], 'pass': l1['triangles'] <= b['lod1']},
        'lod1_reduces': {'value': round(l1['triangles'] / max(l0['triangles'], 1), 3), 'pass': l1['triangles'] < l0['triangles']},
        'max_texture_px': {'value': max_px, 'ceiling': TEXTURE_MAX_PX, 'pass': max_px <= TEXTURE_MAX_PX},
        'primitives_lod0': {'value': l0['primitives_draw_calls'], 'ceiling': 2, 'pass': l0['primitives_draw_calls'] <= 2},
        'one_mesh': {'value': l0['meshes'], 'pass': l0['meshes'] == 1 and l1['meshes'] == 1},
        'wind_anchors': {'value': sorted(a['name'] for a in rec['anchors'] if a['kind'] == 'wind'),
                         'pass': {'wind_root', 'wind_sway', 'wind_tip'} <= {a['name'] for a in rec['anchors']}},
    }
    if 'crown_base_m' in rec['measure']:
        checks['crown_base_headroom'] = {'value': rec['measure']['crown_base_m'], 'ceiling_note': '>= 2.3 m at scale 1',
                                         'pass': rec['measure']['crown_base_m'] >= 2.3}
    alpha = [mm for mm in l0['material_modes'] + l1['material_modes'] if mm['alphaMode'] != 'OPAQUE']
    return {'asset': rec['name'], 'budget_source': b['source'],
            'lod0': {k: l0[k] for k in ('file', 'triangles', 'primitives_draw_calls', 'bytes', 'materials', 'texture_decoded_bytes')},
            'lod1': {k: l1[k] for k in ('file', 'triangles', 'primitives_draw_calls', 'bytes', 'materials', 'texture_decoded_bytes')},
            'alpha_materials': alpha, 'cast_shadow': rec['shadow'],
            'textures': [{k: i[k] for k in ('name', 'mime', 'width', 'height', 'colorspace', 'bytes')}
                         for i in l0['images'] + [x for x in l1['images'] if x['name'] not in {y['name'] for y in l0['images']}]],
            'collision_bytes': rec['collision'].get('bytes', 0),
            'checks': checks, 'pass': all(c['pass'] for c in checks.values())}


def main():
    argv = sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else []
    out = REPO / 'assets/world/detail-v4/environment/botany'
    only, write_blend = None, True
    i = 0
    while i < len(argv):
        if argv[i] == '--out':
            out = Path(argv[i + 1]).resolve(); i += 1
        elif argv[i] == '--species':
            only = argv[i + 1].split(','); i += 1
        elif argv[i] == '--no-blend':
            write_blend = False
        i += 1
    X.guard_out(out, 'botany')
    out.mkdir(parents=True, exist_ok=True)
    (out / 'textures').mkdir(exist_ok=True)
    L.TEXTURE_DIR = out / 'textures'
    man_path = out / 'manifest.json'
    records = {}
    if only and man_path.exists():
        records = {a['name']: a for a in json.loads(man_path.read_text())['assets']}
    for n, spec in S.SPECIES.items():
        if only and n not in only:
            continue
        print(f'[W4 botany] {n}', flush=True)
        records[n] = build(n, spec, out, write_blend)
    ordered = [records[n] for n in S.SPECIES if n in records]
    layout = json.loads((out.parent / 'layout' / 'layout-snapshot.json').read_text())
    scatter = SC.build_scatter(layout, {r['name']: r for r in ordered})
    (out / 'scatter.json').write_text(json.dumps(scatter, indent=1) + '\n')
    now = datetime.datetime.now(datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')
    manifest = {
        'schema': 'permagent.world.detail-v4.botany.v1', 'lane': 'W4 (Astra ruling 2026-10-12)',
        'status': 'candidate — not published', 'generated': now, 'blender': bpy.app.version_string,
        'metersPerUnit': 1, 'runtimeUp': 'Y',
        'source_scripts': {f'scripts/blender/world_detail/{p.name}': L.sha256(p)
                           for p in sorted(HERE.glob('*')) if p.name.startswith(('botany_', 'build_botany', 'places_export', 'detail_lib'))},
        'provenance': {'geometry': 'authored procedurally in Blender by these scripts; no third-party or generated meshes',
                       'textures': 'bark, foliage atlas (true alpha, colour-dilated) and gradient palette drawn with numpy '
                                   'in botany_lib.py; no AI imagery, no downloads', 'license': 'project-authored',
                       'species_claim': 'readable garden forms; no claim of scientific species identity'},
        'instancing_contract': {
            'structure': 'root empty <species_id> -> one mesh node <species_id>_mesh (identity transform, <= 2 primitives) '
                         '+ anchor empties; build one InstancedMesh per (species, LOD, primitive, spatial cell)',
            'wind': 'wind_root / wind_sway / wind_tip empties on every species and LOD; extras carry profile, amplitude, '
                    'frequency, stiffness and the weight rule; no extra vertex attributes (no COLOR_0, which three.js '
                    'would multiply into base colour)',
            'alpha': 'only "Foliage • alpha-tested atlas" is MASK (cutoff 0.5) + doubleSided; use alphaTest, never '
                     'transparent sorting; shadow casters need a depth material with the same alphaTest',
            'scatter': 'botany/scatter.json'},
        'assets': ordered,
    }
    man_path.write_text(json.dumps(manifest, indent=2) + '\n')
    entries = [census_entry(r) for r in ordered]
    census = {'schema': 'permagent.world.detail-v4.botany.census.v1', 'generated': now, 'assets': entries,
              'scatter': scatter['census']}
    census['totals'] = {
        'lod0_bytes': sum(r['lods']['lod0']['bytes'] for r in ordered),
        'lod1_bytes': sum(r['lods']['lod1']['bytes'] for r in ordered),
        'collision_bytes': sum(e['collision_bytes'] for e in entries),
        'lod0_triangles_one_of_each': sum(r['lods']['lod0']['triangles'] for r in ordered),
        'all_pass': all(e['pass'] for e in entries) and scatter['census']['pass'],
    }
    (out / 'census.json').write_text(json.dumps(census, indent=2) + '\n')
    X.write_top_level(out.parent)
    for e in entries:
        print(f"[W4 botany] {e['asset']:18s} lod0 {e['lod0']['triangles']:5d} lod1 {e['lod1']['triangles']:5d} "
              f"prims {e['lod0']['primitives_draw_calls']} bytes {e['lod0']['bytes']:8d} pass={e['pass']}"
              + ('' if e['pass'] else f" FAIL {[k for k, c in e['checks'].items() if not c['pass']]}"))
    print('[W4 botany] totals', json.dumps(census['totals']))
    print('[W4 botany] scatter', json.dumps({k: v for k, v in scatter['census'].items() if k != 'per_species'}))


if __name__ == '__main__':
    main()
