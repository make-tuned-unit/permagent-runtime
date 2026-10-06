import bpy,math,json,hashlib,shutil
from pathlib import Path
from mathutils import Vector
BASE=Path(__file__).resolve().parent.parent
backup=BASE/'source'/'robot-v1.glb'
if not backup.exists():shutil.copyfile(BASE/'exports'/'robot.glb',backup)
bpy.ops.object.select_all(action='SELECT');bpy.ops.object.delete(use_global=False)
bpy.ops.import_scene.gltf(filepath=str(backup))
root=bpy.data.objects['robot'];mats={m.name.split(' • ')[0]:m for m in bpy.data.materials}
ivory=mats['Ceramic'];dark=mats['Graphite'];brass=mats['Brass'];glass=mats['Faceplate'];eye=mats['Eyes']
def finish(o,name,mat,parent):
 o.name=name;o.data.materials.append(mat);bpy.context.view_layer.update();world=o.matrix_world.copy();o.parent=parent;o.matrix_world=world;bpy.context.view_layer.update()
 for p in o.data.polygons:p.use_smooth=True
 return o
def ell(n,p,s,m,parent=root):
 bpy.ops.mesh.primitive_uv_sphere_add(segments=24,ring_count=12,location=p);o=bpy.context.object;o.scale=s;return finish(o,n,m,parent)
def box(n,p,s,m,bevel=.02,parent=root):
 bpy.ops.mesh.primitive_cube_add(size=1,location=p);o=bpy.context.object;o.dimensions=s;bpy.ops.object.transform_apply(location=False,rotation=False,scale=True);b=o.modifiers.new('Ceramic edge radius','BEVEL');b.width=bevel;b.segments=3;bpy.ops.object.modifier_apply(modifier=b.name);return finish(o,n,m,parent)
def tube(n,a,b,r,m,parent=root):
 a,b=Vector(a),Vector(b);d=b-a;bpy.ops.mesh.primitive_cylinder_add(vertices=16,radius=r,depth=d.length,location=(a+b)/2);o=bpy.context.object;o.rotation_euler=d.to_track_quat('Z','Y').to_euler();return finish(o,n,m,parent)
def ring(n,p,r,minor,m,parent=root):
 bpy.ops.mesh.primitive_torus_add(major_segments=32,minor_segments=8,major_radius=r,minor_radius=minor,location=p);return finish(bpy.context.object,n,m,parent)
def delete_prefix(prefix):
 for o in list(bpy.data.objects):
  if o.type=='MESH' and o.name.startswith(prefix):bpy.data.objects.remove(o,do_unlink=True)
def panel(n,center,radii,t0,t1,p0,p1,parent=root):
 verts=[];faces=[];rows=10;cols=12
 for i in range(rows+1):
  theta=t0+(t1-t0)*i/rows
  for k in range(cols+1):
   phi=p0+(p1-p0)*k/cols;verts.append((center[0]+radii[0]*math.sin(theta)*math.cos(phi),center[1]+radii[1]*math.sin(theta)*math.sin(phi),center[2]+radii[2]*math.cos(theta)))
 for i in range(rows):
  for k in range(cols):a=i*(cols+1)+k;faces.append((a,a+1,a+cols+2,a+cols+1))
 me=bpy.data.meshes.new(n);me.from_pydata(verts,[],faces);o=bpy.data.objects.new(n,me);bpy.context.collection.objects.link(o);finish(o,n,ivory,parent)
 solid=o.modifiers.new('Panel wall thickness','SOLIDIFY');solid.thickness=.009;solid.offset=-1
 bevel=o.modifiers.new('Panel bevel','BEVEL');bevel.width=.003;bevel.segments=2
 bpy.context.view_layer.objects.active=o;o.select_set(True)
 bpy.ops.object.modifier_apply(modifier=solid.name);bpy.ops.object.modifier_apply(modifier=bevel.name);o.select_set(False)
 return o
head=bpy.data.objects['Head'];delete_prefix('Head_Ceramic')
ell('Head underlying gasket',(0,0,1.47),(.307,.263,.287),dark,head)
for band,(t0,t1) in enumerate([(.02,.84),(.862,2.15),(2.173,math.pi-.02)]):
 for sector in range(6):
  phi=sector*math.tau/6;panel(f'Head shell panel {band}.{sector}',(0,0,1.47),(.315,.27,.295),t0,t1,phi+.016,phi+math.tau/6-.016,head)
for sign in [-1,1]:
 for z in [1.58,1.39]:ell('Head recessed fastener',(sign*.294,-.12,z),(.009,.009,.009),brass,head)
delete_prefix('robot_Ceramic');delete_prefix('robot_Graphite');delete_prefix('robot_Brass')
ell('Torso structural core',(0,0,.90),(.235,.173,.292),dark)
for side,(p0,p1) in enumerate([(-math.pi+.03,-math.pi/2-.026),(-math.pi/2+.026,-.03),(.03,math.pi-.03)]):
 for band,(t0,t1) in enumerate([(.23,1.32),(1.36,2.62)]):panel(f'Torso ceramic {side}.{band}',(0,0,.90),(.255,.19,.303),t0,t1,p0,p1)
tube('Neck piston',(0,0,1.15),(0,0,1.31),.069,brass)
for z in [1.19,1.23,1.27]:ring('Neck bearing gasket',(0,0,z),.075,.012,dark)
ring('Collar ceramic rim',(0,0,1.195),.145,.018,ivory)
tube('Waist spindle',(0,0,.61),(0,0,.73),.078,brass)
for z in [.63,.66,.69]:ring('Waist articulated bearing',(0,0,z),.128,.012,dark)
ring('Waist upper flange',(0,0,.725),.16,.014,brass)
box('Chest evidence instrument',(0,-.186,1.04),(.145,.036,.103),brass,.017)
box('Chest reader glass',(0,-.208,1.04),(.112,.014,.070),glass,.009)
for i in range(3):box('Chest instrument line',(-.006,-.217,1.06-i*.018),(.064-i*.008,.004,.004),eye,.001)
for sign in [-1,1]:
 for z in [.84,.92,1.11]:ell('Torso screw',(sign*.16,-.139,z),(.008,.008,.008),brass)
for z in [.90,.93,.96]:box('Rear cooling slit',(0,.172,z),(.13,.014,.008),dark,.002)
box('Asymmetric shoulder service plate',(-.21,-.045,1.105),(.06,.07,.06),brass,.012)
for sign,side in [(-1,'L'),(1,'R')]:
 arm=bpy.data.objects['Arm.'+side];delete_prefix('Arm.'+side+'_Ceramic');delete_prefix('Arm.'+side+'_Graphite')
 ell('Shoulder bearing.'+side,(sign*.28,0,1.07),(.088,.087,.09),dark,arm)
 box('Upper arm armor.'+side,(sign*.33,-.01,.95),(.13,.14,.27),ivory,.052,arm)
 box('Forearm armor.'+side,(sign*.36,-.07,.72),(.13,.135,.205),ivory,.046,arm)
 ring('Wrist coupling.'+side,(sign*.36,-.09,.615),.043,.009,brass,arm)
 box('Hand palm.'+side,(sign*.36,-.10,.583),(.107,.064,.080),dark,.022,arm)
 box('Hand dorsal armor.'+side,(sign*.36,-.133,.594),(.09,.025,.061),ivory,.012,arm)
 for finger in range(4):
  x=sign*.36+(finger-1.5)*.027
  for seg in range(3):
   z=.535-seg*.028;box(f'Finger segment {side}.{finger}.{seg}',(x,-.102-seg*.006,z),(.022,.032,.024),ivory,.007,arm)
   if seg<2:tube('Finger knuckle',(x,-.102-seg*.006,z-.014),(x,-.102-seg*.006,z-.016),.011,dark,arm)
 for seg in range(2):box('Opposable thumb.'+side,(sign*(.425+seg*.008),-.121-seg*.023,.580-seg*.016),(.030,.032,.029),ivory,.009,arm)
 leg=bpy.data.objects['Leg.'+side];delete_prefix('Leg.'+side+'_Ceramic');delete_prefix('Leg.'+side+'_Graphite')
 ell('Hip bearing.'+side,(sign*.125,0,.64),(.087,.085,.08),dark,leg)
 box('Thigh armor.'+side,(sign*.125,0,.49),(.166,.166,.268),ivory,.064,leg)
 box('Shin armor.'+side,(sign*.125,-.005,.218),(.151,.155,.235),ivory,.055,leg)
 box('Foot thin sole.'+side,(sign*.125,-.051,.024),(.185,.284,.035),dark,.016,leg)
 box('Heel support.'+side,(sign*.125,.025,.065),(.164,.127,.081),brass,.026,leg)
 box('Toe ceramic cap.'+side,(sign*.125,-.112,.084),(.177,.159,.076),ivory,.030,leg)
 box('Foot instep.'+side,(sign*.125,-.037,.095),(.152,.111,.108),ivory,.039,leg)
 box('Toe split seam.'+side,(sign*.125,-.123,.123),(.008,.100,.006),dark,.002,leg)
# Preserve named rigid pivots while consolidating material draws.
for parent in [root]+[o for o in root.children if o.type=='EMPTY']:
 batches={}
 for o in list(parent.children):
  if o.type=='MESH':batches.setdefault(o.data.materials[0].name,[]).append(o)
 for name,obs in batches.items():
  bpy.ops.object.select_all(action='DESELECT')
  for o in obs:o.select_set(True)
  bpy.context.view_layer.objects.active=obs[0]
  if len(obs)>1:bpy.ops.object.join()
  bpy.context.object.name=parent.name+'_'+name
bpy.context.view_layer.update();bpy.ops.object.select_all(action='SELECT')
bpy.ops.export_scene.gltf(filepath=str(BASE/'exports'/'robot.glb'),export_format='GLB',use_selection=True,export_apply=True)
bpy.ops.wm.save_as_mainfile(filepath=str(BASE/'source'/'robot-v2.blend'))
# Exactly one low-sample diagnostic, no complete-kit scene import.
exec((BASE/'source'/'robot_diagnostic.py').read_text())
