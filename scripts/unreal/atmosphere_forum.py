"""Versioned day/night art pass on the isolated generated Solar Forum maps."""
import unreal as u
import json, math, os
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
MODE=os.environ.get('FORUM_APPEARANCE','day')
assert MODE in ('day','night')
assert Path(u.Paths.get_project_file_path()).stem=='SolarForum'
E=u.EditorAssetLibrary; M=u.MaterialEditingLibrary
assets=u.AssetToolsHelpers.get_asset_tools()
level=u.get_editor_subsystem(u.LevelEditorSubsystem)
actors=u.get_editor_subsystem(u.EditorActorSubsystem)
base='/Game/SolarForum/Maps/ForumCinematic'
target=base if MODE=='day' else '/Game/SolarForum/Maps/ForumNight'
if E.does_asset_exist(target):assert level.load_level(target)
else:assert level.new_level_from_template(target,base)
for a in actors.get_all_level_actors():
    if a.get_actor_label().startswith('Atmosphere/'):actors.destroy_actor(a)
night=MODE=='night'
def rotation(pitch=0.,yaw=0.,roll=0.):return u.Rotator(pitch=pitch,yaw=yaw,roll=roll)
def material(name, rgb, emission=0., metal=0., rough=.5):
    path='/Game/SolarForum/Environment/Materials'
    # Separate per appearance so generating night cannot mutate the day map.
    name=name+'_'+MODE
    mat=u.load_asset(path+'/'+name) or assets.create_asset(name,path,u.Material,u.MaterialFactoryNew())
    M.delete_all_material_expressions(mat)
    def constant(value, cls):
        n=M.create_material_expression(mat,cls)
        n.set_editor_property('constant' if isinstance(value,tuple) else 'r',u.LinearColor(*value) if isinstance(value,tuple) else value)
        return n
    for value,prop in [(rgb,u.MaterialProperty.MP_BASE_COLOR),(metal,u.MaterialProperty.MP_METALLIC),(rough,u.MaterialProperty.MP_ROUGHNESS)]:
        n=constant(value,u.MaterialExpressionConstant3Vector if isinstance(value,tuple) else u.MaterialExpressionConstant)
        assert M.connect_material_property(n,'',prop)
    if emission:
        n=constant(tuple(x*emission for x in rgb),u.MaterialExpressionConstant3Vector)
        assert M.connect_material_property(n,'',u.MaterialProperty.MP_EMISSIVE_COLOR)
    M.recompile_material(mat);E.save_loaded_asset(mat)
    return mat
bronze=material('M_LanternBronze',(.24,.13,.045),metal=.78,rough=.34)
lamp=material('M_WarmLantern', (1.,.43,.12),emission=12. if night else .15)
stone=material('M_HonedStone',(.48,.44,.35),rough=.63)
def mesh(name,shape,pos,scale,mat):
    a=actors.spawn_actor_from_class(u.StaticMeshActor,u.Vector(*pos));a.set_actor_label('Atmosphere/'+name)
    c=a.static_mesh_component;c.set_static_mesh(u.load_asset('/Engine/BasicShapes/'+shape));c.set_material(0,mat)
    c.set_collision_profile_name('NoCollision');a.set_actor_scale3d(u.Vector(*scale))
    return a
# Shaded architectural details read as honed stone rather than brick wallpaper.
architecture=next(a for a in actors.get_all_level_actors() if isinstance(a,u.StaticMeshActor) and a.get_actor_label().startswith('Solar Forum'))
for i,slot in enumerate(architecture.static_mesh_component.static_mesh.static_materials):
    if 'travertine' in str(slot.material_slot_name).lower():architecture.static_mesh_component.set_material(i,stone)
# Recessed lanterns and raised gallery lights keep every circulation route legible.
lanterns=[]
for i in range(16):
    a=(i+.5)*math.tau/16;lanterns.append((3440*math.cos(a),3440*math.sin(a),15))
for x,y in [(-1250,700),(1250,700),(-700,-1400),(700,-1400),(-1850,-450),(1850,-450)]:lanterns.append((x,y,20))
for x,y in [(-1450,-1350),(0,-2050),(1450,-1350)]:lanterns.append((x,y,445))
for i,(x,y,z) in enumerate(lanterns):
    mesh('Lantern pedestal '+str(i),'Cylinder',(x,y,z+34),(.28,.28,.68),bronze)
    glow=mesh('Lantern lens '+str(i),'Cylinder',(x,y,z+75),(.24,.24,.14),lamp)
    glow.static_mesh_component.set_editor_property('cast_shadow',False)
    mesh('Lantern cap '+str(i),'Cylinder',(x,y,z+86),(.34,.34,.06),bronze)
    if night:
        light=actors.spawn_actor_from_class(u.PointLight,u.Vector(x,y,z+105));light.set_actor_label('Atmosphere/Path light '+str(i))
        c=light.light_component;c.set_mobility(u.ComponentMobility.MOVABLE)
        c.set_editor_property('intensity_units',u.LightUnits.LUMENS)
        c.set_editor_property('intensity',700.);c.set_editor_property('attenuation_radius',720.)
        c.set_editor_property('use_temperature',True);c.set_editor_property('temperature',2800.)
        c.set_editor_property('source_radius',12.);c.set_editor_property('cast_shadows',False)
# Directional sunlight by day, cool moonlight by night.
sun=next(a for a in actors.get_all_level_actors() if isinstance(a,u.DirectionalLight))
sun.set_actor_rotation(rotation(-35,-38),False)
sun.light_component.set_mobility(u.ComponentMobility.MOVABLE)
for key,val in {'intensity':8. if night else 85000.,'light_source_angle':.7,'use_temperature':True,'temperature':8200. if night else 5500.,'atmosphere_sun_light':not night}.items():sun.light_component.set_editor_property(key,val)
sky=next(a for a in actors.get_all_level_actors() if isinstance(a,u.SkyLight))
sky.light_component.set_mobility(u.ComponentMobility.MOVABLE)
sky.light_component.set_editor_property('intensity',1.5 if night else 2.3)
sky.light_component.set_editor_property('real_time_capture',True)
for a in actors.get_all_level_actors():
    if isinstance(a,u.SkyAtmosphere):a.set_actor_hidden_in_game(night);a.set_is_temporarily_hidden_in_editor(night)
    if isinstance(a,u.ExponentialHeightFog):
        a.component.set_editor_property('fog_density',.0015 if night else .005)
        a.component.set_editor_property('fog_inscattering_luminance',u.LinearColor(.015,.025,.06) if night else u.LinearColor(.5,.6,.7))
    if isinstance(a,u.PostProcessVolume):
        s=a.get_editor_property('settings')
        for k,v in {'auto_exposure_min_brightness':1.5 if night else 13.,'auto_exposure_max_brightness':1.5 if night else 13.,'bloom_intensity':.3 if night else .12,'vignette_intensity':.08}.items():s.set_editor_property('override_'+k,True);s.set_editor_property(k,v)
        a.set_editor_property('settings',s)
if night:
    # Directional, static star field. No bitmap, no rotating particles, no UV seam.
    path='/Game/SolarForum/Environment/Materials/M_StarVault'
    mat=u.load_asset(path) or assets.create_asset('M_StarVault',str(Path(path).parent),u.Material,u.MaterialFactoryNew())
    M.delete_all_material_expressions(mat)
    mat.set_editor_property('shading_model',u.MaterialShadingModel.MSM_UNLIT)
    mat.set_editor_property('two_sided',True);mat.set_editor_property('is_sky',True)
    code='''float3 d = normalize(-Direction);
float2 uv = float2(atan2(d.y,d.x)/6.2831853+0.5,d.z*0.5+0.5);
float2 grid = uv*float2(1100,550);
float2 cell=floor(grid), f=frac(grid);
float h=frac(sin(dot(cell,float2(127.1,311.7)))*43758.5453);
float j=frac(sin(dot(cell,float2(269.5,183.3)))*43758.5453);
float2 center=float2(frac(h*73.17),frac(j*43.11))*.6+.2;
float radius=.035+.09*pow(j,12);
float aa=max(length(fwidth(grid))*.5,.01);
float starPoint=(1-smoothstep(radius,radius+aa,length(f-center)))*step(.95,h);
float3 tint=lerp(float3(.64,.77,1),float3(1,.82,.62),j);
float band=pow(saturate(1-abs(d.z*.84+d.y*.42-d.x*.18)*3.5),8);
float dust=(.65+.35*sin(d.x*60)*sin(d.y*47))*band;
float twinkle=.78+.22*sin(Time*(.8+j)+h*231.);
float slot=floor(Time/45.);
float seed=frac(sin(slot*127.1+14.3)*43758.5453);
float age=Time-slot*45.-6.-seed*28.;
float progress=saturate(age/1.4);
float2 origin=float2(frac(seed*7.13),.7+frac(seed*13.7)*.18);
float2 travel=float2(.09,-.055);
float2 head=origin+travel*progress;
float2 delta=uv-head;delta.x=frac(delta.x+.5)-.5;
float along=dot(delta,normalize(travel));
float across=abs(delta.x*normalize(travel).y-delta.y*normalize(travel).x);
float meteor=(1-smoothstep(.00015,.0007,across))*smoothstep(-.026,0.,along)*(1-step(.001,along));
meteor*=step(0.,age)*(1-step(1.4,age))*sin(progress*3.14159265);
float3 galaxies=0;
for(int g=0;g<3;g++){
 float3 center=normalize(g==0?float3(.35,-.8,.48):(g==1?float3(-.85,.25,.37):float3(.7,.63,.32)));
 float facing=dot(d,center);
 float3 right=normalize(cross(center,float3(0,0,1))),up=cross(right,center);
 float size=g==0?.13:(g==1?.08:.095);
 float2 p=float2(dot(d,right),dot(d,up))/size;
 float tilt=g==0?.45:(g==1?-.7:1.2),c=cos(tilt),s=sin(tilt);
 p=float2(c*p.x-s*p.y,s*p.x+c*p.y);p.y*=2.3;
 float r=length(p),a=atan2(p.y,p.x);
 float arms=pow(.5+.5*cos(a*2-log(r+.09)*4.5),5);
 float disk=exp(-r*2.8)*(.18+arms*.82)*smoothstep(.025,.14,r);
 float core=exp(-r*r*95);
 galaxies+=step(.75,facing)*(float3(.18,.27,.46)*disk+float3(.8,.63,.39)*core)*3;
}
return float3(.003,.006,.017)+dust*float3(.018,.025,.05)+galaxies+starPoint*tint*(1.8+9*pow(j,16))*twinkle+meteor*float3(8,10,14);'''
    n=M.create_material_expression(mat,u.MaterialExpressionCustom)
    n.set_editor_property('code',code);n.set_editor_property('output_type',u.CustomMaterialOutputType.CMOT_FLOAT3)
    inp=u.CustomInput();inp.set_editor_property('input_name','Direction');clock_input=u.CustomInput();clock_input.set_editor_property('input_name','Time');n.set_editor_property('inputs',[inp,clock_input])
    clock=M.create_material_expression(mat,u.MaterialExpressionTime);clock.set_editor_property('ignore_pause',True)
    assert M.connect_material_expressions(clock,'',n,'Time')
    direction=M.create_material_expression(mat,u.MaterialExpressionCameraVectorWS)
    assert M.connect_material_expressions(direction,'',n,'Direction')
    assert M.connect_material_property(n,'',u.MaterialProperty.MP_EMISSIVE_COLOR)
    M.recompile_material(mat);E.save_loaded_asset(mat)
    vault=mesh('Star vault','Sphere',(0,0,0),(1000,1000,1000),mat)
    vault.static_mesh_component.set_editor_property('disallow_nanite',True)
    sky_mesh=u.load_asset('/Engine/EngineSky/SM_SkySphere')
    if sky_mesh:vault.static_mesh_component.set_static_mesh(sky_mesh);vault.static_mesh_component.set_material(0,mat)
    vault.static_mesh_component.set_editor_property('cast_shadow',False)
    # Moon is deliberately subtle; the stars surround the entire platform.
    moonmat=material('M_Moon',(.52,.65,.9),emission=1.4)
    moonmat.set_editor_property('shading_model',u.MaterialShadingModel.MSM_UNLIT)
    M.recompile_material(moonmat);E.save_loaded_asset(moonmat)
    moon=mesh('Moon','Sphere',(-180000,-210000,160000),(22,22,22),moonmat)
    moon.static_mesh_component.set_editor_property('cast_shadow',False)
if not night:
    clouds=actors.spawn_actor_from_class(u.VolumetricCloud,u.Vector());clouds.set_actor_label('Atmosphere/Drifting clouds')
    c=clouds.get_component_by_class(u.VolumetricCloudComponent)
    c.set_editor_property('layer_bottom_altitude',1.4);c.set_editor_property('layer_height',2.)
    c.set_material(u.load_asset('/Engine/EngineSky/VolumetricClouds/m_SimpleVolumetricCloud_Inst'))
    # Blender-authored bird; per-vertex wing weights and smooth flight in the shader.
    task=u.AssetImportTask();task.filename=str(ROOT/'assets/world/forum/forum-bird.fbx')
    task.destination_path='/Game/SolarForum/Environment/Birds';task.automated=True;task.save=True;task.replace_existing=True
    o=u.FbxImportUI();o.import_mesh=True;o.import_as_skeletal=False;o.import_materials=False;o.import_textures=False
    o.mesh_type_to_import=u.FBXImportType.FBXIT_STATIC_MESH;o.automated_import_should_detect_type=False
    o.static_mesh_import_data.combine_meshes=True;o.static_mesh_import_data.auto_generate_collision=False
    o.static_mesh_import_data.convert_scene_unit=True
    o.static_mesh_import_data.vertex_color_import_option=u.VertexColorImportOption.REPLACE
    task.options=o;assets.import_asset_tasks([task])
    birdmesh=next(u.load_asset(p) for p in task.imported_object_paths if isinstance(u.load_asset(p),u.StaticMesh))
    birdmat=material('M_Seabird',(.45,.49,.53),rough=.8);birdmat.set_editor_property('two_sided',True)
    n=M.create_material_expression(birdmat,u.MaterialExpressionCustom)
    n.set_editor_property('output_type',u.CustomMaterialOutputType.CMOT_FLOAT3)
    inputs=[]
    for name in ['Time','Origin','Position','Wing']:
        inp=u.CustomInput();inp.set_editor_property('input_name',name);inputs.append(inp)
    n.set_editor_property('inputs',inputs)
    n.set_editor_property('code',"float phase=dot(Origin.xy,float2(.012,.023));float t=Time*.075+phase;float3 p=Position-Origin;float s=sin(t),c=cos(t);float3 rotated=float3(c*p.x-s*p.y,s*p.x+c*p.y,p.z);float flap=max(0.,sin(Time*.43+phase))*sin(Time*4.1+phase)*Wing.r*32.;return rotated-p+float3(c*2800,s*2800,sin(t*1.3)*230+flap);")
    for cls,name in [(u.MaterialExpressionTime,'Time'),(u.MaterialExpressionObjectPositionWS,'Origin'),(u.MaterialExpressionWorldPosition,'Position'),(u.MaterialExpressionVertexColor,'Wing')]:
        source=M.create_material_expression(birdmat,cls)
        assert M.connect_material_expressions(source,'',n,name)
    assert M.connect_material_property(n,'',u.MaterialProperty.MP_WORLD_POSITION_OFFSET)
    M.recompile_material(birdmat);E.save_loaded_asset(birdmat)
    for i in range(6):
        a=actors.spawn_actor_from_class(u.StaticMeshActor,u.Vector(-4500+i*1500,-2500,3800+i*300));a.set_actor_label('Atmosphere/Seabird '+str(i))
        c=a.static_mesh_component;c.set_static_mesh(birdmesh)
        for slot in range(len(birdmesh.static_materials)):c.set_material(slot,birdmat)
        c.set_collision_profile_name('NoCollision');c.set_editor_property('cast_shadow',False);c.set_editor_property('bounds_scale',60.)
sky.light_component.recapture_sky()
assert level.save_current_level()
report={'status':'complete','appearance':MODE,'map':target,'lanterns':len(lanterns),'stars':night,'sunLux':8. if night else 85000.,'exposureEV100':1.5 if night else 13.}
(ROOT/f'docs/design/solar-forum/unreal-atmosphere-{MODE}.json').write_text(json.dumps(report,indent=2)+'\n')
print(report)
