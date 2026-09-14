"""Build native skeletal FBXs with idle and walking clips from the rigid outfits."""
import bpy, math
from pathlib import Path
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[2]
OUT=ROOT/'assets/world/forum/animated';OUT.mkdir(exist_ok=True)
BONES={
'root':((0,0,0),None),'spine':((0,.85,0),'root'),'head':((0,1.78,0),'spine'),
'armL':((.3,1.5,0),'spine'),'foreL':((.42,1.04,0),'armL'),
'armR':((-.3,1.5,0),'spine'),'foreR':((-.42,1.04,0),'armR'),
'thighL':((.12,.66,0),'root'),'calfL':((.12,.18,0),'thighL'),
 'thighR':((-.12,.66,0),'root'),'calfR':((-.12,.18,0),'thighR'),'aura':((0,.02,0),'root')}
for source in sorted((ROOT/'assets/world/forum/characters').glob('*.blend')):
 bpy.ops.wm.open_mainfile(filepath=str(source))
 meshes=[o for o in bpy.context.scene.objects if o.type=='MESH' and o.get('bone') in BONES]
 bpy.ops.object.select_all(action='DESELECT')
 for o in meshes:
  o.select_set(True);bpy.context.view_layer.objects.active=o
  for modifier in list(o.modifiers):bpy.ops.object.modifier_apply(modifier=modifier.name)
  bpy.ops.object.transform_apply(location=True,rotation=True,scale=True)
  group=o.vertex_groups.new(name=o['bone']);group.add(list(range(len(o.data.vertices))),1.,'REPLACE')
  o.select_set(False)
 arm=bpy.data.armatures.new('ForumSkeleton');rig=bpy.data.objects.new('ForumSkeleton',arm);bpy.context.collection.objects.link(rig)
 rig.select_set(True);bpy.context.view_layer.objects.active=rig;bpy.ops.object.mode_set(mode='EDIT')
 for name,(p,parent) in BONES.items():
  b=arm.edit_bones.new(name);b.head=(p[0],-p[2],p[1]);b.tail=b.head+Vector((0,0,.15))
  if parent:b.parent=arm.edit_bones[parent]
 bpy.ops.object.mode_set(mode='OBJECT');rig.select_set(False)
 for o in meshes:o.select_set(True)
 bpy.context.view_layer.objects.active=meshes[0];bpy.ops.object.join();body=bpy.context.object;body.name=source.stem
 body.parent=rig;modifier=body.modifiers.new('Skeleton','ARMATURE');modifier.object=rig
 for old in list(bpy.data.actions):bpy.data.actions.remove(old)
 scene=bpy.context.scene;scene.render.fps=30
 for clip,end in [('Walk',40),('Idle',90)]:
  rig.animation_data_create();rig.animation_data.action=bpy.data.actions.new(clip)
  for f in range(1,end+2):
   phase=(f-1)/end*math.tau;wave=math.sin(phase)
   for b in rig.pose.bones:b.rotation_mode='XYZ';b.rotation_euler=(0,0,0);b.location=(0,0,0)
   if clip=='Walk':
    for side,sign in [('L',1),('R',-1)]:
     rig.pose.bones['thigh'+side].rotation_euler.x=sign*wave*.32
     rig.pose.bones['calf'+side].rotation_euler.x=max(0,sign*wave)*.42
     rig.pose.bones['arm'+side].rotation_euler.x=-sign*wave*.18
    rig.pose.bones['root'].location.y=.012*(1-math.cos(phase*2))
    rig.pose.bones['spine'].rotation_euler.y=wave*.025
   else:
    rig.pose.bones['spine'].rotation_euler.x=wave*.015
    rig.pose.bones['head'].rotation_euler.y=wave*.055
    rig.pose.bones['root'].location.x=wave*.012
   for b in rig.pose.bones:
    b.keyframe_insert(data_path='rotation_euler',frame=f);b.keyframe_insert(data_path='location',frame=f)
 scene.frame_start=1;scene.frame_end=91;scene.frame_set(1)
 bpy.ops.object.select_all(action='DESELECT');rig.select_set(True);body.select_set(True);bpy.context.view_layer.objects.active=rig
 bpy.ops.export_scene.fbx(filepath=str(OUT/(source.stem+'.fbx')),use_selection=True,object_types={'ARMATURE','MESH'},add_leaf_bones=False,bake_anim=True,bake_anim_use_all_actions=True,bake_anim_use_nla_strips=False,axis_forward='-Y',axis_up='Z')
 print('Animated',source.stem)
