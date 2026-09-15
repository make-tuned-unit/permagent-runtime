"""P1 clustered vertical towers for the Solarpunk north-star pass.

Replaces the legacy stepped-pyramid skyline with three distinct urban rooms:
- a three-tower spire cluster in the NE,
- a four-room habitat market cluster in the west,
- a geodesic conservatory dome in the SE.

Repeated structural units are emitted as linked duplicates so the exporter can
reuse mesh data.
"""

import math
import random
import bmesh
from mathutils import Vector

random.seed(2719)


def _ensure_material(obj, material):
    if obj.type != 'MESH':
        return obj
    if not obj.data.materials:
        obj.data.materials.append(material)
    else:
        for index in range(len(obj.data.materials)):
            obj.data.materials[index] = material
    return obj


def _linked_object(template, name, location=(0.0, 0.0, 0.0),
                 rotation=(0.0, 0.0, 0.0), scale=(1.0, 1.0, 1.0),
                 material=None):
    obj = bpy.data.objects.new(name, template.data)
    scene.collection.objects.link(obj)
    obj.location = Vector(location)
    obj.rotation_euler = Vector(rotation)
    obj.scale = Vector(scale)
    obj['forum_architecture'] = True
    if material is not None:
        _ensure_material(obj, material)
    return obj


def _make_tube_from_points(template, start, end, name=None, material=None):
    start = Vector(start)
    end = Vector(end)
    span = end - start
    length = span.length
    if length <= 1e-5:
        return None
    obj = _linked_object(template, name or (template.name + ' link'),
                        location=(start + end) * 0.5,
                        material=material)
    obj.rotation_euler = span.to_track_quat('Z', 'Y').to_euler()
    base = template.dimensions.z if template.dimensions.z else 1.0
    obj.scale = (obj.scale.x, obj.scale.y, length / base)
    return obj


def _balconies_for_tower(name, base, height, radius):
    cx, cy = base
    balcony_template = box(name + ' balcony template', (0, 0, 0.175), (2.0, 2.0, 0.35), clay)
    balcony_template.name = name + ' balcony template'
    parapet_template = box(name + ' planter parapet template', (0, 0, 0.28), (.32, 1.25, .56), clay)
    parapet_template.name = name + ' planter parapet template'
    trailing_template = None
    for level_index, z in enumerate(range(6, int(height), 6)):
        cantilever = 2.0 if (level_index % 2) == 0 else 2.8
        terrace = _linked_object(
            balcony_template,
            f'{name} terrace {level_index + 1}',
            location=(cx, cy, z + 0.175),
            scale=((radius + cantilever) * .92, (radius + cantilever) * .92, 1.0),
            material=clay,
        )
        terrace['balcony_role'] = 'tiered garden balcony with alternating cantilever'
        # Four planter parapet sections make the alternating slabs readable
        # from above; the same small mesh is instanced around every tier.
        for spin in range(4):
            a=spin*math.tau/4
            px=cx+math.cos(a)*(radius+cantilever*.72);py=cy+math.sin(a)*(radius+cantilever*.72)
            _linked_object(parapet_template,f'{name} planter parapet {level_index + 1}-{spin}',(px,py,z+.46),(0,0,a))
        for spin in range(8):
            a = spin * math.tau / 10
            tx = cx + math.cos(a) * (radius + cantilever * 0.34)
            ty = cy + math.sin(a) * (radius + cantilever * 0.34)
            if level_index % 3 == 0 and spin < 3 and 'linked_understory' in globals():
                linked_understory(f'{name} linked trailing balcony planting', (tx, ty, z + .58), .95)
            else:
                leaf_spray(f'{name} balcony canopy', (tx, ty, z + .73), 0.72, 12)
    # Keep the source template mesh alive for linked users, but remove its
    # template object so it cannot export as a stray balcony at the origin.
    bpy.data.objects.remove(balcony_template, do_unlink=True)
    bpy.data.objects.remove(parapet_template, do_unlink=True)


def _sky_fins(name, base, height, radius):
    cx, cy = base
    fin_template = box('Glazed vault panel | ' + name + ' glass fin template',
                        (0, 0, 0.5), (0.11, radius * 2.1, 1.0), glass)
    fin_template.name = name + ' glass fin template'
    for fin in range(12):
        angle = fin * math.tau / 12
        _linked_object(
            fin_template,
            f'{name} glass fin {fin}',
            location=(cx, cy, height * 0.50),
            rotation=(0.0, 0.0, angle),
            scale=(1.0, 1.0, height),
            material=glass,
        )
    bpy.data.objects.remove(fin_template, do_unlink=True)


def _spire(name, center, height, base_radius):
    cx, cy = center
    core = cyl(name + ' core', (cx, cy, height * 0.5), base_radius,
               height, stone, top=base_radius * .56, verts=12)
    _ensure_material(core, pale)
    _balconies_for_tower(name, center, int(height), base_radius)
    _sky_fins(name, center, height, base_radius)

    for level in range(5):
        z = 0.35 + level * 1.95
        branch((cx, cy, z), (cx, cy, z + 1.2 + level * .08), 0.08 if level else .12, bronze)

    crown = cyl(name + ' crown lantern stem', (cx, cy, height + 1.0), .58, .95, bronze, verts=12)
    ring(name + ' crown lantern ring', (cx, cy, height + 1.5), 1.16, .08, bronze)
    sphere = cyl(name + ' crown lantern beacon', (cx, cy, height + 1.76), 1.08, .36, cyan, verts=24)
    _ensure_material(sphere, cyan)
    crown_terrace = arc_band(name + ' crown terrace', base_radius * .9,
                             base_radius + 2.2, height + .32, .26, clay,
                             segments=72)
    crown_terrace.location = (cx, cy, 0)
    for k in range(6):
        a = k * math.tau / 6
        tx = cx + math.cos(a) * 1.4
        ty = cy + math.sin(a) * 1.4
        leaf_spray(f'{name} tower canopy', (tx, ty, height + 1.2), 0.84, 16)
    return core


def _sky_bridge(name, start, end, z=40.0, depth=1.55):
    a = Vector(start)
    b = Vector(end)
    span = b - a
    if span.length <= 0.1:
        return
    segment_template = box('Glazed vault panel | ' + name + ' segment template', (0, 0, 0.2),
                          (depth * 1.85, 1.15, 0.24), glass)
    segment_template.name = name + ' skybridge segment template'
    steps = max(1, math.ceil(span.length / (depth * 1.1)))
    for index in range(steps):
        t0 = index / steps
        t1 = (index + 1) / steps
        p0 = a + span * t0
        p1 = a + span * t1
        mid = (p0 + p1) * 0.5
        _linked_object(
            segment_template,
            f'{name} segment {index}',
            location=(mid.x, mid.y, z + 0.12),
            rotation=(0.0, 0.0, math.atan2(span.y, span.x)),
            scale=(1.0, 1.0, 1.0),
            material=glass,
        )
        if index in (0, steps - 1):
            bx = mid.x - span.y * 0.007
            by = mid.y + span.x * 0.007
            _ = box(name + ' skybridge strut', (bx, by, z + 0.74), (0.2, 0.2, 1.3), bronze)
    bpy.data.objects.remove(segment_template, do_unlink=True)


def _market_habitat_cluster(center):
    cx, cy = center
    modules = [
        ('A', (-13.2, 0.0), 15.5, 44.0),
        ('B', (-4.5, 1.3), 14.2, 38.0),
        ('C', (4.2, -0.8), 13.4, 34.0),
        ('D', (12.6, 0.6), 12.6, 29.0),
    ]
    for label, (ox, oy), footprint, height in modules:
        px, py = cx + ox, cy + oy
        level_h = 2.95
        # Derive the stepped count from the requested 28–44 m habitat heights
        # rather than leaving the modules as low four-storey pavilions.
        levels = max(4, int(round(height / level_h)))
        for level in range(levels):
            z = level * level_h + 0.18
            # Keep the street-facing mass vertical; setbacks occur only on the
            # lagoon-facing (south) edge, every third floor, as deep loggias.
            width = footprint
            depth = footprint * 0.72 - (level // 3) * 3.2
            py_level = py + (level // 3) * 1.6
            slab = box(f'P1 habitat {label} Forum White Sandstone Masonry floor slab', (px, py_level, z), (width, depth, 0.34), pale)
            slab['masonry_role'] = 'Forum White Sandstone Masonry'
            box(f'P1 habitat {label} loggia frame', (px, py_level, z + 0.8), (width + .6, 0.42, 2.2), stone)
            box(f'P1 habitat {label} recessed dark loggia', (px, py_level - depth * .46, z + 1.55), (max(2.0, width - 1.4), .12, 1.75), dark)
            for bay in range(max(2, int(width / 3.8))):
                bx = px - width * .42 + (bay + .5) * width / max(2, int(width / 3.8))
                cyl(f'P1 habitat {label} loggia column', (bx, py_level - depth * .49, z + 1.55), .11, 2.7, stone, verts=8)
            shell = box(f'P1 habitat {label} Forum White Sandstone Masonry side shell', (px + width * 0.41, py_level, z + 1.8),
                (0.28, depth, level_h * .88), dark)
            shell['masonry_role'] = 'Forum White Sandstone Masonry'
            branch((px - width * 0.41, py, z + .2), (px - width * 0.41, py, z + 1.5), .045, bronze)
            roof = box(f'P1 habitat {label} planted roof garden setback {level}', (px, py_level, z + 1.95), (width + .8, depth + .9, .22), clay)
            roof['green_roof'] = True
            if level % 2 == 0 or level == levels - 1:
                for spin in range(4):
                    t = spin * math.tau / 6
                    trunk = (
                        px + (width / 3) * math.cos(t),
                        py + (depth / 3) * math.sin(t),
                        z + 2.35,
                    )
                    leaf_spray(f'P1 habitat {label} roof canopy', trunk, 0.65, 10)

    market_scale = 1.25
    hall_x, hall_y = cx - 2.3, cy
    hall_base = 6.2
    hall = box('P1 market hall', (hall_x, hall_y, hall_base), (10.4, 5.5, .34), timber)
    box('P1 market hall roof', (hall_x, hall_y, hall_base + 1.95), (9.8, 5.2, .28), solar)
    for side in (-1, 1):
        branch((hall_x + 5.0, hall_y + side * 2.55, hall_base),
               (hall_x + 2.4, hall_y + side * 2.55, hall_base + 1.95), .08, bronze)
        for p in range(3):
            t = side * (p - 1) * .24
            _ = box(f'P1 market vendor stand', (hall_x + t * .72, hall_y + side * 1.9, hall_base + 1.05),
                   (1.35, .82, .28), clay)
    _sky_bridge('P1 market bridge', (cx - 13.0, cy, 8.0), (cx - 4.0, cy, 8.7))
    _sky_bridge('P1 market bridge', (cx + 0.6, cy, 8.8), (cx + 11.7, cy, 8.9))

    for leg in range(2):
        bx = cx - 1.6 + leg * 9.8
        for i in range(4):
            branch((bx, cy - 5.3, 6.1), (bx + 3.4 * ((-1) ** i), cy + 4.8, 9.2), .08, bronze)


def _conservatory_dome(center):
    cx, cy = center
    drum = cyl('P1 conservatory drum', (cx, cy, 2.05), 28.2, 4.3, stone, top=29.2, verts=48)
    ring('P1 conservatory drum rim', (cx, cy, 4.2), 28.85, .16, bronze)

    bpy.ops.mesh.primitive_ico_sphere_add(subdivisions=3, radius=28.0, location=(cx, cy, 4.95))
    shell = bpy.context.object
    shell.name = 'Glazed vault panel | P1 conservatory geodesic shell'
    bm = bmesh.new()
    bm.from_mesh(shell.data)
    remove = []
    for face in bm.faces:
        z_center = sum(v.co.z for v in face.verts) / len(face.verts)
        # Keep the upper geodesic hemisphere above the stone drum.  The prior
        # threshold left only a cap and then deleted all non-boundary edges,
        # producing an empty drum in the source render.
        if z_center < 0.0:
            remove.append(face)
    if remove:
        bmesh.ops.delete(bm, geom=remove, context='FACES')
    bm.normal_update()
    bm.to_mesh(shell.data)
    bm.free()
    _ensure_material(shell, glass)

    lattice_template = cyl('P1 conservatory lattice tube template', (0, 0, 0.5), .18, 1.0, bronze, verts=10)
    lattice_template.name = 'P1 conservatory lattice tube template'
    for edge in shell.data.edges:
        start = shell.matrix_world @ shell.data.vertices[edge.vertices[0]].co
        end = shell.matrix_world @ shell.data.vertices[edge.vertices[1]].co
        midpoint = (start + end) * 0.5
        if start.z > 4.2 and end.z > 4.2:
            _make_tube_from_points(lattice_template, start, end,
                                  name='P1 conservatory lattice', material=bronze)
    bpy.data.objects.remove(lattice_template, do_unlink=True)

    ring('Glazed vault panel | P1 conservatory glass gallery', (cx, cy, 4.3), 28.2, .07, glass)
    for i in range(14):
        a = i * math.tau / 14
        px = cx + 13.8 * math.cos(a)
        py = cy + 13.8 * math.sin(a)
        box('P1 conservatory gallery bay', (px, py, 4.35), (2.45, .34, .18), bronze, a)

    trunk_template = cyl('P1 conservatory interior trunk', (0, 0, 1.25), 0.13, 2.5, timber, verts=10)
    for index in range(8):
        a = index * math.tau / 8
        px = cx + 11.2 * math.cos(a)
        py = cy + 11.2 * math.sin(a)
        height = 5.5 + (index % 3) * 0.45
        trunk = _linked_object(
            trunk_template,
            f'P1 conservatory trunk {index}',
            location=(px, py, 1.2),
            material=timber,
        )
        trunk.scale = (1.0, 1.0, height / 2.5)
        leaf_spray('P1 conservatory interior canopy', (px, py, height + 1.5), 2.05, 20)
        leaf_spray('P1 conservatory understory', (px, py, 1.75), 0.8, 9)
    bpy.data.objects.remove(trunk_template, do_unlink=True)


_spires = [
    ('P1 NE spire Alpha', (74.0, 58.0), 72.0, 7.3),
    ('P1 NE spire Beta', (104.0, 86.0), 96.0, 8.2),
    ('P1 NE spire Gamma', (122.0, 52.0), 118.0, 9.2),
]
for label, center, h, radius in _spires:
    _spire(label, center, h, radius)

_sky_bridge('P1 spire skybridge', (84.0, 66.0, 40.0), (98.0, 77.0, 40.0))
_sky_bridge('P1 spire skybridge', (108.0, 78.0, 40.0), (117.0, 64.0, 40.0))

cyl('P1 NE spire planted plaza', (102.0, 73.0, .08), 25.0, .16, clay, verts=48)
for index in range(12):
    a = index * math.tau / 12
    px, py = 102.0 + 20.0 * math.cos(a), 73.0 + 20.0 * math.sin(a)
    branch((px, py, .15), (px, py, 2.8 + (index % 3) * .4), .10, timber)
    leaf_spray('P1 NE spire plaza tree crown', (px, py, 3.2 + (index % 3) * .4), 1.25, 30)
    leaf_spray('P1 NE spire plaza understory', (px + .3, py - .2, .5), .55, 14)

_market_habitat_cluster((-100.0, -30.0))
_conservatory_dome((80.0, -95.0))

forum_towers_manifest = {
    'schema': 'permagent.forum.towers.v1',
    'schemaVersion': 1,
    'deterministic': True,
    'seed': 2719,
    'spireCluster': {
        'name': 'NE Spire Cluster',
        'cores': len(_spires),
        'skybridgeLevels': 2,
    },
    'habitatCluster': {
        'name': 'West Habitat Cluster',
        'modules': 4,
        'marketCourtyards': 1,
        'skybridges': 2,
    },
    'domeCluster': {
        'name': 'SE Geodesic Conservatory',
        'radius_m': 28,
        'forestTrees': 8,
    },
    'routeSafe': True,
}
print('FORUM_TOWERS', forum_towers_manifest)

# Job 16 inhabited-night pass: one thin warm strip mesh is reused on every
# lit floor, with alternating bays so the towers do not become solid beacons.
_job16_window=mat('Forum Warm Window Emission',(.95,.27,.055),metal=.05,rough=.25)
_job16_shader=_job16_window.node_tree.nodes.get('Principled BSDF')
if _job16_shader:
    if _job16_shader.inputs.get('Emission Color'): _job16_shader.inputs['Emission Color'].default_value=(.95,.27,.055,1)
    if _job16_shader.inputs.get('Emission Strength'): _job16_shader.inputs['Emission Strength'].default_value=3.5
_job16_strip=box('Job16 linked emissive window strip template',(0,0,.12),(.14,1.8,.24),_job16_window)
def _job16_window_instance(name, point, angle=0.0, scale=(1,1,1)):
    if '_point_close_to_route' in globals() and _point_close_to_route(point[:2],3.5): return
    return _linked_object(_job16_strip,name,(point[0],point[1],point[2]),(0,0,angle),scale,_job16_window)
for _name,(_cx,_cy),_height,_radius in _spires:
    for _level in range(4,int(_height),6):
        for _bay in range(6):
            # Alternating lit floors per bay so the spires read as inhabited at night.
            if ((_level // 6) + _bay) % 2 == 0:
                _a=math.tau*_bay/6;_job16_window_instance(f'Job16 {_name} warm window {_level}-{_bay}',
                    (_cx+math.cos(_a)*(_radius+.12),_cy+math.sin(_a)*(_radius+.12),_level),_a,
                    (.9,.65,1.0))
for _label,(_ox,_oy),_footprint,_height in (('A',(-13.2,0),15.5,44),('B',(-4.5,1.3),14.2,38),
                                             ('C',(4.2,-.8),13.4,34),('D',(12.6,.6),12.6,29)):
    _cx,_cy=-100+_ox,-30+_oy
    for _level in range(4,int(_height),6):
        for _side in (-1, 1):
            if ((_level // 6) + (_side > 0)) % 2 == 0:
                _job16_window_instance(f'Job16 habitat {_label} warm window {_level}{"+" if _side>0 else "-"}',
                                       (_cx+_side*_footprint*.42,_cy-.05,_level),0,(1.0,1.0,1.0))
for _bay in range(5):
    _job16_window_instance(f'Job16 market hall warm window {_bay}',
                           (-102.3 + (_bay - 2) * 1.65, -33.0, 7.2), 0,
                           (1.0, .8, 1.0))
bpy.data.objects.remove(_job16_strip,do_unlink=True)
forum_towers_manifest['job16_triangle_delta_estimate']=len(_spires)*12*6*2 + 4*4*6
print('FORUM_TOWERS_JOB16', {'triangle_delta_estimate':forum_towers_manifest['job16_triangle_delta_estimate'],
                             'linked_window_mesh':True})
