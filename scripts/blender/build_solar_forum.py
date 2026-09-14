"""Isolated solarpunk forum. Blender source + batched GLB + Unreal FBX.
Run Blender --background --factory-startup --python-exit-code 1 --python this_file.py.
Never touches an interactive Blender scene or the production World assets.
"""
import bpy, math, json, random, re, hashlib, sys
import subprocess
from pathlib import Path
from mathutils import Vector
random.seed(42)
ROOT = Path(__file__).resolve().parents[2]
# FORUM_SCRATCH=<dir> redirects every output (blend, renders, GLB, FBX, manifest)
# to a scratch directory so modeling dry runs never overwrite the shipped assets
# or the asset revision. Production builds leave it unset.
import os
SCRATCH = os.environ.get('FORUM_SCRATCH')
OUT = Path(SCRATCH) / 'assets' if SCRATCH else ROOT / 'assets/world/forum'
WEB = Path(SCRATCH) / 'web' if SCRATCH else ROOT / 'ui/command-center/public/world'
OUT.mkdir(parents=True, exist_ok=True)
WEB.mkdir(parents=True, exist_ok=True)
bpy.ops.object.select_all(action='SELECT')
bpy.ops.object.delete(use_global=False)
bpy.context.preferences.filepaths.save_version = 0
scene = bpy.context.scene
scene.unit_settings.system = 'METRIC'
materials = []
def mat(name, color, metal=0, rough=.65):
    m = bpy.data.materials.new(name); m.diffuse_color = (*color, 1); m.use_nodes = True
    p = m.node_tree.nodes.get('Principled BSDF')
    p.inputs['Base Color'].default_value = (*color, 1)
    p.inputs['Metallic'].default_value = metal; p.inputs['Roughness'].default_value = rough
    materials.append(m); return m
# Read the shared World palette; do not fork Permagent's brand colors.
palette_text = (ROOT/'ui/command-center/src/components/world/shared/palette.ts').read_text()
token_text = (ROOT/'ui/command-center/src/styles/tokens.ts').read_text()
def palette(key):
    code = re.search(r"NEON_ACCENT = '(#[0-9A-Fa-f]+)'", token_text).group(1) if key == 'neonCyan' else re.search(key+r": '(#[0-9A-Fa-f]+)'", palette_text).group(1)
    values = [int(code[i:i+2],16)/255 for i in (1,3,5)]
    return tuple(v/12.92 if v <= .04045 else ((v+.055)/1.055)**2.4 for v in values)
stone = mat('Travertine', palette('marbleVein'))
pale = mat('Limestone', palette('marble'))
clay = mat('Terracotta', (.40,.15,.075))
bronze = mat('Aged bronze', palette('bronze'), .45)
solar = mat('Jade photovoltaic glass', (.025,.16,.15), .4, .25)
water = mat('Water', (.045,.35,.32), .35, .17)
leaf = mat('Olive leaves', (.16,.30,.09))
leaf2 = mat('Sunlit foliage', (.33,.43,.12))
wood = mat('Bark', (.15,.09,.035))
dark = mat('Permagent dark stone', palette('darkStone'))
cyan = mat('Engraved intelligence', palette('neonCyan'), .2, .4)
cyan.node_tree.nodes.get('Principled BSDF').inputs['Emission Color'].default_value=(*palette('neonCyan'),1)
cyan.node_tree.nodes.get('Principled BSDF').inputs['Emission Strength'].default_value=.65
violet = mat('Brain memory inlay', palette('violet'), .25)
def finish(o, name, material):
    o.name = name; o.data.materials.append(material); o['forum_architecture'] = True
    return o

def box(name, loc, scale, material, angle=0):
    sx,sy,sz=[v/2 for v in scale]
    vertices=[(-sx,-sy,-sz),(sx,-sy,-sz),(sx,sy,-sz),(-sx,sy,-sz),(-sx,-sy,sz),(sx,-sy,sz),(sx,sy,sz),(-sx,sy,sz)]
    faces=[(0,3,2,1),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)]
    mesh=bpy.data.meshes.new(name);mesh.from_pydata(vertices,[],faces);mesh.update()
    o=bpy.data.objects.new(name,mesh);scene.collection.objects.link(o);o.location=loc;o.rotation_euler.z=angle
    return finish(o,name,material)

def cyl(name, loc, radius, depth, material, top=None, verts=32):
    upper=radius if top is None else top
    vertices=[(r*math.cos(i*math.tau/verts),r*math.sin(i*math.tau/verts),z) for r,z in [(radius,-depth/2),(upper,depth/2)] for i in range(verts)]
    faces=[tuple(reversed(range(verts))),tuple(range(verts,verts*2))]
    faces += [(i,(i+1)%verts,(i+1)%verts+verts,i+verts) for i in range(verts)]
    mesh=bpy.data.meshes.new(name);mesh.from_pydata(vertices,[],faces);mesh.update()
    for polygon in mesh.polygons:polygon.use_smooth=polygon.index>=2
    o=bpy.data.objects.new(name,mesh);scene.collection.objects.link(o);o.location=loc
    return finish(o,name,material)

def ring(name, loc, radius, thick, material):
    bpy.ops.mesh.primitive_torus_add(major_segments=96,minor_segments=8,location=loc,major_radius=radius,minor_radius=thick)
    return finish(bpy.context.object,name,material)

def branch(a,b,r,material):
    o=cyl('Living branch',(Vector(a)+Vector(b))/2,r,(Vector(b)-Vector(a)).length,material,top=r*.65,verts=8)
    o.rotation_euler=(Vector(b)-Vector(a)).to_track_quat('Z','Y').to_euler()

# Terraced civic island, walkable zero datum, circular paving and radial brass joints.
for r,z,d in [(22,-.95,.5),(21,-.6,.3),(20,-.3,.3),(18.8,-.08,.16)]:
    cyl('Forum terrace',(0,0,z),r,d,stone if r>20 else pale,verts=128)
for r in [5,10,15,18.3]: ring('Inlaid civic ring',(0,0,.014),r,.028,bronze)
for i in range(32):
    a=i*math.tau/32
    box('Paving joint',(math.cos(a)*11.8,math.sin(a)*11.8,.012),(13,.022,.015),stone,a)
# The central water court is replaced by the gathering terrace module below.
# Its impluvium survives as a north-side crescent so the civic routes keep
# their clear sightlines and the central hologram can read from human height.
# Colonnades leave south entrance open. Entablatures, capitals and solar pergolas.
for i in range(21):
    a=math.radians(5+i*8.5); x,y=17*math.cos(a),17*math.sin(a)
    cyl('Column foot',(x,y,.16),.67,.32,stone)
    cyl('Attic base',(x,y,.4),.5,.16,pale)
    cyl('Tapered column',(x,y,3),.34,5.1,pale,top=.27)
    for z,r in [(.55,.39),(5.48,.38),(5.65,.48)]: cyl('Column moulding',(x,y,z),r,.16,pale)
    box('Abacus',(x,y,5.82),(1.12,1.12,.23),pale,a)
    box('Entablature',(x,y,6.08),(1.1,2.7,.32),stone,a)
    box('Solar canopy',(x*1.035,y*1.035,6.52),(4,2.55,.12),solar,a)
    for t in [-.8,0,.8]:
        box('Solar mullion',(x*1.035-math.sin(a)*t,y*1.035+math.cos(a)*t,6.60),(4,.04,.03),bronze,a)
# The archive now lives on the upper garden gallery, authored below.
# Living courtyards, irrigated gardens and olive trees.
for x,y in [(-12,-9),(12,-9),(-13,6),(13,6),(-7,14),(7,14)]:
    cyl('Garden planter',(x,y,.38),1.65,.76,clay)
    cyl('Living soil',(x,y,.79),1.45,.08,wood)
    branch((x,y,.8),(x+.12,y,4),.17,wood)
    for k in range(7):
        a=k*math.tau/7; tx=x+math.cos(a)*1.1; ty=y+math.sin(a)*1.1; tz=3.5+random.random()*.9
        branch((x,y,2.6),(tx,ty,tz),.065,wood)
        bpy.ops.mesh.primitive_ico_sphere_add(subdivisions=2,radius=1,location=(tx,ty,tz+.3))
        o=finish(bpy.context.object,'Olive canopy',leaf if k%2 else leaf2); o.scale=(1.1,.85,.65)
    for k in range(12):
        a=k*math.tau/12
        bpy.ops.mesh.primitive_ico_sphere_add(subdivisions=1,radius=.3,location=(x+1.2*math.cos(a),y+1.2*math.sin(a),.9))
        finish(bpy.context.object,'Herb garden',leaf)
    box('Irrigation channel',(x,y-2,.06),(3,.25,.1),water)
# Four teaching / commissioning tables move out to r≈12.5, leaving the council
# ring available for the eight sofa segments and keeping the cardinal walks open.
for a in [math.radians(35),math.radians(145),math.radians(215),math.radians(325)]:
    x,y=12.5*math.cos(a),12.5*math.sin(a)
    for dx in [-.9,.9]: box('Table trestle',(x+dx,y,.65),(.2,1,1.3),bronze)
    box('Petition table',(x,y,1.35),(2.6,1.2,.16),pale)
    for dy in [-1.3,1.3]:
        box('Civic bench',(x,y+dy,.55),(2.6,.5,.2),stone)
        for dx in [-.9,.9]: box('Bench foot',(x+dx,y+dy,.25),(.2,.4,.5),bronze)
# Broad front arrival stair.
for i in range(5): box('Arrival stair',(0,-19.5-i*.5,-.12-i*.18),(9,1,.25),pale)
# Permagent's civic infrastructure: Build, Brain, Automate and Mesh retain
# the existing product names. Geometry expresses their roles, never live status.
for x,y,label in [(-14,0,'BUILD'),(14,0,'BRAIN'),(-10,12,'AUTOMATE'),(10,12,'MESH')]:
    cyl(label+' threshold',(x,y,.12),2.25,.24,dark)
    ring(label+' threshold bronze',(x,y,.26),2.05,.06,bronze)
    box(label+' lectern',(x,y,1),(.9,.7,1.5),dark)
    box(label+' tablet',(x,y,1.82),(1.25,.85,.1),bronze)
    box(label+' engraved display',(x,y-.12,1.89),(.85,.38,.015),violet if label=='BRAIN' else cyan)
    # A low information stele integrates wayfinding into stone.
    box(label+' wayfinding',(x,y+1.7,1.25),(2,.24,2.5),dark)
    bpy.ops.object.text_add(location=(x-.78,y+1.55,1.65),rotation=(math.pi/2,0,0))
    o=bpy.context.object; o.data.body=label; o.data.size=.3; o.data.extrude=.004
    o.data.materials.append(bronze); o['forum_architecture']=True
    bpy.ops.object.convert(target='MESH')
# Radial inlaid conduits: light is a fine architectural detail, not a status.
for a in [0, math.pi/2, math.pi, math.pi*1.5]:
    x,y=math.cos(a)*9,math.sin(a)*9
    box('Dark service channel',(x,y,.017),(9,.13,.025),dark,a)
    box('Cyan engraved conduit',(x,y,.032),(9,.028,.008),cyan,a)
# Solar roof ribs, hanging vines, lantern housings and an outer garden belt.
for i in range(0,21,2):
    a=math.radians(5+i*8.5); x,y=17*math.cos(a),17*math.sin(a)
    box('Pergola bronze rib',(x*1.035,y*1.035,6.7),(4.1,.1,.12),bronze,a)
    cyl('Lantern cage',(x*.97,y*.97,5.05),.16,.55,bronze,verts=12)
    cyl('Lantern engraved core',(x*.97,y*.97,5.05),.10,.37,cyan,verts=12)
    for j in range(7):
        z=6.3-j*.21
        bpy.ops.mesh.primitive_ico_sphere_add(subdivisions=1,radius=.14,location=(x+.18*math.sin(j),y-.4,z))
        finish(bpy.context.object,'Hanging vine',leaf)
for i in range(32):
    a=math.radians(10+i*5); x,y=19.8*math.cos(a),19.8*math.sin(a)
    cyl('Outer herb pot',(x,y,.14),.38,.35,clay,verts=12)
    bpy.ops.mesh.primitive_ico_sphere_add(subdivisions=1,radius=.44,location=(x,y,.5))
    finish(bpy.context.object,'Pollinator garden',leaf2)

# A second inhabited storey: a continuous garden gallery over a shaded arcade.
# Annular slabs are actual closed meshes, with collision-ready walkable tops.
def arc_band(name,r0,r1,z,depth,material,start=0,end=math.pi,segments=64):
    vertices=[]; faces=[]
    for h in [z-depth,z]:
        for r in [r0,r1]:
            for i in range(segments+1):
                a=start+(end-start)*i/segments
                vertices.append((r*math.cos(a),r*math.sin(a),h))
    n=segments+1
    for i in range(segments):
        faces.extend([(i,i+1,n+i+1,n+i),(2*n+i,3*n+i,3*n+i+1,2*n+i+1),
                      (i,2*n+i,2*n+i+1,i+1),(n+i,n+i+1,3*n+i+1,3*n+i)])
    faces.extend([(0,n,3*n,2*n),(n-1,3*n-1,4*n-1,2*n-1)])
    mesh=bpy.data.meshes.new(name); mesh.from_pydata(vertices,[],faces); mesh.update()
    o=bpy.data.objects.new(name,mesh); scene.collection.objects.link(o); finish(o,name,material)
    return o
GALLERY=4.32
arc_band('Upper garden gallery',18.7,22,GALLERY,.38,pale)
arc_band('Gallery shadow course',18.65,22.05,GALLERY-.38,.16,dark)
arc_band('Gallery outer cornice',21.94,22.05,GALLERY+.04,.06,bronze)
arc_band('Gallery inner cornice',18.65,18.77,GALLERY+.04,.06,bronze)
for i in range(17):
    a=i*math.pi/16; x,y=20.8*math.cos(a),20.8*math.sin(a)
    box('Lower arcade pier',(x,y,1.85),(.65,.7,3.7),stone,a)
    cyl('Arcade capital',(x,y,3.85),.58,.28,pale,verts=16)
    # Balcony balustrades leave the broad arrival stairs unobstructed.
    for r in [18.85,21.8]:
        if r==21.8 and i in (10,11):continue
        xx,yy=r*math.cos(a),r*math.sin(a)
        box('Gallery baluster',(xx,yy,GALLERY+.58),(.14,.14,1.16),bronze,a)
for r in [18.85,21.8]:
    if r==21.8:
        arc_band('Balcony handrail',r-.06,r+.06,GALLERY+1.18,.10,bronze,0,1.88)
        arc_band('Balcony handrail',r-.06,r+.06,GALLERY+1.18,.10,bronze,2.20,math.pi)
    else:arc_band('Balcony handrail',r-.06,r+.06,GALLERY+1.18,.10,bronze)
# Two broad stair flights rise from the commons to the gallery at its open ends.
for sign in [-1,1]:
    x=sign*20.25
    box('Gallery approach landing',(sign*18.35,-8,-.12),(5.9,2.5,.24),pale)
    for i in range(32):
        top=(i+1)*GALLERY/32
        box('Gallery stair tread',(x,-8+(i+.5)*.25,top-.12),(2.7,.28,.24),pale)
        box('Gallery stair riser',(x,-8+(i+.5)*.25,(top-.24)/2),(2.5,.25,max(.01,top-.24)),stone)
    for i in range(9):
        y=-8+i; z=(i/8)*GALLERY+1.1
        for edge in [-1.3,1.3]:
            box('Stair rail upright',(x+edge,y,z-.45),(.075,.075,.9),bronze)
    for edge in [-1.3,1.3]:
        branch((x+edge,-8,1.1),(x+edge,0,GALLERY+1.1),.045,bronze)
# Upper archive stoa follows the curve, with rooms visible through the columns.
for i in range(3,14):
    if i in (10,11):continue # Clear approach and headroom for the observatory stair.
    a=i*math.pi/16; x,y=21.1*math.cos(a),21.1*math.sin(a)
    box('Upper archive bay',(x,y,GALLERY+1.45),(.28,3.0,2.9),dark,a)
    for z in [.5,1.2,1.9,2.6]:
        box('Archive gallery shelf',(x-.27*math.cos(a),y-.27*math.sin(a),GALLERY+z),(.65,2.9,.08),bronze,a)
        for k in range(9):
            tx=x-.33*math.cos(a)-(k-4)*.27*math.sin(a)
            ty=y-.33*math.sin(a)+(k-4)*.27*math.cos(a)
            box('Upper archive volume',(tx,ty,GALLERY+z+.24),(.25,.15,.4),pale if k%2 else clay,a)
    # Raised roof / clerestory gives an unmistakable two-storey silhouette.
    for r in [19.05,21.5]:
        xx,yy=r*math.cos(a),r*math.sin(a)
        cyl('Upper stoa column',(xx,yy,GALLERY+1.95),.19,3.9,pale,top=.15,verts=20)
    if i not in (10,11):
        box('Upper solar roof',(20.3*math.cos(a),20.3*math.sin(a),8.45),(3.9,4.25,.14),solar,a)
        box('Roof bronze seam',(20.3*math.cos(a),20.3*math.sin(a),8.55),(4,.08,.09),bronze,a)
# A third small prospect above the west gallery with a real spiral stair.
ox,oy=-10,20
cyl('Observatory foundation',(ox,oy,-.7),2.5,1.2,dark,verts=48)
cyl('Observatory support',(ox,oy,2),.38,4.1,stone,verts=20)
cyl('Observatory stair forecourt',(ox,oy,GALLERY-.12),2.2,.24,pale,verts=64)
for i in range(28):
    a=math.pi+i*math.tau/28; z=GALLERY+(i+1)*.14
    box('Observatory spiral step',(ox+1.1*math.cos(a),oy+1.1*math.sin(a),z-.10),(1.1,.36,.2),pale,a)
cyl('Observatory stair core',(ox,oy,6.2),.2,3.9,bronze,verts=20)
# Offset the deck so it does not form a ceiling across the climbing stair.
px=ox-2.8
cyl('Prospect support',(px,oy,4.1),.25,8.1,stone,verts=20)
cyl('Observatory prospect',(px,oy,8.15),1.65,.22,dark,verts=64)
ring('Prospect cornice',(px,oy,8.28),1.65,.07,bronze)
for i in range(2,7):
    a=i*math.tau/8
    cyl('Prospect guard',(px+1.5*math.cos(a),oy+1.5*math.sin(a),8.8),.055,1.15,bronze,verts=8)
# Open east edge lets the spiral meet the deck without a rail across the exit.
for i in range(2,6):
    a,b=i*math.tau/8,(i+1)*math.tau/8
    branch((px+1.5*math.cos(a),oy+1.5*math.sin(a),9.35),(px+1.5*math.cos(b),oy+1.5*math.sin(b),9.35),.045,bronze)
cyl('Observatory instrument',(px,oy,8.75),.24,1,bronze,verts=16)
for tilt in [0,math.pi/3,math.pi/2]:
    o=ring('Celestial armillary',(px,oy,9.7),.92,.04,bronze);o.rotation_euler.x=tilt
# Planters and reading benches on the upper walkway establish inhabited scale.
for a in [.15,.55,1.55,2.6,2.98]:
    x,y=20.0*math.cos(a),20.0*math.sin(a)
    cyl('Gallery planter',(x,y,GALLERY+.23),.48,.46,clay,verts=16)
    for j in range(3):
        bpy.ops.mesh.primitive_ico_sphere_add(subdivisions=1,radius=.48,location=(x+.22*math.sin(j*2),y+.22*math.cos(j*2),GALLERY+.7+j*.15))
        finish(bpy.context.object,'Gallery foliage',leaf2)

# The forum is the civic center of a larger walkable garden campus (about 136m).
# A planted ring promenade connects four destination courts via radial bridges.
arc_band('Garden promenade',30,38,0,.38,pale,0,math.tau,128)
arc_band('Promenade dark foundation',29.9,38.1,-.38,1.4,dark,0,math.tau,128)
for r in [30.3,37.7]:arc_band('Promenade bronze edge',r-.035,r+.035,.02,.025,bronze,0,math.tau,128)
for i,(label,a) in enumerate([('Arrival gardens',-math.pi/2),('Maker court',math.pi),('Reading grove',0),('Council garden',math.pi/2)]):
    x,y=55*math.cos(a),55*math.sin(a)
    # The walkway overlaps both the commons terrace and the destination island.
    box(label+' bridge',(37*math.cos(a),37*math.sin(a),-.12),(36,5,.24),pale,a)
    for side in [-1,1]:
        xx=37*math.cos(a)-side*2.35*math.sin(a); yy=37*math.sin(a)+side*2.35*math.cos(a)
        box(label+' bridge rail',(xx,yy,1.1),(36,.07,.08),bronze,a)
        for j in range(13):
            t=20+j*2.8
            box(label+' baluster',(t*math.cos(a)-side*2.35*math.sin(a),t*math.sin(a)+side*2.35*math.cos(a),.53),(.09,.09,1.06),bronze)
    cyl(label+' foundation',(x,y,-1.35),12,2.2,dark,verts=64)
    cyl(label+' court',(x,y,-.15),11.8,.3,pale,verts=64)
    ring(label+' civic inlay',(x,y,.02),9,.03,bronze)
    # Porticoes frame the destination without hiding the approach.
    for j in range(9):
        t=a-math.pi/2+j*math.pi/8
        xx=x+9.3*math.cos(t);yy=y+9.3*math.sin(t)
        cyl(label+' garden column',(xx,yy,2.3),.24,4.6,pale,top=.18,verts=16)
        box(label+' shade canopy',(xx,yy,4.8),(2.8,3.8,.12),solar,t)
    for j in range(7):
        t=a-math.pi*.75+j*math.pi*1.5/6
        xx=x+7*math.cos(t); yy=y+7*math.sin(t)
        cyl(label+' planted urn',(xx,yy,.36),.85,.72,clay,verts=16)
        branch((xx,yy,.7),(xx,yy,3.6),.10,wood)
        for k in range(3):
            bpy.ops.mesh.primitive_ico_sphere_add(subdivisions=1,radius=1.1,location=(xx+.65*math.sin(k*2),yy+.65*math.cos(k*2),3.7+k*.3))
            finish(bpy.context.object,label+' living canopy',leaf if k%2 else leaf2)
    for sign in [-1,1]:box(label+' resting bench',(x-sign*3*math.sin(a),y+sign*3*math.cos(a),.45),(2.4,.55,.9),stone,a)
    # Distinct activities expressed in furnishings, not invented live work.
    if label=='Maker court':
        for dy in [-2,2]:
            box('Maker worktable',(x,y+dy,1),(3,1.2,.2),bronze)
            for dx in [-1,1]:box('Workbench leg',(x+dx,y+dy,.45),(.15,.7,.9),dark)
            for j in range(3):box('Prototype block',(x-1+j,y+dy,1.35),(.4,.4,.5),pale)
    elif label=='Council garden':
        for j in range(12):
            t=j*math.tau/12
            box('Council seat',(x+4*math.cos(t),y+4*math.sin(t),.4),(1,.7,.8),stone,t)
        cyl('Council discussion table',(x,y,.8),2,.15,bronze,verts=48)
    elif label=='Reading grove':
        for dy in [-2,2]:
            box('Garden reading desk',(x,y+dy,.9),(2,1,.15),pale)
            box('Garden archive case',(x+5,y+dy,1.5),(.6,2.2,3),dark)
            for z in [.6,1.3,2]:box('Garden book shelf',(x+4.7,y+dy,z),(.4,2.2,.08),bronze)
# Rhythm and views on the outer promenade; keep the central path open.
for i in range(40):
    a=i*math.tau/40
    if abs(math.sin(a*2))<.18:continue
    x,y=36.4*math.cos(a),36.4*math.sin(a)
    cyl('Promenade planter',(x,y,.25),.7,.5,clay,verts=12)
    bpy.ops.mesh.primitive_ico_sphere_add(subdivisions=1,radius=.95,location=(x,y,1.3))
    finish(bpy.context.object,'Promenade living edge',leaf2)
    cyl('Path lantern housing',(31.3*math.cos(a),31.3*math.sin(a),.6),.14,1.2,bronze,verts=8)
    cyl('Path lantern inlay',(31.3*math.cos(a),31.3*math.sin(a),1.14),.145,.06,cyan,verts=8)

exec(compile((ROOT/'scripts/blender/forum_landscape.py').read_text(),str(ROOT/'scripts/blender/forum_landscape.py'),'exec'))

exec(compile((ROOT/'scripts/blender/forum_celestial.py').read_text(),str(ROOT/'scripts/blender/forum_celestial.py'),'exec'))

exec(compile((ROOT/'scripts/blender/forum_civic_planting.py').read_text(),str(ROOT/'scripts/blender/forum_civic_planting.py'),'exec'))

# North-star direction modules (docs/design/solar-forum/jobs/12-north-star-direction.md).
# Each module is optional until authored: a missing file is skipped, never faked.
for _module in ('forum_towers.py','forum_landform.py','forum_terrace.py','forum_meshgate.py'):
    _path=ROOT/'scripts/blender'/_module
    if _path.exists(): exec(compile(_path.read_text(),str(_path),'exec'))
    else: print('FORUM_MODULE_SKIPPED',_module)
arc_band('Civic lagoon',22,27,-1.45,.08,water,segments=128)

exec(compile((ROOT/'scripts/blender/forum_source_materials.py').read_text(),str(ROOT/'scripts/blender/forum_source_materials.py'),'exec'))

# Authored source retains object identities; exported runtime batches by material.
scene.world.color=(.55,.55,.55)
bpy.ops.object.light_add(type='SUN',location=(0,-10,20)); sun=bpy.context.object; sun.rotation_euler=(.45,-.4,-.6); sun.data.energy=2.5; sun.data.angle=.12
bpy.ops.object.light_add(type='AREA',location=(0,-5,17)); bpy.context.object.data.energy=1800; bpy.context.object.data.shape='DISK'; bpy.context.object.data.size=20
bpy.ops.object.camera_add(location=(33,-42,29)); cam=bpy.context.object; cam.rotation_euler=(Vector((0,2,3))-cam.location).to_track_quat('-Z','Y').to_euler(); cam.data.type='ORTHO'; cam.data.ortho_scale=56; scene.camera=cam
scene.render.engine='CYCLES'; scene.cycles.samples=64; scene.cycles.use_denoising=True
scene.render.resolution_x=1500; scene.render.resolution_y=1100; scene.render.resolution_percentage=100
scene.render.filepath=str(OUT/'forum-preview.png')
bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'solar-forum.blend'))
# Shared-mesh templates (tree variants, quay segments, props) live at the origin
# only so linked copies can reuse their datablocks; they are never exported or
# rendered. Everything else keeps the forum_architecture tag it was given.
for _o in list(scene.objects):
    if _o.type=='MESH' and 'template' in _o.name.lower():
        _o['forum_architecture']=False; _o.hide_render=True; _o.hide_viewport=True
exports=[]
for m in materials:
    bpy.ops.object.select_all(action='DESELECT')
    group=[o for o in scene.objects if o.type=='MESH' and o.get('forum_architecture') and o.data.materials[0]==m]
    for o in group:o.select_set(True)
    if not group:continue
    bpy.context.view_layer.objects.active=group[0]
    if len(group)>1:bpy.ops.object.join()
    o=bpy.context.object; o.name='Forum_'+m.name.replace(' ','_'); exports.append(o)
bpy.ops.object.select_all(action='DESELECT')
for o in exports:o.select_set(True)
# Blender writes an uncompressed GLB (~62 MB: float32 geometry + embedded JPEG).
# That never ships: `npm run world:compress` rewrites it in place with
# EXT_meshopt_compression + KHR_mesh_quantization (~14 MB), which the runtime
# and the Node tests decode through three's MeshoptDecoder. Under FORUM_SCRATCH
# WEB already points into the scratch directory, so a dry run compresses there
# and the shipped asset is never touched.
RAW_GLB = WEB/'solar-forum.raw.glb'
bpy.ops.export_scene.gltf(filepath=str(RAW_GLB),export_format='GLB',export_image_format='JPEG',export_jpeg_quality=85,use_selection=True,export_yup=True,export_animations=False,export_cameras=False,export_lights=False)
bpy.ops.export_scene.fbx(filepath=str(OUT/'solar-forum.fbx'),use_selection=True,object_types={'MESH'},apply_unit_scale=True,bake_anim=False,mesh_smooth_type='FACE',axis_forward='-Y',axis_up='Z')
raw_bytes=RAW_GLB.stat().st_size
compress=subprocess.run(['npm','run','--silent','world:compress','--',str(RAW_GLB),str(WEB/'solar-forum.glb')],cwd=str(ROOT/'ui/command-center'),capture_output=True,text=True)
if compress.returncode!=0:
    sys.stderr.write(compress.stdout);sys.stderr.write(compress.stderr)
    raise SystemExit('world:compress failed; run `npm install` in ui/command-center')
RAW_GLB.unlink()
triangles=sum(sum(len(p.vertices)-2 for p in o.data.polygons) for o in exports)
bpy.context.view_layer.update()
world_vertices=[o.matrix_world @ v.co for o in exports for v in o.data.vertices]
exact_min=[min(v[axis] for v in world_vertices) for axis in range(3)]
exact_max=[max(v[axis] for v in world_vertices) for axis in range(3)]
manifest=dict(schema='permagent.forum.v1',metersPerUnit=1,runtimeUp='Y',sourceUp='Z',meshes=len(exports),triangles=triangles,bytes=(WEB/'solar-forum.glb').stat().st_size,rawBytes=raw_bytes,liveState=False, levels=[0,4.32,8.26], campusDiameterMeters=round(max(exact_max[axis]-exact_min[axis] for axis in [0,1]),2))
# Budget raised for the north-star pass (job 12); repeated props must be linked duplicates.
# `bytes` is the compressed runtime file; forumAssets.test.ts holds the same 20 MB line.
assert triangles<1200000 and manifest['rawBytes']<95000000 and manifest['bytes']<20*1024*1024,manifest
manifest['geodiscDiameterMeters']=358.79
manifest['celestial']=celestial_manifest
(WEB/'solar-forum.manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
revision=hashlib.sha256((WEB/'solar-forum.glb').read_bytes()).hexdigest()[:16]
if not SCRATCH: (ROOT/'ui/command-center/src/components/world/forum/assetRevision.ts').write_text('export const FORUM_ASSET_REVISION = '+json.dumps(revision)+';\n')
print('FORUM_VERIFIED',json.dumps(manifest))
if '--no-render' in sys.argv: sys.exit(0)
bpy.ops.render.render(write_still=True)

cam.location=(240,-310,225)
cam.rotation_euler=(Vector((0,0,2))-cam.location).to_track_quat('-Z','Y').to_euler()
cam.data.ortho_scale=405
scene.render.filepath=str(OUT/'campus-preview.png')
bpy.ops.render.render(write_still=True)

# Separate night art inspection. Browser lighting follows prefers-color-scheme;
# these Cycles previews are source-art evidence, not browser screenshots.
sun.data.energy=.35; sun.data.color=(.45,.60,1)
for o in scene.objects:
    if o.type=='LIGHT' and o.data.type=='AREA': o.data.energy=250
scene.world.use_nodes=True
scene.world.node_tree.nodes.get('Background').inputs['Color'].default_value=(*palette('deepVoid'),1)
scene.world.node_tree.nodes.get('Background').inputs['Strength'].default_value=.3
for loc in [(0,0,5),(0,20,6),(-17,0,4),(17,0,4)]:
    bpy.ops.object.light_add(type='POINT',location=loc);o=bpy.context.object;o.data.energy=220;o.data.color=(1,.55,.2);o.data.shadow_soft_size=1
scene.render.filepath=str(OUT/'forum-night-preview.png')
bpy.ops.render.render(write_still=True)
