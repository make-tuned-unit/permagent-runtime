"""Render the downloaded GLB itself from two sides, with bounded CPU settings."""
import bpy, sys, math
from pathlib import Path
from mathutils import Vector

source = Path(sys.argv[sys.argv.index('--') + 1]).resolve()
args = sys.argv[sys.argv.index('--') + 1:]
dest = Path(args[1]) if len(args)>1 else source.parent.parent / 'previews'
dest.mkdir(exist_ok=True)
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=str(source))
if len(args)>2:
    root=bpy.data.objects.get(args[2])
    if not root: raise RuntimeError('Missing requested asset '+args[2])
    retained={root,*root.children_recursive}
    for obj in list(bpy.context.scene.objects):
        if obj not in retained: bpy.data.objects.remove(obj,do_unlink=True)
if source.parent.name == 'processed':
    bpy.ops.file.pack_all()
    bpy.ops.wm.save_as_mainfile(filepath=str(source.with_suffix('.blend')))
objects = [o for o in bpy.context.scene.objects if o.type == 'MESH']
points = [o.matrix_world @ Vector(c) for o in objects for c in o.bound_box]
lo = Vector(tuple(min(p[i] for p in points) for i in range(3)))
hi = Vector(tuple(max(p[i] for p in points) for i in range(3)))
size = max(hi-lo)
center = (lo+hi)/2
scene=bpy.context.scene
scene.render.engine='CYCLES'
scene.cycles.device='CPU'
scene.cycles.samples=12
scene.render.threads_mode='FIXED'
scene.render.threads=1
scene.render.resolution_x=640
scene.render.resolution_y=640
scene.render.resolution_percentage=100
scene.render.image_settings.file_format='PNG'
scene.world=bpy.data.worlds.new('Neutral studio')
scene.world.use_nodes=True
scene.world.node_tree.nodes['Background'].inputs[0].default_value=(.35,.35,.35,1)
scene.world.node_tree.nodes['Background'].inputs[1].default_value=.65
for name,offset,power in [('Key',(-2,-3,4),450),('Fill',(3,-1,2),230),('Rim',(0,3,3),300)]:
    light=bpy.data.lights.new(name,'AREA');light.energy=power*size*size;light.shape='DISK';light.size=size*2
    obj=bpy.data.objects.new(name,light);scene.collection.objects.link(obj);obj.location=center+Vector(offset)*size
    obj.rotation_euler=(center-obj.location).to_track_quat('-Z','Y').to_euler()
camera=bpy.data.objects.new('Diagnostic camera',bpy.data.cameras.new('Camera'))
scene.collection.objects.link(camera);scene.camera=camera
camera.data.type='ORTHO';camera.data.ortho_scale=size*1.5
for name,offset in [('front',(2,-3,1.7)),('back',(-2,3,1.4))]:
    camera.location=center+Vector(offset)*size
    camera.rotation_euler=(center-camera.location).to_track_quat('-Z','Y').to_euler()
    scene.render.filepath=str(dest/(source.stem+'-'+name+'.png'))
    bpy.ops.render.render(write_still=True)
