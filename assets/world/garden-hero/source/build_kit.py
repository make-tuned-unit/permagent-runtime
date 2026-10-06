import bpy, math, random, json, os, hashlib
from mathutils import Vector
from pathlib import Path
random.seed(10626)
BASE=Path(__file__).resolve().parent.parent
bpy.ops.object.select_all(action='SELECT'); bpy.ops.object.delete(use_global=False)
def mat(n,c,rough=.5,metal=0):
 m=bpy.data.materials.new(n); m.diffuse_color=(*c,1); m.use_nodes=True
 p=m.node_tree.nodes.get('Principled BSDF'); p.inputs['Base Color'].default_value=(*c,1); p.inputs['Roughness'].default_value=rough; p.inputs['Metallic'].default_value=metal
 return m
ivory=mat('Ceramic • warm ivory',(.82,.79,.67),.25); dark=mat('Graphite • joint rubber',(.026,.038,.041),.65)
glass=mat('Faceplate • deep teal glass',(.012,.037,.043),.16,.28); brass=mat('Brass • brushed hardware',(.52,.31,.12),.33,.75)
eye=mat('Eyes • jade phosphor',(.14,.62,.48),.24); eye.node_tree.nodes['Principled BSDF'].inputs['Emission Color'].default_value=(.14,.62,.48,1); eye.node_tree.nodes['Principled BSDF'].inputs['Emission Strength'].default_value=.6
wood=mat('Timber • oiled walnut',(.22,.105,.044),.48); grain=mat('Timber • lighter grain',(.34,.19,.075),.52)
stone=mat('Stone • warm travertine',(.56,.51,.41),.82); soil=mat('Soil • humus',(.075,.043,.022),1); fabric=mat('Fabric • moss weave',(.20,.27,.17),.91)
bark=mat('Bark • cinnamon',(.17,.092,.039),.94)
leaves=[mat('Leaf • '+str(i),c,.69) for i,c in enumerate([(.09,.21,.065),(.15,.30,.085),(.25,.37,.10),(.105,.26,.14),(.32,.41,.15)])]
petal=mat('Flower • apricot',(.82,.39,.14),.68)
roots={}; current=None
def root(n):
 global current
 o=bpy.data.objects.new(n,None); bpy.context.collection.objects.link(o); roots[n]=o; current=o; return o
def finish(o,n,m,parent=None):
 o.name=n; o.data.materials.append(m); o.parent=parent or current
 if o.type=='MESH':
  for p in o.data.polygons:p.use_smooth=True
 return o
def ell(n,loc,s,m,parent=None):
 bpy.ops.mesh.primitive_uv_sphere_add(segments=24,ring_count=16,location=loc); o=bpy.context.object; o.scale=s; return finish(o,n,m,parent)
def box(n,loc,s,m,bevel=.04,parent=None):
 bpy.ops.mesh.primitive_cube_add(size=1,location=loc);o=bpy.context.object;o.dimensions=s; bpy.ops.object.transform_apply(location=False,rotation=False,scale=True)
 mod=o.modifiers.new('Soft manufactured edges','BEVEL');mod.width=bevel;mod.segments=3;bpy.context.view_layer.objects.active=o;bpy.ops.object.modifier_apply(modifier=mod.name)
 mod=o.modifiers.new('Weighted normals','WEIGHTED_NORMAL');return finish(o,n,m,parent)
def tube(n,a,b,r,m,r2=None,parent=None,verts=12):
 a,b=Vector(a),Vector(b); d=b-a;bpy.ops.mesh.primitive_cone_add(vertices=verts,radius1=r,radius2=r if r2 is None else r2,depth=d.length,location=(a+b)/2);o=bpy.context.object;o.rotation_euler=d.to_track_quat('Z','Y').to_euler();return finish(o,n,m,parent)
def torus(n,loc,major,minor,m,rot=None,parent=None):
 bpy.ops.mesh.primitive_torus_add(major_segments=36,minor_segments=8,location=loc,major_radius=major,minor_radius=minor);o=bpy.context.object
 if rot:o.rotation_euler=rot
 return finish(o,n,m,parent)
def pivot(n,loc):
 o=bpy.data.objects.new(n,None);bpy.context.collection.objects.link(o);o.parent=current;o.location=loc;return o
def reparent_world(o,p):
 bpy.context.view_layer.update();w=o.matrix_world.copy();o.parent=p;o.matrix_world=w;bpy.context.view_layer.update()
root('robot')
ell('Torso', (0,0,.89),(.255,.19,.33),ivory)
ell('Chest inset',(0,-.173,1.00),(.17,.025,.13),dark)
torus('Collar',(0,0,1.19),.14,.028,brass)
tube('Neck',(0,0,1.15),(0,0,1.33),.085,dark)
h=pivot('Head',(0,0,1.45))
for o in [ell('Head ceramic shell',(0,0,1.47),(.315,.27,.295),ivory),ell('Faceplate',(0,-.235,1.46),(.25,.068,.208),glass),ell('Eye.L',(-.10,-.299,1.49),(.036,.015,.065),eye),ell('Eye.R',(.10,-.299,1.49),(.036,.015,.065),eye),ell('Mouth indicator',(0,-.301,1.365),(.045,.008,.009),brass)]:reparent_world(o,h)
for sign,side in [(-1,'L'),(1,'R')]:
 o=ell('Ear.'+side,(sign*.306,0,1.48),(.032,.12,.12),brass);reparent_world(o,h)
 o=ell('Ear inset.'+side,(sign*.333,-.015,1.48),(.012,.077,.079),dark);reparent_world(o,h)
 arm=pivot('Arm.'+side,(sign*.27,0,1.07))
 for o in [ell('Shoulder.'+side,(sign*.28,0,1.07),(.087,.087,.09),dark),ell('Upper arm shell.'+side,(sign*.33,-.01,.95),(.075,.079,.16),ivory),ell('Elbow.'+side,(sign*.35,-.02,.82),(.061,.065,.067),brass),ell('Forearm shell.'+side,(sign*.36,-.07,.72),(.071,.074,.13),ivory),ell('Palm.'+side,(sign*.36,-.10,.60),(.066,.036,.058),dark)]:reparent_world(o,arm)
 for i in range(3):reparent_world(box('Finger.'+side+str(i),(sign*.36+(i-1)*.032,-.11,.565),(.025,.035,.057),ivory,.012),arm)
 reparent_world(ell('Thumb.'+side,(sign*.425,-.11,.61),(.022,.035,.04),ivory),arm)
 leg=pivot('Leg.'+side,(sign*.125,0,.68))
 for o in [ell('Hip.'+side,(sign*.125,0,.64),(.087,.085,.08),dark),ell('Thigh.'+side,(sign*.125,0,.49),(.091,.092,.16),ivory),ell('Knee.'+side,(sign*.125,-.009,.345),(.079,.080,.068),brass),ell('Shin.'+side,(sign*.125,-.005,.215),(.086,.086,.14),ivory),box('Foot sole.'+side,(sign*.125,-.06,.044),(.19,.29,.067),dark,.026),ell('Foot shell.'+side,(sign*.125,-.067,.086),(.096,.15,.067),ivory)]:reparent_world(o,leg)
for z in [.79,.84,.89]:box('Vent',(0,.183,z),(.11,.016,.009),dark,.003)
ell('Status medallion',(0,-.195,.89),(.027,.014,.027),brass)
# Curved two-sided leaf silhouette: authored midrib and curled tips, no alpha cards.
leaf_mesh=[]
for i in range(5):
 verts=[(0,0,0),(.25,-.34,.06),(.55,-.29,.095),(1,0,.035),(.55,.29,.095),(.25,.34,.06),(.45,0,.13)]
 mesh=bpy.data.meshes.new('Lanceolate curled leaf '+str(i));mesh.from_pydata(verts,[],[(0,1,6),(1,2,6),(2,3,6),(3,4,6),(4,5,6),(5,0,6)]);mesh.materials.append(leaves[i]);leaf_mesh.append(mesh)
def leaf(n,p,d,length,width=1):
 o=bpy.data.objects.new(n,random.choice(leaf_mesh));bpy.context.collection.objects.link(o);o.parent=current;o.location=p;o.rotation_euler=Vector(d).to_track_quat('X','Z').to_euler();o.scale=(length,length*width,length);return o
def spray(p,d,length,count=12,leafsize=.19):
 p=Vector(p);d=Vector(d);end=p+d*length;tube('Fine woody twig',p,end,.012,bark,.003,verts=7)
 for i in range(count):
  t=(i+.3)/count;q=p+(end-p)*t;ang=i*2.399;v=Vector((math.cos(ang),math.sin(ang),random.uniform(.05,.6)))
  leaf('Broadleaf',q,v,leafsize*random.uniform(.72,1.25))
 return end
root('canopy')
trunk=[(0,0,0),(.09,.02,.65),(-.02,.05,1.5),(.13,.04,2.35),(0,.06,3.15)]
for i in range(4):tube('Tapered trunk',trunk[i],trunk[i+1],.14-i*.028,bark,.112-i*.028)
for i in range(14):
 a=i*2.399;z=1.45+i*.10;base=Vector((.04,0,z));end=base+Vector((math.cos(a)*random.uniform(.85,1.35),math.sin(a)*random.uniform(.85,1.35),random.uniform(.6,1.2)))
 tube('Curving primary branch',base,(base+end)/2+Vector((0,0,-.15)),.042,bark,.023);tube('Branch tip',(base+end)/2+Vector((0,0,-.15)),end,.023,bark,.006)
 for j in range(6):
  q=end+Vector((random.uniform(-.3,.3),random.uniform(-.3,.3),random.uniform(-.15,.35)))
  spray(q,(math.cos(a+j),math.sin(a+j),.35),random.uniform(.32,.56),10,.20)
for a in range(6):tube('Root flare',(0,0,.15),(.32*math.cos(a),.32*math.sin(a),.02),.07,bark,.012)
root('shrub')
for i in range(25):
 a=i*2.399; p=Vector((math.cos(a)*.13,math.sin(a)*.13,0)); d=Vector((math.cos(a)*.65,math.sin(a)*.65,1));spray(p,d,random.uniform(.35,.72),12,.14)
root('fern')
for j in range(11):
 a=j*2.399;points=[]
 for i in range(9):
  t=i/8;points.append(Vector((math.cos(a)*t*.59,math.sin(a)*t*.59,math.sin(t*math.pi*.86)*.43+.035)))
 for i in range(8):
  tube('Fern rachis',points[i],points[i+1],.006,leaves[0],.003,verts=6)
  for s in [-1,1]:leaf('Fern pinna',points[i+1],(math.cos(a+s*1.1),math.sin(a+s*1.1),.15),(.17*(1-i/10)),.45)
root('vine')
for j in range(3):
 last=(j*.15,0,1.6)
 for i in range(22):
  q=(j*.15+math.sin(i*.7+j)*.10,math.cos(i*.42+j)*.09,1.6-i*.075)
  tube('Vine stem',last,q,.008,bark,.007,verts=6);leaf('Vine leaf',q,(math.cos(i*2.399),-1,.2),random.uniform(.12,.19),1.25);last=q
root('flowers')
for j in range(9):
 a=j*2.399;q=(math.cos(a)*.23,math.sin(a)*.23,.27+random.random()*.19);tube('Flower stem',(q[0],q[1],0),q,.005,leaves[1],verts=6)
 for k in range(5):
  b=k*math.tau/5;ell('Apricot petal',(q[0]+math.cos(b)*.027,q[1]+math.sin(b)*.027,q[2]),(.025,.022,.010),petal)
 ell('Pollen heart',q,(.012,.012,.012),brass);leaf('Flower basal leaf',(q[0],q[1],.07),(math.cos(a),math.sin(a),.4),.16)
root('table')
tube('Table top',(0,0,.73),(0,0,.82),1.12,wood,verts=96);torus('Brass table edge',(0,0,.794),1.104,.014,brass)
for r in [.18,.40,.63,.88]:torus('Concentric joinery',(0,0,.821),r,.0035,grain)
for j in range(3):
 a=j*math.tau/3; tube('Splayed table leg',(.75*math.cos(a),.75*math.sin(a),.04),(.41*math.cos(a),.41*math.sin(a),.735),.055,wood,.04)
 torus('Pedestal collar',(0,0,.31),.36,.035,brass)
root('bench')
for x in [-.58,.58]:box('Stone bench foot',(x,0,.22),(.15,.48,.44),stone,.045)
for i in range(4):box('Timber seat slat',(0,-.21+i*.14,.465),(1.6,.115,.075),wood,.018)
for x in [-.60,.60]:tube('Back support',(x,.20,.35),(x,.24,.96),.033,brass)
for z in [.71,.84,.97]:box('Back rail',(0,.235,z),(1.62,.070,.090),wood,.02)
for x in [-.43,.43]:box('Seat cushion',(x,0,.535),(.73,.43,.075),fabric,.035)
root('planter')
# Lathed hollow vessel keeps visible soil inset and thick stone lip.
profile=[(.43,0),(.47,.05),(.53,.55),(.55,.62),(.55,.67),(.49,.67),(.48,.60),(.43,.11)]
v=[];f=[];N=64
for r,z in profile:
 for i in range(N):v.append((r*math.cos(i*math.tau/N),r*math.sin(i*math.tau/N),z))
for j in range(len(profile)-1):
 for i in range(N):f.append((j*N+i,j*N+(i+1)%N,(j+1)*N+(i+1)%N,(j+1)*N+i))
mesh=bpy.data.meshes.new('Hollow stone vessel');mesh.from_pydata(v,[],f);o=bpy.data.objects.new('Stone planter shell',mesh);bpy.context.collection.objects.link(o);finish(o,o.name,stone)
tube('Visible soil',(0,0,.56),(0,0,.575),.476,soil,verts=64)
for z in [.14,.18,.52]:torus('Carved stone groove',(0,0,z),.47+(z/.62)*.055,.006,stone)
root('instrument')
box('Instrument base',(0,0,.045),(.25,.20,.09),wood,.028);tube('Brass mounting',(0,0,.09),(0,0,.23),.032,brass)
body=box('Evidence reader',(0,0,.26),(.29,.045,.23),brass,.018);body.rotation_euler.x=math.radians(-14)
screen=box('Reader dark display',(0,-.028,.263),(.25,.012,.185),glass,.01);screen.rotation_euler.x=math.radians(-14)
for i in range(3):box('Reader evidence line',(-.02,-.040,.29-i*.035),(.15-i*.025,.004,.006),eye,.002)

def descendants(r):return [r]+list(r.children_recursive)
def export(path,objs):
 bpy.ops.object.select_all(action='DESELECT')
 for o in objs:o.select_set(True)
 bpy.context.view_layer.objects.active=objs[0]
 bpy.ops.export_scene.gltf(filepath=str(path),export_format='GLB',use_selection=True,export_apply=True,export_yup=True)
manifest={'version':'hero-kit-v1','units':'metres','up':'glTF +Y','robot_forward':'glTF +Z','source':'Blender authored; north-star.png art direction; no purchased/generated third-party assets','license':'project-authored','textures':0,'clips':[],'assets':{}}
for n,r in roots.items():
 bpy.context.view_layer.update()
 objs=descendants(r);export(BASE/'exports'/f'{n}.glb',objs)
 coords=[o.matrix_world@Vector(c) for o in objs if o.type=='MESH' for c in o.bound_box]
 lo=[min(v[i] for v in coords) for i in range(3)];hi=[max(v[i] for v in coords) for i in range(3)]
 tris=sum(sum(len(p.vertices)-2 for p in o.data.polygons) for o in objs if o.type=='MESH');p=BASE/'exports'/f'{n}.glb'
 manifest['assets'][n]={'file':str(p),'root':n,'blender_bounds':[lo,hi],'triangles':tris,'bytes':p.stat().st_size,'sha256':hashlib.sha256(p.read_bytes()).hexdigest(),'pivot':'floor centre','materials':sorted(set(m.name for o in objs if o.type=='MESH' for m in o.data.materials))}
export(BASE/'exports'/'hero-kit.glb',[o for r in roots.values() for o in descendants(r)])
(BASE/'manifest.json').write_text(json.dumps(manifest,indent=2))
bpy.ops.wm.save_as_mainfile(filepath=str(BASE/'source'/'hero-kit.blend'))
if os.environ.get('KIT_EXPORT_ONLY')=='1':
 print('KIT_READY '+str(BASE));raise SystemExit(0)
# Bounded neutral asset review: one scene render, no concurrent renderer.
for i,(n,r) in enumerate(roots.items()):
 r.location=((i%5)*2.7,(i//5)*3.1,0)
bpy.ops.mesh.primitive_plane_add(size=200,location=(0,0,-.012));floor=bpy.context.object;floor.data.materials.append(mat('Review floor',(.19,.20,.18),.85))
world=bpy.context.scene.world;world.use_nodes=True;world.node_tree.nodes['Background'].inputs[0].default_value=(.30,.36,.42,1);world.node_tree.nodes['Background'].inputs[1].default_value=.45
def area(loc,power,size,color):
 bpy.ops.object.light_add(type='AREA',location=loc);l=bpy.context.object;l.data.energy=power;l.data.shape='DISK';l.data.size=size;l.data.color=color;l.rotation_euler=(Vector((5,2,1))-l.location).to_track_quat('-Z','Y').to_euler()
area((1,-5,9),1800,7,(1,.82,.63));area((10,5,8),2200,6,(.65,.79,1))
bpy.ops.object.camera_add(location=(14,-14,10));cam=bpy.context.object;cam.rotation_euler=(Vector((5.3,1.4,1))-cam.location).to_track_quat('-Z','Y').to_euler();cam.data.type='ORTHO';cam.data.ortho_scale=15;bpy.context.scene.camera=cam
sc=bpy.context.scene;sc.render.engine='CYCLES';sc.cycles.samples=24;sc.cycles.use_denoising=True;sc.render.resolution_x=1500;sc.render.resolution_y=1000;sc.render.resolution_percentage=100;sc.render.image_settings.file_format='PNG';sc.render.filepath=str(BASE/'previews'/'kit-neutral.png');bpy.ops.render.render(write_still=True)
for r in roots.values():r.hide_render=True
roots['robot'].hide_render=False;roots['robot'].location=(0,0,0)
for o in descendants(roots['robot']):o.hide_render=False
cam.location=(2.5,-4,2.2);cam.rotation_euler=(Vector((0,0,.88))-cam.location).to_track_quat('-Z','Y').to_euler();cam.data.ortho_scale=2.1;sc.render.resolution_x=900;sc.render.resolution_y=1100;sc.render.filepath=str(BASE/'previews'/'robot-close.png');bpy.ops.render.render(write_still=True)
print('KIT_READY '+str(BASE))
