"""Landform and terrain corrections for Job 12 (P2/P3).

This module edits the base meadow, adds the lagoon and islands, inserts stepped
terraces and cliffs, and replaces rim outcrops with fractured groups.
"""

import json
import math
import random
import bmesh

from mathutils import Vector
from mathutils import noise

random.seed(2311)


def _as_list(v):
    return [float(v[0]), float(v[1]), float(v[2])] if v else [0.0, 0.0, 0.0]


def _route_polylines():
    """Authored walk routes as per-route polylines from patrolRoutes.json
    (Blender x,y from Three.js x,-z). Loaded from the file directly so the
    route-safety checks can never be silently disabled."""
    route_file = ROOT / 'ui/command-center/src/components/world/forum/patrolRoutes.json'
    if not route_file.exists():
        return []
    payload = json.loads(route_file.read_text())
    return [[(float(x), -float(z)) for x, _, z in route] for route in payload.get('routes', {}).values()]


_ROUTE_POLYLINES = _route_polylines()
# Flat list kept for callers that only need waypoints.
_ROUTE_POINTS = [point for polyline in _ROUTE_POLYLINES for point in polyline]


def _point_distance_sq(a, b):
    dx = a[0] - b[0]
    dy = a[1] - b[1]
    return dx * dx + dy * dy


def _segment_distance_sq(p, a, b):
    ax, ay = a; bx, by = b; px, py = p
    dx, dy = bx - ax, by - ay
    length_sq = dx * dx + dy * dy
    if length_sq < 1e-9:
        return _point_distance_sq(p, a)
    u = max(0.0, min(1.0, ((px - ax) * dx + (py - ay) * dy) / length_sq))
    return _point_distance_sq(p, (ax + u * dx, ay + u * dy))


def _point_close_to_route(point, radius):
    """True when point lies within radius of any authored route polyline
    (exact per-route segments, so long legs are never treated as gaps)."""
    limit_sq = radius * radius
    for polyline in _ROUTE_POLYLINES:
        previous = None
        for current in polyline:
            if previous is not None and _segment_distance_sq(point, previous, current) < limit_sq:
                return True
            if _point_distance_sq(point, current) < limit_sq:
                return True
            previous = current
    return False

def _in_island(x, y):
    for center, radius in (((-88.0, 0.0), 18.0),
                          ((88.0, 0.0), 17.0),
                          ((0.0, 92.0), 16.5)):
        if (x - center[0]) ** 2 + (y - center[1]) ** 2 <= radius * radius:
            return True
    return False


def _linked_object(template, name, location=(0.0, 0.0, 0.0), rotation=(0.0, 0.0, 0.0),
                  scale=(1.0, 1.0, 1.0), material=None):
    obj = bpy.data.objects.new(name, template.data)
    scene.collection.objects.link(obj)
    obj.location = Vector(location)
    obj.rotation_euler = rotation
    obj.scale = Vector(scale)
    obj['forum_architecture'] = True
    if material is not None:
        obj.data.materials.clear()
        obj.data.materials.append(material)
    return obj


def _make_island(name, center, radius, top_height):
    cx, cy = center
    cyl(name + ' island foundation', (cx, cy, -1.35), radius, 2.7, stone, top=radius * 0.86, verts=80)
    cyl(name + ' island platform', (cx, cy, top_height - .11), radius * .74, .26, pale, top=radius * .53, verts=96)
    # No raised wall around the island: the radial routes cross its edge at grade.

    # A six metre shallow-water halo separates each destination island from
    # the deep lagoon and makes the island silhouettes legible in profile.
    shallow_island = mat('Forum Lagoon Island Shallow Water', (.14, .48, .43), metal=.02, rough=.2)
    halo = arc_band(name + ' shallow water ring', radius * .72, radius * 1.03,
                    -2.03, .045, shallow_island, segments=64)
    halo.location = (cx, cy, 0)
    for i in range(9):
        gx = cx + math.cos(i * math.tau / 9) * (radius * .6)
        gy = cy + math.sin(i * math.tau / 9) * (radius * .6)
        if _point_close_to_route((gx, gy), 8.0):
            continue  # the radial route crosses this island; origin plus crown radius
        if '_linked_tree' in globals():
            _linked_tree(name + ' linked island grove', (gx, gy, top_height), i % 3, .72 + (i % 3) * .12)
        else:
            leaf_spray(name + ' island grove', (gx, gy, top_height + .45), .9, 19)
    for i in range(16):
        sx = cx + math.cos(i * math.tau / 16) * (radius * .34)
        sy = cy + math.sin(i * math.tau / 16) * (radius * .34)
        if _point_close_to_route((sx, sy), 4.5): continue
        if 'linked_understory' in globals(): linked_understory(name + ' linked island understory', (sx, sy, top_height), .55)
        else: leaf_spray(name + ' island understory', (sx, sy, top_height + .05), .35, 8)
    for i in range(8):
        a = i * math.tau / 8
        tx = cx + math.cos(a) * (radius * .82)
        ty = cy + math.sin(a) * (radius * .82)
        if _point_close_to_route((tx, ty), 3.5):
            continue  # the radial routes cross the island edge here
        box(name + ' quay lip', (tx, ty, top_height + .22), (1.2, .62, .2), bronze, a)
        cyl(name + ' timber quay mooring post', (tx, ty, top_height + .72), .11, 1.0, timber, verts=10)
    # A small, legible pavilion gives each destination island a civic purpose.
    # The pavilion stands 9 m off the radial route that crosses the island.
    _d = math.hypot(cx, cy) or 1.0
    pcx, pcy = cx + (-cy / _d) * 9.0, cy + (cx / _d) * 9.0
    box(name + ' island pavilion floor', (pcx, pcy, top_height + .48), (4.8, 3.4, .18), timber)
    for px in (-2.0, 2.0):
        for py in (-1.25, 1.25):
            cyl(name + ' island pavilion post', (pcx + px, pcy + py, top_height + 1.65), .10, 2.4, timber, verts=8)
    box(name + ' island pavilion roof', (pcx, pcy, top_height + 2.9), (5.4, 4.0, .18), solar)


def _mesh(name, r0, r1, z, depth, material, a0, a1, segments=12):
    verts = []
    step = (a1 - a0) / segments
    for ring in (r0, r1):
        for i in range(segments + 1):
            a = a0 + step * i
            verts.append((ring * math.cos(a), ring * math.sin(a), z))
            verts.append((ring * math.cos(a), ring * math.sin(a), z + depth))
    faces = []
    for i in range(segments):
        i0 = 2 * i
        i1 = 2 * (i + 1)
        i2 = i1 + 2
        i3 = i0 + 2
        faces.append((i0, i1, i2, i3))
        faces.append((i0, i1, i1 + 1, i0 + 1))
        faces.append((i1 + 1, i2 + 1, i2, i1))
        faces.append((i0, i3, i7 := i3 + 1, i0 + 1)) if (i3 + 1) < len(verts) else None
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata(verts, [], faces)
    mesh.update()
    o = bpy.data.objects.new(name, mesh)
    scene.collection.objects.link(o)
    o.data.materials.append(material)
    o['forum_architecture'] = True
    return o


def _make_quay_segment_template():
    template = box('Forum lagoon quay segment template', (0, 0, -0.14),  # deck flush with the z=0 routes
                  (4.8, 3.5, .28), timber)
    template.name = 'Forum lagoon quay segment template'
    return template


def _build_timber_quay(name, start, end):
    start = Vector((start[0], start[1], 0.0))
    end = Vector((end[0], end[1], 0.0))
    span = end - start
    if span.length < 0.01:
        return
    template = _make_quay_segment_template()
    steps = max(1, int(math.ceil(span.length / 4.4)))
    for step in range(steps):
        t0 = step / steps
        t1 = (step + 1) / steps
        center = start + span * (t0 + t1) * 0.5
        segment = _linked_object(
            template,
            f'{name} quay segment {step}',
            location=(center.x, center.y, -.14),  # deck top flush with the z=0 routes
        )
        segment.rotation_euler = (0.0, 0.0, math.atan2(span.y, span.x))
    # Preserve the linked mesh datablock while removing the origin template.
    bpy.data.objects.remove(template, do_unlink=True)


def _shape_lagoon(meadow):
    mesh = meadow.data
    for vertex in mesh.vertices:
        radius = math.hypot(vertex.co.x, vertex.co.y)
        if 62.0 <= radius <= 118.0 and not _in_island(vertex.co.x, vertex.co.y):
            vertex.co.z = -2.2


def _add_lagoon_water():
    """A real water surface sits below the route-safe decks and islands."""
    # Explicit filled annulus: a ring edge alone leaves the cut meadow reading
    # as void in source renders.  This surface is deliberately below all
    # route pads, quays, and island platforms.
    segments = 192
    verts = []
    for r in (62.0, 118.0):
        for i in range(segments):
            a = math.tau * i / segments
            verts.append((r * math.cos(a), r * math.sin(a), -2.05))
    faces = []
    for i in range(segments):
        j = (i + 1) % segments
        # Counter-clockwise from above so the visible water face receives light.
        faces.append((i, segments + i, segments + j, j))
    lagoon_mesh = bpy.data.meshes.new('P2 filled inner lagoon water mesh')
    lagoon_mesh.from_pydata(verts, [], faces); lagoon_mesh.update()
    lagoon = bpy.data.objects.new('P2 filled inner lagoon water', lagoon_mesh)
    scene.collection.objects.link(lagoon); finish(lagoon, lagoon.name, water)
    shallow = mat('Forum Lagoon Shallow Teal', (.10, .42, .39), metal=.03, rough=.20)
    arc_band('P2 lagoon shallow shore band', 61.2, 66.0, -2.08, .045, shallow,
             start=0.0, end=math.tau, segments=192)
    arc_band('P2 lagoon clear teal inner band', 114.5, 118.4, -2.12, .045, shallow,
             start=0.0, end=math.tau, segments=192)
    # Small reed shelves break the perfect annulus at the sheltered coves.
    for index, angle in enumerate((.42, 1.52, 2.48, 3.68, 4.72, 5.56)):
        radius = 72.0 + (index % 3) * 8.0
        x, y = radius * math.cos(angle), radius * math.sin(angle)
        arc_band('P2 lagoon reed shelf', 3.8, 5.8, -2.06, .08, stone,
                 start=angle - .65, end=angle + .65, segments=16).location = (x, y, 0)
        for reed in range(10):
            a = angle - .55 + reed * 1.1 / 9
            branch((x + 4.4 * math.cos(a), y + 4.4 * math.sin(a), -2.0),
                   (x + 4.4 * math.cos(a) + .16, y + 4.4 * math.sin(a) + .12,
                    -.8 + .12 * (reed % 3)), .018, leaf)


def _add_step_terraces():
    for idx, (inner, outer, rise) in enumerate(((40.0, 46.2, 0.0),
                                                (46.2, 53.0, 1.2),
                                                (53.0, 60.2, 2.4))):
        arc_band(f'P2 stepped terrace floor {idx}', inner, outer, rise + .11,
                 .34, pale, segments=168)
        for wall_radius in (inner, outer):
            wall = arc_band(
                f'Workshop coursed stone P2 terrace retaining wall {idx}',
                wall_radius - .18, wall_radius + .18, rise + .42, .7, stone,
                segments=168)
            wall['masonry_role'] = 'Forum White Sandstone Masonry'
        for i in range(18):
            a = .10 + i * math.tau / 18 + idx * .2
            radius = inner + .8 + (i % 4) * 1.05
            px, py = radius * math.cos(a), radius * math.sin(a)
            if _point_close_to_route((px, py), 4.5): continue
            leaf_spray(f'P2 terrace planting {idx}',
                       (px, py, rise + .34), .82, 20)
            if i % 3 == 0:
                branch((px, py, rise + .34),
                       (px + .35 * math.cos(a), py + .35 * math.sin(a), rise + 2.2),
                       .08, timber)


def _add_foothill_band():
    """Low rolling terrain that softens the hard lagoon-to-ridge transition."""
    random.seed(2397)
    angles=range(100,216,4); radii=[118.0+i*3.0 for i in range(7)]
    verts=[]
    def hill(r,a):
        t=max(0.0,min(1.0,(r-118.0)/18.0))
        rolling=(math.sin(math.radians(a*3.1))+math.cos(math.radians(a*7.7)))*1.15
        fine=math.sin(r*.31+math.radians(a)*9.0)*.6
        return max(0.0, (3.0+9.0*t)*(.55+.45*math.sin(math.pi*t)) + rolling + fine)
    for r in radii:
        for a in angles: verts.append((r*math.cos(math.radians(a)),r*math.sin(math.radians(a)),hill(r,a)))
    width=len(angles); faces=[]
    for j in range(len(radii)-1):
        for i in range(width-1):
            q=j*width+i; cx=(verts[q][0]+verts[q+1][0]+verts[q+width+1][0]+verts[q+width][0])*.25; cy=(verts[q][1]+verts[q+1][1]+verts[q+width+1][1]+verts[q+width][1])*.25
            if not _point_close_to_route((cx,cy),4.5): faces.append((q,q+1,q+width+1,q+width))
    mesh=bpy.data.meshes.new('P2 foothill rolling band mesh');mesh.from_pydata(verts,[],faces);mesh.update()
    hill_obj=bpy.data.objects.new('P2 foothill rolling band',mesh);scene.collection.objects.link(hill_obj);finish(hill_obj,hill_obj.name,terrain)
    for j,(r,a) in enumerate(((121,112),(125,136),(130,161),(123,190),(134,207),(127,148))):
        cx,cy=r*math.cos(math.radians(a)),r*math.sin(math.radians(a))
        if _point_close_to_route((cx,cy),4.5): continue
        for k in range(8):
            x=cx+random.uniform(-7,7);y=cy+random.uniform(-6,6)
            if _point_close_to_route((x,y),8.5): continue  # origin plus crown radius
            if '_linked_tree' in globals(): _linked_tree('P2 foothill grove tree', (x,y,hill(r,a)), k%3, .7+random.random()*.35)
            if 'linked_understory' in globals(): linked_understory('P2 foothill grove understory',(x+.5,y-.3,hill(r,a)),.7)


def _add_ridge_and_waterfalls():
    local_mist = globals().get('celestial_mist')
    if local_mist is None:
        local_mist = mat('Celestial waterfall mist', (.16, .42, .46), metal=.02, rough=.34)
    # Remove every previous ridge recipe, including cones and stepped bands.
    for old in [o for o in list(scene.objects) if any(tag in o.name for tag in (
            'P2 fractured stratified mountain ridge', 'P2 ridge peak',
            'P2 ridge foothill', 'Basalt stratum', 'Ridge mossy ledge'))]:
        bpy.data.objects.remove(old, do_unlink=True)
    start_a, end_a = math.radians(100.0), math.radians(215.0)
    nr, na = 41, 116
    verts = []
    peaks = ((100, 58, 18), (119, 72, 22), (137, 61, 16),
             (154, 75, 24), (171, 64, 19), (190, 70, 21), (208, 56, 17))
    def smoothstep(t): return t * t * (3.0 - 2.0 * t)
    def terrain_z(r, a):
        # A broad envelope leaves a meadow-facing foothill and a broken void edge.
        env = smoothstep(max(0.0, min(1.0, (r - 128.0) / 32.0)))
        if r > 164.0: env *= 1.0 - smoothstep((r - 164.0) / 16.0)
        # Peaks blend by a soft maximum so overlapping gaussians never stack
        # into a wall taller than the spires; the ridge crest stays 55-75 m.
        bumps = [h * math.exp(-((math.degrees(a) - pa) / w) ** 2) for pa, h, w in peaks]
        crest = max(bumps) + .18 * (sum(bumps) - max(bumps))
        n = noise.fractal(Vector((r * .045, math.cos(a) * 16.0, math.sin(a) * 16.0)),
                          .85, 2.1, 5)
        fine = noise.noise_vector(Vector((r * .22, a * 38.0, 2.0))).z
        z = env * (crest + max(-1.5, min(1.5, n)) * 11.0 + fine * 2.5)
        z = min(z, 78.0)
        # Two broad water-cut saddles, with a raised 10 m spur below each notch.
        for notch in (125.0, 185.0):
            z -= 24.0 * math.exp(-((math.degrees(a) - notch) / 4.0) ** 2) * env
            spur = math.exp(-((math.degrees(a) - notch) / 3.8) ** 2)
            if r < 150.0: z += spur * max(0.0, (150.0 - r) / 32.0) * 10.0
        return max(-.15, z)
    for j in range(nr):
        r = 128.0 + 52.0 * j / (nr - 1)
        for i in range(na):
            a = start_a + (end_a - start_a) * i / (na - 1)
            verts.append((r * math.cos(a), r * math.sin(a), terrain_z(r, a)))
    faces = []
    for j in range(nr - 1):
        for i in range(na - 1):
            q = j * na + i; faces.append((q, q + 1, q + na + 1, q + na))
    bands = [[] for _ in range(4)]; moss_faces = []
    for face in faces:
        z = sum(verts[k][2] for k in face) / 4.0
        bands[min(3, int(z / 15.0)) if z >= 0 else 0].append(face)
    for face in faces:
        p0, p1, p2 = (Vector(verts[k]) for k in face[:3])
        if (p1 - p0).cross(p2 - p0).normalized().z > math.cos(math.radians(35)):
            moss_faces.append(face)
    for index, selected in enumerate(bands):
        me = bpy.data.meshes.new(f'Basalt stratum {index} ridge mesh'); me.from_pydata(verts, [], selected); me.update()
        ob = bpy.data.objects.new(f'Basalt stratum {index} ridge', me); scene.collection.objects.link(ob); finish(ob, ob.name, strata[index])
        for poly in me.polygons: poly.use_smooth = True
        mod = ob.modifiers.new('Edge split 40 degrees', 'EDGE_SPLIT'); mod.split_angle = math.radians(40)
    me = bpy.data.meshes.new('Ridge mossy ledge mesh'); me.from_pydata(verts, [], moss_faces); me.update()
    mossmat = bpy.data.materials.get('Weathered coastal rock', rockmat)
    moss = bpy.data.objects.new('Ridge mossy ledge', me); scene.collection.objects.link(moss); finish(moss, moss.name, mossmat)
    moss['vertex_colour_variation'] = 'moss/scree on upward faces'
    color = me.color_attributes.new(name='moss_scree', type='BYTE_COLOR', domain='CORNER')
    for datum in color.data: datum.color = (0.18 + random.random() * .12, .28 + random.random() * .18, .08, 1.0)

    def waterfall_sheet(name, angle, top_radius, width, phase):
        radial = Vector((math.cos(angle), math.sin(angle), 0))
        tangent = Vector((-math.sin(angle), math.cos(angle), 0))
        verts = []
        sides = []
        segments = 8
        for side in (-.06, .06):
            line = []
            for i in range(segments + 1):
                t = i / segments
                r = top_radius + (118.0 - top_radius) * t
                z = terrain_z(r, angle) + .4 if t < .98 else -1.95
                sway = math.sin(phase + t * math.tau * 1.7) * .7
                half = width * (.5 - .08 * t) + .08 * math.sin(phase + t * 8)
                point = radial * (r + side) + tangent * sway + Vector((0, 0, z))
                line.append(len(verts)); verts.extend([tuple(point - tangent * half), tuple(point + tangent * half)])
            sides.append(line)
        faces = []
        for i in range(segments):
            j = i + 1
            faces.extend([(sides[0][i], sides[0][j], sides[0][j] + 1, sides[0][i] + 1),
                          (sides[1][i] + 1, sides[1][j] + 1, sides[1][j], sides[1][i])])
        me = bpy.data.meshes.new(name); me.from_pydata(verts, [], faces); me.update()
        ob = bpy.data.objects.new(name, me); scene.collection.objects.link(ob); finish(ob, name, water)
        return ob

    for index, (angle, radius, width) in enumerate(((math.radians(125), 164.0, 8.2),
                                                     (math.radians(185), 163.0, 7.2))):
        # A rocky spur visibly connects each notch to the lagoon shoreline.
        radial = Vector((math.cos(angle), math.sin(angle), 0))
        tangent = Vector((-math.sin(angle), math.cos(angle), 0))
        for spur in range(9):
            rr = radius - spur * 5.75
            p = radial * rr + tangent * ((spur - 4) * 1.25) + Vector((0, 0, terrain_z(rr, angle) + .2))
            cyl(f'P2 waterfall rocky spur {index}', p, 2.2 - spur * .18, 4.0, strata[(spur + index) % 4], top=1.3, verts=7)
        waterfall_sheet(f'P2 mountain waterfall {index}', angle, radius, width, .6 + index * 1.8)
        for stream in (-.34, .0, .34):
            waterfall_sheet(f'P2 mountain waterfall {index} substream {stream}', angle, radius, width * .25, 1.1 + stream * 3)
        inward = Vector((math.cos(angle), math.sin(angle), 0)) * -1.5
        base = Vector((118.0 * math.cos(angle), 118.0 * math.sin(angle), -2.0)) + inward
        cyl(f'P2 mountain waterfall splash mist disc {index}', base, 9.0, .08, local_mist, verts=32)
        cyl(f'P2 mountain waterfall mist bank {index}', base + Vector((0, 0, .08)), 12.0, .04, local_mist, verts=32)
        for mist in range(5):
            p = base + Vector((random.uniform(-2.4, 2.4), random.uniform(-2.4, 2.4), .06 + mist * .05))
            cyl(f'P2 mountain waterfall mist veil {index}', p, 1.3 + mist * .3, .05,
                local_mist, verts=20)
        box(f'P2 waterfall stream into lagoon {index}', (base.x - radial.x * 3, base.y - radial.y * 3, -1.8), (1.2, 9.0, .16), water, math.atan2(radial.y, radial.x))


def _make_outcrop_template():
    # A faceted, split-shoulder block reads as a fractured outcrop after the
    # linked instances are rotated and partly buried into the rim.
    vertices = []
    for z, ring_scale in ((-1.1, .72), (.86, 1.0)):
        for side in range(6):
            a = side * math.tau / 6
            radius = ring_scale * (1.2 + .35 * math.sin(side * 2.3))
            vertices.append((radius * math.cos(a), radius * math.sin(a), z + random.uniform(-.14, .14)))
    vertices.append((0, 0, 1.35))
    faces = []
    for side in range(6):
        nxt = (side + 1) % 6
        faces.append((side, nxt, 6 + nxt, 6 + side))
        faces.append((6 + side, 6 + nxt, 12))
    me = bpy.data.meshes.new('P2 fractured outcrop block')
    me.from_pydata(vertices, [], faces); me.update()
    o = bpy.data.objects.new('P2 outcrop template', me); scene.collection.objects.link(o); finish(o, o.name, rockmat)
    return o


def _make_outcrops():
    for obj in [o for o in scene.objects if o.name.startswith('Eroded coastal outcrop')]:
        bpy.data.objects.remove(obj, do_unlink=True)

    templates = [_make_outcrop_template() for _ in range(3)]
    clusters = 6
    for cluster in range(clusters):
        count = random.randint(4, 9)
        c_angle = random.uniform(math.radians(95.0), math.radians(250.0))
        c_r = random.uniform(126.0, 170.0)
        base = Vector((c_r * math.cos(c_angle), c_r * math.sin(c_angle), -0.2))
        for block in range(count):
            a = random.uniform(c_angle - .9, c_angle + .9)
            r = c_r + random.uniform(-2.0, 2.0)
            pt = Vector((r * math.cos(a), r * math.sin(a), -0.55))
            if _point_close_to_route((pt.x, pt.y), 4.5):
                continue
            _ = (base.x + random.uniform(-.18, .18), base.y + random.uniform(-.18, .18))
            name = f'Eroded coastal outcrop cluster {cluster} block {block}'
            obj = _linked_object(templates[block % len(templates)], name,
                                location=(pt.x, pt.y, pt.z + 0.1),
                                rotation=(0, 0, random.uniform(0, math.tau)),
                                scale=(0.62 + random.uniform(.1, .52),
                                       0.6 + random.uniform(.07, .44),
                                       0.56 + random.uniform(.06, .5)),
                                material=rockmat)
            obj.location.z = -0.45 + random.uniform(-0.34, 0.15)

    for template in templates:
        bpy.data.objects.remove(template, do_unlink=True)


def _island_bridge_network():
    bridges = [
        ('maker', (-55.0, 0.0), (-85.0, 0.0)),
        ('reading', (55.0, 0.0), (85.0, 0.0)),
        ('council', (0.0, 55.0), (0.0, 88.0)),
        ('arrival', (0.0, -55.0), (0.0, -103.0)),
    ]
    for name, start, end in bridges:
        _build_timber_quay('P2 ' + name, start, end)

    # Island decks are flush with the z=0 routes that cross them.
    _make_island('P2 Maker destination island', (-88.0, 0.0), 16.0, -0.02)
    _make_island('P2 Reading destination island', (89.0, 0.0), 18.0, -0.02)
    _make_island('P2 Council destination island', (0.0, 92.0), 17.0, -0.02)
    # Four small low-poly boats stay inside the lagoon and out of all route decks.
    for index, (x, y, angle) in enumerate(((-35, 29, .2), (34, 28, 2.4), (-24, -34, 1.1), (28, -39, 2.9))):
        if _point_close_to_route((x, y), 4.5):
            continue
        box(f'P2 lagoon boat {index} hull', (x, y, -1.68), (4.2, 1.25, .38), timber, angle)
        branch((x, y, -1.48), (x, y, 1.0), .055, timber)
        box(f'P2 lagoon boat {index} sail', (x + math.cos(angle) * .25, y + math.sin(angle) * .25, -.25), (1.9, .05, 2.4), glass, angle)


def _paint_tree_clusters():
    for cluster in range(14):
        r = random.uniform(39.0, 59.0)
        a = random.uniform(0.0, math.tau)
        cx = r * math.cos(a)
        cy = r * math.sin(a)
        if _point_close_to_route((cx, cy), 5.8):
            continue
        count = random.randint(5, 11)
        for k in range(count):
            kx = cx + random.uniform(-8.0, 8.0)
            ky = cy + random.uniform(-8.0, 8.0)
            tz = random.uniform(.8, 2.4)
            if _point_close_to_route((kx, ky), 4.5):
                continue  # groves never stand on an authored walk route
            if '_linked_tree' in globals():
                _linked_tree('P2 grouped linked tree', (kx, ky, -.35), k % 3, .88 + tz * .08)
                linked_understory('P2 grouped linked understory', (kx + .35, ky - .25, .02), .72)
            else:
                branch((kx, ky, -0.35), (kx + random.uniform(-.34, .34), ky + random.uniform(-.34, .34), 2.1 + tz), .12, timber)
                leaf_spray('P2 grouped tree canopy', (kx, ky, 0.65 + tz), .78, 24)
        cyl('P2 planted meadow bed', (cx, cy, -.08), 7.0, .16, clay, verts=24)


if 'Sculpted coastal ground' in bpy.data.objects:
    meadow_obj = bpy.data.objects['Sculpted coastal ground']
    _shape_lagoon(meadow_obj)
    _add_lagoon_water()
    _island_bridge_network()
    _add_step_terraces()
    _paint_tree_clusters()
    meadow_obj.data.update()
    # A subtle path-stable layer keeps the major route circle and approach lines above
    # walk height where they cross the lagoon band.
    for point in [pt for pt in _ROUTE_POINTS if math.hypot(*pt) < 118.0 and math.hypot(*pt) > 62.0]:
        cyl('P2 route terrace' , (point[0], point[1], -.09), 5.2, .18, pale, verts=24)  # flush with the z=0 routes
else:
    print('FORUM_LANDFORM_WARNING', 'Missing Sculpted coastal ground')

_add_ridge_and_waterfalls()
_add_foothill_band()
_make_outcrops()

forum_landform_manifest = {
    'schema': 'permagent.forum.landform.v1',
    'deterministic': True,
    'seed': 2311,
    'lagoonRadiusRange_m': [62.0, 118.0],
    'islands': 3,
    'terraces': 3,
    'outcropClusters': 6,
    'routeAxisPads': ['maker', 'reading', 'council', 'arrival'],
}
print('FORUM_LANDFORM', forum_landform_manifest)
