import json,struct,hashlib
from pathlib import Path
base=Path(__file__).resolve().parent.parent;p=base/'exports'/'hero-kit-v3b.glb';data=p.read_bytes();magic,version,length=struct.unpack_from('<4sII',data);assert magic==b'glTF' and version==2 and length==len(data)
n=struct.unpack_from('<I',data,12)[0];j=json.loads(data[20:20+n]);manifest=json.loads((base/'hero-kit-v3b-manifest.json').read_text());assert hashlib.sha256(data).hexdigest()==manifest['sha256']
roots=[j['nodes'][i]['name'] for i in j['scenes'][0]['nodes']];assert set(roots)=={'robot','canopy','shrub','fern','vine','flowers','table','bench','planter','instrument'};assert {'Head','Arm.L','Arm.R','Leg.L','Leg.R'}.issubset({x['name'] for x in j['nodes']})
for v in j['bufferViews']:assert v.get('byteOffset',0)+v['byteLength']<=j['buffers'][0]['byteLength']
assert all('uri' not in x for x in j['images']+j['buffers']);assert len(j['images'])==1 and len(j['textures'])==1
for a in j['accessors']:assert 0<=a['bufferView']<len(j['bufferViews'])
for m in j['meshes']:
 for pr in m['primitives']:
  assert 'NORMAL' in pr['attributes'];assert 0<=pr['material']<len(j['materials']);assert all(0<=v<len(j['accessors']) for v in pr['attributes'].values())
leaf=[m for m in j['materials'] if m['name']=='Leaf • approved RGBA atlas'];assert len(leaf)==1;assert leaf[0]['alphaMode']=='MASK' and leaf[0]['alphaCutoff']==.55 and leaf[0]['doubleSided'];assert leaf[0]['pbrMetallicRoughness']['baseColorTexture']['index']==0
assert hashlib.sha256((base/'exports'/'hero-kit.glb').read_bytes()).hexdigest()=='99a1ad1632e4a8adc8820767f6b684321a37b945692d3aa019e1c9d3a7f57139','v2 must remain preserved'
assert manifest['bytes']<=40*1024**2
report={'file':str(p),'sha256':manifest['sha256'],'bytes':len(data),'triangles':manifest['triangles'],'mesh_primitives':manifest['mesh_primitives'],'embedded_images':len(j['images']),'texture_bindings':len(j['textures']),'samplers':len(j['samplers']),'atlas_rgba_bytes':manifest['atlas']['decoded_rgba_bytes'],'atlas_mips_rgba_estimate_bytes':manifest['atlas']['mips_rgba_estimate_bytes'],'v2_hash_preserved':True,'checks':['GLB header/version/length','buffer views/accessor and material references','embedded media only','all roots and robot pivots','single shared atlas image/texture/material','double-sided alpha MASK.55','normals','v2 unchanged','40MiB ceiling'],'runtime_acceptance':False}
(base/'hero-kit-v3b-validation.json').write_text(json.dumps(report,indent=2));print(json.dumps(report,indent=2))
