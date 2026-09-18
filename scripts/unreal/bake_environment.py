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
show = component.get_editor_property('show_flags')
for flag in ('static_meshes', 'skeletal_meshes', 'instanced_static_meshes', 'landscape'):
    try:
        show.set_editor_property(flag, False)
    except Exception:  # noqa: BLE001 — a flag that this engine build does not expose
        pass

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
        'note': 'Sky only — island geometry excluded so the map is light arriving, not light bounced.',
    },
    'axes': 'three.x = unreal.y, three.y = unreal.z, three.z = -unreal.x',
}
(OUT / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')

status = ROOT / f'docs/design/solar-forum/unreal-envmap-{MODE}.json'
status.write_text(json.dumps({'status': 'complete', **manifest}, indent=2) + '\n')
print('baked environment faces:', written, '->', OUT)
