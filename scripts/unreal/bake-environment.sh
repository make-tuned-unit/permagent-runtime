#!/bin/zsh
# Bake Unreal's sky to cube faces the web world ships with. See bake_environment.py.
set -eu
FORUM_ROOT="${0:A:h:h:h}"
FORUM_ENGINE="${FORUM_ENGINE:-/Users/Shared/Epic Games/UE_5.8}"
export FORUM_APPEARANCE="${1:-day}"
export FORUM_ENV_FACE="${2:-512}"
[[ "$FORUM_APPEARANCE" == day || "$FORUM_APPEARANCE" == night ]]
python3 -c 'import json,sys; json.dump({"status":"pending"},open(sys.argv[1],"w"))' \
  "$FORUM_ROOT/docs/design/solar-forum/unreal-envmap-$FORUM_APPEARANCE.json"
# -RenderOffscreen, never -NullRHI: a null renderer cannot capture a scene.
"$FORUM_ENGINE/Engine/Binaries/Mac/UnrealEditor-Cmd" \
  "$FORUM_ROOT/unreal/SolarForum/SolarForum.uproject" \
  "-ExecutePythonScript=$FORUM_ROOT/scripts/unreal/bake_environment.py" \
  -RenderOffscreen -unattended -nosplash -nosound -stdout \
  > "$FORUM_ROOT/docs/design/solar-forum/unreal-envmap-$FORUM_APPEARANCE.log" 2>&1
python3 -c 'import json,sys; r=json.load(open(sys.argv[1])); assert r.get("status")=="complete", r; print("baked:", r["faces"])' \
  "$FORUM_ROOT/docs/design/solar-forum/unreal-envmap-$FORUM_APPEARANCE.json"
