import json,struct,hashlib
from pathlib import Path
BASE=Path(__file__).resolve().parent.parent
reports={}
for path in sorted((BASE/'exports').glob('*.glb')):
 data=path.read_bytes();magic,version,length=struct.unpack_from('<4sII',data)
 assert magic==b'glTF' and version==2 and length==len(data),path
 n,kind=struct.unpack_from('<II',data,12);assert kind==0x4e4f534a
 j=json.loads(data[20:20+n]);assert all('uri' not in im for im in j.get('images',[])),path
 assert all('uri' not in b for b in j['buffers']),path
 for view in j['bufferViews']:assert view.get('byteOffset',0)+view['byteLength']<=j['buffers'][view.get('buffer',0)]['byteLength']
 primitives=[p for m in j['meshes'] for p in m['primitives']]
 triangles=sum(j['accessors'][p['indices']]['count']//3 for p in primitives)
 roots=[j['nodes'][i]['name'] for i in j['scenes'][j.get('scene',0)]['nodes']]
 assert all('NORMAL' in p['attributes'] for p in primitives)
 reports[path.name]={'bytes':len(data),'triangles':triangles,'mesh_primitives':len(primitives),'mesh_nodes':sum('mesh' in n for n in j['nodes']),'roots':roots,'images':len(j.get('images',[])),'materials':len(j.get('materials',[])),'sha256':hashlib.sha256(data).hexdigest()}
 if path.name=='hero-kit.glb':
  assert set(roots)=={'robot','canopy','shrub','fern','vine','flowers','table','bench','planter','instrument'}
  assert {'Head','Arm.L','Arm.R','Leg.L','Leg.R'}.issubset({n['name'] for n in j['nodes']})
  assert all(m.get('doubleSided',False) for m in j['materials'] if m['name'].startswith('Leaf'))
total=sum(x['bytes'] for x in reports.values());assert total<=40*1024*1024,total
(BASE/'validation.json').write_text(json.dumps({'export_total_bytes':total,'checks':'GLB headers/lengths, embedded buffers/images, buffer ranges, normals, root/pivot names, double-sided foliage, 40MiB ceiling','files':reports},indent=2))
print(json.dumps({'export_total_bytes':total,'mono':reports['hero-kit.glb']},indent=2))
