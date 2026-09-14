"""P4 MESH threshold and court for the Solar Forum north-star pass.

The court is at polar angle 135 degrees and radius 56 metres in Blender's
horizontal X/Y plane.  The ring is a real swept tube with no inner disc.  All
authored gate objects carry a ``Mesh gate`` name prefix so source inspection
and downstream material batching can identify the threshold as one family.
"""

import json
import math
import random

from mathutils import Vector


random.seed(9103)


def _mesh_object(name, vertices, faces, material):
    mesh = bpy.data.meshes.new(name + ' mesh')
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    scene.collection.objects.link(obj)
    return finish(obj, name, material)


def _branch(name, start, end, radius, material):
    start, end = Vector(start), Vector(end)
    obj = cyl(name, (start + end) / 2, radius, (end - start).length,
              material, top=radius * .65, verts=8)
    obj.rotation_euler = (end - start).to_track_quat('Z', 'Y').to_euler()
    return obj


def _rail_segment(name, start, end, height=1.1):
    start, end = Vector(start), Vector(end)
    delta = end - start
    count = max(1, math.ceil(delta.length / 2.5))
    for index in range(count + 1):
        point = start + delta * index / count
        cyl('%s anchored post %d' % (name, index + 1),
            (point.x, point.y, point.z + height * .5), .045, height,
            bronze, verts=8)
        box('%s base shoe %d' % (name, index + 1),
            (point.x, point.y, point.z + .055), (.19, .19, .11), bronze)
    _branch(name + ' top rail', start + Vector((0, 0, height)),
            end + Vector((0, 0, height)), .045, bronze)
    _branch(name + ' lower rail', start + Vector((0, 0, .45)),
            end + Vector((0, 0, .45)), .018, bronze)


def _hex_to_linear(value):
    values = [int(value[i:i + 2], 16) / 255 for i in (1, 3, 5)]
    return tuple(v / 12.92 if v <= .04045 else ((v + .055) / 1.055) ** 2.4
                 for v in values)


def _set_emission(material, color, strength):
    shader = material.node_tree.nodes.get('Principled BSDF')
    if shader is not None:
        if shader.inputs.get('Emission Color') is not None:
            shader.inputs['Emission Color'].default_value = (*color, 1)
        elif shader.inputs.get('Emission') is not None:
            shader.inputs['Emission'].default_value = (*color, 1)
        if shader.inputs.get('Emission Strength') is not None:
            shader.inputs['Emission Strength'].default_value = strength
    return material


def _vertical_torus(name, material, major_radius, tube_radius,
                    major_segments=96, minor_segments=12):
    """A torus in the local tangent/Z plane; local Y is the gate normal."""
    vertices = []
    for major_index in range(major_segments):
        theta = math.tau * major_index / major_segments
        radial = t * math.cos(theta) + Vector((0, 0, 1)) * math.sin(theta)
        for minor_index in range(minor_segments):
            phi = math.tau * minor_index / minor_segments
            ring_radial = radial * math.cos(phi)
            normal_offset = gate_normal * math.sin(phi)
            point = radial * major_radius + ring_radial * tube_radius + normal_offset * tube_radius
            vertices.append(tuple(point))
    faces = []
    for major_index in range(major_segments):
        for minor_index in range(minor_segments):
            a = major_index * minor_segments + minor_index
            b = major_index * minor_segments + (minor_index + 1) % minor_segments
            c = ((major_index + 1) % major_segments) * minor_segments + (minor_index + 1) % minor_segments
            d = ((major_index + 1) % major_segments) * minor_segments + minor_index
            faces.append((a, b, c, d))
    obj = _mesh_object(name, vertices, faces, material)
    obj.location = ring_center
    return obj


def _chevron(name, theta, material, depth_front, depth_back, width=.62, height=.34):
    """Small pentagonal housing placed on the forum-facing ring fascia."""
    radial = t * math.cos(theta) + Vector((0, 0, 1)) * math.sin(theta)
    tangent = -t * math.sin(theta) + Vector((0, 0, 1)) * math.cos(theta)
    anchor = radial * 5.08
    shape = [(-width / 2, -height / 2), (width / 2, -height / 2),
             (width / 2 + .16, 0), (0, height / 2),
             (-width / 2 - .16, 0)]
    vertices = []
    for normal_depth in (depth_front, depth_back):
        for u, v in shape:
            vertices.append(tuple(anchor + tangent * u + radial * v + gate_normal * normal_depth))
    count = len(shape)
    faces = [tuple(reversed(range(count))), tuple(range(count, count * 2))]
    for index in range(count):
        next_index = (index + 1) % count
        faces.append((index, next_index, count + next_index, count + index))
    return _mesh_object(name, vertices, faces, material)


def _exedra_wall(name, material, radius=7.0, wall_thickness=.48,
                  height=5.0, segments=64):
    """Semicircle opening in the outward gate direction ``gate_normal``."""
    # Put the back of the exedra one metre beyond the ring.  Its outer edge
    # reaches the eight-metre walk-through limit without hiding the portal.
    exedra_center = gate_center_xy + gate_normal * 8.0
    vertices = []
    for z in (0, height):
        for radial_radius in (radius - wall_thickness, radius):
            for index in range(segments + 1):
                theta = -math.pi / 2 + math.pi * index / segments
                point = exedra_center + t * (radial_radius * math.sin(theta))
                # The arc bows outward, behind the portal.  The forum-facing
                # opening therefore keeps the full ring and plaque readable.
                point += gate_normal * (radial_radius * math.cos(theta))
                point.z = z
                vertices.append(tuple(point))
    stride = segments + 1

    def idx(layer, radial, index):
        return layer * 2 * stride + radial * stride + index

    faces = []
    for index in range(segments):
        next_index = index + 1
        faces.extend([
            (idx(0, 0, index), idx(0, 0, next_index),
             idx(0, 1, next_index), idx(0, 1, index)),
            (idx(1, 1, index), idx(1, 1, next_index),
             idx(1, 0, next_index), idx(1, 0, index)),
            (idx(0, 0, index), idx(1, 0, index),
             idx(1, 0, next_index), idx(0, 0, next_index)),
            (idx(0, 1, next_index), idx(1, 1, next_index),
             idx(1, 1, index), idx(0, 1, index)),
        ])
    faces.extend([
        (idx(0, 0, 0), idx(0, 1, 0), idx(1, 1, 0), idx(1, 0, 0)),
        (idx(0, 1, segments), idx(0, 0, segments),
         idx(1, 0, segments), idx(1, 1, segments)),
    ])
    return _mesh_object(name, vertices, faces, material)


def _text_object(name, body, location, normal, material, size):
    bpy.ops.object.text_add(location=location,
                             rotation=normal.to_track_quat('Z', 'Y').to_euler())
    obj = bpy.context.object
    obj.name = name
    obj.data.body = body
    obj.data.align_x = 'CENTER'
    obj.data.align_y = 'CENTER'
    obj.data.size = size
    obj.data.extrude = .018
    obj.data.materials.append(material)
    obj['forum_architecture'] = True
    bpy.ops.object.convert(target='MESH')
    obj.name = name
    return obj


# Exact court frame.  ``t`` is left/right across the spur and gate_normal is
# the outward walk direction from the promenade through the portal.
radius = 56.0
angle = math.radians(135.0)
gate_center_xy = Vector((radius * math.cos(angle), radius * math.sin(angle), 0))
gate_normal = Vector((math.cos(angle), math.sin(angle), 0))
t = Vector((-gate_normal.y, gate_normal.x, 0))
gate_center = Vector((gate_center_xy.x, gate_center_xy.y, 4.77))  # inner opening meets the dais top: the walker passes through, the lower tube is set into the dais
ring_center = gate_center.copy()
gate_angle = math.atan2(t.y, t.x)

gate_stone = dark
horizon_blue = _set_emission(
    mat('Mesh gate HorizonBlue Inlay', _hex_to_linear('#5599FF'), .1, .28),
    _hex_to_linear('#5599FF'), .72)
chitin = mat('Mesh gate Chitin stone', (.12, .13, .16), .18, .64)
chitin_channel = mat('Mesh gate Chitin engraved channel', (.18, .19, .22),
                     .12, .54)


# A 6m-wide spur from the existing promenade edge r=38 to the court edge r=47.
bridge_center = gate_normal * 42.5
box('Mesh gate spur bridge stone foundation',
    (bridge_center.x, bridge_center.y, -.09), (6.0, 9.0, .18), stone, gate_angle)
box('Mesh gate spur bridge raised timber deck',
    (bridge_center.x, bridge_center.y, .005), (5.82, 8.85, .05),
    oiled_timber if 'oiled_timber' in globals() else wood, gate_angle)
for side in (-1, 1):
    offset = t * (side * 3.0)
    _rail_segment('Mesh gate spur bridge rail %d' % (1 if side < 0 else 2),
                  tuple(gate_normal * 38 + offset),
                  tuple(gate_normal * 47 + offset), 1.1)

# Continuous 6m-wide zero-datum route pad, including the promenade approach.
for index, r0 in enumerate([36.0 + 2.0 * i for i in range(15)], 1):
    r1 = min(64.0, r0 + 2.2)
    midpoint = gate_normal * ((r0 + r1) * .5)
    box('Mesh gate route pad %02d' % index,
        (midpoint.x, midpoint.y, -.015), (6.0, r1 - r0 + .08, .03),
        stone, gate_angle)

# Open any intersecting terrace/retaining geometry so the route remains a real
# through-way. The cutter is temporary and is removed before export.
# The cutter spans the full walking height: terrace walls and stepped floors
# stand above z=0, so a below-grade cutter used to leave them untouched.
route_cutter = box('Mesh gate route boolean cutter',
                   (gate_normal.x * 50, gate_normal.y * 50, 3.0),
                   (6.4, 28.0, 7.0), stone, gate_angle)
_PROP_WORDS = ('branch', 'canopy', 'hedgerow', 'understory', 'planter',
               'planting', 'layered edge', 'leaf', 'shrub', 'grove')
def _inside_corridor(obj, limit=3):
    hits = 0
    for vertex in obj.data.vertices:
        w = obj.matrix_world @ vertex.co
        if 35.0 < w.dot(gate_normal) < 65.0 and abs(w.dot(t)) < 3.2 and .1 < w.z < 6.5:
            hits += 1
            if hits >= limit:
                return True
    return False
for candidate in list(scene.objects):
    if candidate.type != 'MESH' or candidate == route_cutter:
        continue
    label = candidate.name.lower()
    if label.startswith('mesh gate'):
        continue
    if any(word in label for word in _PROP_WORDS):
        # Vegetation and small props on the approach are removed outright so
        # the Sovereign's walk to the gate is never through a hedge.
        corners = [candidate.matrix_world @ Vector(c) for c in candidate.bound_box]
        if (max(c.dot(gate_normal) for c in corners) > 35.0
                and min(c.dot(gate_normal) for c in corners) < 65.0
                and _inside_corridor(candidate)):
            bpy.data.objects.remove(candidate, do_unlink=True)
        continue
    if 'terrace' not in label and 'retaining' not in label and 'stepped' not in label:
            continue
    # Only pieces whose world bounding box reaches the corridor are cut; the
    # rest are left alone so hundreds of far-away linked duplicates are not
    # touched. Linked duplicates that do intersect get their own mesh copy,
    # because Blender refuses to apply a modifier to multi-user data.
    corners = [candidate.matrix_world @ Vector(c) for c in candidate.bound_box]
    # World-space bounding boxes: a long arc wall crosses the corridor even
    # when none of its eight corners lies near the axis, so test box overlap
    # against the corridor's own box rather than corner distances.
    cx = [c.x for c in corners]; cy = [c.y for c in corners]; cz = [c.z for c in corners]
    corridor = [gate_normal * r + t * side for r in (36.0, 64.0) for side in (-3.2, 3.2)]
    kx = [c.x for c in corridor]; ky = [c.y for c in corridor]
    if (max(cx) < min(kx) or min(cx) > max(kx) or max(cy) < min(ky) or min(cy) > max(ky)
            or min(cz) > 6.0 or max(cz) < -1.0):
        continue
    if candidate.data.users > 1:
        candidate.data = candidate.data.copy()
    modifier = candidate.modifiers.new('Mesh gate route clearance', 'BOOLEAN')
    modifier.operation = 'DIFFERENCE'
    modifier.solver = 'EXACT'
    modifier.object = route_cutter
    try:
        bpy.context.view_layer.objects.active = candidate
        candidate.select_set(True)
        bpy.ops.object.modifier_apply(modifier=modifier.name)
    except RuntimeError:
        pass
    finally:
        candidate.select_set(False)
bpy.data.objects.remove(route_cutter, do_unlink=True)


# Nine-metre stone court with civic horizon-blue inlay and a clear axis.
cyl('Mesh gate court stone plaza', (gate_center_xy.x, gate_center_xy.y, -.08),
    9.0, .16, stone, verts=96)
ring('Mesh gate court horizonBlue civic inlay',
     (gate_center_xy.x, gate_center_xy.y, .018), 7.8, .075, horizon_blue)
box('Mesh gate court horizonBlue axis inlay',
    (gate_center_xy.x, gate_center_xy.y, .026), (.075, 16.0, .025),
    horizon_blue, gate_angle)


# Three-step dais; the ring's outer bottom tangent rests exactly on step three.
for index, (step_radius, top) in enumerate([(3.8, .24), (3.45, .48),
                                             (3.1, .72)], 1):
    cyl('Mesh gate three-step dais %d' % index,
        (gate_center_xy.x, gate_center_xy.y, top / 2), step_radius, top,
        stone if index < 3 else pale, verts=96)


# Monumental portal tube and a narrow horizon-blue channel on its inner face.
_vertical_torus('Mesh gate monumental bronze ring', bronze, 4.6, .55)
_vertical_torus('Mesh gate inner horizonBlue inlaid channel', horizon_blue,
                4.08, .065, major_segments=96, minor_segments=8)


# Nine engraved housings are readable on the forum-facing side of the ring.
for index in range(9):
    theta = math.tau * index / 9
    _chevron('Mesh gate chevron housing %02d' % (index + 1), theta, bronze,
             -.77, -.48)
    _chevron('Mesh gate chevron engraving %02d' % (index + 1), theta,
             gate_stone, -.785, -.775, width=.30, height=.17)


# The floor continues eight metres beyond the ring; the exedra occupies its
# far end and opens outward, away from the forum approach.
floor_center = gate_normal * 4.0
box('Mesh gate antechamber floor',
    (gate_center_xy.x + floor_center.x, gate_center_xy.y + floor_center.y, -.06),
    (14.0, 8.0, .12), stone, gate_angle)
_exedra_wall('Mesh gate dark stone semicircular exedra', gate_stone)

plaque_center = gate_center_xy + gate_normal * 7.55
box('Mesh gate engraved MESH plaque',
    (plaque_center.x, plaque_center.y, 2.28), (4.2, .24, 1.7),
    gate_stone, gate_angle)
_text_object('Mesh gate engraved MESH plaque lettering', 'MESH',
             (plaque_center.x - gate_normal.x * .16,
              plaque_center.y - gate_normal.y * .16, 2.30),
             -gate_normal, bronze, .60)


# Two unlit Chitin sigil steles flank the plaque.
for index, side in enumerate((-1, 1), 1):
    position = plaque_center + t * (side * 3.05)
    box('Mesh gate Chitin sigil stele %d' % index,
        (position.x, position.y, 1.60), (.60, .30, 3.20), chitin, gate_angle)
    for channel_index, z in enumerate((1.16, 1.78, 2.40), 1):
        box('Mesh gate Chitin sigil stele %d unlit channel %d' %
            (index, channel_index),
            (position.x - gate_normal.x * .48,
             position.y - gate_normal.y * .48, z),
            (.075, .035, .34), chitin_channel, gate_angle)


# One austere downlight housing on the antechamber axis.
downlight = gate_center_xy + gate_normal * 4.5
cyl('Mesh gate single downlight housing', (downlight.x, downlight.y, 4.68),
    .42, .26, bronze, verts=32)
cyl('Mesh gate single downlight underside',
    (downlight.x, downlight.y, 4.545), .23, .025, gate_stone, verts=24)


# Low parapet at the cliff edge after the 8m walk-through.
parapet_center = gate_center_xy + gate_normal * 8.15
_rail_segment('Mesh gate low parapet',
              tuple(parapet_center - t * 7.0), tuple(parapet_center + t * 7.0), .86)


floor_bounds_blender = {
    'court_disk': {
        'x': [round(gate_center_xy.x - 9, 6), round(gate_center_xy.x + 9, 6)],
        'y': [round(gate_center_xy.y - 9, 6), round(gate_center_xy.y + 9, 6)],
        'z_top': 0.0,
    },
    'walk_through_axis': {
        'start_r': 38.0, 'court_edge_r': 47.0, 'ring_r': 56.0,
        'past_ring_r': 64.0,
    },
}
scene['mesh_gate_ring_center_blender'] = json.dumps([round(value, 6) for value in ring_center])
scene['mesh_gate_ring_plane_normal_blender'] = json.dumps([round(value, 6) for value in gate_normal])
scene['mesh_gate_walk_direction_blender'] = json.dumps([round(value, 6) for value in gate_normal])
scene['mesh_gate_court_floor_bounds_blender'] = json.dumps(floor_bounds_blender)
scene['mesh_gate_court_polar_angle_degrees'] = 135.0
scene['mesh_gate_court_radius_m'] = 56.0
