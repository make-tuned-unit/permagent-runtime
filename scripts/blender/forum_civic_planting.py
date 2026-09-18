"""Replace coarse civic canopy proxies with rooted layered planting.

Executed after the landscape and celestial modules, before source materials and
export.  The old objects are identified by their authored names and by their
low-poly canopy dimensions; pots, soil, trunks, routes and water objects stay
in the scene.
"""

from mathutils import Vector


_CIVIC_START = set(scene.objects)
_REMOVED = []
_REPLACEMENTS = []


def _base_name(name):
    """Return the authored name without Blender's duplicate suffix."""
    return name.rsplit('.', 1)[0] if name.rsplit('.', 1)[-1].isdigit() else name


def _triangle_count(obj):
    return sum(max(0, len(poly.vertices) - 2) for poly in obj.data.polygons)


def _coarse_canopy(obj):
    """Target only the known old faceted canopy groups, never tiny leaves."""
    if obj.type != 'MESH' or not obj.get('forum_architecture'):
        return False
    base = _base_name(obj.name)
    authored_group = (base in {'Olive canopy', 'Gallery foliage',
                               'Promenade living edge'} or
                      base.endswith(' living canopy'))
    if not authored_group:
        return False
    # The small Herb garden (.6 m) and Hanging vine (.28 m) meshes are left in
    # place.  This size gate also prevents future tiny botanical details from
    # being swept into the replacement.
    return max(obj.dimensions) >= .8


def _record_removed(obj):
    _REMOVED.append({
        'name': obj.name,
        'authored_group': _base_name(obj.name),
        'material_names': [slot.material.name for slot in obj.material_slots
                           if slot.material],
        'vertices': len(obj.data.vertices),
        'triangles': _triangle_count(obj),
        'dimensions_m': [round(value, 3) for value in obj.dimensions],
        'location_m': [round(value, 3) for value in obj.location],
    })


def _replace_canopy(obj):
    """Root foliage at the old canopy location while retaining scale cues."""
    base = _base_name(obj.name)
    loc = Vector(obj.location)
    diameter = max(obj.dimensions)
    if base == 'Olive canopy':
        # The authored primary branches end just below these canopies.
        branch((loc.x, loc.y, loc.z - .34), loc, .055, wood)
        leaf_spray('Civic olive layered canopy', loc, diameter * .42, 25)
        leaf_spray('Civic olive understory', loc + Vector((0, 0, -.42)),
                   min(.58, diameter * .24), 12)
        return 'olive layered canopy + anchored understory'
    if base.endswith(' living canopy'):
        # Destination urns already carry a long trunk; this short twig makes
        # the replacement visibly connect to that authored stem.
        branch((loc.x, loc.y, loc.z - .48), loc, .05, wood)
        leaf_spray('Civic destination layered canopy', loc, diameter * .40, 22)
        leaf_spray('Civic destination understory', loc + Vector((0, 0, -.46)),
                   min(.62, diameter * .24), 11)
        return 'destination layered canopy + anchored understory'
    if base == 'Promenade living edge':
        # Keep the replacement inside the existing .7 m planter and clear of
        # the promenade by using a compact stem and two low clumps.
        branch((loc.x, loc.y, .48), (loc.x, loc.y, loc.z - .12), .065, wood)
        leaf_spray('Civic promenade layered edge', loc, min(.66, diameter * .35), 20)
        leaf_spray('Civic promenade understory', (loc.x, loc.y, .55), .34, 10)
        return 'promenade layered edge + planter understory'
    if base == 'Gallery foliage':
        # Gallery planters are small; preserve their authored footprint.
        branch((loc.x, loc.y, loc.z - .3), loc, .04, wood)
        leaf_spray('Civic gallery layered foliage', loc, diameter * .38, 18)
        leaf_spray('Civic gallery understory', loc + Vector((0, 0, -.3)),
                   min(.34, diameter * .22), 9)
        return 'gallery layered foliage + anchored understory'
    return 'unclassified'


# Resolve the old proxy objects before creating replacements.  Iterating a
# snapshot keeps object removal safe and leaves every pot, trunk, route and
# water mesh untouched.
bpy.context.view_layer.update()
for obj in list(scene.objects):
    if not _coarse_canopy(obj):
        continue
    _record_removed(obj)
    replacement = _replace_canopy(obj)
    _REPLACEMENTS.append({'source': obj.name, 'replacement': replacement})
    bpy.data.objects.remove(obj, do_unlink=True)


bpy.context.view_layer.update()
_civic_objects = [obj for obj in scene.objects
                  if obj not in _CIVIC_START and obj.type == 'MESH'
                  and obj.get('forum_architecture')]
_added_triangles = sum(_triangle_count(obj) for obj in _civic_objects)

civic_planting_manifest = {
    'schema': 'permagent.forum.civic-planting.v1',
    'deterministic': True,
    'removed_count': len(_REMOVED),
    'removed_objects': _REMOVED,
    'replacement_groups': _REPLACEMENTS,
    'added_objects': len(_civic_objects),
    'added_triangles': _added_triangles,
    'preserved_groups': ['Garden planter', 'Living soil', 'Herb garden',
                         'Hanging vine', 'Pollinator garden', 'Water',
                         'all routes and civic paving'],
    'playable': False,
}
assert _added_triangles < 100000, civic_planting_manifest
