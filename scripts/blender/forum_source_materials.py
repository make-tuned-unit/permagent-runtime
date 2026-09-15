"""Exportable PBR source surfacing for the Solar Forum.

Executed in the globals of ``build_solar_forum.py`` after the celestial
geometry has been authored and before the source save/export section.  The
module uses only the already downloaded CC0 Poly Haven images recorded in
``docs/design/solar-forum/environment-sources.json``.  It adds UVs and
Principled texture nodes to a small set of masonry, geological, rock, timber
and architectural-glass surfaces; the rest of the authored palette remains
intact.

The normal images supplied for the selected rock and sandstone sets are
DirectX (green-channel orientation), so they are deliberately omitted here
instead of being connected with an incorrect tangent convention.  The
existing procedural source bump on the original stone materials remains in
place for surfaces that keep those materials.  Runtime surface detail is
handled separately by the browser material pass.
"""

MAX_TEXTURE_DIMENSION = 1024


def _input(node, *names):
    for name in names:
        socket = node.inputs.get(name)
        if socket is not None:
            return socket
    return None


def _new_principled_material(name, source=None):
    material = source.copy() if source is not None else bpy.data.materials.new(name)
    material.name = name
    material.use_nodes = True
    nodes = material.node_tree.nodes
    links = material.node_tree.links
    nodes.clear()
    output = nodes.new('ShaderNodeOutputMaterial')
    output.name = 'Forum Material Output'
    shader = nodes.new('ShaderNodeBsdfPrincipled')
    shader.name = 'Forum Principled BSDF'
    links.new(shader.outputs['BSDF'], output.inputs['Surface'])
    return material, nodes, links, shader


def _register(material):
    if material not in materials:
        materials.append(material)
    return material


def _load_image(relative_path, non_color=False, max_dimension=MAX_TEXTURE_DIMENSION):
    path = ROOT / relative_path
    if not path.exists():
        raise FileNotFoundError(path)
    image = bpy.data.images.load(str(path), check_existing=True)
    # Keep the authored files untouched while lowering the in-memory export
    # payload. The source builder embeds the current image buffer in GLB.
    width, height = image.size
    longest = max(width, height)
    if longest > max_dimension:
        scale = max_dimension / longest
        image.scale(max(1, round(width * scale)), max(1, round(height * scale)))
    image.colorspace_settings.name = 'Non-Color' if non_color else 'sRGB'
    return image


def _image_node(nodes, image, name):
    node = nodes.new('ShaderNodeTexImage')
    node.name = name
    node.label = name
    node.image = image
    node.extension = 'REPEAT'
    return node


def _uv_layer_for_object(obj, tile_meters):
    """Create a deterministic object-space box projection in metres.

    Each UV unit represents ``tile_meters`` in the object's scaled local
    coordinates. The projection follows the dominant face normal, so it works
    for slabs, blocks, columns and irregular rock without bpy operators.
    """
    mesh = obj.data
    uv_layer = mesh.uv_layers.get('ForumSurfaceUV') or mesh.uv_layers.new(name='ForumSurfaceUV')
    mesh.uv_layers.active = uv_layer
    uv_layer.active_render = True
    sx, sy, sz = (abs(value) or 1.0 for value in obj.scale)
    mesh.update()
    for polygon in mesh.polygons:
        nx, ny, nz = abs(polygon.normal.x), abs(polygon.normal.y), abs(polygon.normal.z)
        if nz >= nx and nz >= ny:
            axes = (0, 1, sx, sy)
        elif ny >= nx:
            axes = (0, 2, sx, sz)
        else:
            axes = (1, 2, sy, sz)
        u_axis, v_axis, u_scale, v_scale = axes
        for loop_index in polygon.loop_indices:
            vertex = mesh.vertices[mesh.loops[loop_index].vertex_index].co
            u = vertex[u_axis] * u_scale / tile_meters
            v = vertex[v_axis] * v_scale / tile_meters
            uv_layer.data[loop_index].uv = (u, v)
    return uv_layer


def _connect_texture_material(material, diffuse_path, roughness_path, tile_meters,
                              base_color=None, metallic=0.0, roughness=.7,
                              texture_max_dimension=MAX_TEXTURE_DIMENSION,
                              tint_strength=0.0):
    source_shader = material.node_tree.nodes.get('Principled BSDF') if material.use_nodes else None
    if base_color is None and tint_strength > 0 and source_shader is not None:
        source_color = _input(source_shader, 'Base Color')
        if source_color is not None:
            base_color = tuple(source_color.default_value[:3])
    material, nodes, links, shader = _new_principled_material(material.name, material)
    if base_color is not None:
        _input(shader, 'Base Color').default_value = (*base_color, 1)
    _input(shader, 'Metallic').default_value = metallic
    _input(shader, 'Roughness').default_value = roughness

    texcoord = nodes.new('ShaderNodeUVMap')
    texcoord.name = 'Forum metre UVs'
    texcoord.uv_map = 'ForumSurfaceUV'
    diffuse = _image_node(nodes, _load_image(diffuse_path, max_dimension=texture_max_dimension), 'Forum diffuse / sRGB')
    links.new(texcoord.outputs['UV'], diffuse.inputs['Vector'])
    if tint_strength > 0 and base_color is not None:
        tint = nodes.new('ShaderNodeMixRGB')
        tint.name = 'Authored geological colour tint'
        tint.blend_type = 'MULTIPLY'
        tint.inputs[0].default_value = tint_strength
        tint.inputs[2].default_value = (*base_color, 1)
        links.new(diffuse.outputs['Color'], tint.inputs[1])
        links.new(tint.outputs['Color'], _input(shader, 'Base Color'))
    else:
        links.new(diffuse.outputs['Color'], _input(shader, 'Base Color'))
    if roughness_path is not None:
        rough_map = _image_node(nodes, _load_image(roughness_path, non_color=True, max_dimension=texture_max_dimension), 'Forum roughness / Non-Color')
        links.new(texcoord.outputs['UV'], rough_map.inputs['Vector'])
        links.new(rough_map.outputs['Color'], _input(shader, 'Roughness'))
    return _register(material), tile_meters


def _glass_material(source):
    material, nodes, links, shader = _new_principled_material('Forum Architectural Glass', source)
    _input(shader, 'Base Color').default_value = (.055, .22, .19, 1)
    _input(shader, 'Metallic').default_value = .02
    _input(shader, 'Roughness').default_value = .14
    transmission = _input(shader, 'Transmission Weight', 'Transmission')
    if transmission is not None:
        transmission.default_value = .55
    ior = _input(shader, 'IOR')
    if ior is not None:
        ior.default_value = 1.45
    coat = _input(shader, 'Coat Weight', 'Clearcoat')
    if coat is not None:
        coat.default_value = .08
    return _register(material)


def _replace_material(obj, replacement):
    if obj.type != 'MESH':
        return False
    changed = False
    for index, current in enumerate(obj.data.materials):
        if current != replacement:
            obj.data.materials[index] = replacement
            changed = True
    if changed:
        return True
    return bool(obj.data.materials)


def _assign(objects, replacement, tile_meters):
    assigned = []
    for obj in objects:
        if _replace_material(obj, replacement):
            _uv_layer_for_object(obj, tile_meters)
            assigned.append(obj.name)
    return assigned


def _mesh_objects(predicate):
    return [obj for obj in scene.objects if obj.type == 'MESH' and predicate(obj)]


def _contains_name(*needles):
    return lambda obj: any(needle in obj.name for needle in needles)


# White sandstone is used for selected structural masonry only. Finely modeled
# civic paving and pale slabs retain their existing material and scale language.
masonry, _ = _connect_texture_material(
    pale.copy(),
    'assets/world/forum/third-party/white_sandstone_blocks_02/white_sandstone_blocks_02_diff_2k.jpg',
    'assets/world/forum/third-party/white_sandstone_blocks_02/white_sandstone_blocks_02_rough_2k.jpg',
    tile_meters=2.8,
    roughness=.74,
)
masonry.name = 'Forum White Sandstone Masonry'
_register(masonry)
masonry_objects = _mesh_objects(_contains_name(
    'Workshop coursed stone',
    'Arrival tower',
    'Archive gardens foundation',
    'Research studios foundation',
    'Civic residences foundation',
    'Learning ateliers foundation',
))
masonry_assigned = _assign(masonry_objects, masonry, 2.8)


# The same mossy-rock set is appropriate for the exposed coastal outcrops and
# remains sparse enough that it does not turn every geological layer green.
rock_source = bpy.data.materials.get('Weathered coastal rock', rockmat)
rock_surface, _ = _connect_texture_material(
    rock_source.copy(),
    'assets/world/forum/third-party/mossy_rock/mossy_rock_diff_2k.jpg',
    'assets/world/forum/third-party/mossy_rock/mossy_rock_rough_2k.jpg',
    tile_meters=3.6,
    roughness=.86,
)
rock_surface.name = 'Forum Mossy Coastal Rock'
_register(rock_surface)
rock_objects = _mesh_objects(_contains_name('Eroded coastal outcrop'))
rock_assigned = _assign(rock_objects, rock_surface, 3.6)


# Geological strata receive a restrained rock texture while retaining their
# four authored base-color bands. Each copy reuses the two decoded images.
strata_assigned = []
strata_materials = []
for source in [material for material in bpy.data.materials if material.name.startswith('Basalt stratum ')]:
    textured, _ = _connect_texture_material(
        source.copy(),
        'assets/world/forum/third-party/rock_09/textures/rock_09_diff_2k.jpg',
        None,
        tile_meters=5.5,
        roughness=.9,
        tint_strength=.32,
    )
    textured.name = 'Forum ' + source.name
    _register(textured)
    strata_materials.append((source, textured))
for source, replacement in strata_materials:
    assigned = _assign(
        _mesh_objects(lambda obj, source=source: bool(obj.data.materials) and obj.data.materials[0] == source),
        replacement,
        5.5,
    )
    strata_assigned.extend(assigned)


# Use the existing timber base tone with a fine directional procedural grain.
# Blender source renders show the grain; the GLTF exporter retains the
# Principled base colour/roughness factors while treating these procedural
# nodes as source-only. Branches and every small foliage stem keep their
# authored flat material so the architecture does not acquire bark everywhere.
timber_source_shader = timber.node_tree.nodes.get('Principled BSDF')
timber_base_color = tuple(_input(timber_source_shader, 'Base Color').default_value[:3])
timber_roughness_value = _input(timber_source_shader, 'Roughness').default_value
textured_timber, timber_nodes, timber_links, timber_shader = _new_principled_material(
    'Forum Oiled Timber Grain', timber.copy())
_input(timber_shader, 'Base Color').default_value = (*timber_base_color, 1)
_input(timber_shader, 'Roughness').default_value = timber_roughness_value
_input(timber_shader, 'Metallic').default_value = 0.0
timber_uv = timber_nodes.new('ShaderNodeUVMap')
timber_uv.name = 'Forum metre UVs'
timber_uv.uv_map = 'ForumSurfaceUV'
timber_wave = timber_nodes.new('ShaderNodeTexWave')
timber_wave.name = 'Fine directional wood grain'
timber_wave.wave_type = 'BANDS'
timber_wave.bands_direction = 'X'
_input(timber_wave, 'Scale').default_value = 7.0
_input(timber_wave, 'Distortion').default_value = 2.2
_input(timber_wave, 'Detail').default_value = 2.0
timber_noise = timber_nodes.new('ShaderNodeTexNoise')
timber_noise.name = 'Subtle grain relief'
_input(timber_noise, 'Scale').default_value = 5.0
_input(timber_noise, 'Detail').default_value = 2.0
timber_ramp = timber_nodes.new('ShaderNodeValToRGB')
timber_ramp.name = 'Warm timber tone from authored base'
timber_ramp.color_ramp.elements[0].color = tuple(max(.0, channel * .58) for channel in timber_base_color) + (1,)
timber_ramp.color_ramp.elements[1].color = tuple(min(1., channel * 1.22 + .006) for channel in timber_base_color) + (1,)
timber_bump = timber_nodes.new('ShaderNodeBump')
timber_bump.name = 'Fine grain source relief'
_input(timber_bump, 'Strength').default_value = .12
_input(timber_bump, 'Distance').default_value = .012
timber_links.new(timber_uv.outputs['UV'], timber_wave.inputs['Vector'])
timber_links.new(timber_uv.outputs['UV'], timber_noise.inputs['Vector'])
timber_links.new(timber_wave.outputs['Fac'], timber_ramp.inputs['Fac'])
timber_links.new(timber_ramp.outputs['Color'], _input(timber_shader, 'Base Color'))
timber_links.new(timber_noise.outputs['Fac'], timber_bump.inputs['Height'])
timber_links.new(timber_bump.outputs['Normal'], _input(timber_shader, 'Normal'))
textured_timber = _register(textured_timber)
_register(textured_timber)
timber_objects = _mesh_objects(_contains_name(
    'Harbor boardwalk',
    'Theatre timber canopy',
    'Workbench top',
    'Commons bench timber slat',
    'Solar shading louvre',
    'Overlook seat slat',
))
timber_assigned = _assign(timber_objects, textured_timber, 1.35)


# Glass uses the Principled transmission/IOR model. Solar photovoltaic glass
# remains on its authored accent material and is intentionally not reassigned.
architectural_glass = _glass_material(glass)
glass_objects = _mesh_objects(_contains_name('Glazed vault panel', 'Recessed atelier glazing'))
glass_assigned = _assign(glass_objects, architectural_glass, 2.4)


forum_source_materials_manifest = {
    'schema': 'permagent.forum.source-materials.v1',
    'textureMaxDimension': MAX_TEXTURE_DIMENSION,
    'normalMaps': 'omitted: supplied sandstone/mossy-rock normals are DirectX; no green-channel conversion was authored',
    'materials': {
        'masonry': {'name': masonry.name, 'tileMeters': 2.8, 'objects': masonry_assigned},
        'coastalRock': {'name': rock_surface.name, 'tileMeters': 3.6, 'objects': rock_assigned},
        'geologicalStrata': {'materials': [textured.name for _, textured in strata_materials], 'tileMeters': 5.5, 'objects': strata_assigned},
        'timber': {'name': textured_timber.name, 'tileMeters': 1.35, 'objects': timber_assigned},
        'architecturalGlass': {'name': architectural_glass.name, 'tileMeters': 2.4, 'objects': glass_assigned},
    },
}
# Job 16 material registry: the geometry modules create these before this
# final surfacing pass; ensure standalone source-material inspection sees the
# complete authored palette without replacing their tuned node values.
job16_material_names = [
    'Forum Harbour Amber', 'Forum Harbour Rope', 'Forum Plunge Pool Dark Water',
    'Forum Island Quay Amber', 'Forum Warm Window Emission',
    'Forum Info Display Line', 'Forum Brazier Amber Embers',
    'Mesh gate cyan display glass', 'Celestial waterfall mist',
]
forum_source_materials_manifest['job16Materials'] = [
    name for name in job16_material_names if bpy.data.materials.get(name) is not None
]
print('FORUM_SOURCE_MATERIALS', {key: len(value['objects']) for key,value in forum_source_materials_manifest['materials'].items()}, flush=True)
