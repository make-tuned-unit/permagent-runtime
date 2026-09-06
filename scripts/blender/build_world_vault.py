"""Blender-authored, exportable cyber-classical hall. No third-party assets.

Run Blender --background --factory-startup --python this_file.py -- --render.
Editable source is saved before material batching for efficient GLB delivery.

Art direction (2026-09-06): material hierarchy and proportion before polygons.
The runtime lights this GLB with two directionals, a hemisphere bounce and no
environment map, so materials are tuned for that forward renderer, never for
the Cycles preview, which is inspection only.
"""
import bpy
import json
import math
import sys
from pathlib import Path
from mathutils import Vector

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / 'assets' / 'world' / 'blender'
PUBLIC = ROOT / 'ui' / 'command-center' / 'public' / 'world'
SOURCE.mkdir(parents=True, exist_ok=True)
PUBLIC.mkdir(parents=True, exist_ok=True)
bpy.ops.object.select_all(action='SELECT')
bpy.ops.object.delete(use_global=False)
scene = bpy.context.scene
bpy.context.preferences.filepaths.save_version = 0
scene.unit_settings.system = 'METRIC'

def material(name, color, metallic=0, roughness=.4, emission=0):
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    node = mat.node_tree.nodes.get('Principled BSDF')
    rgb = tuple(((int(color[i:i+2], 16) / 255 + .055) / 1.055) ** 2.4
                if int(color[i:i+2], 16) / 255 > .04045
                else int(color[i:i+2], 16) / 255 / 12.92 for i in (0, 2, 4))
    node.inputs['Base Color'].default_value = (*rgb, 1)
    node.inputs['Metallic'].default_value = metallic
    node.inputs['Roughness'].default_value = roughness
    node.inputs['Emission Color'].default_value = (*rgb, 1)
    node.inputs['Emission Strength'].default_value = emission
    return mat

# Bronze at metallic .72 keeps ~28% diffuse and, with no environment map, no
# reflections: it rendered as the same khaki as key-lit limestone and the whole
# hall read as one tan. Dark, saturated bronze at moderate metallic separates
# from pale matte stone in the forward renderer, by day and by night.
stone = material('Stone · warm limestone', 'E8E4DD', roughness=.56)
dark = material('Stone · midnight basalt', '262638', roughness=.38)
bronze = material('Metal · brushed bronze', '6B4720', .5, .34)
light = material('Light · engraved intelligence', '00D5FF', .1, .3, 1.3)
warm = material('Light · warm scholarship', 'FFB347', .1, .35, 1.2)
leaf = material('Garden · jade foliage', '4E9A5C', .05, .72)
materials = [stone, dark, bronze, light, warm, leaf]

def finish(obj, name, mat):
    obj.name = name
    obj.data.materials.append(mat)
    return obj

def cylinder(name, xy, z, r, depth, mat, top=None):
    bpy.ops.mesh.primitive_cone_add(vertices=48, radius1=r, radius2=r if top is None else top,
                                    depth=depth, location=(*xy, z))
    obj = finish(bpy.context.object, name, mat)
    for face in obj.data.polygons:
        face.use_smooth = len(face.vertices) == 4
    return obj

def ring(name, r, z, thickness, mat, xy=(0, 0)):
    # A hairline under 3 cm reads identically with four minor segments; at
    # eight, each large ring costs 2048 triangles and they were the bulk.
    bpy.ops.mesh.primitive_torus_add(major_radius=r, minor_radius=thickness,
        major_segments=128 if r > 3 else 40, minor_segments=4 if thickness < .03 else 8,
        location=(*xy, z))
    obj = finish(bpy.context.object, name, mat)
    for face in obj.data.polygons:
        face.use_smooth = True
    return obj

def tube(name, points, radius, mat, taper=None, sides=1):
    curve = bpy.data.curves.new(name, 'CURVE')
    curve.dimensions = '3D'
    curve.resolution_u = 1
    curve.bevel_depth = radius
    curve.bevel_resolution = sides
    spline = curve.splines.new('POLY')
    spline.points.add(len(points) - 1)
    last = max(len(points) - 1, 1)
    for k, (dst, src) in enumerate(zip(spline.points, points)):
        dst.co = (*src, 1)
        if taper:
            # Per-point radius scales the bevel: a rib can carry weight at the
            # spring and refine toward the crown at no polygon cost.
            dst.radius = taper[0] + (taper[1] - taper[0]) * k / last
    obj = bpy.data.objects.new(name, curve)
    bpy.context.collection.objects.link(obj)
    curve.materials.append(mat)
    return obj

def polar(r, a, z):
    # Blender Z-up -> glTF Y-up: preserve app X/Z plane orientation.
    return (r * math.cos(a), -r * math.sin(a), z)

def box(name, location, scale, mat, angle=0):
    bpy.ops.mesh.primitive_cube_add(size=1, location=location)
    obj = finish(bpy.context.object, name, mat)
    obj.scale = scale
    obj.rotation_euler.z = angle
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    bevel = obj.modifiers.new('Carved edges', 'BEVEL')
    bevel.width = .06
    bevel.segments = 2
    bpy.context.view_layer.objects.active = obj
    bpy.ops.object.modifier_apply(modifier=bevel.name)
    return obj

def sheet(name, vertices, faces, mat):
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.collection.objects.link(obj)
    mesh.materials.append(mat)
    return obj

def band(name, profile, mat, segments=128):
    """Sweep a (radius, z) profile around the axis. An entablature is a
    rectangular band with fascia steps, not a round hoop. The profile runs
    counter-clockwise in the r/z plane so every face points outward."""
    verts, faces = [], []
    n = len(profile)
    for i in range(segments):
        a = i * math.tau / segments
        verts.extend(polar(r, a, z) for r, z in profile)
    for i in range(segments):
        j = (i + 1) % segments
        for k in range(n - 1):
            faces.append((i*n+k, i*n+k+1, j*n+k+1, j*n+k))
    return sheet(name, verts, faces, mat)

DOME_CENTER = Vector((0, 0, 22.05))

def inward(p, d):
    """Move a vault point d metres toward the dome centre (negative: outward)."""
    v = Vector(p) - DOME_CENTER
    v.normalize()
    return tuple(Vector(p) - v * d)

def panel(name, grid, mat, outward):
    """5x5 point grid -> quad sheet whose normal is forced toward or away from
    the dome centre, so single-sided runtime rendering shows the face that is
    meant to be seen."""
    faces = [(r*5+c, r*5+c+1, r*5+c+6, r*5+c+5) for r in range(4) for c in range(4)]
    obj = sheet(name, grid, faces, mat)
    centre = sum((Vector(p) for p in grid), Vector()) / len(grid)
    if (obj.data.polygons[0].normal.dot(centre - DOME_CENTER) > 0) != outward:
        obj.data.flip_normals()
    return obj

def folded_leaf(p, direction, length):
    # Opaque two-sided leaf with a raised central vein; no coincident polygons.
    tip = (p[0]+math.cos(direction)*length, p[1]+math.sin(direction)*length, p[2]+.09)
    mid = tuple((p[j]+tip[j])/2 for j in range(3))
    lateral = (-math.sin(direction)*.12, math.cos(direction)*.12, 0)
    verts = [p, (mid[0]+lateral[0], mid[1]+lateral[1], mid[2]), (mid[0], mid[1], mid[2]+.07),
             tip, (mid[0]-lateral[0], mid[1]-lateral[1], mid[2])]
    sheet('Conservatory · leaf', verts, [(0,1,2),(1,3,2),(0,2,4),(2,3,4)], leaf)

# Suspended conservatory gardens sit above head height (lowest vine 2.66 m),
# leaving ground navigation and the Mesh approach untouched. Green has to READ
# at 51 m: faceted canopy masses carry the orbit view, fronds the close view,
# and cascading vines make them hanging gardens rather than potted plants.
for garden in range(12):
    a = garden*math.tau/12
    if garden in (7,8):
        continue
    xy = polar(23.2,a,0)[:2]
    cylinder(f'Conservatory {garden} · terraced vessel',xy,4.7,1.15,.6,stone,.92)
    ring(f'Conservatory {garden} · brass lip',1.15,5.0,.04,bronze,xy)
    for offset in (-.03,.03):
        tube('Garden suspension',[polar(23.2,a+offset,z) for z in (4.9,8.9)],.03,bronze)
    for k,(dx,dy,dz,s) in enumerate([(0,0,.55,1.35),(.7,.35,.25,1.0),(-.6,.5,.3,.95),(.1,-.7,.2,.9)]):
        bpy.ops.mesh.primitive_ico_sphere_add(subdivisions=1,radius=s,location=(xy[0]+dx,xy[1]+dy,5.0+dz))
        canopy = finish(bpy.context.object,f'Conservatory {garden} · canopy {k}',leaf)
        canopy.scale = (1,1,.7)
        bpy.ops.object.transform_apply(location=False,rotation=False,scale=True)
    for branch in range(5):
        theta = a+branch*math.tau/5
        points=[]
        for k in range(7):
            t=k/6
            r=1.9*t
            points.append((xy[0]+r*math.cos(theta),xy[1]+r*math.sin(theta),5.4+1.0*math.sin(t*2.6)))
        tube('Living frond stem',points,.023,leaf)
        for k in range(1,6):
            for side in (-1,1):
                folded_leaf(points[k],theta+side*.9,.55*(1-k/9))
    for v in range(4):
        theta = a + v*math.tau/4 + .4
        pts=[(xy[0]+math.cos(theta)*(1.08+.3*math.sin(k*1.7)),
              xy[1]+math.sin(theta)*(1.08+.3*math.cos(k*1.3)), 4.5-k*.46) for k in range(5)]
        tube('Cascading vine',pts,.035,leaf)
        for k in range(1,5):
            for side in (-1,1):
                folded_leaf(pts[k],theta+side*1.2,.32)
    # Photovoltaic parasol: five dark petals fanned over the vessel on bronze
    # ribs. From the orbit view they ring the outer order as dark rosettes and
    # double as shade over the hanging gardens.
    for s in range(5):
        pa = a+(s-2)*.045
        base = polar(22.5,a,8.95)
        tip = polar(25.1,pa,8.45+abs(s-2)*.06)
        mid = tuple((base[j]+tip[j])/2 for j in range(3))
        lateral = (-math.sin(pa)*.5, -math.cos(pa)*.5, 0)
        pts = [base,(mid[0]+lateral[0],mid[1]+lateral[1],mid[2]+.02),tip,
               (mid[0]-lateral[0],mid[1]-lateral[1],mid[2]+.02)]
        sheet('Conservatory · photovoltaic petal',pts,[(0,1,2,3)],dark)
        tube('Solar petal bronze rib',[base,tip],.03,bronze)

# Seven original colonnade positions; the Mesh doorway stays completely clear.
for i in range(8):
    if i == 5:
        continue
    a = i * math.tau / 8
    xy = polar(14, a, 0)[:2]
    box(f'Column {i} · plinth', (*xy, .3), (1.95, 1.95, .6), dark, -a)
    cylinder(f'Column {i} · foot', xy, .72, .95, .24, stone)
    ring(f'Column {i} · base molding', .76, .96, .13, stone, xy)
    cylinder(f'Column {i} · tapered shaft', xy, 9.3, .64, 16.5, stone, .5)
    for flute in range(16):
        t = flute * math.tau / 16
        pts = [(xy[0] + (.655 - .13*k/12)*math.cos(t),
                xy[1] + (.655 - .13*k/12)*math.sin(t), 1.25 + 16*k/12)
               for k in range(13)]
        tube(f'Column {i} · carved flute {flute}', pts, .027, stone)
    ring(f'Column {i} · neck', .56, 17.35, .10, bronze, xy)
    cylinder(f'Column {i} · echinus', xy, 17.67, .58, .5, stone, .88)
    box(f'Column {i} · abacus', (*xy, 18.03), (1.9, 1.9, .28), stone, -a)
    # One engraved channel on the inward-facing stone, following the taper so
    # it stays cut into the shaft instead of floating off it near the capital.
    tube(f'Column {i} · light channel',
         [polar(14 - (.64 - .14*(h-1.05)/16.5) + .006, a, h) for h in (1.3, 8.8, 17)], .018, light)

# Entablatures are rectangular bands with fascia steps and a projecting
# cornice, not round hoops: horizontal weight is what makes an order read from
# the orbit view. The warm hairline sits under the inner cornice lip, engraved
# into a soffit rather than free-floating.
band('Vault · inner entablature', [(13.5,18.2),(14.9,18.2),(14.9,18.55),(15.1,18.55),
     (15.1,18.95),(15.35,18.95),(15.35,19.25),(13.5,19.25)], stone)
band('Vault · inner bronze fillet', [(14.9,18.5),(14.98,18.5),(14.98,18.6),(14.9,18.6)], bronze)
ring('Vault · engraved soffit light', 15.12, 18.93, .026, warm)
band('Vault · outer entablature', [(21.0,22.1),(23.0,22.1),(23.0,22.6),(23.25,22.6),
     (23.25,23.1),(23.6,23.1),(23.6,23.45),(21.0,23.45)], stone)
band('Vault · outer bronze fillet', [(23.0,22.55),(23.1,22.55),(23.1,22.66),(23.0,22.66)], bronze)
for r, z, thickness, mat in [(3.25,36.15,.18,bronze), (3.25,36.38,.055,light), (25.5,-.08,.15,bronze)]:
    ring('Vault · oculus / promenade edge', r, z, thickness, mat)

# A second, monumental order opens a 51-meter Rotunda around the intimate
# working heart. The original library, stairs and task anchors stay in place.
# The outer piers must outweigh the inner columns, not be slenderer than them.
for i in range(16):
    if i in (9, 10):  # preserve generous Mesh approach, not just a slit
        continue
    a = i * math.tau/16
    xy = polar(22, a, 0)[:2]
    box(f'Outer order {i} · podium', (*xy,.3), (2.1,2.1,.6), dark, -a)
    cylinder(f'Outer order {i} · base', xy, .85, 1.12, .5, stone, .92)
    cylinder(f'Outer order {i} · pier', xy, 11.2, .88, 20.2, stone, .74)
    ring(f'Outer order {i} · impost', .84, 17.0, .13, bronze, xy)   # the arch spring line
    ring(f'Outer order {i} · collar', .82, 21.4, .12, bronze, xy)
    box(f'Outer order {i} · capital', (*xy,21.85), (2.2,2.2,.5), stone,-a)
    # Reveals are cut into the pier surface at every height, following the
    # taper, instead of standing at a fixed radius half a metre in front of it.
    psi = math.pi - a
    for t in (-.38, .38):
        pts = []
        for z in (1.2, 21.3):
            radius = .88 - .14*(z-1.1)/20.2 + .015
            pts.append((xy[0] + radius*math.cos(psi+t), xy[1] + radius*math.sin(psi+t), z))
        tube(f'Outer order {i} · bronze reveal', pts, .035, bronze)
    # Structural round arches make the outer order an inhabited arcade, not
    # a ring of isolated poles. Mesh approach keeps its open bay.
    if i not in (8, 15):
        end = (i+1)*math.tau/16
        p, q = polar(22,a,0), polar(22,end,0)
        chord = math.dist(p,q)
        points = []
        for k in range(25):
            t = k/24
            points.append((p[0]*(1-t)+q[0]*t, p[1]*(1-t)+q[1]*t,
                           17.1+math.sin(t*math.pi)*chord*.48))
        tube(f'Outer arcade {i} · stone archivolt', points, .34, stone, sides=2)
        # Hung below the archivolt so it is actually visible, not buried in it.
        tube(f'Outer arcade {i} · bronze intrados', [(x,y,z-.40) for x,y,z in points], .06, bronze)

# Radial promenade, below and outside the existing live floor (no coplanar layer).
verts, faces = [], []
for i in range(129):
    a = i * math.tau/128
    verts.extend([polar(19.95,a,-.08), polar(25.5,a,-.08)])
for i in range(128):
    faces.append((i*2+2,i*2+3,i*2+1,i*2))
sheet('Rotunda · promenade', verts, faces, dark)
for r in (20.15,24.9):
    ring('Promenade · inlaid border',r,-.04,.028,bronze)

# Pietra-dura promenade: a QUIET floor. Basalt wedges, hairline bronze rays and
# one pale stone ray on each pier axis. The old alternating pale wedges were the
# loudest element in the orbit view and fought the colonnade rhythm.
# The assembly floor, props and dais inside r=15 remain live and unobstructed.
for i in range(48):
    a = i*math.tau/48
    b = (i+1)*math.tau/48
    vertices = [polar(r,t,.012) for r,t in [(16.2,a+.009),(24.5,a+.009),(24.5,b-.009),(16.2,b-.009)]]
    sheet(f'Promenade · radial inlay {i}', vertices, [(3,2,1,0)], dark)
    if i % 3 == 0:
        ray = [polar(r,t,.03) for r,t in [(16.4,a-.012),(24.3,a-.012),(24.3,a+.012),(16.4,a+.012)]]
        sheet(f'Promenade · stone ray {i}', ray, [(3,2,1,0)], stone)
    tube(f'Promenade · bronze radial {i}',[polar(16.2,a,.04),polar(24.5,a,.04)],.014,bronze)
for r in (16.1,16.3,19.6,19.8,24.4,24.6):
    ring('Promenade · concentric engraving',r,.02,.018,bronze)

# Open coffered vault: enough sky between carved ribs to keep the room breathable.
# Dome spring at 22, oculus at 36; no roof texture or transparency sorting cost.
def vault(t, a):
    r = 3.25 + 18.75 * math.cos(t * math.pi / 2)
    return polar(r, a, 22.05 + 14.1 * math.sin(t * math.pi / 2))

RIB_SPRING, RIB_CROWN = .30, .13
for i in range(24):
    a = i * math.tau / 24
    # Ribs carry weight at the spring and refine toward the crown; the taper is
    # a per-point bevel radius, so it costs no polygons. Eight sides read round.
    tube(f'Vault · stone rib {i}', [vault(k/24, a) for k in range(25)], RIB_SPRING, stone,
         taper=(1, RIB_CROWN/RIB_SPRING), sides=2)
    # One engraved warm hairline on each rib's inner face: at night the vault
    # reads as carved light, not a wire cage. Replaces the buried bronze seam.
    pts = []
    for k in range(1, 24):
        r = RIB_SPRING + (RIB_CROWN-RIB_SPRING)*k/24
        pts.append(inward(vault(k/24, a), r + .012))
    tube(f'Vault · engraved rib light {i}', pts, .016, warm)
for j in range(1, 6):
    t = j / 7
    p = vault(t, 0)
    ring(f'Vault · latitude {j}', p[0], p[2], .095, bronze)
    for i in range(24):
        a = (i+.5) * math.tau/24
        if i in (0,1,2,3,4,5):  # deliberate quarter-cutaway toward the orbit view
            continue
        # Each coffer: a raised stone lip framing a recessed inner field, plus
        # an outer shell so the orbit view sees a pale coffered dome instead of
        # looking through culled back faces at the dark interior.
        frame = [vault(t-.058,a-.095), vault(t-.058,a+.095), vault(t+.058,a+.095), vault(t+.058,a-.095)]
        tube(f'Vault · coffer lip {j}.{i}', frame+[frame[0]], .05, stone)
        grid = [(row, col) for row in range(5) for col in range(5)]
        panel(f'Vault · recessed coffer {j}.{i}',
              [inward(vault(t-.055+row*.0275, a-.09+col*.045), -.10) for row, col in grid], stone, False)
        panel(f'Vault · coffer shell {j}.{i}',
              [inward(vault(t-.055+row*.0275, a-.09+col*.045), -.16) for row, col in grid], stone, True)

# Render-only stage: never exported over the live floor or existing agents.
architecture = list(scene.objects)
for obj in architecture:
    obj['permagent_role'] = 'static_architecture'
stage = cylinder('PREVIEW ONLY · floor', (0,0), -.32, 20, .45, dark)
camera_data = bpy.data.cameras.new('Preview camera')
camera = bpy.data.objects.new('Preview camera', camera_data)
scene.collection.objects.link(camera)
camera.location = (43, 50, 26)
camera.rotation_euler = (Vector((0,0,17))-camera.location).to_track_quat('-Z','Y').to_euler()
camera_data.lens = 29
scene.camera = camera
for name, loc, energy, size, color in [
    ('Warm key', (2,8,27), 14000, 16, (1,.83,.63)),
    ('Cool fill', (-15,-8,16), 10500, 12, (.4,.68,1)),
    ('Front softbox', (12,22,14), 9000, 18, (1,.94,.83))]:
    data = bpy.data.lights.new(name, 'AREA')
    data.energy, data.shape, data.size, data.color = energy, 'DISK', size, color
    obj = bpy.data.objects.new(name, data)
    scene.collection.objects.link(obj)
    obj.location = loc
    obj.rotation_euler = (Vector((0,0,10))-obj.location).to_track_quat('-Z','Y').to_euler()
scene.world.color = (.025,.035,.065)
scene.render.engine = 'CYCLES'
scene.cycles.samples = 24
scene.render.resolution_x = 1200
scene.render.resolution_y = 1000
scene.render.resolution_percentage = 100
scene.render.image_settings.file_format = 'PNG'
scene.render.filepath = str(SOURCE / 'observatory-vault-preview.png')
bpy.ops.wm.save_as_mainfile(filepath=str(SOURCE / 'observatory-vault.blend'))
if '--render' in sys.argv:
    bpy.ops.render.render(write_still=True)

# Export only architecture, collapsing material families into six bounded draws.
bpy.ops.object.select_all(action='DESELECT')
for obj in architecture:
    obj.select_set(True)
bpy.context.view_layer.objects.active = architecture[0]
bpy.ops.object.convert(target='MESH')
exported = []
for mat in materials:
    bpy.ops.object.select_all(action='DESELECT')
    group = [o for o in scene.objects if o.get('permagent_role') == 'static_architecture'
             and o.type == 'MESH' and o.data.materials[0] == mat]
    if not group:
        continue
    for obj in group:
        obj.select_set(True)
    bpy.context.view_layer.objects.active = group[0]
    if len(group) > 1:
        bpy.ops.object.join()
    obj = bpy.context.object
    obj.name = 'ObservatoryVault · ' + mat.name
    exported.append(obj)
bpy.ops.object.select_all(action='DESELECT')
for obj in exported:
    obj.select_set(True)
bpy.ops.export_scene.gltf(filepath=str(PUBLIC / 'observatory-vault.glb'),
    export_format='GLB', use_selection=True, export_yup=True,
    export_animations=False, export_cameras=False, export_lights=False)
triangles = sum(sum(len(p.vertices)-2 for p in o.data.polygons) for o in exported)
manifest = dict(schema='permagent.world-asset.v1', asset='observatory-vault.glb',
    authoring='Blender', metersPerUnit=1, runtimeUp='Y', meshes=len(exported),
    triangles=triangles, bytes=(PUBLIC/'observatory-vault.glb').stat().st_size,
    materials=len(materials), meshDoorwayDegrees=225, liveState=False)
assert triangles < 150000, manifest
assert manifest['bytes'] < 8000000, manifest
(PUBLIC/'observatory-vault.manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')
print('WORLD_ASSET_VERIFIED', json.dumps(manifest))
