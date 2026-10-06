"""Combine self-contained individual GLBs without importing the whole kit in Blender."""
import json,struct,hashlib,copy
from pathlib import Path
base=Path(__file__).resolve().parent.parent
out={'asset':{'version':'2.0','generator':'World hero kit authored Blender / local bounded assembly'},'scene':0,'scenes':[{'nodes':[]}],'nodes':[],'meshes':[],'materials':[],'accessors':[],'bufferViews':[]};binary=bytearray();material_names={}
for path in sorted((base/'exports').glob('*.glb')):
 if path.name=='hero-kit.glb':continue
 b=path.read_bytes();n=struct.unpack_from('<I',b,12)[0];j=json.loads(b[20:20+n]);off=20+n;size=struct.unpack_from('<I',b,off)[0];raw=b[off+8:off+8+size]
 assert not j.get('images') and not j.get('textures'),'This bounded assembler expects current geometry-only assets'
 while len(binary)%4:binary.append(0)
 bo=len(binary);binary.extend(raw);vo=len(out['bufferViews']);ao=len(out['accessors']);mo=len(out['meshes']);no=len(out['nodes']);matmap={}
 for i,m in enumerate(j['materials']):
  if m['name'] not in material_names:material_names[m['name']]=len(out['materials']);out['materials'].append(m)
  matmap[i]=material_names[m['name']]
 for v in j['bufferViews']:v=copy.deepcopy(v);v['buffer']=0;v['byteOffset']=v.get('byteOffset',0)+bo;out['bufferViews'].append(v)
 for a in j['accessors']:a=copy.deepcopy(a);a['bufferView']+=vo;out['accessors'].append(a)
 for m in j['meshes']:
  m=copy.deepcopy(m)
  for p in m['primitives']:
   p['indices']+=ao;p['material']=matmap[p['material']];p['attributes']={k:v+ao for k,v in p['attributes'].items()}
  out['meshes'].append(m)
 for node in j['nodes']:
  node=copy.deepcopy(node)
  if 'mesh' in node:node['mesh']+=mo
  if 'children' in node:node['children']=[i+no for i in node['children']]
  out['nodes'].append(node)
 out['scenes'][0]['nodes'].extend(i+no for i in j['scenes'][0]['nodes'])
out['buffers']=[{'byteLength':len(binary)}];js=json.dumps(out,separators=(',',':')).encode();js+=b' '*((-len(js))%4);binary+=b'\0'*((-len(binary))%4)
b=struct.pack('<4sII',b'glTF',2,28+len(js)+len(binary))+struct.pack('<II',len(js),0x4e4f534a)+js+struct.pack('<II',len(binary),0x004e4942)+binary
(base/'exports'/'hero-kit.glb').write_bytes(b)
manifest=json.loads((base/'manifest.json').read_text());manifest['version']='hero-kit-v2-robot-panels';manifest['editable_robot_source']=str(base/'source'/'robot-v2.blend');manifest['mono_kit_bytes']=len(b)
for name,a in manifest['assets'].items():
 p=base/'exports'/f'{name}.glb';bb=p.read_bytes();size=struct.unpack_from('<I',bb,12)[0];g=json.loads(bb[20:20+size]);a['bytes']=len(bb);a['sha256']=hashlib.sha256(bb).hexdigest();a['triangles']=sum(g['accessors'][pr['indices']]['count']//3 for m in g['meshes'] for pr in m['primitives'])
(base/'manifest.json').write_text(json.dumps(manifest,indent=2));print('ASSEMBLED',len(b),'bytes')
