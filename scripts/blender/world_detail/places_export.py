"""Lane W4 shared export/inspection helpers (copied from W3's build_hero.py
inspection so W4 does not depend on W3's entry script), plus the merged
top-level environment manifest/census writer used by both W4 builders."""
import json
import struct
import datetime
from pathlib import Path
from mathutils import Vector
import detail_lib as L

ENV_SCHEMA = 'permagent.world.detail-v4.environment.v1'


def to_gltf(v):
    return [round(v[0], 4), round(v[2], 4), round(-v[1], 4)]


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
    if data[:4] == b'RIFF' and data[8:12] == b'WEBP':
        kind = data[12:16]
        if kind == b'VP8X':
            w = 1 + int.from_bytes(data[24:27], 'little')
            h = 1 + int.from_bytes(data[27:30], 'little')
        elif kind == b'VP8L':
            b = int.from_bytes(data[21:25], 'little')
            w, h = (b & 0x3FFF) + 1, ((b >> 14) & 0x3FFF) + 1
        else:
            w = int.from_bytes(data[26:28], 'little') & 0x3FFF
            h = int.from_bytes(data[28:30], 'little') & 0x3FFF
        return w, h, 'image/webp'
    return 0, 0, 'unknown'


def inspect_glb(path):
    raw = Path(path).read_bytes()
    magic, version, length = struct.unpack('<4sII', raw[:12])
    assert magic == b'glTF' and version == 2 and length == len(raw), 'bad GLB header'
    jlen, _ = struct.unpack('<I4s', raw[12:20])
    gltf = json.loads(raw[20:20 + jlen])
    bin_off = 20 + jlen + 8
    tris = prims = 0
    for mesh in gltf.get('meshes', []):
        for p in mesh['primitives']:
            prims += 1
            acc = p['indices'] if 'indices' in p else p['attributes']['POSITION']
            tris += gltf['accessors'][acc]['count'] // 3
    def src(t):
        tex = gltf['textures'][t]
        return tex.get('source', tex.get('extensions', {}).get('EXT_texture_webp', {}).get('source'))

    colorspace = {}
    for mat in gltf.get('materials', []):
        pbr = mat.get('pbrMetallicRoughness', {})
        for key, cs in (('baseColorTexture', 'sRGB'), ('metallicRoughnessTexture', 'linear')):
            if key in pbr:
                colorspace[src(pbr[key]['index'])] = cs
        for key in ('normalTexture', 'occlusionTexture'):
            if key in mat:
                colorspace[src(mat[key]['index'])] = 'linear'
    images = []
    for i, img in enumerate(gltf.get('images', [])):
        bv = gltf['bufferViews'][img['bufferView']]
        o = bin_off + bv.get('byteOffset', 0)
        w, h, mime = _image_size(raw[o:o + bv['byteLength']])
        images.append({'name': img.get('name'), 'mime': img.get('mimeType', mime), 'width': w, 'height': h,
                       'bytes': bv['byteLength'], 'colorspace': colorspace.get(i, 'unused'),
                       'decoded_bytes_with_mips': int(w * h * 4 * 4 / 3)})
    mats = [{'name': m.get('name'), 'alphaMode': m.get('alphaMode', 'OPAQUE'), 'alphaCutoff': m.get('alphaCutoff'),
             'doubleSided': m.get('doubleSided', False)} for m in gltf.get('materials', [])]
    return {'bytes': len(raw), 'sha256': L.sha256(path), 'triangles': tris, 'primitives_draw_calls': prims,
            'meshes': len(gltf.get('meshes', [])), 'nodes': len(gltf.get('nodes', [])),
            'materials': [m['name'] for m in mats], 'extensions_required': gltf.get('extensionsRequired', []), 'material_modes': mats, 'images': images,
            'texture_decoded_bytes': sum(i['decoded_bytes_with_mips'] for i in images),
            'node_names': [n.get('name') for n in gltf.get('nodes', [])]}


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


def anchors_of(objs):
    out = []
    for o in objs:
        if o.type == 'EMPTY' and 'anchor' in o:
            rec = {'name': o.name, 'kind': o['anchor'], 'parent': o.parent.name if o.parent else None,
                   'gltf_position_in_root': to_gltf(o.matrix_world.translation),
                   'gltf_yaw_rad': round(o.matrix_world.to_euler('XYZ').z, 4), 'note': o.get('note', '')}
            for k in o.keys():
                if k.startswith('wind_') or k.startswith('size_') or k in ('normal', 'faces'):
                    v = o[k]
                    rec[k] = list(v) if hasattr(v, '__len__') and not isinstance(v, str) else v
            out.append(rec)
    return out


def guard_out(out, leaf):
    s = str(Path(out).resolve())
    if 'public/world' in s or 'ui/command-center' in s:
        raise SystemExit('refusing to write into the published tree (W6 publishes)')
    if not s.endswith(f'detail-v4/environment/{leaf}') and not s.endswith(f'detail-v4/environment'):
        raise SystemExit(f'refusing to write outside detail-v4/environment: {s}')


def write_top_level(env_dir):
    """Merge botany/ and places/ sub-manifests into environment/manifest.json and census.json."""
    env = Path(env_dir)
    subs = {}
    for leaf in ('botany', 'places'):
        p = env / leaf / 'manifest.json'
        c = env / leaf / 'census.json'
        if p.exists():
            subs[leaf] = (json.loads(p.read_text()), json.loads(c.read_text()) if c.exists() else None)
    now = datetime.datetime.now(datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')
    man = {'schema': ENV_SCHEMA, 'lane': 'W4 (Astra ruling 2026-10-12)', 'generated': now,
           'status': 'candidate — not published; W6 integrates placements/colliders and publishes',
           'metersPerUnit': 1, 'runtimeUp': 'Y', 'forward': 'glTF +Z (Blender -Y)',
           'place_ids': ['commons', 'gallery', 'maker', 'council_garden', 'observatory_garden', 'conservatory'],
           'parts': {k: f'{k}/manifest.json' for k in subs}}
    for k, (m, _) in subs.items():
        man[k] = {'assets': [{'name': a['name'], 'class': a.get('class'), 'lod0': f"{k}/{a['lods']['lod0']['file']}",
                              'lod1': f"{k}/{a['lods']['lod1']['file']}",
                              'collision': f"{k}/{a['collision']['file']}" if a.get('collision', {}).get('file') else None,
                              'bounds': a.get('bounds'), 'anchors': [x['name'] for x in a.get('anchors', [])]}
                             for a in m['assets']]}
        if k == 'botany':
            man['scatter_spec'] = 'botany/scatter.json'
        if k == 'places':
            man['suggested_placements'] = m.get('suggested_placements')
            man['route_clearance'] = m.get('route_clearance')
    (env / 'manifest.json').write_text(json.dumps(man, indent=2) + '\n')
    census = {'schema': ENV_SCHEMA + '.census', 'generated': now,
              'budgets_reference': 'wq-reports/astra/ruling-world-production-20261012.md section 2 Budgets', 'parts': {}}
    tot_b = 0
    for k, (_, c) in subs.items():
        if c:
            census['parts'][k] = c['totals']
            tot_b += c['totals']['lod0_bytes'] + c['totals']['lod1_bytes'] + c['totals'].get('collision_bytes', 0)
    census['totals'] = {'all_glb_bytes': tot_b,
                        'detail_kit_ceiling_bytes': 18 * 1024 * 1024,
                        'note': 'W4 share of the 18 MiB replacement detail kit (W3 hero kit ~2.6 MB of GLBs is separate)',
                        'all_pass': all(c['totals']['all_pass'] for _, c in subs.values() if c)}
    (env / 'census.json').write_text(json.dumps(census, indent=2) + '\n')
