"""Render authored scene assets with Unreal's real Metal renderer, without UI capture."""
import unreal as u, time, json, os
from pathlib import Path
# Unreal Python's positional order is roll/pitch/yaw; use explicit fields.
def rotation(pitch=0., yaw=0., roll=0.):
    return u.Rotator(pitch=pitch, yaw=yaw, roll=roll)
ROOT=Path(__file__).resolve().parents[2]
MODE=os.environ.get('FORUM_APPEARANCE','day')
assert MODE in ('day','night')
EXPOSURE=1.5 if MODE=='night' else 13.
OUT=ROOT/'assets/world/forum/unreal';OUT.mkdir(parents=True,exist_ok=True)
assert Path(u.Paths.get_project_file_path()).stem=='SolarForum'
level=u.get_editor_subsystem(u.LevelEditorSubsystem)
assert level.load_level('/Game/SolarForum/Maps/ForumNight' if MODE=='night' else '/Game/SolarForum/Maps/ForumCinematic')
actors=u.get_editor_subsystem(u.EditorActorSubsystem)
world=u.get_editor_subsystem(u.UnrealEditorSubsystem).get_editor_world()
for actor in actors.get_all_level_actors():
    if isinstance(actor,u.TextRenderActor) and actor.get_actor_label()=='Inhabitant/Sovereign nameplate':
        actor.text_render.set_editor_property('absolute_rotation',True)
        actor.set_actor_rotation(rotation(yaw=45),False)
    if isinstance(actor,u.SkyLight):
        actor.light_component.set_mobility(u.ComponentMobility.MOVABLE)
        actor.light_component.set_editor_property('real_time_capture',True)
        actor.light_component.recapture_sky()
    if isinstance(actor,u.StaticMeshActor) and actor.get_actor_label()=='Environment/Reflecting lagoon':
        actor.set_actor_scale3d(u.Vector(54,54,54))
    if isinstance(actor,u.PlayerStart):actor.set_actor_rotation(rotation(yaw=-90),False)
    if isinstance(actor,u.PostProcessVolume):
        settings=actor.get_editor_property('settings')
        settings.set_editor_property('auto_exposure_min_brightness',EXPOSURE)
        settings.set_editor_property('auto_exposure_max_brightness',EXPOSURE)
        actor.set_editor_property('settings',settings)
assert level.save_current_level()
for command in ['sg.ViewDistanceQuality 3','sg.ShadowQuality 3','sg.GlobalIlluminationQuality 3','sg.ReflectionQuality 3','sg.PostProcessQuality 3','sg.TextureQuality 3','sg.EffectsQuality 3','r.ScreenPercentage 100']:
    u.SystemLibrary.execute_console_command(world,command)
cap=actors.spawn_actor_from_class(u.SceneCapture2D,u.Vector(1500,2400,550),rotation(-9,-122,0))
c=cap.capture_component2d
rt=u.RenderingLibrary.create_render_target2d(world,1600,1000,u.TextureRenderTargetFormat.RTF_RGBA8)
c.set_editor_property('texture_target',rt)
c.set_editor_property('capture_source',u.SceneCaptureSource.SCS_FINAL_COLOR_LDR)
c.set_editor_property('fov_angle',65.)
c.set_editor_property('capture_every_frame',False)
c.set_editor_property('capture_on_movement',False)
c.set_editor_property('always_persist_rendering_state',True)
# Scene captures require their own explicit Lumen override.
settings=c.get_editor_property('post_process_settings')
for key,value in {'dynamic_global_illumination_method':u.DynamicGlobalIlluminationMethod.LUMEN,'reflection_method':u.ReflectionMethod.LUMEN,'auto_exposure_min_brightness':EXPOSURE,'auto_exposure_max_brightness':EXPOSURE}.items():
    settings.set_editor_property('override_'+key,True);settings.set_editor_property(key,value)
c.set_editor_property('post_process_settings',settings)
u.EditorPythonScripting.set_keep_python_script_alive(True)
started=time.monotonic();stage=0;frames=0;frame_times=[]
views=[(f'unreal-forum-{MODE}.png',(1500,2400,550),(-9,-122,0)),(f'unreal-walk-{MODE}.png',(0,1400,170),(3,-90,0)),(f'unreal-campus-{MODE}.png',(23000,32000,19000),(-16,-121,0))]
views.append((f'unreal-sovereign-{MODE}.png',(1050,550,255),(-5,-132,0)))
if MODE=='night':views.append(('unreal-galaxies-night.png',(19000,0,500),(29,-66,0)))
views.append((f'unreal-conservatory-{MODE}.png',(9500,500,170),(9,-13,0)))
capture_filter=os.environ.get('FORUM_CAPTURE_FILTER','')
if capture_filter:
    views=[view for view in views if capture_filter in view[0]]
    assert views,('No matching capture',capture_filter)
def tick(dt):
    global stage,frames
    try:
        elapsed=time.monotonic()-started
        if elapsed<90:return # Allow shaders, distance fields, texture streaming and sky capture.
        if stage>=len(views):
            u.unregister_slate_post_tick_callback(handle)
            (ROOT/f'docs/design/solar-forum/unreal-capture-timing-{MODE}.json').write_text(json.dumps({'scope':'Editor offscreen capture timing; not gameplay FPS or isolated GPU timing','samples':frame_times},indent=2)+'\n')
            result={'status':'complete','renderer':'Unreal Engine / Metal','views':[x[0] for x in views],'elapsedSeconds':elapsed,'appearance':MODE,'captureFramesPerView':64}
            (ROOT/f'docs/design/solar-forum/unreal-render-{MODE}.json').write_text(json.dumps(result,indent=2)+'\n')
            environment_report=ROOT/f'docs/design/solar-forum/unreal-environment-{MODE}.json'
            if environment_report.is_file():
                environment=json.loads(environment_report.read_text())
                environment['renderVerified']=True
                environment['renderFiles']=result['views']
                environment_report.write_text(json.dumps(environment,indent=2)+'\n')
            u.EditorPythonScripting.set_keep_python_script_alive(False)
            return
        name,pos,rot=views[stage]
        if frames==0:cap.set_actor_location_and_rotation(u.Vector(*pos),rotation(*rot),False,False)
        before=time.monotonic();c.capture_scene();frames+=1
        frame_times.append({'view':name,'callbackDeltaMs':dt*1000.,'enqueueMs':(time.monotonic()-before)*1000.})
        if frames>=64:
            u.RenderingLibrary.export_render_target(world,rt,str(OUT),name)
            assert (OUT/name).is_file() and (OUT/name).stat().st_size > 1024, ('Missing render', name)
            u.log('FORUM_RENDER_WRITTEN '+name)
            stage+=1;frames=0
    except Exception:
        u.unregister_slate_post_tick_callback(handle);u.EditorPythonScripting.set_keep_python_script_alive(False);raise
handle=u.register_slate_post_tick_callback(tick)
