#!/usr/bin/env bash
#
# Build and optionally install the Permagent desktop bundle.
#
# The desktop app owns the daemon when no launchd plist is present. A launchd
# daemon and an app-sidecar must never be guessed at or started together. This
# script builds first, preserves a rollback copy, and only touches the running
# app after an explicit --install.

set -euo pipefail

if [[ -t 1 ]]; then
  C_RED=$'\033[0;31m'; C_GREEN=$'\033[0;32m'; C_YELLOW=$'\033[0;33m'
  C_CYAN=$'\033[0;36m'; C_DIM=$'\033[2m'; C_RESET=$'\033[0m'
else
  C_RED=''; C_GREEN=''; C_YELLOW=''; C_CYAN=''; C_DIM=''; C_RESET=''
fi

step() { echo "${C_CYAN}▶${C_RESET} $*"; }
ok() { echo "${C_GREEN}✓${C_RESET} $*"; }
note() { echo "  ${C_DIM}$*${C_RESET}"; }
warn() { echo "${C_YELLOW}!${C_RESET} $*"; }
fail() { echo "${C_RED}✗${C_RESET} $*" >&2; return 1; }

usage() {
  cat <<'HELP'
Build and optionally install the Permagent desktop bundle.

Usage:
  ./scripts/redeploy-daemon.sh [options]

Options:
  --dry-run              Inspect ownership and print the plan; do not build,
                         quit, install, open, or mutate anything.
  --skip-build           Use an existing Tauri app/DMG artifact.
  --install              Opt in to quitting the current app and manual DMG
                         installation. Without this flag the script only builds.
  --allow-launchd-daemon Permit installation while launchd owns the daemon;
                         the script never unloads or restarts that daemon.
  --rollback-dir PATH    Preserve the installed app at PATH before install.
  --help                 Show this help.

Ownership rules:
  no plist + installed-app daemon on :3001 (or no listener) = app-sidecar;
  loaded launchd plist = launchd-owned;
  any mixed/unknown state = refusal, to avoid duplicate daemons.

The script does not support --clean-cache or broad process killing. Clear
cache or repair a separately managed daemon as an explicit operator action.
HELP
}

# These command variables make ownership and safety gates deterministic in the
# shell regression tests without changing production behavior.
LAUNCHCTL_BIN="${LAUNCHCTL_BIN:-launchctl}"
LSOF_BIN="${LSOF_BIN:-lsof}"
NPM_BIN="${NPM_BIN:-npm}"
DITTO_BIN="${DITTO_BIN:-ditto}"
OSASCRIPT_BIN="${OSASCRIPT_BIN:-osascript}"
CODESIGN_BIN="${CODESIGN_BIN:-codesign}"

DRY_RUN=0
SKIP_BUILD=0
INSTALL=0
ALLOW_LAUNCHD=0
ROLLBACK_DIR=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run) DRY_RUN=1 ;;
    --skip-build) SKIP_BUILD=1 ;;
    --install) INSTALL=1 ;;
    --allow-launchd-daemon) ALLOW_LAUNCHD=1 ;;
    --rollback-dir)
      shift
      [[ $# -gt 0 ]] || { fail "--rollback-dir requires a path"; exit 2; }
      ROLLBACK_DIR="$1"
      ;;
    --rollback-dir=*) ROLLBACK_DIR="${1#*=}" ;;
    --clean-cache) fail "--clean-cache was removed; cache deletion is not part of a safe redeploy" || exit 2 ;;
    --help|-h) usage; exit 0 ;;
    *) fail "unknown argument: $1 (try --help)" || exit 2 ;;
  esac
  shift
done

REPO_ROOT="${REDEPLOY_REPO_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
DAEMON_PORT="${DAEMON_PORT:-3001}"
APP_INSTALLED="${APP_INSTALLED:-/Applications/Permagent.app}"
PLIST="${PLIST:-$HOME/Library/LaunchAgents/ai.permagent.daemon.plist}"
LAUNCHD_LABEL="${LAUNCHD_LABEL:-ai.permagent.daemon}"
DMG_DIR="$REPO_ROOT/ui/desktop/src-tauri/target/release/bundle/dmg"
APP_BUNDLE="$REPO_ROOT/ui/desktop/src-tauri/target/release/bundle/macos/Permagent.app"
DAEMON_TOKEN_FILE="${DAEMON_TOKEN_FILE:-$HOME/.permagent/secrets/daemon_token.json}"
DAEMON_LOG="${DAEMON_LOG:-$HOME/.permagent/logs/daemon-sidecar.log}"

port_pid() {
  local output status
  if output=$("$LSOF_BIN" -nP -iTCP:"$DAEMON_PORT" -sTCP:LISTEN -t 2>/dev/null); then
    printf '%s\n' "$output" | head -1
    return 0
  else
    status=$?
  fi
  # lsof uses exit 1 for a valid query with no matching listener. Any other
  # failure is an ownership-observation failure, not an empty/free port.
  if [[ "$status" == 1 ]]; then
    return 0
  fi
  fail "could not inspect TCP port $DAEMON_PORT with lsof (exit $status)"
  return "$status"
}

pid_binary() {
  local pid="$1"
  [[ -n "$pid" ]] || return 0
  local output status
  if output=$("$LSOF_BIN" -p "$pid" -a -d txt -Fn 2>/dev/null); then
    printf '%s\n' "$output" | sed -n 's/^n//p' | head -1
    return 0
  else
    status=$?
  fi
  fail "could not identify executable for listener PID $pid with lsof (exit $status)"
  return "$status"
}

launchd_job_pid() {
  local info
  info=$("$LAUNCHCTL_BIN" print "gui/$(id -u)/$LAUNCHD_LABEL" 2>/dev/null) || return 1
  printf '%s\n' "$info" | sed -n 's/^[[:space:]]*pid = \([0-9][0-9]*\).*/\1/p' | head -1
}

# Prints app-sidecar or launchd. Unknown ownership is an error by design.
detect_ownership() {
  local pid binary launchd_pid plist_present=0 launchd_loaded=0
  [[ -f "$PLIST" ]] && plist_present=1
  if launchd_pid="$(launchd_job_pid)"; then
    launchd_loaded=1
  else
    launchd_pid=""
  fi
  if ! pid="$(port_pid)"; then
    return 1
  fi

  if [[ "$launchd_loaded" == 1 && "$plist_present" == 1 ]]; then
    if [[ -n "$pid" && ( -z "$launchd_pid" || "$pid" != "$launchd_pid" ) ]]; then
      fail "ambiguous daemon ownership: launchd job PID ${launchd_pid:-unknown} does not own listener PID $pid"
      return 1
    fi
    echo launchd
    return 0
  fi
  if [[ "$launchd_loaded" == 1 && "$plist_present" == 0 ]]; then
    fail "ambiguous daemon ownership: launchd job is loaded but its plist is absent"
    return 1
  fi
  if [[ "$plist_present" == 1 ]]; then
    fail "ambiguous daemon ownership: plist exists but launchd job is not loaded (port PID=${pid:-none})"
    return 1
  fi
  if [[ -z "$pid" ]]; then
    echo app-sidecar
    return 0
  fi
  if ! binary="$(pid_binary "$pid")"; then
    return 1
  fi
  if [[ "$binary" == "$APP_INSTALLED/Contents/MacOS/permagentd" ]]; then
    echo app-sidecar
    return 0
  fi
  fail "ambiguous daemon ownership: :$DAEMON_PORT is held by ${binary:-PID $pid}, not the installed app"
  return 1
}

revalidate_ownership() {
  local expected="$1" current
  current="$(detect_ownership)" || return 1
  if [[ "$current" != "$expected" ]]; then
    fail "daemon ownership changed during build ($expected -> $current); refusing install"
    return 1
  fi
  printf '%s\n' "$current"
}

build_app() {
  step "Building desktop bundle with ui/desktop npm run build:all"
  (cd "$REPO_ROOT/ui/desktop" && "$NPM_BIN" run build:all)
}

find_artifacts() {
  [[ -d "$APP_BUNDLE" ]] || { fail "Tauri app bundle missing: $APP_BUNDLE"; return 1; }
  DMG_PATH="$(ls -t "$DMG_DIR"/Permagent_*.dmg 2>/dev/null | head -1 || true)"
  [[ -n "$DMG_PATH" ]] || { fail "DMG missing in $DMG_DIR"; return 1; }
}

preserve_rollback_app() {
  local destination="$1"
  local target="$destination/Permagent.app"
  [[ -d "$APP_INSTALLED" ]] || { fail "installed app missing; cannot preserve rollback: $APP_INSTALLED"; return 1; }
  [[ ! -e "$target" ]] || { fail "rollback target already exists; refusing to overwrite: $target"; return 1; }
  mkdir -p "$destination"
  "$DITTO_BIN" "$APP_INSTALLED" "$target"
  [[ -d "$target" ]] || { fail "rollback copy did not produce an app bundle: $target"; return 1; }
  note "rollback copy: $target"
}

codesign_details() {
  "$CODESIGN_BIN" -d --verbose=4 "$1" 2>&1
}

validate_installed_app() {
  [[ -d "$APP_INSTALLED" ]] || { fail "$APP_INSTALLED missing after manual installation"; return 1; }
  "$CODESIGN_BIN" --verify --deep --strict "$APP_INSTALLED" >/dev/null 2>&1 || {
    fail "operator step unverified: installed app failed codesign verification"
    return 1
  }
  local expected actual expected_id actual_id expected_hash actual_hash
  expected="$(codesign_details "$APP_BUNDLE")" || {
    fail "operator step unverified: could not inspect built artifact signature"
    return 1
  }
  actual="$(codesign_details "$APP_INSTALLED")" || {
    fail "operator step unverified: could not inspect installed artifact signature"
    return 1
  }
  expected_id="$(printf '%s\n' "$expected" | sed -n 's/^Identifier=//p' | head -1)"
  actual_id="$(printf '%s\n' "$actual" | sed -n 's/^Identifier=//p' | head -1)"
  expected_hash="$(printf '%s\n' "$expected" | sed -n 's/^CDHash=//p' | head -1)"
  actual_hash="$(printf '%s\n' "$actual" | sed -n 's/^CDHash=//p' | head -1)"
  if [[ -z "$expected_id" || -z "$actual_id" || "$expected_id" != "$actual_id" ||
    -z "$expected_hash" || -z "$actual_hash" || "$expected_hash" != "$actual_hash" ]]; then
    fail "operator step unverified: installed signature identity does not match built artifact"
    return 1
  fi
}

stop_app_sidecar_for_install() {
  local listener
  step "Stopping the current app-owned sidecar gracefully"
  "$OSASCRIPT_BIN" -e 'quit app "Permagent"' 2>/dev/null || true
  for _ in {1..30}; do
    if ! listener="$(port_pid)"; then
      fail "cannot confirm app-sidecar stopped; refusing install after listener inspection failed"
      return 1
    fi
    if [[ -z "$listener" ]]; then
      ok "app-sidecar stopped; port $DAEMON_PORT is free"
      return 0
    fi
    sleep 1
  done
  fail "app-sidecar did not release port $DAEMON_PORT; refusing install without killing it"
}

install_bundle() {
  local ownership="$1"
  [[ "$INSTALL" == 1 ]] || { fail "internal error: install requested without --install"; return 1; }
  if [[ "$ownership" == launchd && "$ALLOW_LAUNCHD" != 1 ]]; then
    fail "launchd owns the daemon; add --allow-launchd-daemon to install UI only (daemon is never unloaded)"
    return 1
  fi
  if [[ -z "$ROLLBACK_DIR" ]]; then
    ROLLBACK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/permagent-rollback.XXXXXX")" || {
      fail "could not create a fresh rollback directory"
      return 1
    }
  fi
  preserve_rollback_app "$ROLLBACK_DIR"
  if [[ "$ownership" == app-sidecar ]]; then
    stop_app_sidecar_for_install
  else
    step "Leaving launchd daemon running (no unload/restart performed)"
  fi
  step "Opening DMG for manual signed-app installation"
  open "$DMG_PATH"
  echo "Drag Permagent.app to Applications, then press ENTER to continue."
  read -r
  validate_installed_app || return 1
  ok "Installed app present; rollback preserved at $ROLLBACK_DIR/Permagent.app"
}

main() {
  cd "$REPO_ROOT"
  [[ -d "$REPO_ROOT/crates/goose-server" ]] || { fail "not in permagent-runtime repo root"; return 1; }
  local ownership
  ownership="$(detect_ownership)" || return 1
  note "daemon ownership: $ownership"

  if [[ "$DRY_RUN" == 1 ]]; then
    echo "dry-run: ownership=$ownership"
    echo "dry-run: build=$([[ "$SKIP_BUILD" == 1 ]] && echo skipped || echo npm-run-build-all)"
    echo "dry-run: install=$([[ "$INSTALL" == 1 ]] && echo requested || echo not-requested)"
    [[ "$ownership" == launchd && "$INSTALL" == 1 && "$ALLOW_LAUNCHD" != 1 ]] && \
      echo "dry-run: install would refuse without --allow-launchd-daemon"
    return 0
  fi

  if [[ "$SKIP_BUILD" == 0 ]]; then
    build_app || { fail "build failed; running app was not stopped"; return 1; }
  fi
  find_artifacts || return 1
  ok "artifacts ready: $APP_BUNDLE and $DMG_PATH"
  if [[ "$INSTALL" == 1 ]]; then
    local post_build_ownership
    post_build_ownership="$(revalidate_ownership "$ownership")" || return 1
    ownership="$post_build_ownership"
    install_bundle "$ownership"
  else
    note "build-only mode: no app quit, install, daemon restart, or cache mutation"
  fi
}

if [[ "${REDEPLOY_SOURCE_ONLY:-0}" != 1 ]]; then
  main "$@"
fi
