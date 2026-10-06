"""Bounded geometry/image/material GLB assembly. Does not modify kit v2."""
import json,struct,hashlib,copy,shutil
from pathlib import Path
BASE=Path(__file__).resolve().parent.parent
assert shutil.disk_usage(BASE).free>=3*1024**3,'Disk floor'
out={'asset':{'version':'2.0','generator':'World kit v3b local assembly; Blender-authored geometry'},'scene':0,'scenes':[{'nodes':[]}],'nodes':[],'meshes':[],'materials':[],'images':[],'textures':[],'samplers':[],'accessors':[],'bufferViews':[]}
binary=bytearray();cache={'materials':{},'images':{},'textures':{},'samplers':{}};extensions=set();sources={}
def key(o):return json.dumps(o,sort_keys=True,separators=(',',':'))
def dedupe(kind,obj,identity=None):
 k=identity or key(obj)
 if k not in cache[kind]:cache[kind][k]=len(out[kind]);out[kind].append(obj)
 return cache[kind][k]
def append_view(data,original):
 while len(binary)%4:binary.append(0)
 v=copy.deepcopy(original);v['buffer']=0;v['byteOffset']=len(binary);binary.extend(data);out['bufferViews'].append(v);return len(out['bufferViews'])-1
for name in ['bench','canopy','fern','flowers','instrument','planter','robot','shrub','table','vine']:
 p=BASE/'exports'/('foliage-v3b' if name in ['canopy','shrub'] else '')/f'{name}.glb';data=p.read_bytes();n=struct.unpack_from('<I',data,12)[0];j=json.loads(data[20:20+n]);off=20+n;size=struct.unpack_from('<I',data,off)[0];raw=data[off+8:off+8+size];sources[name]={'path':str(p),'bytes':len(data),'sha256':hashlib.sha256(data).hexdigest()};extensions.update(j.get('extensionsUsed',[]))
 imageviews={im['bufferView'] for im in j.get('images',[])};viewmap={}
 for i,v in enumerate(j['bufferViews']):
  if i not in imageviews:viewmap[i]=append_view(raw[v.get('byteOffset',0):v.get('byteOffset',0)+v['byteLength']],v)
 imagemap={}
 for i,im in enumerate(j.get('images',[])):
  assert 'uri' not in im;v=j['bufferViews'][im['bufferView']];imbytes=raw[v.get('byteOffset',0):v.get('byteOffset',0)+v['byteLength']];h=hashlib.sha256(imbytes).hexdigest()
  if h not in cache['images']:
   vnew=append_view(imbytes,v);newim=copy.deepcopy(im);newim['bufferView']=vnew;dedupe('images',newim,h)
  imagemap[i]=cache['images'][h]
 samplermap={i:dedupe('samplers',s) for i,s in enumerate(j.get('samplers',[]))}
 texmap={}
 for i,t in enumerate(j.get('textures',[])):
  t=copy.deepcopy(t);t['source']=imagemap[t['source']]
  if 'sampler' in t:t['sampler']=samplermap[t['sampler']]
  texmap[i]=dedupe('textures',t)
 def remap_material(obj):
  if isinstance(obj,dict):
   result={k:remap_material(v) for k,v in obj.items()}
   for k,v in result.items():
    if k.endswith('Texture') and isinstance(v,dict) and 'index' in v:v['index']=texmap[v['index']]
   return result
  if isinstance(obj,list):return [remap_material(v) for v in obj]
  return obj
 matmap={i:dedupe('materials',remap_material(m)) for i,m in enumerate(j['materials'])};ao=len(out['accessors']);mo=len(out['meshes']);no=len(out['nodes'])
 for a in j['accessors']:a=copy.deepcopy(a);a['bufferView']=viewmap[a['bufferView']];out['accessors'].append(a)
 for m in j['meshes']:
  m=copy.deepcopy(m)
  for pr in m['primitives']:pr['indices']+=ao;pr['material']=matmap[pr['material']];pr['attributes']={k:v+ao for k,v in pr['attributes'].items()}
  out['meshes'].append(m)
 for node in j['nodes']:
  node=copy.deepcopy(node)
  if 'mesh' in node:node['mesh']+=mo
  if 'children' in node:node['children']=[i+no for i in node['children']]
  out['nodes'].append(node)
 out['scenes'][0]['nodes'].extend(i+no for i in j['scenes'][0]['nodes'])
out['buffers']=[{'byteLength':len(binary)}]
if extensions:out['extensionsUsed']=sorted(extensions)
js=json.dumps(out,separators=(',',':')).encode();js+=b' '*((-len(js))%4);binary+=b'\0'*((-len(binary))%4);data=struct.pack('<4sII',b'glTF',2,28+len(js)+len(binary))+struct.pack('<II',len(js),0x4e4f534a)+js+struct.pack('<II',len(binary),0x004e4942)+binary
path=BASE/'exports'/'hero-kit-v3b.glb';path.write_bytes(data)
manifest={'version':'hero-kit-v3b','status':'improved draft foliage, parent approved advance; runtime/final acceptance open','file':str(path),'bytes':len(data),'sha256':hashlib.sha256(data).hexdigest(),'triangles':sum(out['accessors'][pr['indices']]['count']//3 for m in out['meshes'] for pr in m['primitives']),'mesh_primitives':sum(len(m['primitives']) for m in out['meshes']),'node_count':len(out['nodes']),'embedded_images':len(out['images']),'texture_bindings':len(out['textures']),'source_exports':sources,'editable_source':{'base_kit':str(BASE/'source'/'hero-kit.blend'),'robot':str(BASE/'source'/'robot-v2.blend'),'foliage':str(BASE/'source'/'foliage-v3b.blend'),'assembler':str(Path(__file__).resolve())},'atlas':{'source':str(BASE/'source'/'foliage-v3'/'broadleaf-atlas-v1.png'),'provenance':str(BASE/'source'/'foliage-v3'/'PROVENANCE.md'),'width':1254,'height':1254,'decoded_rgba_bytes':1254*1254*4,'mips_rgba_estimate_bytes':int(1254*1254*4*4/3),'alpha_mode':'MASK','alpha_cutoff':.55,'colorspace':'sRGB','deduplication':'identical embedded PNG byte hash; shared image, sampler, texture and material binding'},'collision':{'canopy_solid_radial_exclusion_metres':.49,'scale_applies':True,'route_clearance_add_player_radius_metres':.28,'below_2m_wood_radial_extent_metres':.4653503951,'lowest_primary_branch_underside_metres':1.555,'source_measurements':str(BASE/'foliage-v3b-stats.json')},'preserved_v2':str(BASE/'exports'/'hero-kit.glb')}
assert len(data)<=40*1024**2
(BASE/'hero-kit-v3b-manifest.json').write_text(json.dumps(manifest,indent=2));print(json.dumps(manifest,indent=2))
