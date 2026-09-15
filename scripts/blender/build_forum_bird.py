"""Small distant seabird with tapered wings; native material animates wing weights."""
import bpy, math
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
OUT=ROOT/'assets/world/forum';WEB=ROOT/'ui/command-center/public/world'
bpy.ops.object.select_all(action='SELECT');bpy.ops.object.delete(use_global=False)
bpy.context.scene.unit_settings.system='METRIC'
m=bpy.data.materials.new('Soft ivory feathers');m.diffuse_color=(.55,.58,.6,1)
# Forward is +Y; wing red channel encodes smooth root-to-tip flex.
def finish(o,name,weight=False):
 o.name=name;o.data.materials.append(m)
 c=o.data.color_attributes.new(name='Wing',type='FLOAT_COLOR',domain='POINT')
 for v in o.data.vertices:c.data[v.index].color=(max(0.,min(1.,abs(v.co.x)/1.45)) if weight else 0.,0.,0.,1.)
 for p in o.data.polygons:p.use_smooth=True
bpy.ops.mesh.primitive_uv_sphere_add(segments=16,ring_count=8,location=(0,0,0));o=bpy.context.object;o.scale=(.13,.38,.12);bpy.ops.object.transform_apply(location=False,rotation=False,scale=True);finish(o,'Body')
bpy.ops.mesh.primitive_uv_sphere_add(segments=12,ring_count=8,location=(0,.3,.05));o=bpy.context.object;o.scale=(.095,.14,.09);bpy.ops.object.transform_apply(location=False,rotation=False,scale=True);finish(o,'Head')
for side in [-1,1]:
 verts=[]
 for x,front,back,z in [(0,.19,-.2,0),(.32,.20,-.29,.055),(.72,.06,-.31,.08),(1.12,-.13,-.37,.04),(1.48,-.34,-.38,0)]:
  verts.extend([(side*x,front,z),(side*x,back,z),(side*x,(front+back)/2,z+.045)])
 faces=[]
 for i in range(4):
  a=i*3;b=(i+1)*3
  faces.extend([(a,b,b+2,a+2),(a+2,b+2,b+1,a+1),(a+1,b+1,b,a)])
 mesh=bpy.data.meshes.new('Wing');mesh.from_pydata(verts,[],faces);mesh.update();o=bpy.data.objects.new('WingL' if side<0 else 'WingR',mesh);bpy.context.collection.objects.link(o);finish(o,o.name,True)
mesh=bpy.data.meshes.new('Tail');mesh.from_pydata([(-.08,-.25,0),(.08,-.25,0),(.22,-.65,.02),(-.22,-.65,.02)],[],[(0,1,2,3)]);o=bpy.data.objects.new('Tail',mesh);bpy.context.collection.objects.link(o);finish(o,'Tail')
bpy.ops.object.select_all(action='SELECT')
bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'forum-bird.blend'))
bpy.ops.export_scene.gltf(filepath=str(WEB/'forum-bird.glb'),export_format='GLB',export_yup=True,export_animations=False,export_cameras=False,export_lights=False)
bpy.ops.export_scene.fbx(filepath=str(OUT/'forum-bird.fbx'),object_types={'MESH'},apply_unit_scale=True,bake_anim=False,axis_forward='-Y',axis_up='Z')
