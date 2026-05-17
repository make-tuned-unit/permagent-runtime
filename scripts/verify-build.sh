#!/bin/bash
# verify-build.sh — Run after every rebuild to confirm the installed app
# matches the source. Catches the "rebuilt but stale" failure class.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_BINARY="/Applications/Permagent.app/Contents/MacOS/permagent-app"
DAEMON_URL="http://localhost:3001/api/version"
FAIL=0

echo "=== Build Verification ==="
echo ""

# 1. Daemon version matches source
SOURCE_SHA=$(cd "$REPO_ROOT" && git rev-parse --short=9 HEAD)
RUNNING_SHA=$(curl -sf "$DAEMON_URL" 2>/dev/null \
  | python3 -c "import sys,json; print(json.load(sys.stdin).get('git_sha','UNAVAILABLE'))" 2>/dev/null \
  || echo "DAEMON_NOT_RUNNING")

echo "Source SHA:  $SOURCE_SHA"
echo "Daemon SHA:  $RUNNING_SHA"
if [ "$RUNNING_SHA" = "DAEMON_NOT_RUNNING" ]; then
    echo "-- SKIP: daemon not running"
elif [ "$RUNNING_SHA" != "$SOURCE_SHA" ]; then
    echo "!! FAIL: daemon is running old code"
    FAIL=1
else
    echo "** PASS: daemon matches source"
fi
echo ""

# 2. App binary freshness and embedded frontend
if [ -f "$APP_BINARY" ]; then
    APP_DATE=$(stat -f "%Sm" -t "%Y-%m-%d %H:%M" "$APP_BINARY")
    echo "App binary:  $APP_DATE"

    # Tauri v2 compresses embedded assets, so strings won't find them.
    # Instead, check binary age: warn if older than the most recent
    # HUD-related commit.
    BINARY_EPOCH=$(stat -f "%m" "$APP_BINARY")
    HUD_COMMIT_EPOCH=$(cd "$REPO_ROOT" && git log -1 --format="%ct" -- \
        ui/command-center/src/components/world/HenryHUD.tsx \
        ui/command-center/src/components/world/HudShell.tsx 2>/dev/null || echo "0")

    if [ "$HUD_COMMIT_EPOCH" -gt "$BINARY_EPOCH" ]; then
        echo "!! FAIL: app binary is older than latest UI commit"
        echo "         Last UI change: $(cd "$REPO_ROOT" && git log -1 --format='%ci %s' -- \
            ui/command-center/src/components/world/HenryHUD.tsx \
            ui/command-center/src/components/world/HudShell.tsx 2>/dev/null)"
        FAIL=1
    else
        echo "** PASS: app binary is newer than latest UI commit"
    fi
else
    echo "!! FAIL: app binary not found at $APP_BINARY"
    FAIL=1
fi
echo ""

# 3. Dist base path (should be / for Tauri, not /ui/)
DIST_INDEX="$REPO_ROOT/ui/command-center/dist/index.html"
if [ -f "$DIST_INDEX" ]; then
    if grep -q 'src="/ui/' "$DIST_INDEX"; then
        echo "!! FAIL: dist has base /ui/ — was built without TAURI_ENV_PLATFORM"
        echo "         Run 'npm run tauri build' to fix (sets TAURI_ENV_PLATFORM)"
        FAIL=1
    else
        echo "** PASS: dist has base / (Tauri-compatible)"
    fi
else
    echo "-- SKIP: dist/index.html not found"
fi
echo ""

# 4. Stale daemon processes
DAEMON_COUNT=$(pgrep -f "permagentd agent" 2>/dev/null | wc -l | tr -d ' ')
echo "Daemon PIDs: $DAEMON_COUNT"
if [ "$DAEMON_COUNT" -gt 1 ]; then
    echo "!! WARN: multiple daemon processes — stale daemon likely"
elif [ "$DAEMON_COUNT" -eq 0 ]; then
    echo "-- INFO: no daemon running"
else
    echo "** PASS: single daemon process"
fi
echo ""

# 5. WebKit cache check
WEBKIT_CACHE="$HOME/Library/WebKit/ai.permagent.desktop"
APP_CACHE="$HOME/Library/Caches/ai.permagent.desktop"
CACHE_FILES=0
[ -d "$WEBKIT_CACHE" ] && CACHE_FILES=$((CACHE_FILES + $(find "$WEBKIT_CACHE" -type f 2>/dev/null | wc -l | tr -d ' ')))
[ -d "$APP_CACHE" ] && CACHE_FILES=$((CACHE_FILES + $(find "$APP_CACHE" -type f 2>/dev/null | wc -l | tr -d ' ')))
echo "Cache files: $CACHE_FILES"
if [ "$CACHE_FILES" -gt 50 ]; then
    echo "-- INFO: large WebKit cache — consider clearing after rebuild"
    echo "         rm -rf ~/Library/WebKit/ai.permagent.* ~/Library/Caches/ai.permagent.*"
fi
echo ""

# Summary
echo "=== Result ==="
if [ "$FAIL" -ne 0 ]; then
    echo "!! VERIFICATION FAILED — see errors above"
    exit 1
else
    echo "** ALL CHECKS PASSED"
fi
