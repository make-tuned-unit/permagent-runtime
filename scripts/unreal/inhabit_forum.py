"""Persist ambient skeletal inhabitants in the generated day/night maps.

Sequencer owns only ambient motion, not agent jobs. No camera or input tracks.
See Epic's Python Scripting in Sequencer documentation for binding APIs.
"""
import unreal as u
import json, math, os, urllib.request
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
MODE=os.environ.get('FORUM_APPEARANCE','day')
assert MODE in ('day','night')
assert Path(u.Paths.get_project_file_path()).stem=='SolarForum'
report=ROOT/f'docs/design/solar-forum/unreal-inhabit-{MODE}.json'
report.write_text(json.dumps({'status':'pending'}))
E=u.EditorAssetLibrary;assets=u.AssetToolsHelpers.get_asset_tools()
level=u.get_editor_subsystem(u.LevelEditorSubsystem);actors=u.get_editor_subsystem(u.EditorActorSubsystem)
assert level.load_level('/Game/SolarForum/Maps/'+('ForumCinematic' if MODE=='day' else 'ForumNight'))
world=level.get_world()
for actor in actors.get_all_level_actors():
    if actor.get_actor_label().startswith('Inhabitant/'):actors.destroy_actor(actor)
routes=json.loads((ROOT/'ui/command-center/src/components/world/forum/patrolRoutes.json').read_text())
name=os.environ.get('FORUM_ORCHESTRATOR_NAME','').strip()
if not name:
    try:
        request=urllib.request.Request('http://127.0.0.1:3001/api/agent/identity')
        token_path=Path.home()/'.permagent/secrets/daemon_token.json'
        if token_path.exists():request.add_header('Authorization','Bearer '+json.loads(token_path.read_text())['token'])
        with urllib.request.urlopen(request,timeout=2) as response:name=json.load(response).get('first_name','').strip()
    except Exception:pass
name_resolved=bool(name)
name=name or 'Your sovereign orchestrator'
u.SystemLibrary.execute_console_command(world,'Interchange.FeatureFlags.Import.FBX 0')
people=[];results=[]
for i,(identity,route_name) in enumerate(routes['agents'].items()):
    folder='/Game/SolarForum/AgentsV2/'+identity
    existing=[u.load_asset(p) for p in E.list_assets(folder)] if E.does_directory_exist(folder) else []
    if not (any(isinstance(a,u.SkeletalMesh) for a in existing) and sum(isinstance(a,u.AnimSequence) for a in existing)>=2):
        task=u.AssetImportTask();task.filename=str(ROOT/f'assets/world/forum/animated/{identity}.fbx')
        task.destination_path=folder;task.automated=True;task.replace_existing=True;task.save=True
        options=u.FbxImportUI();options.import_mesh=True;options.import_as_skeletal=True
        options.import_animations=True;options.import_materials=True;options.import_textures=False;options.create_physics_asset=False
        options.mesh_type_to_import=u.FBXImportType.FBXIT_SKELETAL_MESH;options.automated_import_should_detect_type=False
        options.skeletal_mesh_import_data.convert_scene_unit=True
        task.options=options;assets.import_asset_tasks([task])
        E.save_directory(folder,only_if_is_dirty=False,recursive=True)
    imported=[u.load_asset(p) for p in E.list_assets(folder)]
    mesh=next(a for a in imported if isinstance(a,u.SkeletalMesh))
    animations=[a for a in imported if isinstance(a,u.AnimSequence)]
    u.log('FORUM_AGENT_ASSETS '+identity+' '+str([(a.get_name(),a.get_class().get_name()) for a in imported if a]))
    walk=next(a for a in animations if 'walk' in a.get_name().lower())
    idle=next(a for a in animations if 'idle' in a.get_name().lower())
    route=routes['routes'][route_name];start=int((i*.381966%1)*len(route));route=route[start:]+route[:start]
    def position(p):return (p[0]*100,p[2]*100,p[1]*100)
    first=position(route[0])
    actor=actors.spawn_actor_from_class(u.SkeletalMeshActor,u.Vector(*first))
    actor.set_actor_label('Inhabitant/'+(name+' — sovereign orchestrator' if identity=='henry' else identity))
    actor.tags=[u.Name('forum_agent'),u.Name(identity)]
    component=actor.skeletal_mesh_component;component.set_skeletal_mesh_asset(mesh)
    origin,extent=actor.get_actor_bounds(False)
    assert 150<extent.z*2<300,('character height cm',identity,extent.z*2)
    component.set_collision_profile_name('NoCollision');component.set_mobility(u.ComponentMobility.MOVABLE)
    component.set_update_animation_in_editor(True)
    component.set_animation_mode(u.AnimationMode.ANIMATION_SINGLE_NODE);component.set_animation(idle)
    if identity=='henry':
        label=actors.spawn_actor_from_class(u.TextRenderActor,u.Vector(first[0],first[1],first[2]+250))
        label.set_actor_label('Inhabitant/Sovereign nameplate')
        label.text_render.set_text(u.Text(name))
        label.text_render.set_world_size(18.)
        label.text_render.set_mobility(u.ComponentMobility.MOVABLE)
        label.attach_to_actor(actor,u.Name(''),u.AttachmentRule.KEEP_WORLD,u.AttachmentRule.KEEP_WORLD,u.AttachmentRule.KEEP_WORLD,False)
        label.text_render.set_editor_property('absolute_rotation',True)
        label.set_actor_rotation(u.Rotator(yaw=45),False)
    # Each route loops at its own exact duration; no jump back across the campus.
    seq_path='/Game/SolarForum/Life/'+MODE
    seq=u.load_asset(seq_path+'/'+identity) or assets.create_asset(identity,seq_path,u.LevelSequence,u.LevelSequenceFactoryNew())
    for binding in seq.get_bindings():binding.remove()
    seq.set_display_rate(u.FrameRate(30,1));seq.set_playback_start(0)
    binding=seq.add_possessable(actor)
    transform=binding.add_track(u.MovieScene3DTransformTrack).add_section()
    channels=transform.get_all_channels()
    for channel in channels[6:9]:channel.set_default(1.)
    anim_track=binding.add_track(u.MovieSceneSkeletalAnimationTrack)
    frame=0;last_yaw=None;segments=[]
    def key(frame,pos,yaw):
        for channel,value in zip(channels[:6],[*pos,0.,0.,yaw]):
            channel.add_key(u.FrameNumber(frame),float(value),interpolation=u.MovieSceneKeyInterpolation.LINEAR)
    last_clip=None
    def clip(animation,start,end):
        global last_clip
        if last_clip and last_clip['animation']==animation and last_clip['end']==start:
            last_clip['section'].set_range(last_clip['start'],end)
            last_clip['end']=end
            return
        section=anim_track.add_section();section.set_range(start,end)
        params=section.get_editor_property('params');params.animation=animation;section.set_editor_property('params',params)
        last_clip={'animation':animation,'section':section,'start':start,'end':end}
    for j,p in enumerate(route):
        q=route[(j+1)%len(route)];a=position(p);b=position(q)
        length=math.dist(a,b)
        if length<.1:continue
        yaw=math.degrees(math.atan2(b[1]-a[1],b[0]-a[0]))
        if last_yaw is not None:yaw=last_yaw+((yaw-last_yaw+180)%360-180)
        # Ease heading during a short idle turn before reversing a gallery route.
        turn=abs(yaw-last_yaw) if last_yaw is not None else 0
        pause=90 if j%12==0 else (35 if turn>45 else 0)
        if pause:
            key(frame,a,last_yaw if last_yaw is not None else yaw)
            key(frame+pause,a,yaw);clip(idle,frame,frame+pause);frame+=pause
        key(frame,a,yaw)
        duration=max(1,round(length/78.75*30))
        key(frame+duration,b,yaw);clip(walk,frame,frame+duration)
        segments.append({'start':frame,'end':frame+duration,'distanceCm':length})
        frame+=duration;last_yaw=yaw
    transform.set_range(0,frame);seq.set_playback_end(frame)
    E.save_loaded_asset(seq)
    sequence_actor=actors.spawn_actor_from_class(u.LevelSequenceActor,u.Vector())
    sequence_actor.set_actor_label('Inhabitant/Life '+identity);sequence_actor.set_sequence(seq)
    settings=sequence_actor.get_editor_property('playback_settings')
    settings.auto_play=True;settings.loop_count=u.MovieSceneSequenceLoopCount(-1);settings.disable_camera_cuts=True
    sequence_actor.set_editor_property('playback_settings',settings)
    people.append((actor,seq,first))
    results.append({'id':identity,'route':route_name,'mesh':mesh.get_path_name(),'animations':[a.get_name() for a in animations],'durationFrames':frame,'travelSegments':len(segments)})
# Evaluate authored tracks in the editor and verify that all native bodies move.
level.save_current_level()
checks=[];sample_frames=[]
for actor,seq,first in people:
    assert u.LevelSequenceEditorBlueprintLibrary.open_level_sequence(seq)
    def seek(frame):
        u.LevelSequenceEditorBlueprintLibrary.set_current_time(frame)
    seek(0);before=actor.get_actor_location();sample=min(600,seq.get_playback_end()//3);sample_frames.append(sample);seek(sample);after=actor.get_actor_location()
    moved=math.dist((before.x,before.y,before.z),(after.x,after.y,after.z))
    checks.append(moved);seek(0);u.LevelSequenceEditorBlueprintLibrary.close_level_sequence()
    assert moved>50,('native sequence did not move',actor.get_actor_label(),moved)
assert len(people)==12
level.save_current_level();E.save_directory('/Game/SolarForum/AgentsV2',only_if_is_dirty=True,recursive=True)
report.write_text(json.dumps({'status':'complete','appearance':MODE,'inhabitants':results,'evaluatedMovementCm':checks,'sampleFramesAt30FPS':sample_frames,'identitySource':'configured identity snapshot' if name_resolved else 'generic fallback','scope':'Ambient authored routes; no native job dispatch or live daemon activity binding'},indent=2)+'\n')
