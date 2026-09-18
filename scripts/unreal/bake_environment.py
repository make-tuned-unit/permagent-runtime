"""Bake Unreal's sky into a cube map the web world ships with.

Unreal cannot be the renderer in the product: the world lives inside the app,
and nobody else has an engine installed. So Unreal is used the way film uses a
render farm — offline, on this machine, producing assets that ship.

This is the first and most valuable of those bakes. Unreal's Sky Atmosphere,
sun and volumetric cloud layer produce a sky the browser cannot compute in real
time; captured once as six cube faces, that same sky becomes the environment
map lighting every surface in the three.js world. Water, glass and bronze then
reflect Unreal's sky rather than a flat colour, which is most of the difference
between "coloured plastic" and "a place".

Output: ui/command-center/public/world/env/<appearance>/{px,nx,py,ny,pz,nz}.png
plus a manifest recording what produced them. A few hundred kilobytes, no
engine on the user's machine, no runtime cost beyond one filtered cube map.

Run with a real RHI (never -NullRHI, which cannot render):
  scripts/unreal/bake-environment.sh [day|night]
"""
import json
import os
from pathlib import Path

import unreal as u

ROOT = Path(__file__).resolve().parents[2]
MODE = os.environ.get('FORUM_APPEARANCE', 'day')
assert MODE in ('day', 'night'), MODE
FACE_SIZE = int(os.environ.get('FORUM_ENV_FACE', '512'))
OUT = ROOT / 'ui/command-center/public/world/env' / MODE
OUT.mkdir(parents=True, exist_ok=True)

assert Path(u.Paths.get_project_file_path()).stem == 'SolarForum'


def rotation(pitch=0.0, yaw=0.0, roll=0.0):
    """Unreal Python takes Rotator positionally as roll/pitch/yaw; be explicit."""
    return u.Rotator(pitch=pitch, yaw=yaw, roll=roll)


# Cube faces, named for the THREE.js axis each one feeds, with the Unreal
# camera orientation that looks down that axis.
#
# The two engines disagree about handedness and about which way is up: Unreal is
# Z-up with +X forward, three.js is Y-up with -Z forward. The island itself
# crosses that gap in Blender's exporters (`export_yup=True` for the GLB,
# `axis_up='Z'` for the FBX), so the sky has to cross it the same way or the sun
# would sit in the wrong quarter of the sky from inside the app. The mapping
# used here is three.x = unreal.y, three.y = unreal.z, three.z = -unreal.x.
FACES = [
    ('px', rotation(yaw=90.0)),    # three +X  ← unreal +Y
    ('nx', rotation(yaw=-90.0)),   # three -X  ← unreal -Y
    ('py', rotation(pitch=90.0)),  # three +Y  ← unreal +Z (straight up)
    ('ny', rotation(pitch=-90.0)), # three -Y  ← unreal -Z (straight down)
    ('pz', rotation(yaw=180.0)),   # three +Z  ← unreal -X
    ('nz', rotation(yaw=0.0)),     # three -Z  ← unreal +X (the forward view)
]

level = u.get_editor_subsystem(u.LevelEditorSubsystem)
level_name = '/Game/SolarForum/Maps/ForumNight' if MODE == 'night' else '/Game/SolarForum/Maps/ForumCinematic'
assert level.load_level(level_name), level_name

world = u.get_editor_subsystem(u.UnrealEditorSubsystem).get_editor_world()
actors = u.get_editor_subsystem(u.EditorActorSubsystem)

# Captured from high above the island's centre so the horizon ring is the real
# sky rather than the near terrace the walker happens to stand behind. The
# island's own geometry is excluded: an environment map is the light ARRIVING at
# the island, and letting it see itself doubles its own bounce into a haze.
CAPTURE_HEIGHT = 9000.0
capture = actors.spawn_actor_from_class(u.SceneCapture2D, u.Vector(0, 0, CAPTURE_HEIGHT), rotation())
component = capture.capture_component2d
component.set_editor_property('capture_source', u.SceneCaptureSource.SCS_FINAL_COLOR_LDR)
component.set_editor_property('fov_angle', 90.0)
component.set_editor_property('capture_every_frame', False)
component.set_editor_property('capture_on_movement', False)
# Hiding the island is best-effort, and deliberately so.
#
# Which knob exists here varies by engine build: 5.8's Python exposes neither
# `show_flags` nor a `USE_SHOW_ONLY_LIST` enum member, and discovering that cost
# two failed bakes. So each candidate is tried in turn and the one that worked
# is RECORDED in the manifest, rather than assuming an API and producing a
# silently wrong map.
#
# If none works the bake still runs, and that is an acceptable outcome rather
# than a broken one: the island then appears in the downward face only, which
# reads as ground bounce — real light, slightly overstated — instead of as the
# missing sky the earlier failures produced.
hidden_via = 'none (island visible as ground bounce)'
try:
    mode = getattr(u.SceneCapturePrimitiveRenderMode, 'USE_SHOW_ONLY_LIST', None)
    if mode is None:
        # Some builds spell it without the USE_ prefix.
        mode = getattr(u.SceneCapturePrimitiveRenderMode, 'SHOW_ONLY_LIST', None)
    if mode is not None:
        component.set_editor_property('primitive_render_mode', mode)
        component.set_editor_property('show_only_actors', [])
        hidden_via = 'primitive_render_mode=show-only, empty list'
    else:
        island = [
            a for a in actors.get_all_level_actors()
            if isinstance(a, u.StaticMeshActor)
        ]
        component.set_editor_property('hidden_actors', island)
        hidden_via = f'hidden_actors ({len(island)} static meshes)'
except Exception as error:  # noqa: BLE001 — engine-version differences only
    hidden_via = f'none ({type(error).__name__})'

# Exposure, copied from the proven path in `render_forum.py`.
#
# A scene capture does NOT inherit the level's locked exposure: it runs its own
# auto-exposure, and pointed at a bright sky with no geometry to average against
# it saturates to pure white — which is exactly what the first successful bake
# produced, six blank faces that would have lit the world with flat nothing.
# Pinning min and max brightness to the same value is how this project locks
# exposure elsewhere, and Lumen has to be re-stated because a capture does not
# inherit the level's renderer settings either.
EXPOSURE = 1.5 if MODE == 'night' else 13.0
component.set_editor_property('always_persist_rendering_state', True)
settings = component.get_editor_property('post_process_settings')
capture_exposure = {
    'dynamic_global_illumination_method': u.DynamicGlobalIlluminationMethod.LUMEN,
    'reflection_method': u.ReflectionMethod.LUMEN,
    'auto_exposure_min_brightness': EXPOSURE,
    'auto_exposure_max_brightness': EXPOSURE,
}
if MODE == 'night':
    capture_exposure.update({
        'auto_exposure_method': u.AutoExposureMethod.AEM_HISTOGRAM,
        'auto_exposure_bias': 0.0,  # pinned by the min/max brightness above
    })
for key, value in capture_exposure.items():
    settings.set_editor_property('override_' + key, True)
    settings.set_editor_property(key, value)
component.set_editor_property('post_process_settings', settings)

render_target = u.RenderingLibrary.create_render_target2d(
    world, FACE_SIZE, FACE_SIZE, u.TextureRenderTargetFormat.RTF_RGBA8
)
component.set_editor_property('texture_target', render_target)

written = []
for name, orientation in FACES:
    capture.set_actor_rotation(orientation, False)
    component.capture_scene()
    u.RenderingLibrary.export_render_target(world, render_target, str(OUT), f'{name}.png')
    written.append(name)

actors.destroy_actor(capture)

manifest = {
    'schema': 'permagent.forum.environment.v1',
    'appearance': MODE,
    'faceSize': FACE_SIZE,
    'faces': written,
    'source': {
        'engine': 'Unreal Engine 5.8',
        'level': level_name,
        'captureHeightCm': CAPTURE_HEIGHT,
        'exposureEV100': EXPOSURE,
        'note': 'Sky capture; how the island was excluded is recorded below.',
        'islandHiddenVia': hidden_via,
    },
    'axes': 'three.x = unreal.y, three.y = unreal.z, three.z = -unreal.x',
}
(OUT / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')

status = ROOT / f'docs/design/solar-forum/unreal-envmap-{MODE}.json'
status.write_text(json.dumps({'status': 'complete', **manifest}, indent=2) + '\n')
print('baked environment faces:', written, '->', OUT)
