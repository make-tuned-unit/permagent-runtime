"""Render the actual Blender geometry at human scale and as a campus overview."""
import bpy, json
from mathutils import Vector
from pathlib import Path
import os
ROOT=Path(__file__).resolve().parents[2]
# FORUM_SCRATCH renders a scratch build (see build_solar_forum.py) instead of the shipped assets.
_S=os.environ.get('FORUM_SCRATCH');OUT=Path(_S)/'assets' if _S else ROOT/'assets/world/forum'
bpy.ops.wm.open_mainfile(filepath=str(OUT/'solar-forum.blend'))
scene=bpy.context.scene;camera=scene.camera
forest=json.loads(scene.get('forum_forest_json','[]'))
if forest:(OUT/'forest.json').write_text(json.dumps(forest,indent=2)+'\n')
scene.render.engine='CYCLES';scene.cycles.samples=48;scene.cycles.use_denoising=True
# Review renders use a sunrise key and a sane exposure so emissives and fabrics
# keep their authored colour instead of clipping to white (job 12 direction).
scene.view_settings.view_transform='AgX';scene.view_settings.exposure=-0.7
for _o in scene.objects:
    if _o.type=='LIGHT' and _o.data.type=='SUN':
        _o.data.color=(1.0,.82,.62);_o.data.energy=3.2;_o.rotation_euler=(1.30,.35,2.35)
    elif _o.type=='LIGHT' and _o.data.type=='AREA':
        _o.data.energy=_o.data.energy*.45
scene.render.resolution_x=1500;scene.render.resolution_y=1000
# FORUM_RENDER_SCALE=50 renders at half size for quick review on a memory-constrained host.
scene.render.resolution_percentage=int(os.environ.get('FORUM_RENDER_SCALE','100'))
views=[
 ('celestial-habitat',(230,-320,190),(0,65,65),True),
 ('campus-expanded',(245,-285,200),(0,0,0),True),
 ('geodisc-profile',(285,-330,80),(0,0,-6),True),
 ('garden-district',(139,-78,5),(120,-55,9),False),
 ('conservatory-walk',(95,-5,1.7),(116,0,5),False),
 ('maker-hall-walk',(-98,-3,1.7),(-117,0,3.3),False),
 ('harbor-walk',(7,-104,1.7),(0,-123,8),False),
 ('terrace-human',(0,-9,1.7),(0,0,1.4),False),
 ('mesh-gate-human',(-31.1,31.1,1.7),(-39.6,39.6,3),False),
 ('commons-detail',(7,-9,1.7),(0,0,1.1),False),
]
# FORUM_VIEWS=a,b renders only the named views.
_only=os.environ.get('FORUM_VIEWS');views=[v for v in views if not _only or v[0] in _only.split(',')]
for name,position,target,overview in views:
 camera.location=position;camera.rotation_euler=(Vector(target)-camera.location).to_track_quat('-Z','Y').to_euler()
 camera.data.type='ORTHO' if overview else 'PERSP';camera.data.ortho_scale=570 if name=='celestial-habitat' else 410;camera.data.lens=24 if overview else 28
 scene.render.filepath=str(OUT/(name+'.png'));bpy.ops.render.render(write_still=True)
 print('SOURCE_VIEW_COMPLETE',name,flush=True)
