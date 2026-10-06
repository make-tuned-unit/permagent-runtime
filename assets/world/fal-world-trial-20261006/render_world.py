"""Render original visible world geometry with its baked vertex colors."""
import bpy,json,math
from pathlib import Path
HERE=Path(__file__).resolve().parent
bpy.ops.wm.read_factory_settings(use_empty=True)
for source in sorted((HERE/'diagnostic-meshes').glob('*.ply')):
    bpy.ops.wm.ply_import(filepath=str(source),forward_axis='Y',up_axis='Z')
    obj=bpy.context.object;obj.name=source.stem
    color=obj.data.color_attributes.active_color
    if color is None:raise RuntimeError('PLY vertex colors missing')
    mat=bpy.data.materials.new(source.stem+' baked color');mat.use_nodes=True
    nodes=mat.node_tree.nodes;nodes.clear()
    color_node=nodes.new('ShaderNodeVertexColor');color_node.layer_name=color.name
    emission=nodes.new('ShaderNodeEmission');emission.inputs['Strength'].default_value=1
    output=nodes.new('ShaderNodeOutputMaterial')
    mat.node_tree.links.new(color_node.outputs['Color'],emission.inputs['Color']);mat.node_tree.links.new(emission.outputs[0],output.inputs['Surface'])
    obj.data.materials.clear();obj.data.materials.append(mat)
scene=bpy.context.scene;scene.render.engine='CYCLES';scene.cycles.device='CPU';scene.cycles.samples=4
scene.render.threads_mode='FIXED';scene.render.threads=1
scene.render.resolution_x=768;scene.render.resolution_y=432;scene.render.resolution_percentage=100
scene.render.image_settings.file_format='PNG';scene.view_settings.view_transform='Standard';scene.view_settings.look='None'
scene.world=bpy.data.worlds.new('No relighting');scene.world.use_nodes=True;scene.world.node_tree.nodes['Background'].inputs[1].default_value=0
camera=bpy.data.objects.new('Origin diagnostic',bpy.data.cameras.new('Camera'));scene.collection.objects.link(camera);scene.camera=camera
camera.data.sensor_fit='HORIZONTAL';camera.data.angle=math.radians(75);camera.data.clip_start=.03;camera.data.clip_end=200
camera.rotation_euler=(math.pi/2,0,math.pi/2)
output=HERE/'previews';output.mkdir(exist_ok=True)
for view in json.loads((HERE/'mesh-inspection.json').read_text())['views']:
    camera.location=view['position'];scene.render.filepath=str(output/(view['name']+'.png'));bpy.ops.render.render(write_still=True)
