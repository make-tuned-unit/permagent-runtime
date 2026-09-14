"""Run only in the isolated SolarForum project; rebuilds its generated map."""
import unreal, json
from pathlib import Path
# Unreal Python's positional order is roll/pitch/yaw; use explicit fields.
def rotation(pitch=0., yaw=0., roll=0.):
    return unreal.Rotator(pitch=pitch, yaw=yaw, roll=roll)
ROOT=Path(__file__).resolve().parents[2]
assert Path(unreal.Paths.get_project_file_path()).stem == 'SolarForum'
# Use the legacy FBX options explicitly; Interchange ignores some FbxImportUI flags.
unreal.SystemLibrary.execute_console_command(None,'Interchange.FeatureFlags.Import.FBX 0')
task=unreal.AssetImportTask()
task.filename=str(ROOT/'assets/world/forum/solar-forum.fbx')
task.destination_path='/Game/SolarForum/Architecture'
task.automated=True; task.replace_existing=True; task.save=True
options=unreal.FbxImportUI()
options.import_mesh=True; options.import_as_skeletal=False
options.import_materials=True; options.import_textures=False
options.mesh_type_to_import=unreal.FBXImportType.FBXIT_STATIC_MESH
options.automated_import_should_detect_type=False
options.static_mesh_import_data.combine_meshes=True
options.static_mesh_import_data.auto_generate_collision=False
options.static_mesh_import_data.convert_scene_unit=True
options.static_mesh_import_data.transform_vertex_to_absolute=True
task.options=options
unreal.AssetToolsHelpers.get_asset_tools().import_asset_tasks([task])
meshes=[unreal.load_asset(p) for p in task.imported_object_paths if isinstance(unreal.load_asset(p),unreal.StaticMesh)]
assert len(meshes)==1, task.imported_object_paths
mesh=meshes[0]
box=mesh.get_bounding_box(); diameter=max(box.max.x-box.min.x,box.max.y-box.min.y)
manifest=json.loads((ROOT/'ui/command-center/public/world/solar-forum.manifest.json').read_text())
expected=manifest.get('campusDiameterMeters',44)*100
assert abs(diameter-expected)<200, ('campus diameter in cm',diameter,expected)
# Triangle collision leaves the colonnade and courtyards open. Static architecture
# only; a later optimization can replace it with authored simple collision pieces.
mesh.get_editor_property('body_setup').set_editor_property('collision_trace_flag',unreal.CollisionTraceFlag.CTF_USE_COMPLEX_AS_SIMPLE)
unreal.EditorAssetLibrary.save_loaded_asset(mesh)
level=unreal.get_editor_subsystem(unreal.LevelEditorSubsystem)
map_path='/Game/SolarForum/Maps/Forum'
assert level.load_level(map_path) if unreal.EditorAssetLibrary.does_asset_exist(map_path) else level.new_level(map_path)
actors=unreal.get_editor_subsystem(unreal.EditorActorSubsystem)
def actor(cls, location, rotation=rotation()):
    # The generated map owns one actor of each class; reuse on regeneration.
    found=next((a for a in actors.get_all_level_actors() if a.get_class()==cls.static_class()),None)
    if found:
        found.set_actor_location_and_rotation(location,rotation,False,False)
        return found
    return actors.spawn_actor_from_class(cls,location,rotation)

a=actor(unreal.StaticMeshActor,unreal.Vector(0,0,0))
a.set_actor_label('Solar Forum · Blender architecture')
a.static_mesh_component.set_static_mesh(mesh)
a.static_mesh_component.set_collision_profile_name('BlockAll')
# UE's template supplies tested input/character logic, copied locally from the
# installed engine. We do not distribute Epic assets in this repository.
gamemode=unreal.EditorAssetLibrary.load_blueprint_class('/Game/FirstPerson/Blueprints/BP_FirstPersonGameMode')
assert gamemode
for path in ['/Game/Input/Actions/IA_Move','/Game/Input/Actions/IA_Look','/Game/Input/Actions/IA_MouseLook','/Game/Input/Actions/IA_Jump','/Game/Characters/Mannequins/Meshes/SK_Mannequin']:
    assert unreal.load_asset(path), path
bp=unreal.load_asset('/Game/FirstPerson/Blueprints/BP_FirstPersonCharacter')
unreal.BlueprintEditorLibrary.compile_blueprint(bp)
blueprint_status=str(bp.get_editor_property('status'))
assert 'ERROR' not in blueprint_status.upper(), blueprint_status
world=unreal.get_editor_subsystem(unreal.UnrealEditorSubsystem).get_editor_world()
world.get_world_settings().set_editor_property('default_game_mode',gamemode)
start=actor(unreal.PlayerStart,unreal.Vector(0,1500,110),rotation(0,-90,0))
start.set_actor_label('Forum arrival · first person')
sun=actor(unreal.DirectionalLight,unreal.Vector(0,0,1500),rotation(-45,-35,0))
sun.light_component.set_editor_property('intensity',3.0)
sun.light_component.set_editor_property('light_color',unreal.Color(255,240,212,255))
actor(unreal.SkyAtmosphere,unreal.Vector())
sky=actor(unreal.SkyLight,unreal.Vector(0,0,1000))
sky.light_component.set_editor_property('real_time_capture',True)
# Review camera plus native material import. Unreal polish is now editable.
cam=actor(unreal.CameraActor,unreal.Vector(3100,4000,3100),rotation(-31,-128,0))
cam.set_actor_label('Forum review camera')
assert level.save_current_level()
report={'status':'complete','engine':unreal.SystemLibrary.get_engine_version(),'mesh':mesh.get_path_name(),'diameterCm':diameter,'materialSlots':len(mesh.static_materials),'collision':'complex-as-simple, static architecture','map':'/Game/SolarForum/Maps/Forum','gameMode':gamemode.get_path_name(),'playerStart':[0,1500,110],'blueprintStatus':blueprint_status,'playTested':False}
(ROOT/'docs/design/solar-forum/unreal-import.json').write_text(json.dumps(report,indent=2)+'\n')
unreal.log('SOLAR_FORUM_IMPORTED '+json.dumps(report))
