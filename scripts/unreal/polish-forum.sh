#!/bin/zsh
set -eu
FORUM_ROOT="${0:A:h:h:h}"
FORUM_ENGINE="${FORUM_ENGINE:-/Users/Shared/Epic Games/UE_5.8}"
MODE="${1:-day}"
case "$MODE" in
  day|night) ;;
  *) print -u2 "usage: $0 [day|night]"; exit 2 ;;
esac
REPORT="$FORUM_ROOT/docs/design/solar-forum/unreal-environment-$MODE.json"
LOG="$FORUM_ROOT/docs/design/solar-forum/unreal-environment-$MODE.log"
python3 -c 'import json,sys; json.dump({"status":"pending","appearance":sys.argv[2]},open(sys.argv[1],"w"))' "$REPORT" "$MODE"
export FORUM_APPEARANCE="$MODE"
"$FORUM_ENGINE/Engine/Binaries/Mac/UnrealEditor-Cmd" "$FORUM_ROOT/unreal/SolarForum/SolarForum.uproject" "-ExecutePythonScript=$FORUM_ROOT/scripts/unreal/polish_forum.py" -unattended -NullRHI -nosplash -nosound -stdout > "$LOG" 2>&1

python3 -c 'import json,sys; r=json.load(open(sys.argv[1])); assert r.get("status")=="complete" and r.get("appearance")==sys.argv[2],r; print(r)' "$REPORT" "$MODE"
