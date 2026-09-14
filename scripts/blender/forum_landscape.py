"""Authored outer districts and construction detail, executed by build_solar_forum.py.
Coordinates remain Blender Z-up. Routes at z=0 are continuous through the grounds.
"""
import bmesh
import json
# A landform makes the civic buildings part of a place instead of isolated discs.
terrain=mat('Coastal meadow',(.105,.155,.065),rough=.96)
rockmat=mat('Weathered coastal rock',(.20,.19,.16),rough=.88)
timber=mat('Oiled structural timber',(.22,.12,.055),rough=.66)
coastleaf=mat('Coastal canopy',(.115,.23,.055),rough=.83)
glass=mat('Conservatory glass',(.12,.30,.27),metal=.08,rough=.18)
glass.node_tree.nodes.get('Principled BSDF').inputs['Transmission Weight'].default_value=.45

def detailed_box(name,loc,scale,material,angle=0,bevel=.025):
    o=box(name,loc,scale,material,angle)
    bm=bmesh.new();bm.from_mesh(o.data)
    bmesh.ops.bevel(bm,geom=list(bm.edges),offset=bevel,segments=2,affect='EDGES',clamp_overlap=True)
    bm.to_mesh(o.data);bm.free()
    return o

def leaf_spray(name,center,radius,count=36):
    """Build an anchored, layered spray of folded lanceolate leaves.

    The old spray used narrow radial spikes, which read as sparse triangles at
    medium distance.  Each leaf now has a broad shoulder, a folded center and
    a varied upward/outward attitude.  Bases remain distributed around the
    supplied center so terrace planters and tree crowns stay rooted rather
    than becoming opaque green spheres.
    """
    vertices=[];faces=[];center=Vector(center)
    for n in range(count):
        t=n*2.39996+random.uniform(-.10,.10)
        ring=radius*(.16+.84*math.sqrt((n+.5)/count))
        layer=n%4
        zoff=(layer-1.5)*radius*.17+random.uniform(-.07,.07)*radius
        anchor=center+Vector((ring*math.cos(t),ring*math.sin(t),zoff))
        outward=Vector((math.cos(t),math.sin(t),0))
        tangent=Vector((-math.sin(t),math.cos(t),0))
        direction=(outward*random.uniform(.38,.72)+
                   tangent*random.uniform(-.24,.24)+
                   Vector((0,0,random.uniform(.34,.72)))).normalized()
        length=radius*random.uniform(.34,.62)
        width=radius*random.uniform(.11,.19)
        side=Vector((-direction.y,direction.x,0))
        if side.length<.01:side=Vector((1,0,0))
        side.normalize()
        tip=anchor+direction*length
        fold=anchor+direction*(length*.53)+Vector((0,0,.025*radius))
        shoulder=anchor+direction*(length*.62)
        base=anchor+direction*(length*.05)
        b=len(vertices)
        vertices.extend([base,anchor+side*width*.58,anchor-side*width*.58,
                         shoulder+side*width,shoulder-side*width,fold,tip])
        faces.extend([(b,b+1,b+5),(b,b+5,b+2),
                      (b+1,b+3,b+5),(b+5,b+4,b+2),
                      (b+3,b+6,b+5),(b+5,b+6,b+4)])
    me=bpy.data.meshes.new(name);me.from_pydata(vertices,[],faces);me.update();o=bpy.data.objects.new(name,me);scene.collection.objects.link(o);finish(o,name,coastleaf);return o

# Overview planting uses a few shared mesh variants rather than one mesh per
# leaf spray.  The crowns are deliberately broad and layered so a grove reads
# as a mass from the campus cameras without consuming the export budget.
_route_polylines_for_planting=[]
try:
    _route_file=ROOT/'ui/command-center/src/components/world/forum/patrolRoutes.json'
    if _route_file.exists():
        _route_payload=json.loads(_route_file.read_text())
        for _route in _route_payload.get('routes',{}).values():
            _route_polylines_for_planting.append([(float(p[0]),-float(p[2])) for p in _route])
except Exception:
    _route_polylines_for_planting=[]

def _point_close_to_route(point,radius):
    """True when point lies within radius of any authored walk route polyline.

    Routes are tested as exact per-route polylines (segments between
    consecutive waypoints of the same route), so long legs such as the
    harbour approach are never mistaken for gaps between routes.
    """
    px,py=point; limit=radius*radius
    for polyline in _route_polylines_for_planting:
        previous=None
        for current in polyline:
            if previous is not None:
                ax,ay=previous; bx,by=current; dx,dy=bx-ax,by-ay; ll=dx*dx+dy*dy
                if ll>1e-9:
                    u=max(0.0,min(1.0,((px-ax)*dx+(py-ay)*dy)/ll))
                    if (px-(ax+u*dx))**2+(py-(ay+u*dy))**2 < limit:return True
            if (px-current[0])**2+(py-current[1])**2 < limit:return True
            previous=current
    return False

_tree_templates={};_shrub_template=None
def _tree_mesh(variant):
    """One rooted trunk, two limbs and a volumetric crown of faceted lobes.

    The crown is six to seven low-poly spheres of varied size clustered
    around a centre at 3.2-5.6 m, so a tree reads as a rounded mass from the
    campus cameras and as foliage at human height, never as a flat disc.
    Shared by every instance, so the per-variant cost is a few hundred faces.
    """
    import bmesh
    random_state=random.getstate();random.seed(8100+variant)
    bm=bmesh.new()
    def add_cone(cx,cy,cz,r0,r1,height,sides=7):
        geom=bmesh.ops.create_cone(bm,cap_ends=True,cap_tris=True,segments=sides,radius1=r0,radius2=r1,depth=height)
        bmesh.ops.translate(bm,verts=geom['verts'],vec=(cx,cy,cz+height*.5))
    add_cone(0,0,0,.24,.15,3.3)
    for angle,length in ((.7,1.9),(3.6,1.6)):
        geom=bmesh.ops.create_cone(bm,cap_ends=True,cap_tris=True,segments=5,radius1=.11,radius2=.06,depth=length)
        bmesh.ops.rotate(bm,verts=geom['verts'],cent=(0,0,0),matrix=__import__('mathutils').Matrix.Rotation(.55,3,'Y')@__import__('mathutils').Matrix.Rotation(angle,3,'Z'))
        bmesh.ops.translate(bm,verts=geom['verts'],vec=(.5*math.cos(angle),.5*math.sin(angle),2.6+length*.35))
    lobes=[(0,0,4.5,2.3),(-1.3,.4,3.9,1.7),(1.25,-.3,4.1,1.8),(.3,1.35,4.0,1.6),(-.2,-1.3,3.7,1.5),(.5,.3,5.6,1.5)]
    if variant==1: lobes.append((-.9,-.9,4.9,1.3))
    for x,y,z,r in lobes:
        r*=1+variant*.07
        geom=bmesh.ops.create_icosphere(bm,subdivisions=1,radius=r)
        bmesh.ops.scale(bm,verts=geom['verts'],vec=(1.0+random.uniform(-.08,.08),1.0+random.uniform(-.08,.08),.82))
        bmesh.ops.translate(bm,verts=geom['verts'],vec=(x*(1+variant*.05),y,z*(1+variant*.06)))
    mesh=bpy.data.meshes.new('Shared overview tree variant %d'%variant);bm.to_mesh(mesh);bm.free();mesh.update()
    random.setstate(random_state)
    return mesh

def _ensure_tree_templates():
    global _shrub_template
    if _tree_templates and _shrub_template is not None:
        return
    for variant in range(3):
        mesh=_tree_mesh(variant);obj=bpy.data.objects.new('Tree variant template %d'%variant,mesh);scene.collection.objects.link(obj);finish(obj,obj.name,coastleaf);_tree_templates[variant]=obj
    # A linked low shrub has ten folded blades and stays below the tree crowns.
    vertices=[];faces=[]
    for i in range(10):
        a=math.tau*i/10; b=len(vertices); x,y=.55*math.cos(a),.55*math.sin(a)
        vertices.extend([(x,y,0),(x+.34*math.cos(a),y+.34*math.sin(a),.7),(x-.16*math.sin(a),y+.16*math.cos(a),.26)])
        faces.append((b,b+1,b+2))
    mesh=bpy.data.meshes.new('Shared overview understory shrub');mesh.from_pydata(vertices,[],faces);mesh.update();_shrub_template=bpy.data.objects.new('Understory shrub template',mesh);scene.collection.objects.link(_shrub_template);finish(_shrub_template,_shrub_template.name,coastleaf)

def _linked_tree(name,location,variant=0,scale=1.0):
    _ensure_tree_templates();o=bpy.data.objects.new(name,_tree_templates[variant].data);scene.collection.objects.link(o);o.location=Vector(location);o.scale=(scale,scale,scale);o['forum_architecture']=True;return o

def linked_understory(name,location,scale=1.0):
    _ensure_tree_templates();o=bpy.data.objects.new(name,_shrub_template.data);scene.collection.objects.link(o);o.location=Vector(location);o.scale=(scale,scale,scale);o['forum_architecture']=True;return o

def translated_band(name,cx,cy,r0,r1,z,depth,material,start=0,end=math.tau,segments=64):
    o=arc_band(name,r0,r1,z,depth,material,start,end,segments);o.location.x=cx;o.location.y=cy;return o

def rail_segment(name,a,b,height=1.1):
    a,b=Vector(a),Vector(b);delta=b-a;n=max(1,math.ceil(delta.length/2.5))
    for i in range(n+1):
        p=a+delta*i/n
        cyl(name+' anchored post',(p.x,p.y,p.z+height*.5),.045,height,bronze,verts=8)
        detailed_box(name+' base shoe',(p.x,p.y,p.z+.055),(.19,.19,.11),bronze,bevel=.012)
    branch(a+Vector((0,0,height)),b+Vector((0,0,height)),.045,bronze)
    branch(a+Vector((0,0,.45)),b+Vector((0,0,.45)),.018,bronze)

# Smooth inland meadow, eroded coast outside the destination circuit.
vertices=[];faces=[];segments=160;rings=[27,42,60]+list(range(69,175,5))
for j,r in enumerate(rings):
    for i in range(segments):
        a=i*math.tau/segments
        coast=1 if r<150 else 1+.035*math.sin(a*7)+.023*math.cos(a*13)
        x,y=r*math.cos(a),r*math.sin(a)
        path_distance=min(abs(r-104),min(abs(x),abs(y)) if r<150 else 100)
        mask=max(0,min(1,(path_distance-6)/10))
        hill=max(0,math.sin(x*.065+.8)*math.cos(y*.047))*3.2+max(0,math.cos(x*.11-y*.08))*.7
        height=-.25+hill*mask if r>=69 else -.65
        if r>149:height-=((r-149)/25)**2*3.5
        vertices.append((r*coast*math.cos(a),r*coast*math.sin(a),height))
for j in range(len(rings)-1):
    for i in range(segments):
        a=j*segments+i;b=j*segments+(i+1)%segments
        faces.append((a,b,b+segments,a+segments))
mesh=bpy.data.meshes.new('Sculpted coastal ground');mesh.from_pydata(vertices,[],faces);mesh.update()
o=bpy.data.objects.new('Sculpted coastal ground',mesh);scene.collection.objects.link(o);finish(o,o.name,terrain)
for p in mesh.polygons:p.use_smooth=True
# Continuous outer circuit and four long approaches, all at the shared walk datum.
arc_band('District garden circuit',101,107,0,.3,pale,segments=192)
for r in [101.1,106.9]:arc_band('Circuit stone kerb',r-.08,r+.08,.12,.28,stone,segments=192)
for a in [0,math.pi/2,math.pi,-math.pi/2]:
    box('District approach',(84*math.cos(a),84*math.sin(a),-.14),(39,6,.3),pale,a)
    for side in [-1,1]:
        for t in range(68,101,4):
            x=t*math.cos(a)-side*3.2*math.sin(a);y=t*math.sin(a)+side*3.2*math.cos(a)
            detailed_box('Approach planting wall',(x,y,.25),(3.5,.3,.5),stone,a)
            for k in range(3):
                xx=x+(k-1)*.8*math.cos(a);yy=y+(k-1)*.8*math.sin(a)
                leaf_spray('Fine-leaf approach planting',(xx,yy,.5),.4,18)
# Reed-lined rain gardens between routes. Layered banks give real near-ground relief.
for i in range(8):
    a=(i+.5)*math.tau/8;x,y=79*math.cos(a),79*math.sin(a)
    o=cyl('Rain garden retaining bowl',(x,y,-.3),7,.5,stone,verts=40);o.scale=(1.5,.75,1)
    o=cyl('Rain garden water',(x,y,-.025),6.6,.025,water,verts=40);o.scale=(1.5,.75,1)
    for k in range(30):
        t=k*math.tau/30;xx=x+9.3*math.cos(t);yy=y+4.8*math.sin(t)
        for h in [0,.17]:branch((xx,yy,-.02),(xx+.14,yy+.09,.8+h+random.random()*.5),.015,leaf)
# East: a ribbed conservatory, seed archive and open central nave.
cx,cy=116,0
box('Conservatory plinth',(cx,cy,-.18),(32,26,.4),stone)
for x in [102,106,110,114,118,122,126,130]:
    for side in [-1,1]:detailed_box('Conservatory pier',(x,side*10,1.5),(.45,.55,3),stone)
    for k in range(16):
        a=k*math.pi/16;b=(k+1)*math.pi/16
        branch((x,10*math.cos(a),3+10*math.sin(a)),(x,10*math.cos(b),3+10*math.sin(b)),.12,bronze)
for x in [102,110,118,126]:
    for k in range(12):
        a=k*math.pi/12;b=(k+1)*math.pi/12
        points=[(x,9.9*math.cos(a),3+9.9*math.sin(a)),(x+3.8,9.9*math.cos(a),3+9.9*math.sin(a)),(x+3.8,9.9*math.cos(b),3+9.9*math.sin(b)),(x,9.9*math.cos(b),3+9.9*math.sin(b))]
        me=bpy.data.meshes.new('Glazed vault panel');me.from_pydata(points,[],[(0,1,2,3)]);me.update();ob=bpy.data.objects.new('Glazed vault panel',me);scene.collection.objects.link(ob);finish(ob,ob.name,glass)
for x in [108,116,124]:
    for side in [-1,1]:
        detailed_box('Conservatory planting bed',(x,side*6,.5),(5.5,2.5,1),clay,bevel=.05)
        for k in range(4):
            xx=x-1.8+k*1.2;yy=side*6
            branch((xx,yy,1),(xx,yy,3.1),.06,wood)
            for h in [1.6,2.2,2.8]:
                leaf_spray('Conservatory specimen leaves',(xx,yy,h),.7)
# West: maker hall, timber roof, an accessible mezzanine and furnished work bays.
cx=-116
box('Maker hall slab',(cx,0,-.18),(32,30,.4),stone)
for y in [-14,14]:
    for x in [-130,-126,-122,-118,-114,-110,-106,-102]:
        detailed_box('Workshop buttress',(x,y,3.6),(.6,.8,7.2),stone,bevel=.035)
    for z in [.35,1,1.65,2.3]:
        for k in range(17):
            x=-130+k*1.5
            detailed_box('Workshop coursed stone',(x,y,z),(1.47,.55,.6),pale,bevel=.02)
for x in [-129,-124,-119,-114,-109,-104]:
    branch((x,-14,7),(x,0,10),.18,timber);branch((x,0,10),(x,14,7),.18,timber)
    branch((x,-14,7),(x,14,7),.10,bronze)
    for side in [-1,1]:
        o=box('Workshop roof cassette',(x,side*7,8.65),(4.7,14.1,.15),solar);o.rotation_euler.x=-side*math.atan(3/14)
box('Maker mezzanine',(-126,0,4.35),(8,27,.3),pale)
for y in [-13,13]:rail_segment('Mezzanine railing',(-130,y,4.5),(-122,y,4.5))
rail_segment('Mezzanine front railing',(-122,-13,4.5),(-122,7,4.5))
for i in range(30):
    top=(i+1)*.15;box('Maker stair tread',(-113-i*.3,10,top-.12),(.33,2.6,.24),pale)
rail_segment('Maker stair handrail',(-113,11.3,0),(-122,11.3,4.5))
for x in [-112,-122]:
    for y in [-7,7]:
        detailed_box('Workbench top',(x,y,1.02),(5,1.5,.18),timber,bevel=.04)
        for dx in [-2,2]:detailed_box('Workbench steel trestle',(x+dx,y,.47),(.12,1.15,.94),bronze)
        for k in range(5):detailed_box('Machined component',(x-1.6+k*.75,y,1.25),(.35,.5,.28),bronze,bevel=.045)
# North: open-air debating theatre, radial aisles and a framed view back to the forum.
for j in range(8):
    for start,end in [(0,.65),(.75,1.48),(1.66,2.38),(2.48,math.pi)]:
        translated_band('Theatre seating tier',0,115,10+j*1.3,11.1+j*1.3,j*.32+.32,.32,stone,start,end,20)
box('Theatre forecourt',(0,108,-.13),(42,22,.3),pale)
cyl('Debate dais',(0,115,.14),6.5,.28,pale,verts=64)
for x in [-19,19]:
    for y in [107,113,119]:cyl('Theatre portico column',(x,y,3.4),.33,6.8,pale,top=.26)
    detailed_box('Theatre timber canopy',(x,113,7),(5,16,.22),timber)
# South: a monumental arrival gate and a harbor quay, scaled to a walking player.
box('Harbor plaza',(0,-118,-.16),(44,34,.36),pale)
for side in [-1,1]:
    x=side*15
    detailed_box('Arrival tower base',(x,-122,1),(6,6,2),stone,bevel=.12)
    detailed_box('Arrival tower',(x,-122,10.5),(4.3,4.3,19),pale,bevel=.055)
    for h in [4,8,12,16,20]:detailed_box('Tower belt course',(x,-122,h),(4.6,4.6,.18),bronze)
    for y in [-124.3,-119.7]:
        for dx in [-.8,.8]:detailed_box('Tower recessed fin',(x+dx,y,11),(.16,.12,15),bronze)
    cyl('Tower lantern crown',(x,-122,22),2.8,3,bronze,verts=12)
for k in range(32):
    a=k*math.pi/32;b=(k+1)*math.pi/32
    branch((11*math.cos(a),-121,9+11*math.sin(a)),(11*math.cos(b),-121,9+11*math.sin(b)),.45,stone)
box('Harbor boardwalk',(0,-142,-.12),(48,7,.3),timber)
box('Quay approach',(0,-135,-.125),(7,20,.3),pale)
for x in range(-23,24):box('Quay deck joint',(x,-142,.012),(.025,6.8,.018),dark)
rail_segment('Harbor safety rail',(-24,-145.3,0),(24,-145.3,0))
for x in [-21,-14,14,21]:
    cyl('Mooring bollard',(x,-143,.35),.16,.7,bronze,verts=12)
    ring('Bollard collar',(x,-143,.55),.17,.035,bronze)
# Overlooks and detailed slatted benches along the circuit.
for i in range(12):
    a=(i+.5)*math.tau/12;x,y=109*math.cos(a),109*math.sin(a)
    o=box('Garden overlook',(x,y,-.15),(8,5,.3),stone,a)
    for slat in range(5):detailed_box('Overlook seat slat',(x-math.sin(a)*(slat-2)*.11,y+math.cos(a)*(slat-2)*.11,.58),(3,.09,.10),timber,a,bevel=.018)
    for d in [-1.1,1.1]:detailed_box('Seat bracket',(x+math.cos(a)*d,y+math.sin(a)*d,.27),(.10,.48,.54),bronze,a)
exec(compile((ROOT/'scripts/blender/forum_habitat.py').read_text(),str(ROOT/'scripts/blender/forum_habitat.py'),'exec'))
# Coastal trees and rock outcrops, kept off all authored routes and entrances.
forest_layout=[]
for i in range(180):
    a=i*2.39996
    # Keep the lagoon clear; the occasional inner tree belongs to the civic
    # meadow and is filtered away from the four cardinal walk axes.
    r=(132+random.random()*30) if i%3 else (40+random.random()*18)
    x,y=r*math.cos(a),r*math.sin(a)
    if min(abs(x),abs(y))<9 or any(abs(x-cx)<19 and abs(y-cy)<19 for cx,cy in habitat_centers):continue
    height=3.8+random.random()*3.7
    forest_layout.append({'x':x,'y':y,'height':height+1})
    crown_radius=1.45+random.random()*0.8
    trunk_x=x+.18*math.sin(a*1.7);trunk_y=y+.18*math.cos(a*1.3)
    branch((x,y,-.4),(trunk_x,trunk_y,height),.16,wood)
    for k in range(4):
        t=k*math.tau/4+a*.31+random.uniform(-.15,.15)
        branch_z=height*(.46+.07*(k%2))
        px=trunk_x+crown_radius*(.62+.10*(k%2))*math.cos(t)
        py=trunk_y+crown_radius*(.62+.10*(k%2))*math.sin(t)
        pz=height*(.73+.045*(k%2))+random.uniform(-.12,.18)
        branch((trunk_x,trunk_y,branch_z),(px,py,pz),.075,wood)
        leaf_spray('Layered coastal canopy',(px,py,pz),crown_radius*.88,22)
        # A smaller raised clump closes the crown without hiding the branch
        # structure, producing a readable multi-tiered tree silhouette.
        leaf_spray('Upper coastal canopy',(px*.72+trunk_x*.28,
                   py*.72+trunk_y*.28,pz+.72),
                   crown_radius*.58,11)
# Fourteen dense overview groves occupy the remaining meadow bands.  Every
# placement is segment-safe; the shared meshes keep 400+ trees inexpensive.
_ensure_tree_templates()
for grove in range(14):
    angle=math.radians(13.0+grove*25.7)
    radius=44.0+(grove%4)*4.5 if grove<8 else 120.0+(grove%3)*6.0
    gx,gy=radius*math.cos(angle),radius*math.sin(angle)
    if _point_close_to_route((gx,gy),4.5): continue
    cyl('Dense meadow grove bed',(gx,gy,-.08),7.2,.16,clay,verts=24)
    for tree in range(8+(grove%9)):
        a=angle+tree*2.39996
        tx=gx+random.uniform(-6.0,6.0);ty=gy+random.uniform(-6.0,6.0)
        if _point_close_to_route((tx,ty),8.5): continue  # tree origin plus crown radius must clear the route
        _linked_tree('Dense meadow grove tree %02d'%grove,(tx,ty,0),tree%3,random.uniform(.92,1.18))
        linked_understory('Dense meadow grove understory',(tx+random.uniform(-1.2,1.2),ty+random.uniform(-1.2,1.2),.02),random.uniform(.8,1.3))

for hedge in range(12):
    a = math.radians(15 + hedge * 30)
    for k in range(9):
        r = 32 + k * 2.4
        x, y = r * math.cos(a), r * math.sin(a)
        if min(abs(x), abs(y)) < 8: continue
        if _point_close_to_route((x,y),4.5): continue
        leaf_spray('Meadow path hedgerow', (x, y, .35), .38, 12)
for bed in range(8):
    a = math.radians(22.5 + bed * 45)
    x, y = 49 * math.cos(a), 49 * math.sin(a)
    for step in range(3):
        if _point_close_to_route((x,y),4.5): continue
        translated_band('Meadow terraced garden bed', x, y, 1.2 + step * 1.5, 3.8 + step * 1.5, .18 + step * .35, .16, clay, a - .45, a + .45, 12)
scene['forum_forest_json']=json.dumps(forest_layout)
(OUT/'forest.json').write_text(json.dumps(forest_layout,indent=2)+'\n')
for i in range(48):
    a=i*math.tau/48;r=164+random.random()*9
    bpy.ops.mesh.primitive_ico_sphere_add(subdivisions=2,radius=1,location=(r*math.cos(a),r*math.sin(a),-1.4))
    ob=finish(bpy.context.object,'Eroded coastal outcrop',rockmat);ob.scale=(2.5+random.random()*4,2+random.random()*3,2+random.random()*5);ob.rotation_euler.z=a
    for vertex in ob.data.vertices:vertex.co*=random.uniform(.88,1.12)
    for polygon in ob.data.polygons:polygon.use_smooth=True
# Close-range craft: segmented pool coping, bench edges, drainage and fasteners.
for i in range(48):translated_band('Individually cut pool coping',0,0,3.39,3.69,.52,.15,pale,i*math.tau/48+.008,(i+1)*math.tau/48-.008,3)
for x,y in [(-7,-4),(7,-4),(-7,9),(7,9)]:
    for dy in [-1.3,1.3]:
        for slat in range(5):detailed_box('Commons bench timber slat',(x,y+dy+(slat-2)*.085,.69),(2.5,.07,.06),timber,bevel=.012)
        for dx in [-.92,.92]:
            for sy in [-.13,.13]:cyl('Bench countersunk fixing',(x+dx,y+dy+sy,.724),.022,.008,bronze,verts=8)
for i in range(28):
    a=(i+.5)*math.tau/28;x,y=18.1*math.cos(a),18.1*math.sin(a)
    for j in range(5):box('Terrace drainage slot',(x-math.sin(a)*j*.075,y+math.cos(a)*j*.075,.021),(.32,.025,.022),dark,a)
# Fine surface relief is visible in Blender while exported material factors remain stable.
for material in [stone,pale,rockmat]:
    nodes=material.node_tree.nodes;links=material.node_tree.links;shader=nodes.get('Principled BSDF')
    noise=nodes.new('ShaderNodeTexNoise');noise.inputs['Scale'].default_value=38;noise.inputs['Detail'].default_value=3
    bump=nodes.new('ShaderNodeBump');bump.inputs['Strength'].default_value=.14;bump.inputs['Distance'].default_value=.022
    links.new(noise.outputs['Fac'],bump.inputs['Height']);links.new(bump.outputs['Normal'],shader.inputs['Normal'])

# A physical daylight sky reveals construction detail and glass in source renders.
scene.world.use_nodes=True
background=scene.world.node_tree.nodes.get('Background')
sky=scene.world.node_tree.nodes.new('ShaderNodeTexSky');sky.sky_type='MULTIPLE_SCATTERING';sky.sun_elevation=.55;sky.sun_rotation=-2.4;sky.sun_disc=False
scene.world.node_tree.links.new(sky.outputs['Color'],background.inputs['Color']);background.inputs['Strength'].default_value=.3
# Meadow color variation follows surface coordinates, giving the landform scale.
nodes=terrain.node_tree.nodes;links=terrain.node_tree.links
noise=nodes.new('ShaderNodeTexNoise');noise.inputs['Scale'].default_value=18;noise.inputs['Detail'].default_value=4
ramp=nodes.new('ShaderNodeValToRGB');ramp.color_ramp.elements[0].color=(.055,.09,.022,1);ramp.color_ramp.elements[1].color=(.20,.25,.075,1)
links.new(noise.outputs['Fac'],ramp.inputs['Fac']);links.new(ramp.outputs['Color'],nodes.get('Principled BSDF').inputs['Base Color'])

# Persist route points for later landform modules that need route-safe quays.
_forum_routes=''
try:
    _forum_routes=ROOT/'ui/command-center/src/components/world/forum/patrolRoutes.json'
    if _forum_routes.exists():
        payload=json.loads(_forum_routes.read_text())
        _route_points=[]
        for route in payload.get('routes',{}).values():
            for x,_,z in route:
                _route_points.append([float(x),float(-z),0.0])
        scene['forum_route_points']=json.dumps(_route_points)
except Exception as exc:
    scene['forum_route_points']='[]'
