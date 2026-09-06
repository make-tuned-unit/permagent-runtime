#!/usr/bin/env bash
# Deterministic shell regressions for redeploy-daemon.sh. These tests source
# only its pure ownership/build/rollback functions and never build, install,
# quit an app, touch launchd, or touch real user data.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT/scripts/redeploy-daemon.sh"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/permagent-redeploy-test.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

pass=0
failures=0
ok() { echo "ok - $1"; pass=$((pass + 1)); }
bad() { echo "not ok - $1" >&2; failures=$((failures + 1)); }

make_launchctl() {
  local path="$1" status="$2" pid="${3:-}"
  if [[ "$status" == 0 && -n "$pid" ]]; then
    printf '#!/bin/sh\nprintf "  pid = %s\\n"\n' "$pid" >"$path"
  else
    printf '#!/bin/sh\nexit %s\n' "$status" >"$path"
  fi
  chmod +x "$path"
}

make_lsof() {
  local path="$1" pid="$2" binary="$3"
  printf '#!/bin/sh\ncase " $* " in\n  *" -t "*) echo "%s" ;;\n  *) echo "n%s" ;;\nesac\n' "$pid" "$binary" >"$path"
  chmod +x "$path"
}

case_app_owned() (
  local dir="$TMP/app-owned"; mkdir -p "$dir/Applications/Permagent.app/Contents/MacOS"
  local launchctl="$dir/launchctl" lsof="$dir/lsof"
  make_launchctl "$launchctl" 1
  make_lsof "$lsof" 101 "$dir/Applications/Permagent.app/Contents/MacOS/permagentd"
  export APP_INSTALLED="$dir/Applications/Permagent.app" PLIST="$dir/missing.plist" \
    LAUNCHCTL_BIN="$launchctl" LSOF_BIN="$lsof" REDEPLOY_SOURCE_ONLY=1
  source "$SCRIPT"
  [[ "$(detect_ownership)" == app-sidecar ]]
)
case_app_owned && ok "app-sidecar with no plist" || bad "app-sidecar with no plist"

case_launchd_owned() (
  local dir="$TMP/launchd-owned"; mkdir -p "$dir/Applications/Permagent.app/Contents/MacOS" "$dir/Library"
  local launchctl="$dir/launchctl" lsof="$dir/lsof"
  make_launchctl "$launchctl" 0 102
  make_lsof "$lsof" 102 "$dir/Applications/Permagent.app/Contents/MacOS/permagentd"
  : >"$dir/Library/daemon.plist"
  export APP_INSTALLED="$dir/Applications/Permagent.app" PLIST="$dir/Library/daemon.plist" \
    LAUNCHCTL_BIN="$launchctl" LSOF_BIN="$lsof" REDEPLOY_SOURCE_ONLY=1
  source "$SCRIPT"
  [[ "$(detect_ownership)" == launchd ]]
)
case_launchd_owned && ok "loaded launchd ownership" || bad "loaded launchd ownership"

case_ambiguous() (
  local dir="$TMP/ambiguous"; mkdir -p "$dir/Applications/Permagent.app/Contents/MacOS"
  local launchctl="$dir/launchctl" lsof="$dir/lsof"
  make_launchctl "$launchctl" 1
  make_lsof "$lsof" 103 "/tmp/manual/permagentd"
  export APP_INSTALLED="$dir/Applications/Permagent.app" PLIST="$dir/missing.plist" \
    LAUNCHCTL_BIN="$launchctl" LSOF_BIN="$lsof" REDEPLOY_SOURCE_ONLY=1
  source "$SCRIPT"
  ! detect_ownership 2>"$dir/error"
  grep -q "ambiguous daemon ownership" "$dir/error"
)
case_ambiguous && ok "unknown listener is refused" || bad "unknown listener is refused"

case_mixed_launchd_app() (
  local dir="$TMP/mixed-launchd-app"; mkdir -p "$dir/Applications/Permagent.app/Contents/MacOS" "$dir/Library"
  local launchctl="$dir/launchctl" lsof="$dir/lsof"
  make_launchctl "$launchctl" 0 104
  make_lsof "$lsof" 105 "$dir/Applications/Permagent.app/Contents/MacOS/permagentd"
  : >"$dir/Library/daemon.plist"
  export APP_INSTALLED="$dir/Applications/Permagent.app" PLIST="$dir/Library/daemon.plist" \
    LAUNCHCTL_BIN="$launchctl" LSOF_BIN="$lsof" REDEPLOY_SOURCE_ONLY=1
  source "$SCRIPT"
  ! detect_ownership 2>"$dir/error"
  grep -q "does not own listener PID 105" "$dir/error"
)
case_mixed_launchd_app && ok "mixed launchd/app ownership is refused" || bad "mixed launchd/app ownership is refused"

case_lsof_failure_is_not_free_port() (
  local dir="$TMP/lsof-failure"; mkdir -p "$dir/Applications/Permagent.app/Contents/MacOS"
  local launchctl="$dir/launchctl" lsof="$dir/lsof"
  make_launchctl "$launchctl" 1
  printf '#!/bin/sh\nexit 2\n' >"$lsof"; chmod +x "$lsof"
  export APP_INSTALLED="$dir/Applications/Permagent.app" PLIST="$dir/missing.plist" \
    LAUNCHCTL_BIN="$launchctl" LSOF_BIN="$lsof" REDEPLOY_SOURCE_ONLY=1
  source "$SCRIPT"
  ! detect_ownership 2>"$dir/error"
  grep -q "could not inspect TCP port" "$dir/error"
)
case_lsof_failure_is_not_free_port && ok "lsof failure is not treated as a free port" || bad "lsof failure is not treated as a free port"

case_stop_probe_failure_is_refused() (
  export REDEPLOY_SOURCE_ONLY=1 OSASCRIPT_BIN=true
  source "$SCRIPT"
  port_pid() { return 2; }
  if stop_app_sidecar_for_install >"$TMP/stop-probe-error" 2>&1; then return 1; fi
  grep -q "refusing install after listener inspection failed" "$TMP/stop-probe-error"
)
case_stop_probe_failure_is_refused && ok "post-quit inspection failure cannot authorize install" || bad "post-quit inspection failure cannot authorize install"

case_stop_confirmed_free_port() (
  export REDEPLOY_SOURCE_ONLY=1 OSASCRIPT_BIN=true
  source "$SCRIPT"
  port_pid() { return 0; }
  stop_app_sidecar_for_install >/dev/null
)
case_stop_confirmed_free_port && ok "confirmed free port finishes graceful stop" || bad "confirmed free port finishes graceful stop"

case_ownership_change_after_build_is_refused() (
  local dir="$TMP/ownership-change"; mkdir -p "$dir/Applications/Permagent.app/Contents/MacOS" "$dir/bin"
  local launchctl="$dir/launchctl" lsof="$dir/lsof" counter="$dir/count"
  make_launchctl "$launchctl" 1
  cat >"$lsof" <<MOCK
#!/bin/sh
count=0
[[ -f "$counter" ]] && count=\$(cat "$counter")
if [[ " \$* " == *" -t "* ]]; then
  count=\$((count + 1))
  printf '%s' "\$count" >"$counter"
  if [[ "\$count" == 1 ]]; then echo 106; else echo 107; fi
else
  if [[ "\$count" == 1 ]]; then echo 'n$dir/Applications/Permagent.app/Contents/MacOS/permagentd'; else echo 'n/tmp/other/permagentd'; fi
fi
MOCK
  chmod +x "$lsof"
  export APP_INSTALLED="$dir/Applications/Permagent.app" PLIST="$dir/missing.plist" \
    LAUNCHCTL_BIN="$launchctl" LSOF_BIN="$lsof" REDEPLOY_SOURCE_ONLY=1
  source "$SCRIPT"
  initial="$(detect_ownership)" || return 1
  [[ "$initial" == app-sidecar ]] || return 1
  if revalidate_ownership "$initial" 2>"$dir/error"; then return 1; fi
  grep -q "not the installed app" "$dir/error"
)
case_ownership_change_after_build_is_refused && ok "ownership is revalidated before install" || bad "ownership is revalidated before install"

case_failed_build_does_not_stop() (
  local dir="$TMP/failed-build"; mkdir -p "$dir/repo/crates/goose-server" "$dir/repo/ui/desktop"
  local launchctl="$dir/launchctl" lsof="$dir/lsof" npm="$dir/npm" marker="$dir/stopped"
  make_launchctl "$launchctl" 1
  make_lsof "$lsof" "" ""
  printf '#!/bin/sh\nexit 7\n' >"$npm"; chmod +x "$npm"
  export APP_INSTALLED="$dir/Applications/Permagent.app" PLIST="$dir/missing.plist" \
    REDEPLOY_REPO_ROOT="$dir/repo" LAUNCHCTL_BIN="$launchctl" LSOF_BIN="$lsof" \
    NPM_BIN="$npm" OSASCRIPT_BIN="$marker" REDEPLOY_SOURCE_ONLY=1
  source "$SCRIPT"
  ! main >/dev/null 2>&1
  [[ ! -e "$marker" ]]
)
case_failed_build_does_not_stop && ok "failed build stops before app quit" || bad "failed build stops before app quit"

case_rollback() (
  local dir="$TMP/rollback"; mkdir -p "$dir/old/Permagent.app" "$dir/bin"
  echo old >"$dir/old/Permagent.app/version"
  local ditto="$dir/bin/ditto"
  printf '#!/bin/sh\ncp -R "$1" "$2"\n' >"$ditto"; chmod +x "$ditto"
  export APP_INSTALLED="$dir/old/Permagent.app" DITTO_BIN="$ditto" REDEPLOY_SOURCE_ONLY=1
  source "$SCRIPT"
  preserve_rollback_app "$dir/saved"
  [[ "$(cat "$dir/saved/Permagent.app/version")" == old ]]
)
case_rollback && ok "rollback app is preserved before install" || bad "rollback app is preserved before install"

case_rollback_collision_is_refused() (
  local dir="$TMP/rollback-collision"; mkdir -p "$dir/installed/Contents" "$dir/saved/Permagent.app"
  export APP_INSTALLED="$dir/installed" DITTO_BIN=true REDEPLOY_SOURCE_ONLY=1
  source "$SCRIPT"
  ! preserve_rollback_app "$dir/saved" 2>"$dir/error"
  grep -q "rollback target already exists" "$dir/error"
)
case_rollback_collision_is_refused && ok "rollback target collision is refused" || bad "rollback target collision is refused"

case_installed_signature_is_verified() (
  local dir="$TMP/signature"; mkdir -p "$dir/installed" "$dir/built" "$dir/bin"
  local codesign="$dir/bin/codesign"
  cat >"$codesign" <<'MOCK'
#!/bin/sh
case "$1" in
  --verify) exit 0 ;;
  -d) printf 'Identifier=com.permagent.desktop\nCDHash=abc123\n' ;;
  *) exit 2 ;;
esac
MOCK
  chmod +x "$codesign"
  export APP_INSTALLED="$dir/installed" APP_BUNDLE="$dir/built" CODESIGN_BIN="$codesign" REDEPLOY_SOURCE_ONLY=1
  source "$SCRIPT"
  validate_installed_app
)
case_installed_signature_is_verified && ok "manual install requires matching signature identity" || bad "manual install requires matching signature identity"

echo "redeploy-daemon shell tests: $pass passed, $failures failed"
[[ "$failures" -eq 0 ]]
