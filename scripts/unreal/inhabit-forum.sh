#!/bin/zsh
set -eu
FORUM_ROOT="${0:A:h:h:h}"
FORUM_ENGINE="${FORUM_ENGINE:-/Users/Shared/Epic Games/UE_5.8}"
export FORUM_APPEARANCE="${1:-day}"
[[ "$FORUM_APPEARANCE" == day || "$FORUM_APPEARANCE" == night ]]
"$FORUM_ENGINE/Engine/Binaries/Mac/UnrealEditor-Cmd" "$FORUM_ROOT/unreal/SolarForum/SolarForum.uproject" "-ExecutePythonScript=$FORUM_ROOT/scripts/unreal/inhabit_forum.py" -unattended -NullRHI -nosplash -nosound -stdout > "$FORUM_ROOT/docs/design/solar-forum/unreal-inhabit-$FORUM_APPEARANCE.log" 2>&1
python3 -c 'import json,sys; r=json.load(open(sys.argv[1])); assert r.get("status")=="complete",r; print(r)' "$FORUM_ROOT/docs/design/solar-forum/unreal-inhabit-$FORUM_APPEARANCE.json"
