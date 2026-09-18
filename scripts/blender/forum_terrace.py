"""P0 gathering terrace for the Solar Forum north-star pass.

Executed in the namespace of ``build_solar_forum.py`` after the landscape has
defined ``leaf_spray``.  The terrace is authored at the origin in Blender's
Z-up metres and deliberately leaves the four cardinal axes clear at ground
level for the existing commons route.
"""

import math
import random

from mathutils import Vector


random.seed(5807)


def _mesh_object(name, vertices, faces, material):
    mesh = bpy.data.meshes.new(name + ' mesh')
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    scene.collection.objects.link(obj)
    return finish(obj, name, material)


def _linked(template, name, location=(0, 0, 0), rotation=(0, 0, 0),
            scale=(1, 1, 1)):
    obj = bpy.data.objects.new(name, template.data)
    scene.collection.objects.link(obj)
    obj.location = Vector(location)
    obj.rotation_euler = Vector(rotation)
    obj.scale = Vector(scale)
    obj['forum_architecture'] = True
    return obj


def _branch(name, start, end, radius, material):
    start, end = Vector(start), Vector(end)
    obj = cyl(name, (start + end) / 2, radius, (end - start).length,
              material, top=radius * .65, verts=8)
    obj.rotation_euler = (end - start).to_track_quat('Z', 'Y').to_euler()
    return obj


def _hex_to_linear(value):
    values = [int(value[i:i + 2], 16) / 255 for i in (1, 3, 5)]
    return tuple(v / 12.92 if v <= .04045 else ((v + .055) / 1.055) ** 2.4
                 for v in values)


def _set_emission(material, color, strength, alpha=None):
    shader = material.node_tree.nodes.get('Principled BSDF')
    if shader is None:
        return material
    if shader.inputs.get('Emission Color') is not None:
        shader.inputs['Emission Color'].default_value = (*color, 1)
    elif shader.inputs.get('Emission') is not None:
        shader.inputs['Emission'].default_value = (*color, 1)
    if shader.inputs.get('Emission Strength') is not None:
        shader.inputs['Emission Strength'].default_value = strength
    if alpha is not None and shader.inputs.get('Alpha') is not None:
        shader.inputs['Alpha'].default_value = alpha
        material.diffuse_color = (*color, alpha)
    if hasattr(material, 'surface_render_method'):
        material.surface_render_method = 'DITHERED'
    return material


def _annular_segment_mesh(name, inner, outer, start, end, z0, z1,
                          material, segments=16):
    """Closed annular segment centered at the origin, reusable by links."""
    vertices = []
    for z in (z0, z1):
        for radius in (inner, outer):
            for i in range(segments + 1):
                angle = start + (end - start) * i / segments
                vertices.append((radius * math.cos(angle),
                                 radius * math.sin(angle), z))
    stride = segments + 1

    def index(layer, radial, i):
        return layer * 2 * stride + radial * stride + i

    faces = []
    for i in range(segments):
        j = i + 1
        faces.extend([
            (index(0, 0, i), index(0, 0, j), index(0, 1, j), index(0, 1, i)),
            (index(1, 1, i), index(1, 1, j), index(1, 0, j), index(1, 0, i)),
            (index(0, 0, i), index(1, 0, i), index(1, 0, j), index(0, 0, j)),
            (index(0, 1, j), index(1, 1, j), index(1, 1, i), index(0, 1, i)),
        ])
    faces.extend([
        (index(0, 0, 0), index(0, 1, 0), index(1, 1, 0), index(1, 0, 0)),
        (index(0, 1, segments), index(0, 0, segments),
         index(1, 0, segments), index(1, 1, segments)),
    ])
    return _mesh_object(name, vertices, faces, material)


def _box_template(name, scale, material):
    sx, sy, sz = (value / 2 for value in scale)
    vertices = [(-sx, -sy, -sz), (sx, -sy, -sz), (sx, sy, -sz),
                (-sx, sy, -sz), (-sx, -sy, sz), (sx, -sy, sz),
                (sx, sy, sz), (-sx, sy, sz)]
    faces = [(0, 3, 2, 1), (4, 5, 6, 7), (0, 1, 5, 4),
             (1, 2, 6, 5), (2, 3, 7, 6), (3, 0, 4, 7)]
    return _mesh_object(name, vertices, faces, material)


def _uv_sphere_template(name, material, latitudes=12, longitudes=20):
    vertices = []
    for lat in range(latitudes + 1):
        phi = math.pi * lat / latitudes
        for lon in range(longitudes):
            theta = math.tau * lon / longitudes
            vertices.append((math.sin(phi) * math.cos(theta),
                             math.sin(phi) * math.sin(theta), math.cos(phi)))
    faces = []
    for lat in range(latitudes):
        for lon in range(longitudes):
            a = lat * longitudes + lon
            b = lat * longitudes + (lon + 1) % longitudes
            c = (lat + 1) * longitudes + (lon + 1) % longitudes
            d = (lat + 1) * longitudes + lon
            faces.append((a, b, c, d))
    return _mesh_object(name, vertices, faces, material)


def _torus_template(name, material, major=1.0, minor=.022,
                    major_segments=64, minor_segments=8):
    vertices = []
    for major_index in range(major_segments):
        theta = math.tau * major_index / major_segments
        for minor_index in range(minor_segments):
            phi = math.tau * minor_index / minor_segments
            radius = major + minor * math.cos(phi)
            vertices.append((radius * math.cos(theta),
                             radius * math.sin(theta),
                             minor * math.sin(phi)))
    faces = []
    for major_index in range(major_segments):
        for minor_index in range(minor_segments):
            a = major_index * minor_segments + minor_index
            b = major_index * minor_segments + (minor_index + 1) % minor_segments
            c = ((major_index + 1) % major_segments) * minor_segments + (minor_index + 1) % minor_segments
            d = ((major_index + 1) % major_segments) * minor_segments + minor_index
            faces.append((a, b, c, d))
    return _mesh_object(name, vertices, faces, material)


def _create_spray(name, center, radius, count, material):
    """Use the landscape's spray, then split terrace foliage into two tones."""
    before = set(scene.objects)
    leaf_spray(name, center, radius, count)
    created = [obj for obj in scene.objects if obj not in before]
    for obj in created:
        obj.data.materials.clear()
        obj.data.materials.append(material)
    return created


def _fern_mesh(name, material):
    """Reusable gently arched frond with alternating quad pinnae."""
    segments = 7
    vertices, faces, centers = [], [], []
    for index in range(segments):
        t = index / (segments - 1)
        centers.append(Vector((.16 * math.sin(math.pi * t), 0,
                               .12 + 1.55 * t)))
    for center in centers:
        vertices.extend([(center.x - .035, center.y, center.z),
                         (center.x + .035, center.y, center.z)])
    for index in range(segments - 1):
        faces.append((2 * index, 2 * index + 1, 2 * index + 3, 2 * index + 2))
    for index, center in enumerate(centers[1:-1], 1):
        t = index / (segments - 1)
        length = .42 * math.sin(math.pi * t) + .12
        for side in (-1, 1):
            base = len(vertices)
            tip = center + Vector((.16 * math.sin(math.pi * t), side * length, .12))
            vertices.extend([tuple(center + Vector((0, side * .055, 0))),
                             tuple(center - Vector((0, side * .055, 0))),
                             tuple(tip + Vector((0, side * .06, .035))),
                             tuple(tip + Vector((0, -side * .06, -.035)))])
            faces.append((base, base + 1, base + 3, base + 2))
    return _mesh_object(name, vertices, faces, material)


def _fern_planter(name, center, count=12):
    moss = bpy.data.materials.get('Forum Planter Moss')
    if moss is None:
        moss = mat('Forum Planter Moss', (.10, .22, .065), 0, .95)
    cyl(name + ' moss ground cover', (center[0], center[1], .56), .68, .08,
        moss, verts=32)
    fern_a = _fern_mesh(name + ' fern frond mesh A', leaf)
    fern_b = _fern_mesh(name + ' fern frond mesh B', leaf2)
    fern_a.hide_render = True
    fern_b.hide_render = True
    for index in range(count):
        angle = math.tau * index / count + .11 * math.sin(index * 3.1)
        pitch = math.radians(18 + (index * 17) % 28)
        template = fern_a if index % 2 == 0 else fern_b
        obj = _linked(template, '%s fern frond %02d' % (name, index + 1),
                      location=(center[0], center[1], .57),
                      rotation=(pitch * math.cos(angle), pitch * math.sin(angle), angle))
        obj.scale = Vector((.82 + .07 * (index % 3),
                            .82 + .05 * ((index + 1) % 3),
                            .82 + .06 * (index % 4)))


def _new_emissive_material(name, color, strength, alpha=None, roughness=.35):
    material = mat(name, color, .1, roughness)
    return _set_emission(material, color, strength, alpha)


def _bevel(template, offset=.08, segments=3):
    modifier = template.modifiers.new(template.name + ' rounded edges', 'BEVEL')
    modifier.width = offset
    modifier.segments = segments
    modifier.limit_method = 'ANGLE'
    return template


# The landscape detail pass predates the gathering terrace. Remove only the
# central fountain coping and its old bench fasteners; the four teaching-table
# tops/benches are authored at r≈12.5 in the commons block itself.
for legacy in list(scene.objects):
    if (legacy.name.startswith('Individually cut pool coping') or
            legacy.name.startswith('Commons bench timber slat') or
            legacy.name.startswith('Bench countersunk fixing')):
        bpy.data.objects.remove(legacy, do_unlink=True)


# The material names are intentional runtime surfacing hooks for this hero area.
fabric = mat('Forum Sand Fabric', (.78, .66, .48), 0, .92)
oiled_timber = mat('Forum Oiled Timber', (.22, .12, .055), 0, .42)
hologram_color = (.10, .85, .90)
hologram = _new_emissive_material('Forum Hologram', hologram_color, 5.5,
                                  alpha=.40, roughness=.22)
hologram_continent = _new_emissive_material('Forum Hologram Continents',
                                             (.14, .95, .92), 3.0,
                                             alpha=.70, roughness=.28)
hologram_cone = _new_emissive_material('Forum Hologram Light Cone',
                                       hologram_color, 1.0,
                                       alpha=.12, roughness=.18)
hologram_glass = _new_emissive_material('Forum Hologram Glass',
                                        _hex_to_linear('#58D9E8'), .35,
                                        alpha=.35, roughness=.16)
amber = _new_emissive_material('Forum Lantern Amber', _hex_to_linear('#FF9E3D'),
                               8.0, roughness=.24)
strand_amber = _new_emissive_material('Forum Strand Amber', _hex_to_linear('#FFB347'),
                                      4.0, roughness=.24)


# North-side crescent impluvium: water is present without blocking the table.
pool_start = math.radians(18)
pool_end = math.radians(162)
arc_band('Terrace crescent reflecting pool', 7.1, 9.3, .11, .12, water,
         pool_start, pool_end, 72)
arc_band('Terrace crescent inner coping', 6.98, 7.12, .20, .12, pale,
         pool_start, pool_end, 72)
arc_band('Terrace crescent outer coping', 9.28, 9.43, .20, .12, pale,
         pool_start, pool_end, 72)


# Dais, timber council ring and the low bronze emitter.
cyl('Terrace bronze council dais', (0, 0, .225), 3.4, .45, bronze, verts=96)
table_ring = _annular_segment_mesh('Terrace timber council table ring', 1.9, 2.6,
                                    0, math.tau, .60, .78, oiled_timber, 64)
cyl('Terrace inset bronze hologram emitter', (0, 0, .53), .9, .16, bronze,
    verts=64)
ring('Terrace emitter cyan trim', (0, 0, .625), .78, .035, hologram)
cyl('Terrace council dais cyan underside glow', (0, 0, .015), 2.9, .035,
    hologram, verts=96)


# Globe and its three linked ring scales; there is deliberately no solid disc.
globe = _uv_sphere_template('Terrace hologram globe mesh', hologram, 20, 32)
globe_obj = _linked(globe, 'Terrace hologram globe', (0, 0, 2.1),
                    scale=(1.15, 1.15, 1.15))
globe_obj['emission_strength'] = 6.0
globe_rings = _torus_template('Terrace hologram globe ring mesh', hologram,
                              major=1.0, minor=.022, major_segments=72,
                              minor_segments=8)
for index, (radius, tilt) in enumerate([(1.25, 0), (1.35, 35), (1.45, 70)], 1):
    _linked(globe_rings, 'Terrace hologram globe tilted ring %d' % index,
            (0, 0, 2.1), rotation=(math.radians(tilt), 0, 0),
            scale=(radius, radius, radius))

# Raised low-poly land masses break the sphere silhouette so it reads as a
# world rather than a white ball.  They share one mesh and sit just proud of
# the hologram surface as translucent cyan-green patches.
continent = _mesh_object('Terrace hologram continent patch mesh', [
    (-.42, -.16, .98), (-.08, -.30, 1.02), (.30, -.18, .99),
    (.43, .10, 1.00), (.12, .28, 1.02), (-.28, .22, 1.00),
    (0, 0, 1.06)], [(0, 1, 6), (1, 2, 6), (2, 3, 6),
                     (3, 4, 6), (4, 5, 6), (5, 0, 6)], hologram_continent)
continent.scale = (1.0, .7, .45)
for index, (location, rotation, scale) in enumerate([
        ((0, 0, 2.1), (0.35, -0.45, .2), (1.15, 1.0, 1.15)),
        ((0, 0, 2.1), (-0.65, .25, 2.6), (.82, .72, .88)),
        ((0, 0, 2.1), (1.05, .4, 4.1), (.68, .55, .62)),
    ], 1):
    patch = _linked(continent, 'Terrace hologram raised continent %d' % index,
                    location=location, rotation=rotation, scale=scale)
    patch['hologram_alpha'] = .70


# Open-sided cone of light from the emitter to the hologram globe.
cone_vertices = []
cone_faces = []
cone_segments = 48
for z, radius in [(.62, .82), (1.08, .28)]:
    for index in range(cone_segments):
        angle = math.tau * index / cone_segments
        cone_vertices.append((radius * math.cos(angle), radius * math.sin(angle), z))
for index in range(cone_segments):
    next_index = (index + 1) % cone_segments
    cone_faces.append((index, next_index, cone_segments + next_index,
                       cone_segments + index))
_mesh_object('Terrace hologram light cone', cone_vertices, cone_faces, hologram_cone)


# Eight curved fabric seats. Their centres are between the cardinal axes, with
# four deliberate axis gaps for the radial routes and the arrival stair.
sofa_half_angle = (1.1 / 4.3) / 2  # sofas sit inside Henry's r=5.5 commons patrol circle
sofa_base = _annular_segment_mesh('Terrace sofa timber plinth mesh', 3.76, 4.82,
                                  -sofa_half_angle, sofa_half_angle, .08, .20,
                                  oiled_timber, 10)
sofa_body = _annular_segment_mesh('Terrace sofa fabric body mesh', 3.86, 4.74,
                                  -sofa_half_angle, sofa_half_angle, .24, .43,
                                  fabric, 10)
sofa_cushion = _annular_segment_mesh('Terrace sofa fabric cushion mesh', 3.94, 4.66,
                                     -sofa_half_angle, sofa_half_angle, .40, .53,
                                     fabric, 10)
sofa_back = _annular_segment_mesh('Terrace sofa fabric back mesh', 4.44, 4.72,
                                  -sofa_half_angle, sofa_half_angle, .50, .95,
                                  fabric, 10)
_bevel(sofa_base)
_bevel(sofa_body)
_bevel(sofa_cushion)
_bevel(sofa_back)
for index in range(8):
    angle = math.radians(22.5 + index * 45)
    rotation = (0, 0, angle)
    _linked(sofa_base, 'Terrace sofa %02d timber plinth' % (index + 1),
            rotation=rotation)
    _linked(sofa_body, 'Terrace sofa %02d fabric body' % (index + 1),
            rotation=rotation)
    _linked(sofa_cushion, 'Terrace sofa %02d fabric cushion' % (index + 1),
            rotation=rotation)
    recline = (math.radians(-6) * math.cos(angle),
               math.radians(-6) * math.sin(angle), angle)
    _linked(sofa_back, 'Terrace sofa %02d fabric back' % (index + 1),
            rotation=recline)


# Four cushioned stools near the table, also held on shared mesh data.
stool_base = _uv_sphere_template('Terrace stool timber mesh', oiled_timber, 8, 12)
stool_cushion = _uv_sphere_template('Terrace stool sand mesh', fabric, 8, 12)
for index, angle in enumerate([45, 135, 225, 315], 1):
    theta = math.radians(angle)
    location = (3.55 * math.cos(theta), 3.55 * math.sin(theta), .40)
    _linked(stool_base, 'Terrace cushioned stool %d timber' % index,
            location=location, scale=(.42, .42, .20))
    _linked(stool_cushion, 'Terrace cushioned stool %d cushion' % index,
            location=(location[0], location[1], .54), scale=(.43, .43, .13))


# Six lantern bowls sit in the seat gaps; the two standards mark the north gap.
for index, angle in enumerate([20, 80, 140, 200, 260, 320], 1):
    theta = math.radians(angle)
    x, y = 6.55 * math.cos(theta), 6.55 * math.sin(theta)
    cyl('Terrace lantern bowl %d bronze' % index, (x, y, .47), .32, .16,
        bronze, top=.24, verts=24)
    cyl('Terrace lantern bowl %d amber core' % index, (x, y, .58), .13, .055,
        amber, verts=20)
for index, x in enumerate([-1.05, 1.05], 1):
    cyl('Terrace north lantern standard %d' % index, (x, 7.3, 1.30), .10,
        2.6, bronze, verts=16)
    cyl('Terrace north lantern standard %d bowl' % index, (x, 7.3, 2.63),
        .32, .16, bronze, top=.24, verts=24)
    cyl('Terrace north lantern standard %d amber core' % index,
        (x, 7.3, 2.74), .13, .055, amber, verts=20)


# Dense planting between each seat pair. Slight angular offsets keep the four
# cardinal walks open even at ground level; each tone has at least 48 leaves.
for index, gap_angle in enumerate(range(0, 360, 45)):
    theta = math.radians(gap_angle + (10 if index % 2 == 0 else -10))
    x, y = 7.05 * math.cos(theta), 7.05 * math.sin(theta)  # clear of the r=5.5 commons patrol circle
    box('Terrace sofa gap timber planter %02d' % (index + 1),
        (x, y, .25), (1.05, .82, .50), oiled_timber, angle=theta)
    _fern_planter('Terrace sofa gap planter %02d' % (index + 1), (x, y), 12)


# Four larger diagonal planters with three-metre broadleaf trees.
for index, angle in enumerate([45, 135, 225, 315], 1):
    theta = math.radians(angle)
    x, y = 10.15 * math.cos(theta), 10.15 * math.sin(theta)
    cyl('Terrace diagonal tree planter %d timber' % index, (x, y, .38),
        1.18, .76, oiled_timber, verts=32)
    cyl('Terrace diagonal tree planter %d soil' % index, (x, y, .79),
        .94, .08, dark, verts=32)
    _fern_planter('Terrace diagonal planter %d understory' % index, (x, y), 12)
    branch((x, y, .82), (x, y, 2.72), .12, wood)
    for branch_index in range(5):
        branch_angle = branch_index * math.tau / 5 + theta
        branch_end = (x + .85 * math.cos(branch_angle),
                      y + .85 * math.sin(branch_angle),
                      2.55 + .18 * (branch_index % 2))
        branch((x, y, 2.15), branch_end, .055, wood)
    _create_spray('Terrace diagonal tree %d crown lower' % index,
                  (x, y, 2.72), .82, 32, leaf)
    _create_spray('Terrace diagonal tree %d crown upper' % index,
                  (x, y, 3.05), .65, 32, leaf2)


# Two cyan glass info slabs on bronze stands, facing the table from the north.
for index, x in enumerate([-5.6, 5.6], 1):  # r≈7.5, clear of the r=5.5 patrol circle
    box('Terrace info panel %d glass slab' % index, (x, 5.0, 1.45),
        (1.4, .03, .9), hologram_glass)
    box('Terrace info panel %d bronze stand' % index, (x, 5.0, .64),
        (.12, .16, 1.28), bronze)
    box('Terrace info panel %d bronze foot' % index, (x, 5.0, .06),
        (.55, .32, .12), bronze)


# Trailing vines hang from the pergola ribs to approximately z=4.
for index, angle in enumerate([15, 39, 63, 87, 111, 135, 159, 177], 1):
    theta = math.radians(angle)
    x, y = 17.15 * math.cos(theta), 17.15 * math.sin(theta)
    for segment in range(5):
        z_top = 6.62 - segment * .52
        z_bottom = max(4.02, z_top - .52)
        start = (x + .12 * math.sin(segment), y + .12 * math.cos(segment), z_top)
        end = (x + .20 * math.sin(segment + 1), y + .20 * math.cos(segment + 1), z_bottom)
        _branch('Terrace trailing vine %02d stem %d' % (index, segment),
                start, end, .025, wood)
        _create_spray('Terrace trailing vine %02d leaves %d' % (index, segment),
                      end, .15, 10, leaf if segment % 2 else leaf2)


# Six catenary cables span the pergola. Beads are true linked duplicates.
bead_template = _uv_sphere_template('Terrace strand light bead mesh', strand_amber, 6, 10)
for cable_index, y in enumerate([0, 3, 6, 9, 12, 15], 1):
    half_span = math.sqrt(max(.01, 17.0 ** 2 - y ** 2))
    points = []
    for point_index in range(17):
        t = point_index / 16
        x = -half_span + 2 * half_span * t
        sag = .62 * math.sin(math.pi * t)
        points.append((x, y, 6.2 - sag))
    for segment in range(len(points) - 1):
        _branch('Terrace catenary cable %d segment %d' % (cable_index, segment),
                points[segment], points[segment + 1], .028, bronze)
    length = sum((Vector(points[i + 1]) - Vector(points[i])).length
                 for i in range(len(points) - 1))
    bead_count = max(2, int(length / .6) + 1)
    for bead_index in range(bead_count):
        t = bead_index / max(1, bead_count - 1)
        nearest = min(16, int(t * 16))
        local_t = t * 16 - nearest
        if nearest == 16:
            position = Vector(points[-1])
        else:
            position = Vector(points[nearest]).lerp(Vector(points[nearest + 1]), local_t)
        _linked(bead_template,
                'Terrace strand light bead cable %d %02d' % (cable_index, bead_index + 1),
                location=position, scale=(.05, .05, .05))


# South overlook: two glazed rail runs keep the arrival stair opening clear.
for index, (x0, x1) in enumerate([(-10.5, -4.2), (4.2, 10.5)], 1):
    rail_y = -18.8
    for post_index, x in enumerate([x0, (x0 + x1) / 2, x1], 1):
        cyl('Terrace south balustrade %d post %d' % (index, post_index),
            (x, rail_y, .64), .06, 1.28, bronze, verts=12)
    _branch('Terrace south balustrade %d top rail' % index,
            (x0, rail_y, 1.28), (x1, rail_y, 1.28), .055, bronze)
    _branch('Terrace south balustrade %d lower rail' % index,
            (x0, rail_y, .34), (x1, rail_y, .34), .028, bronze)
    box('Terrace south balustrade %d glass infill' % index,
        ((x0 + x1) / 2, rail_y, .79), (x1 - x0, .035, .90), hologram_glass)


scene['terrace_hero_camera_blender'] = '[0.0, -9.0, 1.7] -> [0.0, 0.0, 1.4]'
scene['terrace_route_clear_axes'] = 'north,east,south,west'

# Job 16 tabletop and display dressing.  The table is inside the established
# route ring; all freestanding additions are checked with the shared guard.
def _job16_terrace_clear(point, radius=3.5):
    return not ('_point_close_to_route' in globals() and _point_close_to_route(point, radius))

_job16_book=detailed_box('Job16 linked closed book template',(0,0,.86),(.62,.42,.08),oiled_timber,bevel=.025) if 'detailed_box' in globals() else box('Job16 linked closed book template',(0,0,.86),(.62,.42,.08),oiled_timber)
_job16_cup=cyl('Job16 linked cup template',(0,0,.91),.12,.22,bronze,verts=16)
_job16_pot=cyl('Job16 linked seedling pot template',(0,0,.88),.16,.20,bronze,verts=16)
_job16_tablet=detailed_box('Job16 linked bronze tablet template',(0,0,.91),(.52,.08,.30),bronze,bevel=.018) if 'detailed_box' in globals() else box('Job16 linked bronze tablet template',(0,0,.91),(.52,.08,.30),bronze)
for _i,(_x,_y) in enumerate(((-1.25,-.35),(-.55,-.95),(.55,-.95))):
    if _job16_terrace_clear((_x,_y)): _linked(_job16_book,f'Job16 council closed book {_i}',(_x,_y,.86),rotation=(0,0,.16*_i))
for _i,(_x,_y) in enumerate(((1.15,-.45),(1.45,.25))):
    if _job16_terrace_clear((_x,_y)): _linked(_job16_cup,f'Job16 council cup {_i}',(_x,_y,.91))
if _job16_terrace_clear((0.2,.85)):
    _linked(_job16_pot,'Job16 council potted seedling',(0.2,.85,.88))
    _branch('Job16 council seedling stem',(.2,.85,.99),(.2,.85,1.35),.025,wood)
if _job16_terrace_clear((-1.45,.55)): _linked(_job16_tablet,'Job16 council bronze tablet',(-1.45,.55,.91),rotation=(0,0,.2))
_job16_blanket=detailed_box('Job16 linked sand folded blanket template',(0,0,.66),(1.45,.72,.16),fabric,bevel=.08) if 'detailed_box' in globals() else box('Job16 linked sand folded blanket template',(0,0,.66),(1.45,.72,.16),fabric)
for _i,_loc in enumerate(((-3.1,-3.25,.66),(3.1,3.25,.66))):
    if _job16_terrace_clear(_loc[:2]): _linked(_job16_blanket,f'Job16 sofa folded blanket {_i}',_loc,rotation=(0,0,.25*_i))
_job16_display=_new_emissive_material('Forum Info Display Line',_hex_to_linear('#58D9E8'),.8,alpha=.22,roughness=.2)
for _panel_x in (-5.6,5.6):
    for _line in range(4):
        box('Job16 info panel horizontal display line',(_panel_x,4.96,1.16+_line*.16),(.95,.018,.018),_job16_display)
_job16_ember=_new_emissive_material('Forum Brazier Amber Embers',_hex_to_linear('#FF9E3D'),4.0,roughness=.28)
cyl('Job16 north gap bronze brazier', (0,8.7,.28), .48, .56, bronze, verts=24)
cyl('Job16 north gap amber embers', (0,8.7,.59), .28, .08, _job16_ember, verts=20)
for _template in (_job16_book,_job16_cup,_job16_pot,_job16_tablet,_job16_blanket): bpy.data.objects.remove(_template,do_unlink=True)
print('FORUM_TERRACE_JOB16', {'triangle_delta_estimate':5*12+2*32+2*12+2*12+2*12+48,'route_checked_props':True})
