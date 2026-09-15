"""Distant planted orbital architecture for the Solar Forum.

This module is executed by ``build_solar_forum.py`` after the walkable
landscape has been authored.  It deliberately keeps the landmarks beyond the
geodisc and outside the established routes.  The meshes are original,
deterministic scenery: they establish depth and scale, but do not imply that
the distant terraces are playable.
"""

import math
from mathutils import Vector


_CELESTIAL_START = set(scene.objects)
_CELESTIAL_LANDMARKS = {}
celestial_mist = mat('Celestial waterfall mist', (.16, .42, .46),
                     metal=.02, rough=.34)


def _mesh(name, vertices, faces, material):
    """Create and tag one mesh without invoking an operator."""
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    scene.collection.objects.link(obj)
    return finish(obj, name, material)


def _linked_object(template, name, location=(0.0, 0.0, 0.0),
                  rotation=(0.0, 0.0, 0.0), scale=(1.0, 1.0, 1.0),
                  material=None):
    """Create a linked scenery instance without duplicating mesh data."""
    obj = bpy.data.objects.new(name, template.data)
    scene.collection.objects.link(obj)
    obj.location = Vector(location)
    obj.rotation_euler = Vector(rotation)
    obj.scale = Vector(scale)
    obj['forum_architecture'] = True
    if material is not None:
        obj.data.materials.clear()
        obj.data.materials.append(material)
    return obj


def _vertical_ring(name, center, inner, outer, depth, segments, material):
    """A closed annular solid in the X/Z plane, with real Y thickness."""
    cx, cy, cz = center
    vertices = []
    # Loop order: front inner, front outer, back inner, back outer.
    for y in (cy - depth / 2, cy + depth / 2):
        for radius in (inner, outer):
            for i in range(segments):
                a = math.tau * i / segments
                vertices.append((cx + radius * math.cos(a), y,
                                 cz + radius * math.sin(a)))
    stride = segments
    faces = []
    for i in range(segments):
        j = (i + 1) % segments
        fi, fo = i, stride + i
        bi, bo = 2 * stride + i, 3 * stride + i
        fj, fok = j, stride + j
        bj, bok = 2 * stride + j, 3 * stride + j
        faces.extend([
            (fi, fj, fok, fo),       # front fascia
            (bi, bo, bok, bj),       # rear fascia
            (fo, fok, bok, bo),      # outer structural edge
            (fi, bi, bj, fj),        # inner structural edge
        ])
    return _mesh(name, vertices, faces, material)


def _annular_strip(name, center, r0, r1, a0, a1, y, segments, material):
    """A thin, slightly inset panel or inlay on the front of a vertical ring."""
    cx, cy, cz = center
    vertices = []
    for radius in (r0, r1):
        for i in range(segments + 1):
            a = a0 + (a1 - a0) * i / segments
            vertices.append((cx + radius * math.cos(a), cy + y,
                             cz + radius * math.sin(a)))
    faces = []
    stride = segments + 1
    for i in range(segments):
        faces.append((i, i + 1, stride + i + 1, stride + i))
    return _mesh(name, vertices, faces, material)


def _annular_prism(name, center, r0, r1, a0, a1, y0, y1, segments, material):
    """A closed wedge used for ring planters and projecting service bays."""
    cx, cy, cz = center
    vertices = []
    for y in (y0, y1):
        for radius in (r0, r1):
            for i in range(segments + 1):
                a = a0 + (a1 - a0) * i / segments
                vertices.append((cx + radius * math.cos(a), cy + y,
                                 cz + radius * math.sin(a)))
    stride = segments + 1
    faces = []
    # Index helper for [depth][radial][angle].
    def idx(depth, radial, i):
        return depth * 2 * stride + radial * stride + i
    for i in range(segments):
        j = i + 1
        faces.extend([
            (idx(0, 0, i), idx(0, 0, j), idx(0, 1, j), idx(0, 1, i)),
            (idx(1, 1, i), idx(1, 1, j), idx(1, 0, j), idx(1, 0, i)),
            (idx(0, 0, i), idx(1, 0, i), idx(1, 0, j), idx(0, 0, j)),
            (idx(0, 1, j), idx(1, 1, j), idx(1, 1, i), idx(0, 1, i)),
        ])
    faces.extend([
        (idx(0, 0, 0), idx(0, 1, 0), idx(1, 1, 0), idx(1, 0, 0)),
        (idx(0, 1, segments), idx(0, 0, segments),
         idx(1, 0, segments), idx(1, 1, segments)),
    ])
    return _mesh(name, vertices, faces, material)


def _leaf_cluster(name, center, scale, material):
    """A rooted fan of folded leaves, avoiding opaque faceted spheres."""
    x, y, z = center
    sx, sy, sz = scale
    vertices=[];faces=[]
    for i in range(7):
        a=math.tau*i/7
        radial=Vector((math.cos(a),math.sin(a),0))
        tangent=Vector((-math.sin(a),math.cos(a),0))
        base=Vector((x,y,z))+radial*(.12*sx)+Vector((0,0,(i%3-1)*.12*sz))
        direction=(radial*.62+tangent*((i%2)*.18-.09)+
                   Vector((0,0,.34+.08*(i%3)))).normalized()
        side=Vector((-direction.y,direction.x,0));side.normalize()
        length=.72*max(sx,sy,sz)*(1+.08*(i%3))
        width=.34*max(sx,sy)*(.85+.08*(i%2))
        shoulder=base+direction*length*.62
        fold=base+direction*length*.52+Vector((0,0,.03*sz))
        tip=base+direction*length
        b=len(vertices)
        vertices.extend([base,base+side*width,base-side*width,
                         shoulder+side*width*.85,shoulder-side*width*.85,
                         fold,tip])
        faces.extend([(b,b+1,b+5),(b,b+5,b+2),
                      (b+1,b+3,b+5),(b+5,b+4,b+2),
                      (b+3,b+6,b+5),(b+5,b+6,b+4)])
    return _mesh(name, vertices, faces, material)


def _water_ribbon(name, x, y, z_top, z_bottom, width, material, phase):
    """A broad, broken-edge falling sheet with real thickness."""
    segments = 14
    vertices = []
    thickness = .16
    faces = []
    loops = []
    for side_y in (-thickness / 2, thickness / 2):
        loop = []
        for side_x in (-1, 1):
            line = []
            for i in range(segments + 1):
                t = i / segments
                z = z_top + (z_bottom - z_top) * t
                sway = math.sin(phase + t * math.pi * 2.2) * width * .13
                edge_wave = math.sin(phase * 1.7 + t * math.pi * 3.0)
                half = width * (.50 - .13 * t + .055 * edge_wave)
                line.append(len(vertices))
                vertices.append((x + sway + side_x * half, y + side_y, z))
            loop.append(line)
        loops.append(loop)
    for depth in range(2):
        left, right = loops[depth]
        for i in range(segments):
            j = i + 1
            faces.append((left[i], left[j], right[j], right[i]))
    for i in range(segments):
        j = i + 1
        faces.extend([
            (loops[0][0][i], loops[1][0][i], loops[1][0][j], loops[0][0][j]),
            (loops[0][1][j], loops[1][1][j], loops[1][1][i], loops[0][1][i]),
        ])
    faces.extend([
        (loops[0][0][0], loops[0][1][0], loops[1][1][0], loops[1][0][0]),
        (loops[0][0][-1], loops[1][0][-1], loops[1][1][-1], loops[0][1][-1]),
    ])
    return _mesh(name, vertices, faces, material)


def _ellipse_solid(name, center, top_rx, top_ry, bottom_rx, bottom_ry,
                   top_z, bottom_z, segments, material):
    """Closed floating landform with a faceted underside and a planted top."""
    cx, cy, _ = center
    vertices = [(cx, cy, top_z), (cx, cy, bottom_z)]
    for z, rx, ry in ((top_z, top_rx, top_ry), (bottom_z, bottom_rx, bottom_ry)):
        for i in range(segments):
            a = math.tau * i / segments
            vertices.append((cx + rx * math.cos(a), cy + ry * math.sin(a), z))
    top_start, bottom_start = 2, 2 + segments
    faces = []
    for i in range(segments):
        j = (i + 1) % segments
        faces.extend([
            (0, top_start + i, top_start + j),
            (1, bottom_start + j, bottom_start + i),
            (top_start + i, bottom_start + i, bottom_start + j,
             top_start + j),
        ])
    return _mesh(name, vertices, faces, material)


def _ellipse_slab(name, center, rx, ry, z, thickness, segments, material):
    """Closed shallow elliptical terrace."""
    cx, cy, _ = center
    vertices = [(cx, cy, z - thickness / 2), (cx, cy, z + thickness / 2)]
    for zz in (z - thickness / 2, z + thickness / 2):
        for i in range(segments):
            a = math.tau * i / segments
            vertices.append((cx + rx * math.cos(a), cy + ry * math.sin(a), zz))
    lower, upper = 2, 2 + segments
    faces = []
    for i in range(segments):
        j = (i + 1) % segments
        faces.extend([
            (0, lower + j, lower + i),
            (1, upper + i, upper + j),
            (lower + i, lower + j, upper + j, upper + i),
        ])
    return _mesh(name, vertices, faces, material)


def _ellipse_sector_slab(name, center, rx, ry, z, thickness, a0, a1,
                         segments, material):
    """A closed terrace slice; separated slices keep the garden visible."""
    cx, cy, _ = center
    vertices = [(cx, cy, z - thickness / 2), (cx, cy, z + thickness / 2)]
    for zz in (z - thickness / 2, z + thickness / 2):
        for i in range(segments + 1):
            a = a0 + (a1 - a0) * i / segments
            vertices.append((cx + rx * math.cos(a), cy + ry * math.sin(a), zz))
    lower, upper = 2, 2 + segments + 1
    faces = []
    for i in range(segments):
        j = i + 1
        faces.extend([
            (0, lower + j, lower + i),
            (1, upper + i, upper + j),
            (lower + i, lower + j, upper + j, upper + i),
        ])
    faces.extend([
        (0, lower, upper), (0, upper, 1),
        (0, lower + segments, upper + segments),
        (0, upper + segments, 1),
    ])
    return _mesh(name, vertices, faces, material)


def _low_poly_mound(name, center, radius, height, segments, material, phase):
    """Uneven planted geology with a sloped shoulder instead of a dome."""
    cx, cy, base_z = center
    vertices = [(cx, cy, base_z + height), (cx, cy, base_z)]
    for ring_radius, ring_z in ((radius, base_z),
                                (radius * .62, base_z + height * .78)):
        for i in range(segments):
            a = math.tau * i / segments
            wobble = 1 + .13 * math.sin(a * 3 + phase)
            vertices.append((cx + ring_radius * wobble * math.cos(a),
                             cy + ring_radius * wobble * math.sin(a), ring_z))
    lower, upper = 2, 2 + segments
    faces = []
    for i in range(segments):
        j = (i + 1) % segments
        faces.extend([
            (1, lower + j, lower + i),
            (0, upper + i, upper + j),
            (lower + i, lower + j, upper + j, upper + i),
        ])
    return _mesh(name, vertices, faces, material)


def _ellipse_band(name, center, rx0, ry0, rx1, ry1, z, thickness,
                  segments, material):
    """Closed horizontal elliptical coping around a planted terrace."""
    cx, cy, _ = center
    vertices = []
    for zz in (z - thickness / 2, z + thickness / 2):
        for rx, ry in ((rx0, ry0), (rx1, ry1)):
            for i in range(segments):
                a = math.tau * i / segments
                vertices.append((cx + rx * math.cos(a),
                                 cy + ry * math.sin(a), zz))
    stride = segments
    faces = []
    for i in range(segments):
        j = (i + 1) % segments
        # Bottom, top, inner wall and outer wall.
        faces.extend([
            (i, j, stride + j, stride + i),
            (2 * stride + i, 3 * stride + i, 3 * stride + j,
             2 * stride + j),
            (i, 2 * stride + i, 2 * stride + j, j),
            (stride + j, 3 * stride + j, 3 * stride + i, stride + i),
        ])
    return _mesh(name, vertices, faces, material)


def _habitat(label, center, rx, ry, top_z, phase):
    """Author one distant floating garden with terraces, pavilion and falls."""
    cx, cy, cz = center
    _ellipse_solid(label + ' floating geological body', center, rx, ry,
                   rx * .68, ry * .62, top_z, top_z - 8, 48, dark)
    _ellipse_slab(label + ' planted upper meadow', center, rx - 2, ry - 2,
                  top_z + .26, .52, 48, terrain)
    # Uneven earthworks break the saucer silhouette and establish real
    # planting depth above the floating rock body.
    for i in range(9):
        a = phase + i * math.tau / 9
        px = cx + (rx - 12) * math.cos(a)
        py = cy + (ry - 8) * math.sin(a)
        _low_poly_mound(label + ' planted geological mound',
                        (px, py, top_z + .48),
                        3.2 + (i % 3) * .85, 1.8 + (i % 4) * .7,
                        9, rockmat if i % 3 == 0 else terrain, phase + i)
    # Separate terrace slices leave visible soil, paths and gaps between levels.
    for level, inset in enumerate((0, 4.4, 8.3)):
        z = top_z + .62 + level * 2.35
        terrace_rx = rx - 7 - inset * .24
        terrace_ry = ry - 5 - inset * .18
        for slice_index, (a0, a1) in enumerate(((.15, 1.10),
                                                 (1.32, 2.25),
                                                 (2.52, 3.35),
                                                 (3.62, 4.68),
                                                 (4.88, 5.80))):
            _ellipse_sector_slab(label + ' terrace ' + str(level) + '-' + str(slice_index),
                                 center, terrace_rx, terrace_ry, z, .34,
                                 a0 + phase * .12, a1 + phase * .12, 8,
                                 pale if level else stone)
            _ellipse_sector_slab(label + ' terrace bronze edge', center,
                                 terrace_rx + .10, terrace_ry + .10,
                                 z + .20, .08, a0 + phase * .12,
                                 a1 + phase * .12, 8, bronze)

    # Clustered towers vary in height and footprint so the habitats read as
    # occupied garden districts instead of a single white saucer.
    for i, a in enumerate((.32, 1.18, 2.02, 2.86, 3.74, 4.58, 5.40)):
        px = cx + (rx * .34 + (i % 2) * 2.2) * math.cos(a + phase * .09)
        py = cy + (ry * .34 + (i % 3) * 1.0) * math.sin(a + phase * .09)
        floors = 2 + (i % 3)
        for floor in range(floors):
            z = top_z + 1.0 + floor * 2.25
            width = 5.8 - floor * .65 + (i % 2) * .5
            depth = 4.2 - floor * .42
            box(label + ' clustered garden building', (px, py, z),
                (width, depth, 1.85), stone if floor == 0 else pale,
                a + math.pi / 2)
            box(label + ' building recessed glass',
                (px, py - math.cos(a) * depth * .48, z + .05),
                (width * .68, .06, 1.18), glass, a + math.pi / 2)
        box(label + ' building planted roof', (px, py,
            top_z + 1.0 + floors * 2.25), (width + .3, depth + .3, .18),
            solar, a + math.pi / 2)

    # An inhabited pavilion sits above the garden as a stepped, asymmetric
    # observatory; its smaller roof leaves the garden visible from above.
    pavilion_z = top_z + 5.0
    box(label + ' pavilion foundation', (cx, cy, pavilion_z),
        (14, 10, .5), stone)
    for floor in range(3):
        width = 10.8 - floor * 2.3
        depth = 7.0 - floor * 1.35
        z = pavilion_z + 1.15 + floor * 2.15
        box(label + ' pavilion stepped volume',
            (cx + floor * 1.1, cy - floor * .7, z),
            (width, depth, 1.75), pale, .08 * (floor + 1))
        box(label + ' pavilion front glazing',
            (cx + floor * 1.1, cy - floor * .7 - depth * .5, z),
            (width * .72, .06, 1.16), glass, .08 * (floor + 1))
        for side in (-1, 1):
            branch((cx + floor * 1.1 + side * width * .42,
                    cy - floor * .7 - depth * .47, z - .72),
                   (cx + floor * 1.1 + side * width * .42,
                    cy - floor * .7 - depth * .47, z + .72),
                   .07, bronze)
    box(label + ' pavilion planted roof',
        (cx + 2.2, cy - 1.4, pavilion_z + 7.8), (5.2, 3.6, .22), solar, .12)
    cyl(label + ' pavilion beacon', (cx + 2.2, cy - 1.4, pavilion_z + 8.5),
        1.1, 1.2, cyan, verts=16, top=.78)

    # Planted bays are real projecting wedges with readable 4–8 m trees rooted
    # in their soil rather than tiny decorative dots.
    for i in range(8):
        a = .25 + i * math.tau / 8
        bay_r = rx - 7.6
        px = cx + bay_r * math.cos(a)
        py = cy + bay_r * math.sin(a)
        pz = top_z + 1.1 + (i % 3) * 1.9
        box(label + ' planted bay ' + str(i), (px, py, pz),
            (3.0, 2.0, .55), clay, a + math.pi / 2)
        box(label + ' planted bay soil ' + str(i), (px, py, pz + .3),
            (2.55, 1.55, .12), wood, a + math.pi / 2)
        for j in range(3):
            radial = Vector((math.cos(a), math.sin(a), 0))
            stem = Vector((px, py, pz)) + radial * (j - 1) * .55
            trunk_top = stem + Vector((radial.x * 1.25, radial.y * 1.25,
                                       3.6 + .7 * (j % 2)))
            branch(stem, trunk_top, .13, wood)
            _leaf_cluster(label + ' planted canopy',
                          trunk_top + Vector((radial.x * .45, radial.y * .45, .4)),
                          (2.05, 1.35, 2.15), leaf2 if j % 2 else leaf)
        _leaf_cluster(label + ' planted lower canopy',
                      stem + Vector((radial.x * .8, radial.y * .8, 2.0)),
                      (1.45, 1.05, 1.35), leaf if j % 2 else leaf2)

    # Edge groves hang from real soil pockets around the rock lip, adding a
    # second vegetation layer visible beneath the terraces.
    for i in range(10):
        a = phase + i * math.tau / 10
        root = Vector((cx + (rx - 3.5) * math.cos(a),
                       cy + (ry - 3.0) * math.sin(a), top_z + .35))
        tip = root + Vector((.25 * math.cos(a), .25 * math.sin(a),
                             -2.0 - (i % 3) * .55))
        branch(root, tip, .08, wood)
        _leaf_cluster(label + ' hanging edge grove',
                      tip + Vector((0, 0, .25)), (1.25, .82, 1.35),
                      leaf2 if i % 2 else leaf)

    # Layered falls start from broken terrace lips. Staggered sheets and narrow
    # mist strands make water read as cascading volume rather than turquoise
    # vertical posts in the distant render.
    for i, (offset, width, length) in enumerate(((-.46, 8.5, 58),
                                                  (.02, 6.8, 51),
                                                  (.44, 5.2, 44))):
        x = cx + rx * offset
        y = cy - ry * .34
        box(label + ' waterfall lip', (x, y, top_z - 1.0),
            (width + 1.2, 1.1, .46), stone)
        for layer, (dx, dy, scale, drop) in enumerate(((-1.8, .14, .72, .60),
                                                        (.0, -.02, 1.0, .84),
                                                        (1.65, .18, .55, .48))):
            _water_ribbon(label + ' waterfall sheet', x + dx, y + dy,
                          top_z - 1.15 - layer * 1.4,
                          top_z - length * drop, width * scale, water,
                          phase + i * .9 + layer * .7)
            _water_ribbon(label + ' waterfall mist strand',
                          x + dx + math.sin(phase + layer) * .8,
                          y - .13, top_z - 2.0 - layer * 2.1,
                          top_z - length * (drop + .08),
                          width * .16, celestial_mist,
                          phase + i + layer * 1.3)
    for i, sign in enumerate((-1, 1)):
        x = cx + sign * (rx * .72)
        y = cy - ry * .43
        _water_ribbon(label + ' side waterfall sheet', x, y,
                      top_z - .4, top_z - 35, 2.8, water, phase + 2.4 + i)
        _water_ribbon(label + ' side waterfall mist', x + sign * 1.2, y - .1,
                      top_z - 2.8, top_z - 30, .45, celestial_mist,
                      phase + 3.4 + i)

    _CELESTIAL_LANDMARKS[label] = {
        'center_m': [round(cx, 2), round(cy, 2), round(cz, 2)],
        'top_m': round(top_z + 8.1, 2),
        'playable': False,
        'waterfalls': 5,
    }


# The main orbital garden is deliberately north of the playable geodisc.  Its
# 200 m silhouette reads behind the campus while preserving all ground routes.
RING_CENTER = (0.0, 260.0, 106.0)
RING_INNER, RING_OUTER = 86.0, 101.0
_vertical_ring('Monumental orbital garden ring', RING_CENTER, RING_INNER,
               RING_OUTER, 6.0, 128, pale)
_vertical_ring('Orbital ring shadow structure', RING_CENTER, RING_INNER - .8,
               RING_INNER + .6, 7.0, 128, dark)
_vertical_ring('Orbital continuous service gallery', RING_CENTER,
               RING_INNER + 1.0, RING_INNER + 3.0, 4.4, 128, dark)
_vertical_ring('Orbital outer maintenance gallery', RING_CENTER,
               RING_OUTER + 1.1, RING_OUTER + 2.0, 3.6, 128, bronze)
orbital_panel_width = math.tau / 16 * .94
orbital_panel_template = _annular_strip(
    'Orbital recessed garden panel template', (0.0, 0.0, 0.0),
    RING_INNER + 3.4, RING_OUTER - 1.7, -orbital_panel_width / 2,
    orbital_panel_width / 2, -3.18, 5, stone)
orbital_inlay_template = _annular_strip(
    'Orbital cyan service inlay template', (0.0, 0.0, 0.0),
    RING_INNER + .65, RING_INNER + .9, -orbital_panel_width / 2,
    orbital_panel_width / 2, -3.48, 3, cyan)
for i in range(16):
    a0 = i * math.tau / 16 + .035
    a1 = (i + .96) * math.tau / 16
    mid = (a0 + a1) * .5
    _linked_object(orbital_panel_template, 'Orbital recessed garden panel',
                   location=RING_CENTER, rotation=(0.0, -mid, 0.0), material=stone)
    _linked_object(orbital_inlay_template, 'Orbital cyan service inlay',
                   location=RING_CENTER, rotation=(0.0, -mid, 0.0), material=cyan)
    # Radial bronze ribs bridge the real ring thickness at its front face.
    a = (i + .5) * math.tau / 16
    branch((RING_CENTER[0] + (RING_INNER + .2) * math.cos(a),
            RING_CENTER[1] - 3.28,
            RING_CENTER[2] + (RING_INNER + .2) * math.sin(a)),
           (RING_CENTER[0] + (RING_OUTER - .2) * math.cos(a),
            RING_CENTER[1] - 3.28,
            RING_CENTER[2] + (RING_OUTER - .2) * math.sin(a)),
           .40, bronze)
    # A second tangential member turns the ribs into an articulated gallery
    # rather than a sequence of flat checkerboard panels.
    b0 = (RING_CENTER[0] + (RING_OUTER + 1.45) * math.cos(a0),
          RING_CENTER[1] - 2.0,
          RING_CENTER[2] + (RING_OUTER + 1.45) * math.sin(a0))
    b1 = (RING_CENTER[0] + (RING_OUTER + 1.45) * math.cos(a1),
          RING_CENTER[1] - 2.0,
          RING_CENTER[2] + (RING_OUTER + 1.45) * math.sin(a1))
    branch(b0, b1, .19, bronze)
bpy.data.objects.remove(orbital_panel_template, do_unlink=True)
bpy.data.objects.remove(orbital_inlay_template, do_unlink=True)

# Upper-half planted bays occupy soil troughs on the monumental facade. Trees
# are intentionally monumental at 4–8 m scale so they remain readable in the
# distant shot and break the bare hoop silhouette.
for i in range(15):
    a = .12 + i * (math.pi - .24) / 14
    _annular_prism('Orbital planted bay ' + str(i), RING_CENTER,
                   RING_OUTER - 2.1, RING_OUTER + 1.6, a - .07, a + .07,
                   -3.55, -2.45, 5, clay)
    radius = RING_OUTER + 1.0
    px = RING_CENTER[0] + radius * math.cos(a)
    pz = RING_CENTER[2] + radius * math.sin(a)
    radial = Vector((math.cos(a), 0, math.sin(a)))
    for j in range(2):
        side = -1 if j == 0 else 1
        base = Vector((px, RING_CENTER[1] - 3.55, pz)) + Vector((0, side * .5, 0))
        crown = base + radial * (2.2 + .35 * j) + Vector((0, 0, 3.1 + .55 * j))
        branch(base, crown, .15, wood)
        _leaf_cluster('Orbital garden tree crown',
                      crown + radial * .45, (2.25, 1.45, 2.15),
                      leaf2 if j else leaf)
        _leaf_cluster('Orbital garden tree lower crown',
                      base + radial * 1.15 + Vector((0, 0, 1.65)),
                      (1.65, 1.05, 1.45), leaf if j else leaf2)

# Three monumental pylons and suspended brace arms make the ring read as an
# engineered habitat rather than a decorative torus.
for i, x in enumerate((-52.0, 0.0, 52.0)):
    base_z = RING_CENTER[2] - 82
    cyl('Orbital support pylon', (x, RING_CENTER[1] + 3.2, base_z + 48),
        2.2 if i == 1 else 1.7, 96, dark, top=1.25, verts=16)
    ring('Orbital support collar', (x, RING_CENTER[1] + 3.2, base_z + 88),
         2.9 if i == 1 else 2.3, .16, bronze)
    cyl('Orbital pylon light', (x, RING_CENTER[1] + 1.45, base_z + 48),
        .22, 82, cyan, top=.12, verts=10)
    if i != 1:
        branch((x, RING_CENTER[1] - 3.0, base_z + 83),
               (0, RING_CENTER[1] - 3.0, RING_CENTER[2] + 4), .38, bronze)

_CELESTIAL_LANDMARKS['orbital_ring'] = {
    'center_m': list(RING_CENTER),
    'outer_radius_m': RING_OUTER,
    'inner_radius_m': RING_INNER,
    'bounds_hint_m': [[-RING_OUTER, RING_CENTER[1] - 4, RING_CENTER[2] - RING_OUTER],
                      [RING_OUTER, RING_CENTER[1] + 4, RING_CENTER[2] + RING_OUTER]],
    'playable': False,
}

# A second orbit is tilted away from the first so the sky reads as inhabited
# depth rather than one flat hoop.  Its local X/Z ring geometry is transformed
# as a single rigid assembly; all ring terraces and towers stay outside the
# walkable geodisc.
RING2_CENTER = (0.0, 338.0, 158.0)
RING2_TILT = math.radians(28.0)
RING2_INNER, RING2_OUTER = 108.0, 126.0


def _tilted_ring(name, center, inner, outer, depth, segments, material, tilt):
    obj = _vertical_ring(name, (0.0, 0.0, 0.0), inner, outer, depth, segments, material)
    obj.location = center
    obj.rotation_euler.x = tilt
    return obj


def _tilted_strip(name, center, r0, r1, a0, a1, y, segments, material, tilt):
    obj = _annular_strip(name, (0.0, 0.0, 0.0), r0, r1, a0, a1, y, segments, material)
    obj.location = center
    obj.rotation_euler.x = tilt
    return obj


def _tilted_prism(name, center, r0, r1, a0, a1, y0, y1, segments, material, tilt):
    obj = _annular_prism(name, (0.0, 0.0, 0.0), r0, r1, a0, a1, y0, y1, segments, material)
    obj.location = center
    obj.rotation_euler.x = tilt
    return obj


def _tilted_point(center, tilt, x, y, z):
    c, s = math.cos(tilt), math.sin(tilt)
    return (center[0] + x, center[1] + y * c - z * s,
            center[2] + y * s + z * c)


def _ring_tower_groups(prefix, center, inner, outer, tilt, angles):
    template = cyl(prefix + ' tower silhouette template', (0, 0, 1.5), .9, 3.0,
                   dark, top=.58, verts=8)
    for group, angle in enumerate(angles):
        radial = outer - 2.4
        for tower, (side, height, scale) in enumerate(((-2.2, 13.0, .82),
                                                        (0.0, 20.0, 1.0),
                                                        (2.2, 16.0, .9))):
            point = _tilted_point(center, tilt,
                                  radial * math.cos(angle) - side * math.sin(angle),
                                  -3.7,
                                  radial * math.sin(angle) + side * math.cos(angle))
            obj = _linked_object(template, f'{prefix} tower group {group} tower {tower}',
                                 location=(point[0], point[1], point[2] + height * .5),
                                 material=dark)
            obj.scale = (scale, scale, height / 3.0)
            ring(f'{prefix} tower crown {group} {tower}',
                 (point[0], point[1], point[2] + height), .72 * scale, .06, cyan)
    bpy.data.objects.remove(template, do_unlink=True)


_tilted_ring('Second tilted orbital garden ring', RING2_CENTER, RING2_INNER,
             RING2_OUTER, 7.0, 128, pale, RING2_TILT)
_tilted_ring('Second orbital ring shadow structure', RING2_CENTER,
             RING2_INNER - .9, RING2_INNER + .7, 8.0, 128, dark, RING2_TILT)
_tilted_ring('Second orbital ring service gallery', RING2_CENTER,
             RING2_INNER + 1.2, RING2_INNER + 3.4, 4.6, 128, dark, RING2_TILT)
ring2_panel_width = math.tau / 18 * .90
ring2_panel_template = _annular_strip(
    'Second orbital planted terrace template', (0.0, 0.0, 0.0),
    RING2_INNER + 3.4, RING2_OUTER - 1.6, -ring2_panel_width / 2,
    ring2_panel_width / 2, -3.58, 5, stone)
ring2_soil_template = _annular_prism(
    'Second orbital terrace soil bay template', (0.0, 0.0, 0.0),
    RING2_OUTER - 2.0, RING2_OUTER + 1.7, -ring2_panel_width / 2,
    ring2_panel_width / 2, -3.72, -2.65, 5, clay)
for i in range(18):
    a0 = i * math.tau / 18 + .025
    a1 = (i + .92) * math.tau / 18
    mid = (a0 + a1) * .5
    _linked_object(ring2_panel_template, 'Second orbital planted terrace',
                   location=RING2_CENTER,
                   rotation=(RING2_TILT, -mid, 0.0), material=stone)
    _linked_object(ring2_soil_template, 'Second orbital terrace soil bay',
                   location=RING2_CENTER,
                   rotation=(RING2_TILT, -mid, 0.0), material=clay)
    for side in (-1, 1):
        a = (a0 + a1) * .5
        local = _tilted_point(RING2_CENTER, RING2_TILT,
                              (RING2_OUTER + .6) * math.cos(a) + side * 1.0 * math.sin(a),
                              -3.65,
                              (RING2_OUTER + .6) * math.sin(a) - side * 1.0 * math.cos(a))
        branch(local, (local[0], local[1], local[2] + 4.2), .12, wood)
        _leaf_cluster('Second orbital garden tree crown',
                      (local[0], local[1], local[2] + 4.6), (1.8, 1.2, 1.8),
                      leaf2 if side > 0 else leaf)
bpy.data.objects.remove(ring2_panel_template, do_unlink=True)
bpy.data.objects.remove(ring2_soil_template, do_unlink=True)
_ring_tower_groups('First orbital', RING_CENTER, RING_INNER, RING_OUTER,
                   0.0, (.22, 1.05, 2.0, 2.95, 4.1, 5.15))
_ring_tower_groups('Second orbital', RING2_CENTER, RING2_INNER, RING2_OUTER,
                   RING2_TILT, (.15, .95, 1.82, 2.72, 3.85, 4.92))

_CELESTIAL_LANDMARKS['second_orbital_ring'] = {
    'center_m': list(RING2_CENTER),
    'outer_radius_m': RING2_OUTER,
    'inner_radius_m': RING2_INNER,
    'tilt_degrees': 28.0,
    'playable': False,
}


def _compact_habitat(label, center, rx, ry, top_z, phase):
    """A smaller floating island with a lagoon, towers and one main fall."""
    cx, cy, cz = center
    _ellipse_solid(label + ' floating island body', center, rx, ry, rx * .66, ry * .60,
                   top_z, top_z - 7.0, 40, dark)
    _ellipse_slab(label + ' planted meadow', center, rx - 1.8, ry - 1.8,
                  top_z + .25, .48, 40, terrain)
    _ellipse_slab(label + ' floating lagoon', (cx, cy, top_z + .02), rx * .42,
                  ry * .30, top_z + .52, .10, 32, water)
    tower_template = box(label + ' tower template', (0, 0, .8), (1.5, 1.5, 1.6), stone)
    for index, a in enumerate((phase, phase + 1.8, phase + 3.6, phase + 5.2)):
        px = cx + (rx * .25) * math.cos(a)
        py = cy + (ry * .25) * math.sin(a)
        height = 7.0 + (index % 3) * 2.5
        tower = _linked_object(tower_template, f'{label} tower {index}',
                               location=(px, py, top_z + height * .5), material=stone)
        tower.scale = (1.0 - index * .08, 1.0 - index * .08, height / 1.6)
        leaf_spray(label + ' tower garden', (px, py, top_z + height + .25), .7, 14)
    bpy.data.objects.remove(tower_template, do_unlink=True)
    fall_x, fall_y = cx, cy - ry * .82
    for index, width in enumerate((5.2, 3.4)):
        _water_ribbon(label + ' waterfall sheet ' + str(index), fall_x + index * 1.4,
                      fall_y, top_z - .2, top_z - 28.0 - index * 5.0, width,
                      water, phase + index)
    cyl(label + ' waterfall mist', (fall_x, fall_y, top_z - 29.0), 3.4, .08,
        celestial_mist, verts=24)
    _CELESTIAL_LANDMARKS[label] = {
        'center_m': [round(cx, 2), round(cy, 2), round(cz, 2)],
        'top_m': round(top_z + .52, 2), 'playable': False, 'waterfalls': 1,
        'lagoon': True,
    }

_habitat('Northwest floating garden', (-148.0, 204.0, 92.0), 39.0, 27.0,
         92.0, .4)
_habitat('Northeast floating garden', (148.0, 211.0, 98.0), 39.0, 27.0,
         98.0, 1.7)
_compact_habitat('Northwest lower floating garden', (-78.0, 304.0, 132.0),
                 28.0, 20.0, 132.0, .8)
_compact_habitat('Northeast lower floating garden', (86.0, 322.0, 145.0),
                 28.0, 20.0, 145.0, 2.1)

# Job 21 optional sky dressing: three static low-poly skiffs over the lagoon.
# They use linked hull and fin meshes and sit well above the playable disc.
_job21_skiff = _ellipse_solid('Job21 distant skiff hull template', (0, 0, 0),
                              4.8, 1.35, 3.6, .85, .25, -.25, 12, dark)
_job21_skiff.name = 'Job21 distant skiff hull template'
_job21_fin = box('Job21 distant skiff fin template', (0, 0, .65), (.12, 2.2, 1.35), cyan)
for _index, (_x, _y, _z, _angle) in enumerate(((-46.0, 48.0, 76.0, .35), (31.0, 88.0, 102.0, 2.4), (74.0, 24.0, 118.0, 2.9))):
    _job16_skiff = _linked_object(_job21_skiff, f'Job21 distant airship skiff {_index}',
                                  (_x, _y, _z), rotation=(0, 0, _angle), scale=(1, 1, 1), material=dark)
    _job16_skiff['animated'] = False
    _linked_object(_job21_fin, f'Job21 distant airship skiff fin {_index}',
                   (_x, _y, _z + .55), rotation=(0, 0, _angle), scale=(1, 1, 1), material=cyan)
bpy.data.objects.remove(_job21_skiff, do_unlink=True)
bpy.data.objects.remove(_job21_fin, do_unlink=True)
celestial_manifest_job21 = {'airships': 3, 'linked_meshes': True, 'triangle_delta_estimate': 3 * 12 * 4}
print('FORUM_CELESTIAL_JOB21', celestial_manifest_job21)

# Publish a compact manifest for root integration and review.  Bounds include
# only objects created by this module, so unrelated scene edits are excluded.
bpy.context.view_layer.update()
_celestial_objects = [obj for obj in scene.objects
                       if obj not in _CELESTIAL_START and obj.type == 'MESH'
                       and obj.get('forum_architecture')]
_all_vertices = []
_triangles = 0
for obj in _celestial_objects:
    _triangles += sum(max(0, len(poly.vertices) - 2) for poly in obj.data.polygons)
    _all_vertices.extend(obj.matrix_world @ Vector(corner) for corner in obj.bound_box)
if _all_vertices:
    _mins = [min(point[axis] for point in _all_vertices) for axis in range(3)]
    _maxs = [max(point[axis] for point in _all_vertices) for axis in range(3)]
else:
    _mins = _maxs = [0.0, 0.0, 0.0]

celestial_manifest = {
    'schema': 'permagent.forum.celestial.v1',
    'deterministic': True,
    'objects': len(_celestial_objects),
    'mesh_objects': len(_celestial_objects),
    'triangles': _triangles,
    'bounds_m': {
        'min': [round(value, 2) for value in _mins],
        'max': [round(value, 2) for value in _maxs],
    },
    'landmarks': _CELESTIAL_LANDMARKS,
    'playable': False,
    'integration': {
        'execute_after': 'forum_landscape.py',
        'execute_before': 'source save/export',
        'recommended_camera_far_m': 430,
        'recommended_star_sphere_radius_m': 380,
    },
}
assert _triangles < 100000, celestial_manifest
