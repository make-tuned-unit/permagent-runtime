"""Low-memory glTF mesh batching; preserves robot rigid joint pivots."""
import json,struct,math,hashlib
from pathlib import Path
BASE=Path(__file__).resolve().parent.parent
def transform(n):
 x,y,z,w=n.get('rotation',[0,0,0,1]);s=n.get('scale',[1,1,1]);t=n.get('translation',[0,0,0])
 r=[[1-2*(y*y+z*z),2*(x*y-z*w),2*(x*z+y*w)],[2*(x*y+z*w),1-2*(x*x+z*z),2*(y*z-x*w)],[2*(x*z-y*w),2*(y*z+x*w),1-2*(x*x+y*y)]]
 return r,s,t
def readglb(p):
 b=p.read_bytes();size=struct.unpack_from('<I',b,12)[0];j=json.loads(b[20:20+size]);off=20+size;size=struct.unpack_from('<I',b,off)[0];return j,b[off+8:off+8+size]
def batch(p):
 j,raw=readglb(p);out=bytearray();views=[];access=[];meshes=[];nodes=[]
 def values(ai):
  a=j['accessors'][ai];v=j['bufferViews'][a['bufferView']];dim={'SCALAR':1,'VEC2':2,'VEC3':3,'VEC4':4}[a['type']];fmt={5126:'f',5125:'I',5123:'H',5121:'B'}[a['componentType']];stride=v.get('byteStride',struct.calcsize(fmt)*dim);start=v.get('byteOffset',0)+a.get('byteOffset',0)
  return [struct.unpack_from('<'+fmt*dim,raw,start+i*stride) for i in range(a['count'])]
 def add(vals,typ,component=5126):
  while len(out)%4:out.append(0)
  offset=len(out);fmt='f' if component==5126 else 'I';dim={'SCALAR':1,'VEC2':2,'VEC3':3}[typ]
  for v in vals:out.extend(struct.pack('<'+fmt*dim,*v))
  views.append({'buffer':0,'byteOffset':offset,'byteLength':len(out)-offset});a={'bufferView':len(views)-1,'componentType':component,'count':len(vals),'type':typ}
  if typ=='VEC3':a.update(min=[min(v[k] for v in vals) for k in range(3)],max=[max(v[k] for v in vals) for k in range(3)])
  access.append(a);return len(access)-1
 def visit(oi):
  old=j['nodes'][oi];ni=len(nodes);nodes.append({k:v for k,v in old.items() if k not in ['children','mesh']});groups={};children=[]
  for ci in old.get('children',[]):
   c=j['nodes'][ci]
   if 'mesh' not in c:children.append(visit(ci));continue
   assert not c.get('children');r,s,t=transform(c)
   for prim in j['meshes'][c['mesh']]['primitives']:
    key=prim['material'];g=groups.setdefault(key,{'p':[],'n':[],'uv':[],'i':[]});offset=len(g['p']);attrs=prim['attributes'];pos=values(attrs['POSITION']);norm=values(attrs['NORMAL']);uv=values(attrs['TEXCOORD_0']) if 'TEXCOORD_0' in attrs else [(0,0)]*len(pos)
    for v,no,tex in zip(pos,norm,uv):
     g['p'].append(tuple(sum(r[a][k]*s[k]*v[k] for k in range(3))+t[a] for a in range(3)))
     q=[sum(r[a][k]*no[k]/s[k] for k in range(3)) for a in range(3)];mag=math.sqrt(sum(x*x for x in q));g['n'].append(tuple(x/mag for x in q));g['uv'].append(tex)
    g['i'].extend((v[0]+offset,) for v in values(prim['indices']))
  for material,g in groups.items():
   primitive={'attributes':{'POSITION':add(g['p'],'VEC3'),'NORMAL':add(g['n'],'VEC3'),'TEXCOORD_0':add(g['uv'],'VEC2')},'indices':add(g['i'],'SCALAR',5125),'material':material}
   meshes.append({'name':old['name']+' surface '+str(material),'primitives':[primitive]});children.append(len(nodes));nodes.append({'name':old['name']+'_'+j['materials'][material]['name'],'mesh':len(meshes)-1})
  if children:nodes[ni]['children']=children
  return ni
 scene=[visit(i) for i in j['scenes'][0]['nodes']]
 result={'asset':j['asset'],'scene':0,'scenes':[{'nodes':scene}],'nodes':nodes,'meshes':meshes,'materials':j['materials'],'buffers':[{'byteLength':len(out)}],'bufferViews':views,'accessors':access}
 js=json.dumps(result,separators=(',',':')).encode();js+=b' '*((-len(js))%4);out+=b'\0'*((-len(out))%4)
 b=struct.pack('<4sII',b'glTF',2,28+len(js)+len(out))+struct.pack('<II',len(js),0x4e4f534a)+js+struct.pack('<II',len(out),0x004e4942)+out;p.write_bytes(b)
 print(p.name,len(b),'nodes',len(nodes),'draws',len(meshes))
for p in (BASE/'exports').glob('*.glb'):batch(p)
j=json.loads((BASE/'manifest.json').read_text());j['version']='hero-kit-v1.1-batched'
for a in j['assets'].values():
 p=Path(a['file']);a['bytes']=p.stat().st_size;a['sha256']=hashlib.sha256(p.read_bytes()).hexdigest()
j['mono_kit_bytes']=(BASE/'exports'/'hero-kit.glb').stat().st_size;(BASE/'manifest.json').write_text(json.dumps(j,indent=2))
