#!/usr/bin/env bash
#
# redeploy-daemon.sh — canonical Permagent daemon redeploy procedure for local dev
#
# Bakes the deploy lessons learned during PR 1.6.E into a single command.
# References: issue #180, PR #174, PR 1.6.E session notes.
#
# Usage:
#   ./scripts/redeploy-daemon.sh                # full redeploy
#   ./scripts/redeploy-daemon.sh --clean-cache  # also clear WebKit cache
#   ./scripts/redeploy-daemon.sh --skip-build   # skip cargo build, use existing target/release/permagentd
#   ./scripts/redeploy-daemon.sh --help
#
# What this script does:
#   1. Verifies prerequisites (cargo tauri, npm deps, plist exists)
#   2. Builds permagentd release binary (unless --skip-build)
#   3. Copies fresh binary to Tauri externalBin source-of-truth
#   4. Runs `cargo tauri build` to produce signed DMG
#   5. Stops running daemon (app + launchd) and kills stragglers
#   6. Opens the DMG and waits for you to drag-to-Applications
#   7. Bootstraps launchd
#   8. Verifies the daemon is healthy
#
# Why this exists:
#   Each daemon-code iteration requires ~8 manual commands with multiple
#   footguns (Tauri externalBin source-of-truth, code-signing requirements,
#   launchd lifecycle, port collisions). One wrong step → SIGILL or
#   dual-daemon crash loop. This script captures the working sequence.

set -euo pipefail

# ── colors ──────────────────────────────────────────────────────────────────
if [[ -t 1 ]]; then
  C_RED=$'\033[0;31m'
  C_GREEN=$'\033[0;32m'
  C_YELLOW=$'\033[0;33m'
  C_CYAN=$'\033[0;36m'
  C_DIM=$'\033[2m'
  C_RESET=$'\033[0m'
else
  C_RED='' C_GREEN='' C_YELLOW='' C_CYAN='' C_DIM='' C_RESET=''
fi

step()  { echo "${C_CYAN}▶${C_RESET} $*"; }
ok()    { echo "${C_GREEN}✓${C_RESET} $*"; }
warn()  { echo "${C_YELLOW}!${C_RESET} $*"; }
fail()  { echo "${C_RED}✗${C_RESET} $*" >&2; exit 1; }
note()  { echo "  ${C_DIM}$*${C_RESET}"; }

# ── arg parsing ─────────────────────────────────────────────────────────────
CLEAN_CACHE=0
SKIP_BUILD=0

for arg in "$@"; do
  case "$arg" in
    --clean-cache)  CLEAN_CACHE=1 ;;
    --skip-build)   SKIP_BUILD=1 ;;
--help|-h)
      cat <<'HELP'
redeploy-daemon.sh — canonical Permagent daemon redeploy procedure

Usage:
  ./scripts/redeploy-daemon.sh                # full redeploy
  ./scripts/redeploy-daemon.sh --clean-cache  # also clear WebKit cache
  ./scripts/redeploy-daemon.sh --skip-build   # skip cargo build, use existing target/release/permagentd
  ./scripts/redeploy-daemon.sh --help

Phases:
  1. Preflight checks (cargo tauri, npm deps, plist)
  2. Build permagentd release binary (unless --skip-build)
  3. Copy to Tauri externalBin source-of-truth
  4. cargo tauri build → produce signed DMG
  5. Stop running daemon (app + launchd) and kill stragglers
  6. Open DMG, wait for drag-to-Applications
  7. Bootstrap launchd
  8. Verify daemon healthy (PID, port 3001, GET /api/dashboard/layout)

See issue #180 and PR 1.6.E session notes for context.
HELP
      exit 0
      ;;
    *) fail "unknown arg: $arg (try --help)" ;;
  esac
done

# ── paths ───────────────────────────────────────────────────────────────────
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

DAEMON_BIN_SRC="$REPO_ROOT/target/release/permagentd"
TAURI_EXTERNAL_BIN_DIR="$REPO_ROOT/ui/desktop/src-tauri/binaries"
TAURI_EXTERNAL_BIN="$TAURI_EXTERNAL_BIN_DIR/permagentd-aarch64-apple-darwin"
DMG_DIR="$REPO_ROOT/ui/desktop/src-tauri/target/release/bundle/dmg"
APP_INSTALLED="/Applications/Permagent.app"
PLIST="$HOME/Library/LaunchAgents/ai.permagent.daemon.plist"
LAUNCHD_LABEL="ai.permagent.daemon"
DAEMON_TOKEN_FILE="$HOME/.permagent/secrets/daemon_token.json"
DAEMON_LOG="$HOME/.permagent/logs/daemon.err"
WEBKIT_CACHE_DIRS=(
  "$HOME/Library/Caches/ai.permagent.app"
  "$HOME/Library/WebKit/ai.permagent.app"
)

# ── preflight ───────────────────────────────────────────────────────────────
step "Preflight checks"

[[ -d "$REPO_ROOT/crates/goose-server" ]] || fail "not in permagent-runtime repo root (expected crates/goose-server/)"
note "repo: $REPO_ROOT"

command -v cargo >/dev/null   || fail "cargo not in PATH"
command -v jq >/dev/null      || fail "jq not in PATH (brew install jq)"
command -v curl >/dev/null    || fail "curl not in PATH"

if ! cargo tauri --version >/dev/null 2>&1; then
  fail "tauri-cli not installed. Run: cargo install tauri-cli --version '^2.0'"
fi
note "tauri-cli: $(cargo tauri --version 2>&1 | head -1)"

[[ -f "$PLIST" ]] || fail "launchd plist missing at $PLIST"
note "plist: $PLIST"

for dir in ui/desktop ui/command-center; do
  if [[ ! -d "$dir/node_modules" ]]; then
    warn "node_modules missing in $dir — running npm install"
    (cd "$dir" && npm install)
  fi
done
note "node_modules present in ui/desktop and ui/command-center"

ok "Preflight passed"

# ── build ───────────────────────────────────────────────────────────────────
if [[ $SKIP_BUILD -eq 0 ]]; then
  step "Building permagentd (release)"
  cargo build --release --bin permagentd
  ok "Built $(stat -f '%Sm' -t '%H:%M:%S' "$DAEMON_BIN_SRC") — $(du -h "$DAEMON_BIN_SRC" | cut -f1)"
else
  step "Skipping build (--skip-build set)"
  [[ -f "$DAEMON_BIN_SRC" ]] || fail "no existing $DAEMON_BIN_SRC to skip-build against"
  note "using existing: $(stat -f '%Sm' -t '%Y-%m-%d %H:%M:%S' "$DAEMON_BIN_SRC")"
fi

# ── copy to Tauri source-of-truth ───────────────────────────────────────────
step "Copying daemon to Tauri externalBin source-of-truth"
mkdir -p "$TAURI_EXTERNAL_BIN_DIR"
cp "$DAEMON_BIN_SRC" "$TAURI_EXTERNAL_BIN"
note "copied to: $TAURI_EXTERNAL_BIN"
ok "Source-of-truth updated"

# ── tauri build ─────────────────────────────────────────────────────────────
step "Building Tauri bundle (cargo tauri build)"
note "this takes ~5-15 min depending on whether daemon binary changed"
(cd ui/desktop && cargo tauri build) || fail "cargo tauri build failed"

# find the freshest DMG
DMG_PATH="$(ls -t "$DMG_DIR"/Permagent_*.dmg 2>/dev/null | head -1 || true)"
[[ -n "$DMG_PATH" ]] || fail "no DMG produced in $DMG_DIR"
ok "DMG: $DMG_PATH"

# ── stop running daemon ─────────────────────────────────────────────────────
step "Stopping running daemon"

osascript -e 'quit app "Permagent"' 2>/dev/null || true
sleep 2
note "app quit signaled"

if launchctl print "gui/$(id -u)/$LAUNCHD_LABEL" >/dev/null 2>&1; then
  launchctl bootout "gui/$(id -u)/$LAUNCHD_LABEL" 2>/dev/null || true
  sleep 3
  note "launchd unloaded"
else
  note "launchd job not currently loaded"
fi

if pgrep -f permagentd >/dev/null; then
  pkill -9 -f permagentd 2>/dev/null || true
  sleep 2
  note "killed straggler permagentd processes"
fi

if lsof -nP -iTCP:3001 -sTCP:LISTEN >/dev/null 2>&1; then
  fail "port 3001 still bound after daemon shutdown. Check: lsof -nP -iTCP:3001"
fi
ok "Daemon stopped, port 3001 free"

# ── optional cache clear ────────────────────────────────────────────────────
if [[ $CLEAN_CACHE -eq 1 ]]; then
  step "Clearing WebKit cache"
  for dir in "${WEBKIT_CACHE_DIRS[@]}"; do
    if [[ -d "$dir" ]]; then
      rm -rf "$dir"
      note "cleared: $dir"
    fi
  done
  ok "WebKit cache cleared"
fi

# ── install via DMG ─────────────────────────────────────────────────────────
step "Installing via DMG (manual drag-to-Applications required)"
note "macOS code-signing requires drag-install. cp into /Applications/ produces SIGILL."
echo
open "$DMG_PATH"
echo "${C_YELLOW}→${C_RESET} DMG opened. Drag Permagent.app to Applications (overwrite when prompted)."
echo "  Press ENTER once the copy completes and the DMG window can be closed..."
read -r

[[ -d "$APP_INSTALLED" ]] || fail "$APP_INSTALLED missing — did the drag-install complete?"

INSTALLED_TS="$(stat -f '%Sm' -t '%Y-%m-%d %H:%M:%S' "$APP_INSTALLED/Contents/MacOS/permagentd")"
note "installed daemon: $INSTALLED_TS"

# ── bootstrap launchd ───────────────────────────────────────────────────────
step "Bootstrapping launchd"
launchctl bootstrap "gui/$(id -u)" "$PLIST"
sleep 10

# ── verify daemon ───────────────────────────────────────────────────────────
step "Verifying daemon health"

PID="$(pgrep permagentd || true)"
[[ -n "$PID" ]] || fail "permagentd not running after bootstrap. Check: tail $DAEMON_LOG"
note "PID: $PID"

if ! lsof -nP -iTCP:3001 -sTCP:LISTEN >/dev/null 2>&1; then
  fail "daemon running but not listening on port 3001. Check: tail $DAEMON_LOG"
fi
note "listening on 3001"

if [[ ! -f "$DAEMON_TOKEN_FILE" ]]; then
  warn "daemon token file missing at $DAEMON_TOKEN_FILE — skipping HTTP healthcheck"
else
  DAEMON_TOKEN="$(jq -r .token "$DAEMON_TOKEN_FILE")"
  HTTP_CODE="$(curl -s -o /dev/null -w '%{http_code}' \
    -H "Authorization: Bearer $DAEMON_TOKEN" \
    http://localhost:3001/api/dashboard/layout)"
  if [[ "$HTTP_CODE" != "200" ]]; then
    fail "GET /api/dashboard/layout returned $HTTP_CODE (expected 200). Check: tail $DAEMON_LOG"
  fi
  note "GET /api/dashboard/layout → 200"
fi

ok "Daemon healthy"

# ── summary ─────────────────────────────────────────────────────────────────
echo
echo "${C_GREEN}━━━ Redeploy complete ━━━${C_RESET}"
echo "  Daemon PID:     $PID"
echo "  Binary:         $APP_INSTALLED/Contents/MacOS/permagentd"
echo "  Installed:      $INSTALLED_TS"
echo "  Listening:      http://localhost:3001"
echo
echo "  Logs:           tail -f $DAEMON_LOG"
echo "  Open app:       open /Applications/Permagent.app"
echo
