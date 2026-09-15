#!/bin/zsh
set -eu
FORUM_ROOT="${0:A:h:h:h}"
FORUM_ENGINE="${FORUM_ENGINE:-/Users/Shared/Epic Games/UE_5.8}"
export FORUM_APPEARANCE="${1:-day}"
export FORUM_CAPTURE_FILTER="${2:-}"
[[ "$FORUM_APPEARANCE" == day || "$FORUM_APPEARANCE" == night ]]
python3 -c 'import json,sys; json.dump({"status":"pending"},open(sys.argv[1],"w"))' "$FORUM_ROOT/docs/design/solar-forum/unreal-render-$FORUM_APPEARANCE.json"
"$FORUM_ENGINE/Engine/Binaries/Mac/UnrealEditor-Cmd" "$FORUM_ROOT/unreal/SolarForum/SolarForum.uproject" "-ExecutePythonScript=$FORUM_ROOT/scripts/unreal/render_forum.py" -RenderOffscreen -unattended -nosplash -nosound -stdout > "$FORUM_ROOT/docs/design/solar-forum/unreal-render-$FORUM_APPEARANCE.log" 2>&1

python3 -c 'import json,sys; r=json.load(open(sys.argv[1])); assert r.get("status")=="complete",r; print(r)' "$FORUM_ROOT/docs/design/solar-forum/unreal-render-$FORUM_APPEARANCE.json"

# A written PNG can still contain fallback materials after a shader failure.
python3 - "$FORUM_ROOT/docs/design/solar-forum/unreal-render-$FORUM_APPEARANCE.log" <<'PY_CHECK'
from pathlib import Path
import sys
log=Path(sys.argv[1]).read_text(errors='replace')
assert 'Failed to compile Material' not in log, 'Render rejected: a material failed to compile; inspect the render log.'
assert 'Traceback (most recent call last)' not in log, 'Render rejected: Python error; inspect the render log.'
PY_CHECK
