"""Author role-specific rigid-armor character meshes for the existing live rig.

Each mesh carries bone/channel extras. Runtime binds these to its existing
12-bone skeleton; no baked work animation can override live task state.
"""
import bpy
import math
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT/'assets/world/blender/characters'
PUBLIC = ROOT/'ui/command-center/public/world/characters'
SOURCE.mkdir(parents=True, exist_ok=True)
PUBLIC.mkdir(parents=True, exist_ok=True)
bpy.context.preferences.filepaths.save_version = 0

PROFILES = {
    'henry': ('ivory', .31, 'mantle'),
    'librarian': ('bronze', .25, 'archive'),
    'reader': ('ivory', .23, 'lens'),
    'watcher': ('dark', .27, 'antenna'),
    'steward': ('bronze', .33, 'tools'),
    'strix': ('dark', .34, 'crest'),
    'financier': ('bronze', .26, 'scales'),
    'forecaster': ('ivory', .24, 'astrolabe'),
    'council': ('ivory', .32, 'mantle'),
    'polybot': ('dark', .29, 'tools'),
    'picker': ('bronze', .24, 'lens'),
    'growth_measurement': ('ivory', .25, 'astrolabe'),
}

# Clothing is identity, never a fabricated activity indicator. All panels bind
# to the existing skeleton; articulated skirts move with the corresponding leg.
OUTFITS = {
    'henry': ('Ambassador frock', (.10,.17,.24), .36),
    'librarian': ('Archivist vestments', (.22,.075,.055), .32),
    'reader': ('Scholar waistcoat', (.075,.22,.20), .28),
    'watcher': ('Surveyor field jacket', (.10,.16,.12), .33),
    'steward': ('Artisan apron', (.27,.14,.055), .38),
    'strix': ('Sentinel cuirass', (.10,.075,.19), .39),
    'financier': ('Treasurer double breast', (.055,.13,.13), .31),
    'forecaster': ('Celestial navigator', (.10,.12,.30), .30),
    'council': ('Consular stole', (.30,.25,.18), .39),
    'polybot': ('Fabricator utility harness', (.22,.09,.035), .35),
    'picker': ('Curator expedition vest', (.18,.23,.105), .29),
    'growth_measurement': ('Botanical survey tunic', (.06,.24,.14), .31),
}

def mat(name, rgb, metal=.6):
    m = bpy.data.materials.new(name)
    m.diffuse_color = (*rgb,1)
    m.use_nodes = True
    n = m.node_tree.nodes.get('Principled BSDF')
    n.inputs['Base Color'].default_value = (*rgb,1)
    n.inputs['Metallic'].default_value = metal
    n.inputs['Roughness'].default_value = .34
    return m

def position(p):
    return (p[0], -p[2], p[1])

def finish(obj, name, bone, material, channel='metal'):
    obj.name = name
    obj['bone'] = bone
    obj['channel'] = channel
    obj['schema'] = 'permagent.rigid-armor.v1'
    obj.data.materials.append(material)
    return obj

def ellipsoid(name, p, size, bone, material, channel='metal'):
    bpy.ops.mesh.primitive_uv_sphere_add(segments=16, ring_count=8, location=position(p))
    obj = finish(bpy.context.object, name, bone, material, channel)
    obj.scale = (size[0],size[2],size[1])
    for face in obj.data.polygons:
        face.use_smooth = True
    return obj

def plate(name,p,size,bone,material,channel='metal'):
    bpy.ops.mesh.primitive_cube_add(size=1,location=position(p))
    obj = finish(bpy.context.object,name,bone,material,channel)
    obj.scale = (size[0],size[2],size[1])
    bpy.ops.object.transform_apply(location=False,rotation=False,scale=True)
    bevel = obj.modifiers.new('Machined chamfer','BEVEL')
    bevel.width,bevel.segments = min(size)*.22,2
    bpy.context.view_layer.objects.active = obj
    bpy.ops.object.modifier_apply(modifier=bevel.name)
    return obj

def jacket(name, width, material):
    # Open-front, tailored shell with a flared hem and shaped shoulders.
    # A closed thickness avoids double-sided rendering and adds no new draw.
    vertices, faces = [], []
    levels = [(.91,width*.85,.205),(1.18,width*.87,.235),
              (1.47,width,.23),(1.60,width*.72,.15)]
    count = 17
    for inset in (0,.018):
        for y,w,d in levels:
            for i in range(count):
                a = .38 + (math.tau-.76)*i/(count-1)
                vertices.append(position(((w-inset)*math.sin(a),y,(d-inset)*math.cos(a))))
    layer = len(levels)*count
    for side in range(2):
        for row in range(3):
            for i in range(count-1):
                k = side*layer+row*count+i
                face = (k,k+1,k+count+1,k+count)
                faces.append(face if side == 0 else tuple(reversed(face)))
    for row in (0,3):
        for i in range(count-1):
            k=row*count+i
            faces.append((k,k+layer,k+layer+1,k+1))
    for i in (0,count-1):
        for row in range(3):
            k=row*count+i
            faces.append((k,k+count,k+count+layer,k+layer))
    mesh=bpy.data.meshes.new(name)
    mesh.from_pydata(vertices,[],faces)
    mesh.update()
    obj=bpy.data.objects.new(name,mesh)
    bpy.context.collection.objects.link(obj)
    finish(obj,name,'spine',material)
    # Recalculate winding after closed-shell construction.
    bpy.ops.object.select_all(action='DESELECT')
    obj.select_set(True)
    bpy.context.view_layer.objects.active=obj
    bpy.ops.object.mode_set(mode='EDIT')
    bpy.ops.mesh.select_all(action='SELECT')
    bpy.ops.mesh.normals_make_consistent(inside=False)
    bpy.ops.object.mode_set(mode='OBJECT')

def outfit(identity, width, gold, joint, trim):
    name,color,cut=OUTFITS[identity]
    cloth=mat(name+' · woven composite',color,.08)
    jacket(name,cut,cloth)
    for sign,side in [(1,'L'),(-1,'R')]:
        lapel=plate(name+' lapel '+side,(sign*.125,1.43,.235),(.085,.30,.025),'spine',cloth)
        lapel.rotation_euler.y=sign*.20
        plate(name+' cuff '+side,(sign*.42,.80,.025),(.163,.065,.18),'fore'+side,cloth)
        for finger in range(3):
            plate('Articulated digit '+side+str(finger),(sign*.42+(finger-1)*.036,.60,.08),
                  (.027,.10,.065),'fore'+side,joint)
    for i in range(3):
        ellipsoid(name+' clasp '+str(i),(.055,1.17+i*.085,.255),(.018,.018,.012),'spine',gold)
    if identity in ('henry','librarian','council','forecaster'):
        for sign,side in [(1,'L'),(-1,'R')]:
            plate(name+' articulated skirt '+side,(sign*.16,.62,-.135),(.21,.39,.045),'thigh'+side,cloth)
            plate(name+' skirt piping '+side,(sign*.25,.62,-.107),(.015,.34,.014),'thigh'+side,gold)
    if identity == 'henry':
        plate('Ambassador asymmetric sash',(-.18,1.32,.265),(.065,.49,.026),'spine',trim,'trim')
    elif identity == 'librarian':
        for sign in (-1,1):
            plate('Archivist long stole',(sign*.19,1.30,.258),(.105,.57,.03),'spine',gold)
            for i in range(4):
                plate('Archive index embroidery',(sign*.19,1.12+i*.08,.277),(.06,.015,.01),'spine',joint)
    elif identity == 'reader':
        plate('Scholar breast notebook',(-.21,1.30,.26),(.11,.16,.045),'spine',gold)
    elif identity == 'watcher':
        plate('Surveyor shoulder scanner',(-.34,1.64,0),(.18,.11,.22),'armR',joint)
        for i in range(3):
            plate('Scanner signal fin',(-.34,1.66+i*.035,-.12),(.14,.014,.035),'armR',gold)
    elif identity == 'steward':
        plate('Artisan apron bib',(0,1.17,.27),(.33,.43,.035),'spine',cloth)
        for sign in (-1,1):
            plate('Artisan riveted pocket',(sign*.105,1.03,.30),(.13,.12,.045),'spine',joint)
    elif identity == 'strix':
        for sign,side in [(1,'L'),(-1,'R')]:
            for i in range(3):
                plate('Sentinel layered pauldron '+side+str(i),(sign*(.30+i*.035),1.58-i*.065,0),
                      (.23,.08,.32),'arm'+side,cloth)
    elif identity == 'financier':
        for i in range(4):
            ellipsoid('Treasurer second button row',(-.075,1.12+i*.085,.265),(.017,.017,.012),'spine',gold)
        plate('Treasurer ledger case',(.28,.81,-.08),(.11,.23,.19),'root',cloth)
    elif identity == 'forecaster':
        for i in range(7):
            ellipsoid('Navigator constellation',(-.19+math.sin(i*2)*.05,1.12+i*.06,.27),(.015,.015,.01),'spine',gold)
    elif identity == 'council':
        for sign in (-1,1):
            plate('Consular broad stole',(sign*.22,1.34,.26),(.14,.48,.03),'spine',gold)
            plate('Consular shoulder tablet',(sign*.30,1.64,0),(.25,.06,.34),'spine',cloth)
    elif identity == 'polybot':
        for sign in (-1,1):
            plate('Fabricator harness',(sign*.19,1.30,.26),(.065,.5,.04),'spine',joint)
        plate('Fabricator dorsal battery',(0,1.28,-.29),(.30,.4,.12),'spine',cloth)
    elif identity == 'picker':
        for sign in (-1,1):
            plate('Curator sample pouch',(sign*.22,1.17,.255),(.14,.21,.07),'spine',joint)
        plate('Curator expedition brim',(0,2.10,.075),(.52,.035,.39),'head',cloth)
    elif identity == 'growth_measurement':
        for i in range(3):
            ellipsoid('Botanical sample capsule',(.21,1.12+i*.10,.27),(.04,.075,.035),'spine',gold)
        plate('Survey measuring instrument',(-.46,.93,.14),(.08,.22,.075),'foreR',cloth)

for identity,(tone,width,gear) in PROFILES.items():
    bpy.ops.object.select_all(action='SELECT')
    bpy.ops.object.delete(use_global=False)
    palettes = {'ivory':(.72,.68,.58), 'bronze':(.35,.24,.13), 'dark':(.095,.12,.17)}
    body = mat(identity+' · ceramic alloy',palettes[tone])
    joint = mat(identity+' · graphite',(.035,.045,.065))
    gold = mat(identity+' · brushed bronze',(.44,.31,.14),.75)
    trim = mat(identity+' · identity channel',(.7,.7,.7),.15)
    visor = mat(identity+' · state visor',(.4,.8,.9),.1)
    ellipsoid('Articulated thorax',(0,1.29,0),(width,.40,.19),'spine',body)
    plate('Sternum shield',(0,1.35,.165),(.25,.38,.075),'spine',gold)
    ellipsoid('Waist coupling',(0,.94,0),(.17,.16,.135),'spine',joint)
    plate('Pelvis',(0,.76,0),(.34,.22,.25),'root',body)
    ellipsoid('Neck coupling',(0,1.73,0),(.085,.15,.085),'head',joint)
    ellipsoid('Helmet',(0,1.94,0),(.215,.255,.185),'head',body)
    plate('Face mask',(0,1.92,.163),(.29,.15,.08),'head',joint)
    plate('Expressive visor',(0,1.96,.211),(.24,.035,.012),'head',visor,'visor')
    plate('Forehead identity inlay',(0,2.085,.145),(.022,.13,.017),'head',trim,'trim')
    for sign,side in [(1,'L'),(-1,'R')]:
        ellipsoid('Shoulder '+side,(sign*.32,1.51,0),(.145,.15,.16),'arm'+side,gold)
        ellipsoid('Upper arm '+side,(sign*.37,1.3,0),(.095,.22,.1),'arm'+side,body)
        ellipsoid('Elbow '+side,(sign*.42,1.04,0),(.075,.08,.075),'fore'+side,joint)
        plate('Bracer '+side,(sign*.42,.88,.025),(.14,.28,.16),'fore'+side,body)
        plate('Hand '+side,(sign*.42,.68,.045),(.12,.13,.15),'fore'+side,joint)
        plate('Bracer engraving '+side,(sign*.42,.9,.111),(.022,.19,.01),'fore'+side,trim,'trim')
        ellipsoid('Thigh '+side,(sign*.12,.48,0),(.105,.23,.12),'thigh'+side,body)
        plate('Shin '+side,(sign*.12,.17,.025),(.145,.28,.17),'calf'+side,body)
        plate('Foot '+side,(sign*.12,.035,.1),(.18,.07,.30),'calf'+side,joint)
        plate('Collar trim '+side,(sign*.20,1.57,.11),(.12,.04,.05),'spine',trim,'trim')
    if gear == 'mantle':
        for sign in (-1,1):
            plate('Ceremonial layered mantle',(sign*.29,1.57,-.025),(.29,.13,.4),'spine',body)
            plate('Mantle edging',(sign*.37,1.6,.15),(.2,.025,.025),'spine',gold)
    elif gear == 'archive':
        for i in range(5):
            plate('Archive folio '+str(i),((i-2)*.08,1.23,-.25),(.055,.46,.13),'spine',gold)
    elif gear == 'lens':
        ellipsoid('Optical reading lens',(.13,1.96,.24),(.105,.105,.07),'head',gold)
        ellipsoid('Lens aperture',(.13,1.96,.303),(.072,.072,.012),'head',visor,'visor')
    elif gear == 'antenna':
        for sign in (-1,1):
            plate('Watch aerial',(sign*.2,2.17,-.06),(.026,.43,.025),'head',gold)
    elif gear == 'tools':
        for i in range(3):
            plate('Tool cartridge '+str(i),((i-1)*.1,.86,.15),(.06,.23,.075),'root',gold)
        plate('Tool shoulder case',(.4,1.57,-.05),(.22,.20,.29),'armL',body)
    elif gear == 'crest':
        for i in range(5):
            plate('Guard crest '+str(i),(0,2.2+i*.016,(i-2)*.065),(.055,.18,.05),'head',gold)
    elif gear == 'scales':
        plate('Balance yoke',(0,1.56,-.23),(.74,.045,.045),'spine',gold)
        for sign in (-1,1):
            plate('Balance pan',(sign*.34,1.42,-.23),(.16,.045,.12),'spine',gold)
    elif gear == 'astrolabe':
        bpy.ops.mesh.primitive_torus_add(major_radius=.21,minor_radius=.02,
            major_segments=24,minor_segments=6,location=position((0,1.33,-.26)),
            rotation=(math.pi/2,0,0))
        finish(bpy.context.object,'Dorsal astrolabe','spine',gold)
    outfit(identity,width,gold,joint,trim)
    for obj in bpy.context.scene.objects:
        if obj.type == 'MESH':
            obj['identity'] = identity
            obj['outfit'] = OUTFITS[identity][0]
    bpy.context.scene.unit_settings.system = 'METRIC'
    bpy.ops.wm.save_as_mainfile(filepath=str(SOURCE/(identity+'.blend')))
    bpy.ops.export_scene.gltf(filepath=str(PUBLIC/(identity+'.glb')),
        export_format='GLB',export_yup=True,export_extras=True,
        export_animations=False,export_cameras=False,export_lights=False)
    assert (PUBLIC/(identity+'.glb')).stat().st_size < 1_000_000
    print('CHARACTER_EXPORTED',identity)
