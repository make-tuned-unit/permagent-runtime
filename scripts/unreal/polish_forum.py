"""Native Unreal environment pass. Geometry stays in Blender; finish lives here."""
import unreal as u, json, math, os, random, re
from pathlib import Path
# Unreal Python's positional order is roll/pitch/yaw; use explicit fields.
def rotation(pitch=0., yaw=0., roll=0.):
    return u.Rotator(pitch=pitch, yaw=yaw, roll=roll)
ROOT=Path(__file__).resolve().parents[2]
assert Path(u.Paths.get_project_file_path()).stem=='SolarForum'
MODE=os.environ.get('FORUM_APPEARANCE','day').strip().lower()
assert MODE in ('day','night'), 'FORUM_APPEARANCE must be day or night'
random.seed(42)
E=u.EditorAssetLibrary; M=u.MaterialEditingLibrary
assets=u.AssetToolsHelpers.get_asset_tools()
level=u.get_editor_subsystem(u.LevelEditorSubsystem)
actors=u.get_editor_subsystem(u.EditorActorSubsystem)
base='/Game/SolarForum/Maps/Forum'; target='/Game/SolarForum/Maps/'+('ForumCinematic' if MODE=='day' else 'ForumNight')
if not E.does_asset_exist(target):assert level.new_level_from_template(target,base)
else:assert level.load_level(target)
for a in actors.get_all_level_actors():
    if a.get_actor_label().startswith('Environment/'):actors.destroy_actor(a)
# Imported source files stay local; CC0 provenance and digests live in git.
source=ROOT/'assets/world/forum/third-party'
u.SystemLibrary.execute_console_command(None,'Interchange.FeatureFlags.Import.FBX 0')
imports=[]
for file in sorted(source.rglob('*')):
    if file.suffix.lower() not in ('.jpg','.png','.exr','.fbx'):continue
    t=u.AssetImportTask();t.filename=str(file);t.destination_path='/Game/SolarForum/Environment/Imported/'+file.relative_to(source).parts[0]
    t.automated=True;t.save=True;t.replace_existing=False
    if file.suffix.lower()=='.fbx':
        o=u.FbxImportUI();o.import_mesh=True;o.import_materials=False;o.import_textures=False;o.import_as_skeletal=False
        o.mesh_type_to_import=u.FBXImportType.FBXIT_STATIC_MESH;o.automated_import_should_detect_type=False
        o.static_mesh_import_data.combine_meshes=True;o.static_mesh_import_data.convert_scene_unit=True;o.static_mesh_import_data.auto_generate_collision=False;t.options=o
    # Do not repeatedly import the 130 MB tree on each lighting iteration.
    expected=t.destination_path+'/'+file.stem
    if not E.does_asset_exist(expected):imports.append(t)
if imports:assets.import_asset_tasks(imports)
def texture(file):
    tex=u.load_asset('/Game/SolarForum/Environment/Imported/'+file.relative_to(source).parts[0]+'/'+file.stem)
    assert isinstance(tex,u.Texture2D),file
    name=file.stem.lower()
    if '_nor_' in name:
        tex.set_editor_property('srgb',False);tex.set_editor_property('compression_settings',u.TextureCompressionSettings.TC_NORMALMAP)
        tex.set_editor_property('flip_green_channel','_gl_' in name)
    elif any(k in name for k in ['rough','alpha','arm']):tex.set_editor_property('srgb',False)
    E.save_loaded_asset(tex)
    return tex
def newmat(name):
    path='/Game/SolarForum/Environment/Materials';mat=u.load_asset(path+'/'+name)
    if not mat:mat=assets.create_asset(name,path,u.Material,u.MaterialFactoryNew())
    M.delete_all_material_expressions(mat);return mat
def node(mat,cls,**props):
    n=M.create_material_expression(mat,cls)
    for k,v in props.items():n.set_editor_property(k,v)
    return n
def link(a,out,b,inp):assert M.connect_material_expressions(a,out,b,inp),(str(a),out,str(b),inp)
def prop(n,out,mat,p):assert M.connect_material_property(n,out,p)
def scalar(mat,x):return node(mat,u.MaterialExpressionConstant,r=x)
def color(mat,rgb):return node(mat,u.MaterialExpressionConstant3Vector,constant=u.LinearColor(*rgb))
def finish(mat):M.layout_material_expressions(mat);M.recompile_material(mat);E.save_loaded_asset(mat);return mat
def world_sample(mat,file,normal=False):
    tex=node(mat,u.MaterialExpressionTextureObject,texture=texture(file))
    func=node(mat,u.MaterialExpressionMaterialFunctionCall)
    func.set_material_function(u.load_asset('/Engine/Functions/Engine_MaterialFunctions01/Texturing/'+('WorldAlignedNormal' if normal else 'WorldAlignedTexture')))
    link(tex,'',func,'TextureObject');link(color(mat,(200,200,200)),'',func,'TextureSize')
    return func,'XYZ Texture'
def scan_material(asset,name):
    mat=newmat(name)
    files=list((source/asset).glob('*.jpg'))
    for key,p,normal in [('diff',u.MaterialProperty.MP_BASE_COLOR,False),('rough',u.MaterialProperty.MP_ROUGHNESS,False),('nor_dx',u.MaterialProperty.MP_NORMAL,True)]:
        f=next(x for x in files if key in x.stem);n,out=world_sample(mat,f,normal);prop(n,out,mat,p)
    return finish(mat)
stone=scan_material('white_sandstone_blocks_02','M_ScannedLimestone')
rock=scan_material('mossy_rock','M_MossyRock')
# Names used by the FBX mapping table; the source asset names remain explicit
# in the Unreal content browser and in the emitted report.
scannedstone=stone; scannedrock=rock
def simple(name,rgb,rough=.5,metal=0):
    mat=newmat(name);prop(color(mat,rgb),'',mat,u.MaterialProperty.MP_BASE_COLOR)
    prop(scalar(mat,rough),'',mat,u.MaterialProperty.MP_ROUGHNESS);prop(scalar(mat,metal),'',mat,u.MaterialProperty.MP_METALLIC)
    return finish(mat)
bronze=simple('M_BurnishedBronze',(.34,.23,.105),.31,.83)
dark=simple('M_Basalt',(.022,.026,.042),.72)
def emissive(name,rgb,strength=4.0):
    mat=newmat(name)
    prop(color(mat,rgb),'',mat,u.MaterialProperty.MP_BASE_COLOR)
    prop(scalar(mat,.28),'',mat,u.MaterialProperty.MP_ROUGHNESS)
    e=color(mat,tuple(c*strength for c in rgb))
    prop(e,'',mat,u.MaterialProperty.MP_EMISSIVE_COLOR)
    return finish(mat)
def translucent(name,rgb,opacity,strength=0.,rough=.2):
    mat=newmat(name);mat.set_editor_property('blend_mode',u.BlendMode.BLEND_TRANSLUCENT)
    mat.set_editor_property('shading_model',u.MaterialShadingModel.MSM_DEFAULT_LIT)
    prop(color(mat,rgb),'',mat,u.MaterialProperty.MP_BASE_COLOR)
    prop(scalar(mat,rough),'',mat,u.MaterialProperty.MP_ROUGHNESS);prop(scalar(mat,0.),'',mat,u.MaterialProperty.MP_METALLIC)
    prop(scalar(mat,opacity),'',mat,u.MaterialProperty.MP_OPACITY)
    if strength:
        prop(color(mat,tuple(c*strength for c in rgb)),'',mat,u.MaterialProperty.MP_EMISSIVE_COLOR)
    return finish(mat)
amber=emissive('M_EmissiveAmber',(.95,.22,.035),3.5)
amber_strong=emissive('M_EmissiveAmberStrong',(.95,.22,.035),8.0)
cyan=emissive('M_EmissiveCyan',(.02,.65,.95),6.0)
horizon_blue=emissive('M_EmissiveHorizonBlue',(.33,.60,1.0),1.5)
hologram=translucent('M_Hologram',(.10,.85,.90),.45,6.0,.22)
hologram_continents=translucent('M_HologramContinents',(.14,.95,.92),.70,6.0,.28)
hologram_cone=translucent('M_HologramLightCone',(.10,.85,.90),.12,1.0,.18)
hologram_glass=translucent('M_HologramGlass',(.345,.68,.78),.35,.4,.16)
water=newmat('M_LivingWater');prop(color(water,(.017,.09,.095)),'',water,u.MaterialProperty.MP_BASE_COLOR)
prop(scalar(water,.08),'',water,u.MaterialProperty.MP_ROUGHNESS);prop(scalar(water,.35),'',water,u.MaterialProperty.MP_METALLIC)
wp=node(water,u.MaterialExpressionWorldPosition);mask=node(water,u.MaterialExpressionComponentMask,r=True,g=True)
link(wp,'',mask,'');scale=node(water,u.MaterialExpressionMultiply,const_b=.004);link(mask,'',scale,'A')
pan=node(water,u.MaterialExpressionPanner,speed_x=.018,speed_y=.011);link(scale,'',pan,'Coordinate')
wave=node(water,u.MaterialExpressionTextureSample,texture=u.load_asset('/Engine/Functions/Engine_MaterialFunctions02/ExampleContent/Textures/water_n'),sampler_type=u.MaterialSamplerType.SAMPLERTYPE_NORMAL)
link(pan,'',wave,'UVs');prop(wave,'RGB',water,u.MaterialProperty.MP_NORMAL);finish(water)
# Architectural glazing is a native translucent, default-lit dielectric.  The
# engine enum names below are verified in EngineTypes.h; the refraction input
# is the documented EMaterialProperty::MP_Refraction slot.
glass=newmat('M_ArchitecturalGlass');glass.set_editor_property('blend_mode',u.BlendMode.BLEND_TRANSLUCENT)
glass.set_editor_property('shading_model',u.MaterialShadingModel.MSM_DEFAULT_LIT)
prop(color(glass,(.055,.20,.17)),'',glass,u.MaterialProperty.MP_BASE_COLOR)
prop(scalar(glass,.14),'',glass,u.MaterialProperty.MP_ROUGHNESS);prop(scalar(glass,0.),'',glass,u.MaterialProperty.MP_METALLIC)
prop(scalar(glass,.24),'',glass,u.MaterialProperty.MP_OPACITY);prop(scalar(glass,1.45),'',glass,u.MaterialProperty.MP_REFRACTION);finish(glass)
# Keep the authored warm structural timber tone when an FBX material is
# re-imported. The source module carries the finer grain for GLB; this native
# factor material remains stable and lightweight for the Unreal pass.
timber=simple('M_OriginalWarmTone',(.22,.12,.055),.62)
moss=simple('M_PlanterMoss',(.10,.22,.065),.95)
strata=[simple('M_BasaltStratum'+str(i),rgb,rough) for i,(rgb,rough) in enumerate([
    ((.42,.33,.22),.90),((.30,.30,.32),.88),((.20,.15,.11),.95),((.55,.50,.42),.85)])]
hidden=newmat('M_ReplaceBlockoutFoliage');hidden.set_editor_property('blend_mode',u.BlendMode.BLEND_MASKED)
prop(scalar(hidden,0),'',hidden,u.MaterialProperty.MP_OPACITY_MASK);finish(hidden)
architecture=next(a for a in actors.get_all_level_actors() if isinstance(a,u.StaticMeshActor) and a.get_actor_label().startswith('Solar Forum'))
def slot_key(slot):
    return re.sub(r'[^a-z0-9]+',' ',str(slot.material_slot_name).lower()).strip()
def native_mapping(key):
    # Solar accents, inlays, and the detailed foliage imported from Blender
    # retain their authored materials. Only explicit structural categories are
    # overridden, so a broad texture cannot plaster every pale slab or plant.
    if any(token in key for token in ('blockout foliage','legacy foliage','placeholder foliage')): return hidden, 'obsolete foliage proxy'
    preserve=('solar','photovoltaic','memory inlay','engraved intelligence','foliage','leaves','canopy','meadow')
    if any(token in key for token in preserve): return None, None
    if any(token in key for token in ('forum white sandstone masonry','white sandstone masonry','limestone','travertine')): return scannedstone, 'scannedstone'
    for index in range(4):
        if 'basalt stratum '+str(index) in key: return strata[index], 'basalt stratum '+str(index)
    if any(token in key for token in ('forum geological strata','geological strata')): return strata[0], 'basalt stratum 0'
    if any(token in key for token in ('ridge mossy ledge','forum mossy coastal rock','mossy rock')): return rock, 'mossy rock'
    if any(token in key for token in ('forum coastal rock','coastal rock','weathered coastal rock')): return scannedrock, 'scannedrock'
    if 'forum sand fabric' in key: return fabric, 'sand fabric'
    if any(token in key for token in ('hologram glass','cyan display glass')): return hologram_glass, 'hologram glass'
    if 'hologram continents' in key: return hologram_continents, 'hologram continents'
    if 'hologram light cone' in key: return hologram_cone, 'hologram light cone'
    if 'hologram' in key: return hologram, 'hologram'
    if any(token in key for token in ('horizonblue','unlit channel')): return horizon_blue, 'emissive horizon blue'
    if any(token in key for token in ('cyan exedra','info display line')): return cyan, 'emissive cyan'
    if any(token in key for token in ('warm window','emissive window','window strip','strand amber','strand light','light bead')): return amber, 'emissive amber'
    if any(token in key for token in ('lantern amber','amber lantern','amber core','amber head','lantern head','island quay amber','harbour amber','brazier amber')): return amber_strong, 'emissive amber strong'
    if 'celestial waterfall mist' in key: return waterfall_mist, 'waterfall mist'
    if 'forum planter moss' in key: return moss, 'planter moss'
    if 'glass' in key or any(token in key for token in ('architectural glass','conservatory glass','glazed vault')): return glass, 'architectural glass'
    if any(token in key for token in ('forum oiled timber grain','oiled timber grain','oiled structural timber','forum oiled timber','timber')): return timber, 'original warm timber tone'
    if 'bronze' in key: return bronze, 'burnished bronze'
    if 'water' in key or 'lagoon' in key or 'shallow teal' in key: return water, 'living water'
    if any(token in key for token in ('harbour rope','quay rope')): return timber, 'original warm timber tone'
    if any(token in key for token in ('chitin engraved channel','chitin stone')): return dark, 'basalt'
    if 'dark stone' in key or 'dark_stone' in key: return dark, 'basalt'
    return None, None
waterfall_mist=translucent('M_CelestialWaterfallMist',(.72,.92,.95),.30,0.,.18)
fabric=simple('M_SandFabric',(.78,.66,.48),.92)
material_assignments={}; reset_slots=[]; obsolete_proxy_slots=[]; default_slots=[]
# Reimported FBX slots can retain component overrides from an older material
# layout. First restore each slot's imported interface, then apply only the
# explicit mappings above. This also leaves newly authored foliage untouched.
for i,slot in enumerate(architecture.static_mesh_component.static_mesh.static_materials):
    imported=slot.material_interface
    if imported:
        architecture.static_mesh_component.set_material(i,imported);reset_slots.append(str(slot.material_slot_name))
for i,slot in enumerate(architecture.static_mesh_component.static_mesh.static_materials):
    name=slot_key(slot);material,label=native_mapping(name)
    if material:
        architecture.static_mesh_component.set_material(i,material)
        material_assignments.setdefault(label,[]).append(str(slot.material_slot_name))
        if material is hidden: obsolete_proxy_slots.append(str(slot.material_slot_name))
    elif not any(token in name for token in ('solar','photovoltaic','memory inlay','engraved intelligence','foliage','leaves','canopy','meadow')):
        default_slots.append(str(slot.material_slot_name))
# Native foliage materials retain scanned cutout masks and normal/roughness maps.
models={}; model_report={}
for asset in ['jacaranda_tree','fern_02','rock_09']:
    mesh=u.load_asset('/Game/SolarForum/Environment/Imported/'+asset+'/'+asset+'_2k');assert isinstance(mesh,u.StaticMesh),asset
    models[asset]=mesh;files=[f for f in (source/asset/'textures').iterdir() if f.suffix.lower() in ('.png','.jpg','.exr')]
    for i,slot in enumerate(mesh.static_materials):
        slotname=str(slot.material_slot_name).lower();tokens=slotname.replace('.','_').split('_')
        diffs=[f for f in files if '_diff_' in f.name]
        assert diffs, ('No diffuse texture for scanned asset', asset)
        diff=max(diffs,key=lambda f:sum(t in f.stem for t in tokens if len(t)>2))
        prefix=diff.name.split('_diff_')[0]
        mat=newmat('M_'+prefix);mat.set_editor_property('two_sided',True)
        alpha=next((f for f in files if f.name.startswith(prefix+'_alpha_')),None)
        if alpha:mat.set_editor_property('blend_mode',u.BlendMode.BLEND_MASKED)
        for kind,p in [('diff',u.MaterialProperty.MP_BASE_COLOR),('nor_gl',u.MaterialProperty.MP_NORMAL),('rough',u.MaterialProperty.MP_ROUGHNESS),('alpha',u.MaterialProperty.MP_OPACITY_MASK)]:
            file=next((f for f in files if f.name.startswith(prefix+'_'+kind+'_')),None)
            if not file:continue
            tex=texture(file);n=node(mat,u.MaterialExpressionTextureSample,texture=tex,sampler_type=u.MaterialSamplerType.SAMPLERTYPE_NORMAL if kind=='nor_gl' else u.MaterialSamplerType.SAMPLERTYPE_LINEAR_COLOR if kind in ('rough','alpha') else u.MaterialSamplerType.SAMPLERTYPE_COLOR)
            prop(n,'RGB' if kind in ('diff','nor_gl') else 'R',mat,p)
        if alpha:
            mat.set_editor_property('shading_model',u.MaterialShadingModel.MSM_TWO_SIDED_FOLIAGE)
            prop(color(mat,(.11,.19,.04)),'',mat,u.MaterialProperty.MP_SUBSURFACE_COLOR)
        finish(mat)
        if slot.material_interface != mat:mesh.set_material(i,mat)
    E.save_loaded_asset(mesh)
    bounds=mesh.get_bounding_box();model_report[asset]={'heightCm':bounds.max.z-bounds.min.z,'slots':[str(s.material_slot_name) for s in mesh.static_materials]}
def spawn(mesh,label,pos,scale=(1,1,1),yaw=0,material=None):
    a=actors.spawn_actor_from_class(u.StaticMeshActor,u.Vector(*pos),rotation(0,yaw,0));a.set_actor_label('Environment/'+label)
    a.static_mesh_component.set_static_mesh(mesh);a.set_actor_scale3d(u.Vector(*scale))
    a.static_mesh_component.set_collision_profile_name('NoCollision')
    if material:a.static_mesh_component.set_material(0,material)
    return a
def plant(asset,pos,height,label):
    mesh=models[asset];b=mesh.get_bounding_box();s=height/(b.max.z-b.min.z)
    return spawn(mesh,label,(pos[0],pos[1],pos[2]-b.min.z*s),(s,s,s),random.uniform(0,360))
# Retain the architect's pots and walking clearances; replace angular canopies.
for i,(x,y) in enumerate([(-12,-9),(12,-9),(-13,6),(13,6),(-7,14),(7,14)]):
    plant('jacaranda_tree',(x*100,-y*100,80),620,'Forum tree '+str(i))
    for j in range(5):
        a=j*math.tau/5;plant('fern_02',(x*100+105*math.cos(a),-y*100+105*math.sin(a),83),65,'Planter fern')
# Carry the authored outer planting layout into the native photogrammetry pass.
# The coordinates are generated from the saved Blender source, not a second
# random distribution that could put trunks across the circulation routes.
forest_path=ROOT/'assets/world/forum/forest.json'
if forest_path.exists():
    for i,tree in enumerate(json.loads(forest_path.read_text())):
        x,y=float(tree['x']),float(tree['y']);r=math.hypot(x,y)
        path_distance=min(abs(r-104),min(abs(x),abs(y)) if r<150 else 100)
        mask=max(0,min(1,(path_distance-6)/10))
        hill=max(0,math.sin(x*.065+.8)*math.cos(y*.047))*3.2+max(0,math.cos(x*.11-y*.08))*.7
        ground=-.25+hill*mask if r>=69 else -.65
        if r>149:ground-=((r-149)/25)**2*3.5
        plant('jacaranda_tree',(x*100,-y*100,ground*100),max(650,float(tree['height'])*125),'District tree '+str(i))
# A mineral-and-water landscape beneath the bridges, with planted banks beyond.
plane=u.load_asset('/Engine/BasicShapes/Plane');sphere=u.load_asset('/Engine/BasicShapes/Sphere')
spawn(plane,'Reflecting lagoon',(0,0,-150),(54,54,54),material=water)
for i in range(18):
    a=i*math.tau/18;r=random.uniform(7500,10000);x,y=r*math.cos(a),r*math.sin(a)
    spawn(sphere,'Terraced bank',(x,y,-450),(32,23,7),math.degrees(a),rock)
    plant('rock_09',(x,y,-100),random.uniform(500,1000),'Scanned outcrop')
    if i%2==0:plant('jacaranda_tree',(x,y,120),random.uniform(1000,1450),'Distant canopy')
for i in range(32):
    a=i*math.tau/32
    if abs(math.sin(a*2))<.25:continue
    plant('fern_02',(3650*math.cos(a),3650*math.sin(a),45),95,'Promenade planting')
# Native atmosphere, indirect light, reflections and filmic exposure.
sun=next(a for a in actors.get_all_level_actors() if isinstance(a,u.DirectionalLight))
sun.set_actor_rotation(rotation(-28,-40,0) if MODE=='day' else rotation(-16,132,0),False)
sun.light_component.set_editor_property('intensity',60000. if MODE=='day' else 2500.)
sun.light_component.set_editor_property('atmosphere_sun_light',MODE=='day')
sun.light_component.set_editor_property('light_source_angle',1.2)
sky=next(a for a in actors.get_all_level_actors() if isinstance(a,u.SkyLight));sky.light_component.set_editor_property('intensity',1. if MODE=='day' else .22)
sky.light_component.set_mobility(u.ComponentMobility.MOVABLE)
fog=actors.spawn_actor_from_class(u.ExponentialHeightFog,u.Vector(0,0,-100));fog.set_actor_label('Environment/Atmospheric depth')
fog.component.set_editor_property('fog_density',.009);fog.component.set_editor_property('enable_volumetric_fog',True)
post=actors.spawn_actor_from_class(u.PostProcessVolume,u.Vector());post.set_actor_label('Environment/Filmic finish');post.set_editor_property('unbound',True)
settings=post.get_editor_property('settings')
for key,value in {'auto_exposure_min_brightness':13.,'auto_exposure_max_brightness':13.,'bloom_intensity':.18,'vignette_intensity':.12,'lumen_scene_lighting_quality':2.,'lumen_final_gather_quality':2.}.items():
    settings.set_editor_property('override_'+key,True);settings.set_editor_property(key,value)
post.set_editor_property('settings',settings)
cam=next(a for a in actors.get_all_level_actors() if isinstance(a,u.CameraActor))
cam.set_actor_location_and_rotation(u.Vector(1500,2400,550),rotation(-9,-122,0),False,False)
cam.camera_component.set_editor_property('field_of_view',65.)
assert level.save_current_level()
report={'status':'complete','appearance':MODE,'map':target,'scannedAssets':model_report,'environmentActors':sum(a.get_actor_label().startswith('Environment/') for a in actors.get_all_level_actors()),'materialAssignments':material_assignments,'defaultMaterialSlots':default_slots,'unmappedForumSlots':[s for s in default_slots if any(token in re.sub(r'[^a-z0-9]+',' ',s.lower()).strip() for token in ('forum','job16','mesh gate','basalt stratum','ridge mossy','celestial'))],'resetImportedMaterialSlots':reset_slots,'obsoleteProxySlotsHidden':obsolete_proxy_slots,'renderVerified':False}
(ROOT/('docs/design/solar-forum/unreal-environment-'+MODE+'.json')).write_text(json.dumps(report,indent=2)+'\n')
u.log('FORUM_ENVIRONMENT_BUILT '+json.dumps(report))
