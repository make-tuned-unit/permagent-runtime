"""World detail-v4, lane W3: build the hero robot and the Arrival-table social
furniture, export LOD GLBs + collision proxies, and write census.json and
manifest.json.

Usage (one Blender process; light work, a few seconds):
  /Applications/Blender.app/Contents/MacOS/Blender -b -noaudio \
      --python scripts/blender/world_detail/build_hero.py -- [--out DIR] [--assets a,b] [--no-blend]

Default output: assets/world/detail-v4/hero/ (refuses any other directory
unless --allow-out is given, so it can never overwrite another lane or the
published public/world/ tree). Re-running regenerates every output from
scratch; meshes, textures and GLBs are deterministic.
Then: render_review.py (diagnostic renders) and validate_glb.mjs (validator).
"""
import sys
import json
import math
import struct
import datetime
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
REPO = HERE.parent.parent.parent

import bpy  # noqa: E402
from mathutils import Vector  # noqa: E402
import detail_lib as L  # noqa: E402
import hero_robot as R  # noqa: E402
import arrival_furniture as F  # noqa: E402

# ----------------------------------------------------------------------------- asset table
# Budgets: Astra ruling 2026-10-12 section 2. Where the ruling has no class for a
# small prop, the ceiling is this lane's own (marked 'lane').
ASSETS = {
    'hero-robot': dict(root='robot', build=R.build_robot, lods={0: 1.0, 1: 0.45}, image_format='AUTO',
                       budget=dict(source='ruling: characters', lod0=24000, lod1=10000, lod2=3000,
                                   note='ceiling includes runtime wardrobe (agentWardrobe.ts, <= ~2.5k tris)'),
                       wardrobe_allowance=2500),
    'arrival-table': dict(root='arrival_table', build=F.build_table, lods={0: 1.0, 1: 0.45}, image_format='JPEG',
                          budget=dict(source='ruling: hero table / bookcase', lod0=12000, lod1=4000, lod2=1000)),
    'arrival-chair': dict(root='arrival_chair', build=F.build_chair, lods={0: 1.0, 1: 0.45}, image_format='JPEG',
                          budget=dict(source='ruling: repeated desk / bench / planter', lod0=6000, lod1=2000,
                                      lod2=500)),
    'arrival-folio': dict(root='arrival_folio', build=F.build_folio, lods={0: 1.0, 1: 0.45}, image_format='JPEG',
                          budget=dict(source='lane: small hand prop', lod0=2000, lod1=600)),
    'arrival-cup': dict(root='arrival_cup', build=F.build_cup, lods={0: 1.0, 1: 0.45}, image_format='JPEG',
                        budget=dict(source='lane: small hand prop', lod0=1500, lod1=500)),
    'arrival-lantern': dict(root='arrival_lantern', build=F.build_lantern, lods={0: 1.0, 1: 0.45},
                            image_format='JPEG', budget=dict(source='lane: table light prop', lod0=3000, lod1=1000)),
    'arrival-instrument': dict(root='arrival_instrument', build=F.build_instrument, lods={0: 1.0, 1: 0.45},
                               image_format='JPEG',
                               budget=dict(source='ruling: hero instrument 12k/4k/1k; lane target', lod0=4000,
                                           lod1=1500)),
}
TEXTURE_MAX_PX = 1024          # ruling: mostly 1K, no new 4K
ASSET_BYTES_MAX = 1_500_000    # lane ceiling per LOD0 GLB (whole kit must stay far under the 18 MiB detail kit)


def to_gltf(v):
    return [round(v[0], 4), round(v[2], 4), round(-v[1], 4)]


# ----------------------------------------------------------------------------- GLB inspection
def _image_size(data):
    if data[:8] == b'\x89PNG\r\n\x1a\n':
        w, h = struct.unpack('>II', data[16:24])
        return w, h, 'image/png'
    if data[:2] == b'\xff\xd8':
        i = 2
        while i < len(data):
            if data[i] != 0xFF:
                i += 1
                continue
            marker = data[i + 1]
            if marker in (0xC0, 0xC1, 0xC2):
                h, w = struct.unpack('>HH', data[i + 5:i + 9])
                return w, h, 'image/jpeg'
            seglen = struct.unpack('>H', data[i + 2:i + 4])[0]
            i += 2 + seglen
    return 0, 0, 'unknown'


def inspect_glb(path):
    raw = Path(path).read_bytes()
    magic, version, length = struct.unpack('<4sII', raw[:12])
    assert magic == b'glTF' and version == 2 and length == len(raw), 'bad GLB header'
    jlen, jtype = struct.unpack('<I4s', raw[12:20])
    gltf = json.loads(raw[20:20 + jlen])
    bin_off = 20 + jlen + 8
    tris = 0
    prims = 0
    for mesh in gltf.get('meshes', []):
        for p in mesh['primitives']:
            prims += 1
            if 'indices' in p:
                tris += gltf['accessors'][p['indices']]['count'] // 3
            else:
                tris += gltf['accessors'][p['attributes']['POSITION']]['count'] // 3
    images = []
    colorspace = {}
    for mat in gltf.get('materials', []):
        pbr = mat.get('pbrMetallicRoughness', {})
        for key, cs in (('baseColorTexture', 'sRGB'), ('metallicRoughnessTexture', 'linear')):
            if key in pbr:
                colorspace[gltf['textures'][pbr[key]['index']]['source']] = cs
        for key in ('normalTexture', 'occlusionTexture'):
            if key in mat:
                colorspace[gltf['textures'][mat[key]['index']]['source']] = 'linear'
        if 'emissiveTexture' in mat:
            colorspace[gltf['textures'][mat['emissiveTexture']['index']]['source']] = 'sRGB'
    for i, img in enumerate(gltf.get('images', [])):
        bv = gltf['bufferViews'][img['bufferView']]
        data = raw[bin_off + bv.get('byteOffset', 0): bin_off + bv.get('byteOffset', 0) + bv['byteLength']]
        w, h, mime = _image_size(data)
        images.append({'name': img.get('name'), 'mime': img.get('mimeType', mime), 'width': w, 'height': h,
                       'bytes': bv['byteLength'], 'colorspace': colorspace.get(i, 'unused'),
                       'decoded_bytes_with_mips': int(w * h * 4 * 4 / 3)})
    return {
        'bytes': len(raw), 'sha256': L.sha256(path), 'triangles': tris, 'primitives_draw_calls': prims,
        'meshes': len(gltf.get('meshes', [])), 'nodes': len(gltf.get('nodes', [])),
        'materials': [m.get('name') for m in gltf.get('materials', [])],
        'extensions_used': gltf.get('extensionsUsed', []), 'images': images,
        'texture_decoded_bytes': sum(i['decoded_bytes_with_mips'] for i in images),
        'node_names': [n.get('name') for n in gltf.get('nodes', [])],
    }


# ----------------------------------------------------------------------------- per asset
def world_bounds(objs):
    pts = []
    for o in objs:
        if o.type == 'MESH':
            pts += [o.matrix_world @ Vector(c) for c in o.bound_box]
    mn = Vector((min(p.x for p in pts), min(p.y for p in pts), min(p.z for p in pts)))
    mx = Vector((max(p.x for p in pts), max(p.y for p in pts), max(p.z for p in pts)))
    a, b = to_gltf(mn), to_gltf(mx)
    lo = [min(a[i], b[i]) for i in range(3)]
    hi = [max(a[i], b[i]) for i in range(3)]
    return {'min': lo, 'max': hi, 'size': [round(hi[i] - lo[i], 4) for i in range(3)]}


def save_textures(tex_dir):
    return sorted(Path(bpy.path.abspath(img.filepath)) for img in bpy.data.images
                  if img.source == 'FILE' and img.filepath)


def collision_for(name, spec):
    if name == 'hero-robot':
        k = L.Kit('robot_collision', 1.0)
        k.add('robot_collision', L.pbr('collision', (1, 0, 1), 1),
              L.lathe([(0, 0), (0.30, 0.0), (0.32, 0.06), (0.32, 1.70), (0.30, 1.76), (0, 1.76)], 12))
        k.finish()
        return k, {'type': 'capsule', 'radius': 0.32, 'height': 1.76, 'center': [0, 0.88, 0], 'blocking': True,
                   'note': 'same envelope as the legacy hero kit recommendation; seated: lower by 0.30 m'}
    if name == 'arrival-table':
        return F.table_collision()
    if name == 'arrival-chair':
        return F.chair_collision()
    sizes = {'arrival-folio': ((0.34, 0.25, 0.026), (0, 0, 0.013)),
             'arrival-cup': ((0.155, 0.155, 0.086), (0, 0, 0.043)),
             'arrival-lantern': ((0.145, 0.145, 0.29), (0, 0, 0.145)),
             'arrival-instrument': ((0.18, 0.18, 0.22), (0, 0, 0.11))}
    size, center = sizes[name]
    return F.box_collision(spec['root'], size, center, blocking=False)


def build_asset(name, spec, out, write_blend=True):
    tex_dir = out / 'textures'
    tex_dir.mkdir(parents=True, exist_ok=True)
    L.TEXTURE_DIR = tex_dir
    rec = {'name': name, 'root_node': spec['root'], 'lods': {}, 'budget': spec['budget']}
    for lod, detail in spec['lods'].items():
        L.reset_scene()
        F._SHARED.clear()
        kit = spec['build'](detail)
        objs = kit.objects
        glb = out / f'{name}.lod{lod}.glb'
        L.export_glb_fmt(glb, objs, spec['image_format'])
        info = inspect_glb(glb)
        info.update({'file': glb.name, 'detail': detail, 'blender_triangles': L.triangles(objs)})
        rec['lods'][f'lod{lod}'] = info
        if lod == 0:
            rec['bounds'] = world_bounds(objs)
            rec['pivot'] = {'origin': 'floor contact point under the asset centre', 'gltf': [0, 0, 0]}
            rec['nodes'] = [{'name': o.name, 'kind': 'pivot' if o.empty_display_type == 'SPHERE' else
                             ('anchor' if 'anchor' in o else 'root'),
                             'parent': o.parent.name if o.parent else None,
                             'gltf_position_in_root': to_gltf(o.matrix_world.translation)}
                            for o in objs if o.type == 'EMPTY']
            rec['anchors'] = []
            for o in objs:
                if o.type == 'EMPTY' and 'anchor' in o:
                    rec['anchors'].append({
                        'name': o.name, 'kind': o['anchor'], 'parent': o.parent.name,
                        'gltf_position_in_root': to_gltf(o.matrix_world.translation),
                        'gltf_yaw_rad': round(o.matrix_world.to_euler('XYZ').z, 4),
                        'note': o.get('note', '')})
            ckit, cspec = collision_for(name, spec)
            cglb = out / f'{name}.collision.glb'
            L.export_glb_fmt(cglb, ckit.objects, 'AUTO')
            cinfo = inspect_glb(cglb)
            rec['collision'] = dict(cspec, file=cglb.name, bytes=cinfo['bytes'], sha256=cinfo['sha256'],
                                    triangles=cinfo['triangles'])
            textures = save_textures(tex_dir)
            if write_blend:
                bpy.context.preferences.filepaths.save_version = 0   # no .blend1 backups
                blend = out / f'{name}.blend'
                bpy.ops.wm.save_as_mainfile(filepath=str(blend), relative_remap=True, compress=True)
                rec['source_blend'] = {'file': blend.name, 'sha256': L.sha256(blend), 'bytes': blend.stat().st_size,
                                       'contains': 'LOD0 + collision proxy; LOD1 is regenerated by the script'}
            rec['texture_sources'] = [{'file': f'textures/{p.name}', 'sha256': L.sha256(p)} for p in textures]
    return rec


def census_entry(rec, spec):
    b = spec['budget']
    l0, l1 = rec['lods']['lod0'], rec['lods']['lod1']
    allowance = spec.get('wardrobe_allowance', 0)
    max_px = max([max(i['width'], i['height']) for i in l0['images']] or [0])
    checks = {
        'lod0_triangles': {'value': l0['triangles'], 'with_wardrobe_allowance': l0['triangles'] + allowance,
                           'ceiling': b['lod0'], 'pass': l0['triangles'] + allowance <= b['lod0']},
        'lod1_triangles': {'value': l1['triangles'], 'ceiling': b['lod1'], 'pass': l1['triangles'] <= b['lod1']},
        'max_texture_px': {'value': max_px, 'ceiling': TEXTURE_MAX_PX, 'pass': max_px <= TEXTURE_MAX_PX},
        'lod0_bytes': {'value': l0['bytes'], 'ceiling': ASSET_BYTES_MAX, 'pass': l0['bytes'] <= ASSET_BYTES_MAX},
        'collision_present': {'value': rec['collision']['file'], 'pass': True},
        'anchors_present': {'value': len(rec['anchors']), 'pass': len(rec['anchors']) > 0},
    }
    return {
        'asset': rec['name'], 'budget_source': b['source'], 'budget_note': b.get('note', ''),
        'lod0': {k: l0[k] for k in ('file', 'triangles', 'primitives_draw_calls', 'bytes', 'materials',
                                    'texture_decoded_bytes')},
        'lod1': {k: l1[k] for k in ('file', 'triangles', 'primitives_draw_calls', 'bytes', 'materials',
                                    'texture_decoded_bytes')},
        'textures_lod0': [{k: i[k] for k in ('name', 'mime', 'width', 'height', 'colorspace', 'bytes')}
                          for i in l0['images']],
        'checks': checks, 'pass': all(c['pass'] for c in checks.values()),
    }


def main():
    argv = sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else []
    out = REPO / 'assets/world/detail-v4/hero'
    only = None
    write_blend = True
    allow = False
    i = 0
    while i < len(argv):
        a = argv[i]
        if a == '--out':
            out = Path(argv[i + 1]).resolve(); i += 1
        elif a == '--assets':
            only = argv[i + 1].split(','); i += 1
        elif a == '--no-blend':
            write_blend = False
        elif a == '--allow-out':
            allow = True
        i += 1
    if not allow and not str(out).endswith('detail-v4/hero'):
        raise SystemExit(f'refusing to write outside a detail-v4/hero directory: {out} (use --allow-out)')
    if 'public/world' in str(out):
        raise SystemExit('refusing to write into public/world (W6 publishes)')
    out.mkdir(parents=True, exist_ok=True)
    names = [n for n in ASSETS if only is None or n in only]
    records = {}
    man_path = out / 'manifest.json'
    if only and man_path.exists():
        records = {a['name']: a for a in json.loads(man_path.read_text())['assets']}
    for n in names:
        print(f'[W3] building {n}', flush=True)
        records[n] = build_asset(n, ASSETS[n], out, write_blend)
    ordered = [records[n] for n in ASSETS if n in records]
    seated = R.solve_seated(table_reach=F.HAND_REACH, table_top=F.TABLE_TOP, seat_drop=F.SEAT_DROP)
    script_hashes = {p.name: L.sha256(p) for p in sorted(HERE.glob('*.py'))}
    manifest = {
        'schema': 'permagent.world.detail-v4.hero.v1',
        'lane': 'W3 (Astra ruling 2026-10-12)',
        'status': 'candidate — not published; W6 integrates and publishes',
        'generated': datetime.datetime.now(datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ'),
        'blender': bpy.app.version_string,
        'metersPerUnit': 1, 'runtimeUp': 'Y', 'sourceUp': 'Z', 'forward': 'glTF +Z (Blender -Y)',
        'source_scripts': {f'scripts/blender/world_detail/{k}': v for k, v in script_hashes.items()},
        'provenance': {'geometry': 'authored procedurally in Blender by these scripts; no third-party or generated meshes',
                       'textures': 'authored procedurally (numpy) by detail_lib.py / hero_lib.py (walnut v2, leather, '
                                   'palettes), except the chair upholstery base colour, which is the accepted W5 '
                                   'candidate textures/w5_weave_a_basecolor.png (FAL nano-banana-pro, see '
                                   'external_textures and textures/w5_weave_a_basecolor.provenance.json)',
                       'external_textures': [F.WEAVE_SOURCE],
                       'license': 'project-authored geometry and procedural maps; W5 weave under FAL hosted-output '
                                  'commercial-use terms (assets/world/realism-20261012/RIGHTS.md)',
                       'art_direction': 'docs/design/solar-forum/north-star.png; legacy hero-kit v2/v3b robot'},
        'rig_contract': {
            'skinning': 'none; rigid nested pivots, no animation clips',
            'legacy_nodes_preserved': ['robot', 'Head', 'Arm.L', 'Arm.R', 'Leg.L', 'Leg.R'],
            'additive_pivots': ['Forearm.L', 'Forearm.R', 'Hand.L', 'Hand.R', 'Shin.L', 'Shin.R', 'Foot.L', 'Foot.R'] +
                               [f"{dg['name']}.{sd}" for sd in ('L', 'R') for dg in R.DIGITS],
            'digits': {
                'pivots': 'one rigid pivot per digit at the knuckle (Thumb = CMC, fingers = MCP), child of Hand.L/R; '
                          'rest pose = identity = relaxed curl built into the phalanges',
                'axes': 'curl = rotation.x (positive closes towards the palm); spread = rotation.z (positive tilts '
                        'the tip towards glTF +X)',
                'limits_rad': R.DIGIT_LIMITS,
                'anchors': 'anchor_tip_<digit>_<L|R> under each digit pivot; anchor_fingertips_<L|R> (legacy name) '
                           'is now under Middle.<L|R>',
                'lod': 'LOD0: one ceramic primitive per digit (posable); LOD1/LOD2: digits fused into the Hand mesh, '
                       'pivots and anchors kept so poses never reference a missing node',
                'poses': {n: p for n, p in R.HAND_POSES.items()},
            },
            'material_prefixes': {'Ceramic': 'per-agent tint + shadow caster', 'Faceplate': 'forced dark glass',
                                  'Eyes': 'emissive agent colour; blink by mesh scale.y (origin on eye centre line)'},
            'mouth': 'Head-local glTF (0,-0.085,0.307) is free for the runtime voice mouth; '
                     'no Head_Brass mesh, so replaceFixedHeroMouth() is a no-op',
            'rotation_sign': 'joint angles are rotation about the node local X (three.js rotation.x); negative swings forward',
        },
        'poses': {'seated_at_arrival_table': {
            'description': 'robot placed at a chair anchor_seat with the root lowered; soles on the floor, palms flat '
                           'on the table 0.38 m in front of the root',
            'root_offset_y': seated['robot.y_offset'],
            'rotation_x': {k: round(v, 4) for k, v in seated.items() if '.' in k and k != 'robot.y_offset'},
            'check': seated['_check']}},
        'ensemble': {
            'table_radius': F.TABLE_R, 'table_top': F.TABLE_TOP, 'seat_radius': F.SEAT_RADIUS,
            'seat_top': F.SEAT_TOP, 'seats_deg_from_host': F.SEATS,
            'legacy_runtime_note': 'habitatLayout.ts currently places the old table scaled 1.2 (r 1.34, top 0.99) '
                                   'with ORCHESTRATOR_TABLE_SEAT 1.9 m from its centre and a standing-height seated '
                                   'body. This ensemble is lower and tighter so the robot really sits: W6 must move '
                                   'the seat to 1.25 m and use r 1.0 / h 0.60 colliders when adopting it.'},
        'assets': ordered,
    }
    man_path.write_text(json.dumps(manifest, indent=2) + '\n')
    census = {
        'schema': 'permagent.world.detail-v4.hero.census.v1',
        'generated': manifest['generated'],
        'budgets_reference': '/Users/j/Documents/dev/permagent-worktrees/wq-reports/astra/ruling-world-production-20261012.md section 2 Budgets',
        'assets': [census_entry(r, ASSETS[r['name']]) for r in ordered],
    }
    census['totals'] = {
        'lod0_bytes': sum(a['lod0']['bytes'] for a in census['assets']),
        'lod1_bytes': sum(a['lod1']['bytes'] for a in census['assets']),
        'lod0_triangles_one_of_each': sum(a['lod0']['triangles'] for a in census['assets']),
        'lod0_texture_decoded_bytes': sum(a['lod0']['texture_decoded_bytes'] for a in census['assets']),
        'all_pass': all(a['pass'] for a in census['assets']),
    }
    (out / 'census.json').write_text(json.dumps(census, indent=2) + '\n')
    for a in census['assets']:
        print(f"[W3] {a['asset']:20s} lod0 {a['lod0']['triangles']:6d} tris {a['lod0']['primitives_draw_calls']:3d} prims "
              f"{a['lod0']['bytes']:8d} B | lod1 {a['lod1']['triangles']:6d} | pass={a['pass']}")
    print('[W3] totals', json.dumps(census['totals']))


if __name__ == '__main__':
    main()
