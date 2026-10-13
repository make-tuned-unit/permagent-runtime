#!/bin/bash
# Permagent release 2026-10-13 -- step-numbered, idempotent, run ONE step at a time:
#   reports/world-production/release_build_20261013.sh <0|1|2|3a|3|4|5|6>
# See reports/world-production/release-20261013-plan.md for what each step does.
# Set RELEASE_DRY_RUN=1 to print only (no log file, no manifest write).
# This script never stops the live daemon, never pushes, and never touches git state.
set -euo pipefail

STEP="${1:-}"
LANE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TARGET=/Volumes/PermagentBuilds/permagent/targets/world-production-20261006
BUILDS=/Volumes/PermagentBuilds/permagent
LOGDIR=$BUILDS/logs
RUNBUILD=$BUILDS/run-build
LOCKFILE=$BUILDS/world-production-20261006.lock
MANIFEST=$LANE/reports/world-production/release-20261013-manifest.json
PROV=/Users/j/Documents/dev/permagent-worktrees/dagJ-reports/release/provenance-check.sh
TAURI_DIR=$LANE/ui/desktop/src-tauri
SCRIPTS=$LANE/ui/desktop/scripts
DIST=$LANE/ui/command-center/dist
# DAEMON_SHA / EXPECTED_GIT_SHA / LIVE_PID / ROLLBACK are NOT constants: they are computed at run time
# (step 1 -> manifest facts daemon_sha256, daemon_artefact, git_sha9; step 5 -> rollback_path).
AUDIOCAP_REL=ui/desktop/src-tauri/audiocap/permagent-audiocap
# No build-specific sha/tip is encoded here: daemon/CLI shas, git_sha9 (= HEAD at step 1) and the
# audiocap sha (blob at HEAD) are computed at run time and kept in the manifest facts.
BUNDLE=$TARGET/release/bundle/macos/Permagent.app
APP=/Applications/Permagent.app
LAUNCHD_LABEL=ai.permagent.daemon
TS_BIN=/Applications/Tailscale.app/Contents/MacOS/Tailscale
DEVICE_FILE=$HOME/.permagent/secrets/device_tokens.json
TOKEN_FILE=$HOME/.permagent/secrets/daemon_token.json
export CARGO_TARGET_DIR=$TARGET

die()  { echo "FAIL: $*" >&2; exit 1; }
note() { echo "[$STEP] $*"; }
sha()  { shasum -a 256 "$1" | awk '{print $1}'; }

if [ -z "$STEP" ]; then sed -n 2,6p "$0"; exit 2; fi
if [ "${RELEASE_DRY_RUN:-0}" != 1 ]; then
  mkdir -p "$LOGDIR"
  LOG="$LOGDIR/release-20261013-$STEP.log"
  exec > >(tee -a "$LOG") 2>&1
  echo "=== step $STEP $(date '+%F %T') (log $LOG) ==="
else
  echo "=== step $STEP DRY RUN (no log, no manifest) ==="
fi

# record <label> <path>: add {label, path, sha256, bytes} to the manifest (skipped in dry run).
record() {
  local label=$1 path=$2 h
  h=$(sha "$path")
  note "sha256 $label $h"
  [ "${RELEASE_DRY_RUN:-0}" = 1 ] && return 0
  /usr/bin/python3 - "$MANIFEST" "$STEP" "$label" "$path" "$h" "$(stat -f %z "$path")" <<'PY'
import json, os, sys, time
m, step, label, path, h, size = sys.argv[1:]
d = json.load(open(m)) if os.path.exists(m) else {"release": "20261013", "tip": None, "artefacts": {}}
d.setdefault("facts", {})
d["artefacts"][label] = {"step": step, "path": path, "sha256": h, "bytes": int(size),
                         "recorded": time.strftime("%Y-%m-%dT%H:%M:%S%z")}
json.dump(d, open(m, "w"), indent=2); open(m, "a").write("\n")
PY
}

# Manifest facts: run-time computed values live in manifest["facts"], never as literals in this script.
# mset KEY VALUE stores one (skipped in dry run); mget KEY prints it or DIES if it is missing/empty.
mset() {
  note "fact $1 = $2"
  [ "${RELEASE_DRY_RUN:-0}" = 1 ] && return 0
  /usr/bin/python3 - "$MANIFEST" "$STEP" "$1" "$2" <<'PY'
import json, os, sys, time
m, step, k, v = sys.argv[1:]
d = json.load(open(m)) if os.path.exists(m) else {"release": "20261013", "tip": None, "artefacts": {}}
d.setdefault("facts", {})[k] = v
d.setdefault("fact_steps", {})[k] = {"step": step, "recorded": time.strftime("%Y-%m-%dT%H:%M:%S%z")}
json.dump(d, open(m, "w"), indent=2); open(m, "a").write("\n")
PY
}
mget() {
  [ -f "$MANIFEST" ] || die "manifest $MANIFEST missing: the step that records '$1' has not run for real"
  local v
  v=$(/usr/bin/python3 - "$MANIFEST" "$1" <<'PY'
import json, sys
v = json.load(open(sys.argv[1])).get("facts", {}).get(sys.argv[2])
print("" if v is None else v)
PY
) || die "cannot read manifest"
  [ -n "$v" ] || die "manifest fact '$1' missing or empty: run the step that records it (see this script's step list)"
  printf '%s' "$v"
}

# A cargo artefact: release/deps/<name>-<16 hex>. Exactly one candidate or stop and list them.
# (Tauri copies binaries/permagentd and binaries/permagent over target/release/ -- both are externalBin
# in tauri.conf.json -- so those two paths are never trusted; only the deps artefacts are.)
dep_artefact() {
  local name=$1 c=() f b
  for f in "$TARGET"/release/deps/"$name"-*; do
    [ -f "$f" ] && [ -x "$f" ] || continue
    b=${f##*/}; [[ "${b#"$name"-}" =~ ^[0-9a-f]{16}$ ]] || continue
    c+=("$f")
  done
  if [ "${#c[@]}" -ne 1 ]; then
    echo "FAIL: expected exactly one release/deps/$name-<hash> artefact, found ${#c[@]}:" >&2
    [ "${#c[@]}" -gt 0 ] && ls -l -t "${c[@]}" >&2
    exit 1
  fi
  printf '%s' "${c[0]}"
}
dep_daemon() { dep_artefact permagentd; }
dep_cli()    { dep_artefact permagent; }

# sha256 of the tracked audiocap helper AS COMMITTED at HEAD (a dirty working-tree copy cannot match it).
audiocap_head_sha() { git -C "$LANE" show "HEAD:$AUDIOCAP_REL" | shasum -a 256 | awk '{print $1}'; }

# pid of the installed daemon: the launchd agent first, the :3001 listener as fallback. Empty if none.
live_daemon_pid() {
  local p
  p=$(launchctl print "gui/$(id -u)/$LAUNCHD_LABEL" 2>/dev/null | sed -n 's/^[[:space:]]*pid = \([0-9][0-9]*\)$/\1/p' | head -1 || true)
  [ -n "$p" ] || p=$(lsof -nP -iTCP:3001 -sTCP:LISTEN -t 2>/dev/null | head -1 || true)
  printf '%s' "$p"
}

# /api/version git_sha of whatever answers on :3001 (empty if nothing answers or no token).
api_git_sha() {
  local tok v
  [ -f "$TOKEN_FILE" ] || return 0
  tok=$(python3 -c "import json;d=json.load(open('$TOKEN_FILE'));print(d.get('token') or list(d.values())[0])" 2>/dev/null) || return 0
  v=$(curl -s -m 5 -H "Authorization: Bearer $tok" http://127.0.0.1:3001/api/version || true)
  printf '%s' "$v" | /usr/bin/python3 -c 'import sys,json
try: print(json.load(sys.stdin).get("git_sha",""))
except Exception: print("")'
}

# Number of registered devices (count only; nothing from the file is printed).
device_count() {
  /usr/bin/python3 - "$DEVICE_FILE" <<'PY'
import json, os, sys
f = sys.argv[1]
if not os.path.exists(f): print(""); sys.exit(0)
d = json.load(open(f))
if isinstance(d, dict) and isinstance(d.get("devices"), (list, dict)): d = d["devices"]
print(len(d))
PY
}
dev_mtime() { stat -f %m "$DEVICE_FILE" 2>/dev/null || true; }
ts_status() { "$TS_BIN" serve status 2>&1 || true; }

# Provenance script has MAIN hard-coded to the primary checkout; run a copy pointed at this lane.
prov() {
  local tmp="$LOGDIR/provenance-check-lane.sh"
  [ "${RELEASE_DRY_RUN:-0}" = 1 ] && tmp=$(mktemp /tmp/prov-lane.XXXXXX)
  sed "s#^MAIN=.*#MAIN=$LANE#" "$PROV" > "$tmp"
  # The check is a printed report; its last pipeline is a grep that exits 1 when the tree is
  # clean, which under set -e would abort the step precisely when everything is fine.
  /bin/zsh "$tmp" "$@" || true
}

identity_name() {
  security find-identity -v -p codesigning | sed -n 's/.*"\(Developer ID Application:[^"]*\)".*/\1/p' | head -1
}

# Run a command through the launcher (external target, jobs=1, 1h kill, flock). The launcher
# writes the child's output to its own log, so echo that log's tail and enforce its exit code.
via_launcher() {
  local out rc=0 lg
  out=$(mktemp); "$RUNBUILD" "$@" >"$out" 2>&1 || rc=$?
  cat "$out"
  lg=$(sed -n 's/^Build log: //p' "$out" | tail -1)
  [ -n "$lg" ] && [ -f "$lg" ] && { echo "--- tail of launcher log $lg"; tail -40 "$lg"; }
  rm -f "$out"
  return $rc
}

# True Mach-O test without depending on extensions.
macho_files() { find "$1" -type f -exec sh -c 'for f; do file -b "$f" | grep -q "Mach-O" && echo "$f"; done' _ {} +; }

# Hash of __TEXT,__text only: stable across install_name_tool and re-signing, so it shows the
# bundled executable is the reviewed build even though its whole-file sha256 must differ.
# __TEXT,__text bytes only: `otool -s` prints the file path on its first line, so it never matches
# across two paths; segedit extracts the raw section.
text_sha() { local t; t=$(mktemp); segedit "$1" -extract __TEXT __text "$t" >/dev/null 2>&1 || { rm -f "$t"; echo "segedit-failed-$RANDOM"; return; }; shasum -a 256 "$t" | awk '{print $1}'; rm -f "$t"; }

case "$STEP" in
# ---------------------------------------------------------------------------------------------
0) # PREFLIGHT (read-only except the baseline snapshot)
  cd "$LANE"
  note "HEAD $(git rev-parse --short=9 HEAD) (recorded as git_sha9 in step 1; no tip literal to compare against)"
  # "Clean" = nothing that can ship differs: no tracked change or untracked file outside
  # reports/ (evidence JSON churn there is not compiled/bundled). The audiocap helper is NOT excepted:
  # a modified tracked helper must be caught here.
  dirty=$(git status --porcelain --untracked-files=all | grep -v -E '^.. reports/' || true)
  [ -z "$dirty" ] || die "tree not clean outside reports/: $dirty"
  note "tree clean outside reports/ (reports/ changes: $(git status --porcelain | grep -c ' reports/' || true), non-shipping)"
  [ -d "$BUILDS" ] && mount | grep -q '/Volumes/PermagentBuilds ' || die "PermagentBuilds volume not mounted"
  DEP=$(dep_daemon)
  note "cargo daemon artefact (single candidate): $DEP sha256 $(sha "$DEP") (recorded in step 1)"
  CLI_DEP=$(dep_cli)
  note "cargo CLI artefact (single candidate): $CLI_DEP sha256 $(sha "$CLI_DEP") (recorded in step 1)"
  AUDIOCAP_SHA=$(audiocap_head_sha)
  [ "$(sha "$LANE/$AUDIOCAP_REL")" = "$AUDIOCAP_SHA" ] || die "working-tree audiocap helper differs from HEAD's blob ($AUDIOCAP_SHA)"
  note "audiocap helper == HEAD blob $AUDIOCAP_SHA"
  for f in libsherpa-onnx-c-api.dylib libonnxruntime.dylib; do [ -f "$TARGET/release/$f" ] || die "missing $f in release dir"; done
  ID=$(identity_name); [ -n "$ID" ] || die "no 'Developer ID Application' identity in keychain"
  note "signing identity present: $ID"
  free=$(df -g /System/Volumes/Data | awk 'NR==2{print $4}')
  [ "$free" -ge 15 ] || die "internal free ${free} GiB < 15 GiB"
  note "internal free: ${free} GiB (>= 15)"
  /usr/bin/python3 - "$LOCKFILE" <<'PY' || die "launcher lock held: another build is running"
import fcntl, sys
f = open(sys.argv[1], "a"); fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
PY
  note "launcher lock free"
  [ -x "$RUNBUILD" ] || die "run-build missing"
  [ -f "$PROV" ] || die "provenance-check.sh missing"
  note "--- provenance-check pre (lane copy)"
  prov pre
  # BASELINE of the CURRENTLY installed app, written into the manifest (facts baseline.*) for step 6.
  # Never rewritten once an install has happened (it would then describe the new app).
  [ -d "$APP" ] || die "no installed $APP to baseline"
  if [ "${RELEASE_DRY_RUN:-0}" != 1 ] && [ -f "$MANIFEST" ] && /usr/bin/python3 -c 'import json,sys;sys.exit(0 if "rollback_path" in json.load(open(sys.argv[1])).get("facts",{}) else 1)' "$MANIFEST"; then
    note "install already recorded in manifest: baseline NOT rewritten"
  else
    b_shell=$(sha "$APP/Contents/MacOS/permagent-app"); b_daemon=$(sha "$APP/Contents/MacOS/permagentd")
    b_api=$(api_git_sha); b_pid=$(live_daemon_pid)
    [ -n "$b_api" ] || die "baseline: no /api/version git_sha from the live daemon on :3001 (daemon down or no token): start it, then re-run step 0"
    [ -n "$b_pid" ] || die "baseline: cannot identify the live daemon pid (launchctl print / lsof)"
    mset baseline.shell_sha256 "$b_shell"
    mset baseline.daemon_sha256 "$b_daemon"
    mset baseline.api_git_sha "$b_api"
    mset baseline.daemon_pid "$b_pid"
    mset baseline.device_count "$(device_count)"
    mset baseline.device_mtime "$(dev_mtime)"
    mset baseline.tailscale_serve_status "$(ts_status)"
    note "baseline of installed app written to manifest"
  fi
  note "PREFLIGHT OK"
  ;;
# ---------------------------------------------------------------------------------------------
1) # STAGE sidecars, CLI, mcp-runtime, whisper, audiocap  (all outputs are gitignored except audiocap)
  # The uplifted target/release/permagentd is NOT trustworthy: a Tauri build copies
  # ui/desktop/src-tauri/binaries/permagentd-aarch64-apple-darwin over it (2026-10-13: the Sep 29
  # daemon got packaged that way). Cargo's real artefact is release/deps/permagentd-<hash>.
  # Exactly one candidate or dep_daemon stops and lists them; its sha is computed here, not remembered.
  DEP=$(dep_daemon)
  DAEMON_SHA=$(sha "$DEP")
  cd "$LANE"
  # build.rs bakes `git rev-parse --short=9 HEAD` of this worktree at DAEMON COMPILE time; step 6
  # compares the installed daemon's /api/version against it. If commits landed after the daemon was
  # built, the operator passes DAEMON_GIT_SHA=<sha the artefact reports> and this step proves that
  # nothing the daemon compiles (crates/, Cargo.lock, Cargo.toml) changed between that tip and HEAD;
  # otherwise the artefact is stale and must be rebuilt.
  HEAD_SHA9=$(git rev-parse --short=9 HEAD)
  if [ -n "${DAEMON_GIT_SHA:-}" ]; then
    git cat-file -e "${DAEMON_GIT_SHA}^{commit}" || die "DAEMON_GIT_SHA $DAEMON_GIT_SHA is not a commit in this worktree"
    git merge-base --is-ancestor "$DAEMON_GIT_SHA" HEAD || die "DAEMON_GIT_SHA $DAEMON_GIT_SHA is not an ancestor of HEAD"
    rust_changed=$(git diff --name-only "$DAEMON_GIT_SHA" HEAD -- crates Cargo.lock Cargo.toml || true)
    [ -z "$rust_changed" ] || die "daemon-compiled paths changed since $DAEMON_GIT_SHA; rebuild the daemon: $rust_changed"
    GIT_SHA9=$(git rev-parse --short=9 "$DAEMON_GIT_SHA")
    note "daemon tip $GIT_SHA9 (operator-supplied; no crates/Cargo change up to HEAD $HEAD_SHA9)"
  else
    GIT_SHA9=$HEAD_SHA9
  fi
  mset head_sha9 "$HEAD_SHA9"
  cp "$DEP" "$TARGET/release/permagentd"; note "release/permagentd restored from cargo artefact $(basename "$DEP") ($DAEMON_SHA)"
  [ "$(sha "$TARGET/release/permagentd")" = "$DAEMON_SHA" ] || die "restored release/permagentd != artefact"
  # Same rule for the CLI: Tauri's externalBin copy overwrites target/release/permagent, and
  # copy-cli.sh stages from it, so restore it from the deps artefact first.
  CLI_DEP=$(dep_cli); CLI_SHA=$(sha "$CLI_DEP")
  cp "$CLI_DEP" "$TARGET/release/permagent"
  [ "$(sha "$TARGET/release/permagent")" = "$CLI_SHA" ] || die "restored release/permagent != artefact"
  note "release/permagent restored from cargo artefact $(basename "$CLI_DEP") ($CLI_SHA)"
  AUDIOCAP_SHA=$(audiocap_head_sha)
  [ "$(sha "$TAURI_DIR/audiocap/permagent-audiocap")" = "$AUDIOCAP_SHA" ] || die "tracked audiocap helper in the working tree differs from HEAD's blob ($AUDIOCAP_SHA): restore it with git checkout"
  mset daemon_artefact "$DEP"
  mset cli_artefact "$CLI_DEP"
  mset cli_sha256 "$CLI_SHA"
  mset audiocap_sha256 "$AUDIOCAP_SHA"
  mset daemon_sha256 "$DAEMON_SHA"
  mset git_sha9 "$GIT_SHA9"
  "$SCRIPTS/copy-sidecar.sh"
  "$SCRIPTS/copy-cli.sh"
  # copy-sidecar swallows codesign errors (|| true): prove the re-signed staged copies are valid.
  B=$TAURI_DIR/binaries
  for f in libsherpa-onnx-c-api.dylib libonnxruntime.dylib permagentd-aarch64-apple-darwin permagent-aarch64-apple-darwin; do
    codesign --verify --strict "$B/$f" || die "staged $f fails codesign --verify --strict"
  done
  note "staged sidecars verify (ad hoc; Tauri re-signs them with the Developer ID in step 3)"
  # mcp-runtime: copy-mcp-runtime.sh wipes node_modules and re-runs npm install over the network.
  # Reuse the staged runtime when it is already the pinned one; RELEASE_REFRESH_MCP=1 forces it.
  M=$TAURI_DIR/resources/mcp-runtime
  if [ "${RELEASE_REFRESH_MCP:-0}" != 1 ] && [ "$("$M/node" --version 2>/dev/null || true)" = v22.22.1 ] \
     && [ -d "$M/node_modules/tavily-mcp" ] && [ -d "$M/node_modules/@brave/brave-search-mcp-server" ]; then
    note "mcp-runtime already staged and pinned: reusing (RELEASE_REFRESH_MCP=1 to rebuild)"
  else "$SCRIPTS/copy-mcp-runtime.sh"; fi
  "$SCRIPTS/copy-whisper-model.sh"
  # build-audiocap.sh is deliberately NOT run (it compiles into the tracked file). The helper that ships
  # is the one committed at HEAD; its sha was checked against the blob above, so a dirty copy stops here.
  note "audiocap helper == HEAD blob $AUDIOCAP_SHA (build-audiocap.sh not run)"
  record staged-permagentd "$B/permagentd-aarch64-apple-darwin"
  record staged-permagent-cli "$B/permagent-aarch64-apple-darwin"
  record staged-libsherpa "$B/libsherpa-onnx-c-api.dylib"
  record staged-libonnxruntime "$B/libonnxruntime.dylib"
  record staged-mcp-node "$M/node"
  record staged-whisper "$TAURI_DIR/resources/whisper/whisper-base-q8_0.gguf"
  record staged-audiocap "$TAURI_DIR/audiocap/permagent-audiocap"
  record built-permagentd "$TARGET/release/permagentd"
  record built-permagent-cli "$TARGET/release/permagent"
  ;;
# ---------------------------------------------------------------------------------------------
2) # NATIVE FRONTEND BUILD + forum-marker check
  # Tauri's beforeBuildCommand is `cd ../command-center && npm run build` (= tsc && vite build) and
  # Tauri exports TAURI_ENV_PLATFORM itself; step 3 will run it again. Doing it here first lets the
  # markers be checked before the 1h+ compile. If vite output differs between runs, step 4 catches it.
  cd "$LANE/ui/command-center"
  TAURI_ENV_PLATFORM=darwin npm run build
  ls "$DIST"/assets/*.js >/dev/null 2>&1 || die "no dist/assets/*.js"
  grep -l 'permagent.world.legacy' "$DIST"/assets/*.js >/dev/null || die "marker permagent.world.legacy absent from dist/assets/*.js"
  grep -l 'world/vista/' "$DIST"/assets/*.js >/dev/null || die "marker world/vista/ absent from dist/assets/*.js"
  [ -f "$DIST/world/vista/manifest.json" ] || die "dist/world/vista/manifest.json missing"
  note "forum markers present (permagent.world.legacy, world/vista/, vista manifest)"
  record dist-world-vista-manifest "$DIST/world/vista/manifest.json"
  for f in "$DIST"/assets/*.js; do record "dist-assets/$(basename "$f")" "$f"; done
  ;;
# ---------------------------------------------------------------------------------------------
3a) # PRE-WARM the app crate (re-run until exit 0). Launcher kills at 1 h; finished crates persist.
  cd "$LANE"
  via_launcher cargo build --release --manifest-path ui/desktop/src-tauri/Cargo.toml --locked \
    && note "PRE-WARM COMPLETE (exit 0): proceed to step 3" \
    || { note "pre-warm did not finish (exit $?): re-run step 3a"; exit 75; }
  ;;
# ---------------------------------------------------------------------------------------------
3) # TAURI RELEASE BUILD, updater artefacts off, Developer ID signing. Only the .app is built
   # (--bundles app): the dmg step drives Finder via AppleScript, is not installed, and is not wanted.
  # tauri.conf.json's beforeBuildCommand is `cd ../command-center && npm run build`, so the
  # build must run from ui/desktop (as run_guarded_native.py does); the launcher inherits cwd.
  cd "$LANE/ui/desktop"
  ID=$(identity_name); [ -n "$ID" ] || die "no Developer ID Application identity"
  export APPLE_SIGNING_IDENTITY="$ID"
  # APPLE_SIGNING_IDENTITY is inherited by the launcher child (it copies os.environ).
  via_launcher node ../command-center/node_modules/@tauri-apps/cli/tauri.js build \
    --bundles app --config '{"bundle":{"createUpdaterArtifacts":false}}' \
    || { note "tauri build did not finish (exit $?). If it hit the 1h kill, run 3a again then 3"; exit 75; }
  [ -d "$BUNDLE" ] || die "bundle not found at $BUNDLE"
  # Signing audit: Tauri signs main exe, externalBin and frameworks; whether it signs
  # Resources/ Mach-O files (node, permagent-audiocap, native .node addons) is NOT confirmed,
  # so check every Mach-O and fix up only the ones that are not Developer ID signed.
  bad=()
  while IFS= read -r f; do
    codesign -dvv "$f" 2>&1 | grep 'Authority=Developer ID Application' >/dev/null || bad+=("$f")
  done < <(macho_files "$BUNDLE")
  if [ "${#bad[@]}" -gt 0 ]; then
    note "Mach-O files NOT Developer-ID signed after the Tauri step (${#bad[@]}):"; printf '  %s\n' "${bad[@]}"
    ENT=$TAURI_DIR/Entitlements.plist
    for f in "${bad[@]}"; do codesign --force --options runtime --timestamp --entitlements "$ENT" --sign "$ID" "$f"; done
    codesign --force --options runtime --timestamp --entitlements "$ENT" --sign "$ID" "$BUNDLE"
    note "re-signed offenders inner-first, then the bundle"
  else note "all Mach-O files in the bundle carry a Developer ID Application signature"; fi
  record bundle-main-exe "$BUNDLE/Contents/MacOS/permagent-app"
  ;;
# ---------------------------------------------------------------------------------------------
4) # POST-BUILD VERIFICATION (read-only)
  cd "$LANE"
  [ -d "$BUNDLE" ] || die "no bundle at $BUNDLE"
  codesign --verify --deep --strict --verbose=2 "$BUNDLE" || die "codesign --deep --strict failed"
  note "codesign --deep --strict OK"
  codesign -dvv "$BUNDLE" 2>&1 | grep -E 'Authority|TeamIdentifier|flags|Identifier' || true
  # spctl: expectation is REJECTED with "source=Unnotarized Developer ID" (no notarization step exists
  # in this recipe). That is acceptable for a locally copied install (no quarantine flag). A message
  # about an invalid signature or missing sealed resource is a real failure.
  sp=$(spctl -a -vv "$BUNDLE" 2>&1 || true); echo "$sp"
  echo "$sp" | grep -q -i -E 'invalid|sealed resource|not signed|a sealed' && die "spctl reports a signature problem"
  # every Mach-O must be Developer ID signed and every dylib strict-valid
  n=0
  while IFS= read -r f; do
    codesign --verify --strict "$f" 2>/dev/null || die "strict verify failed: $f"
    codesign -dvv "$f" 2>&1 | grep 'Authority=Developer ID Application' >/dev/null || die "not Developer ID signed: $f"
    n=$((n+1))
  done < <(macho_files "$BUNDLE")
  note "$n Mach-O files, all Developer ID signed and strict-valid"
  # Daemon: the bundled copy gained an rpath (install_name_tool) and a new signature, so its
  # whole-file sha CANNOT equal the built one. Compare __TEXT,__text instead, and show the built sha.
  BD=$BUNDLE/Contents/MacOS
  DAEMON_SHA=$(mget daemon_sha256); DEP=$(mget daemon_artefact)
  [ -f "$DEP" ] && [ "$(sha "$DEP")" = "$DAEMON_SHA" ] || die "cargo artefact $DEP no longer matches manifest daemon_sha256 $DAEMON_SHA"
  [ "$(text_sha "$BD/permagentd")" = "$(text_sha "$DEP")" ] || die "bundled permagentd code differs from the cargo artefact ($DAEMON_SHA)"
  note "bundled permagentd __text == cargo artefact $DAEMON_SHA"
  CLI_SHA=$(mget cli_sha256); CLI_DEP=$(mget cli_artefact); AUDIOCAP_SHA=$(mget audiocap_sha256)
  [ -f "$CLI_DEP" ] && [ "$(sha "$CLI_DEP")" = "$CLI_SHA" ] || die "cargo CLI artefact $CLI_DEP no longer matches manifest cli_sha256 $CLI_SHA"
  [ "$(text_sha "$BD/permagent")" = "$(text_sha "$CLI_DEP")" ] || die "bundled CLI code differs from the cargo CLI artefact ($CLI_SHA)"
  note "bundled permagent CLI __text == built CLI"
  [ "$(sha "$BUNDLE/Contents/Resources/permagent-audiocap")" = "$AUDIOCAP_SHA" ] \
    && note "audiocap helper in bundle byte-identical to HEAD's blob (unsigned-by-Tauri path)" \
    || { AC_HEAD=$(mktemp); git -C "$LANE" show "HEAD:$AUDIOCAP_REL" > "$AC_HEAD"
         [ "$(text_sha "$BUNDLE/Contents/Resources/permagent-audiocap")" = "$(text_sha "$AC_HEAD")" ] \
         && note "audiocap helper in bundle: same code as HEAD's blob, re-signed" || die "audiocap helper in bundle differs from HEAD's blob $AUDIOCAP_SHA"; rm -f "$AC_HEAD"; }
  for f in permagentd permagent permagent-app; do record "bundle-$f" "$BD/$f"; done
  record bundle-audiocap "$BUNDLE/Contents/Resources/permagent-audiocap"
  for f in "$BUNDLE"/Contents/Frameworks/*.dylib; do record "bundle-$(basename "$f")" "$f"; done
  # Markers in the bundle's daemon/CLI/UI, via provenance-check against the bundle.
  note "--- provenance-check post against the BUNDLE (not /Applications)"
  prov post "$BUNDLE"
  # Embedded frontend. verify_native_review.mjs is hard-wired to debug/permagent-app and the review
  # bundle identifiers, so it cannot check a release bundle. Same approach, release paths:
  # brotli-decode each tauri-codegen asset that is actually inside this binary and require every
  # dist asset (and the vista manifest) to be among them, byte for byte.
  cat > "$LOGDIR/verify_release_embedded_20261013.mjs" <<'JS'
import fs from 'node:fs'; import path from 'node:path'; import { createHash } from 'node:crypto'; import { brotliDecompressSync } from 'node:zlib';
const [target, exe, dist] = process.argv.slice(2);
const sha = b => createHash('sha256').update(b).digest('hex');
const walk = d => fs.readdirSync(d, { withFileTypes: true }).flatMap(e => e.isDirectory() ? walk(path.join(d, e.name)) : [path.join(d, e.name)]);
const bin = fs.readFileSync(exe), build = path.join(target, 'release/build');
const embedded = new Set(); let js = '';
for (const n of fs.readdirSync(build).filter(n => n.startsWith('permagent-app-'))) {
  const dir = path.join(build, n, 'out/tauri-codegen-assets'); if (!fs.existsSync(dir)) continue;
  for (const f of walk(dir)) { const c = fs.readFileSync(f); let d; try { d = brotliDecompressSync(c); } catch { continue; }
    if (!bin.includes(c)) continue; embedded.add(sha(d)); if (f.endsWith('.js')) js += d.toString(); } }
let missing = [];
for (const f of walk(dist)) { if (f.endsWith('.html')) continue; if (!embedded.has(sha(fs.readFileSync(f)))) missing.push(path.relative(dist, f)); }
const out = { embedded_assets: embedded.size, dist_files_not_embedded_exactly: missing,
  marker_world_legacy: js.includes('permagent.world.legacy'), marker_world_vista: js.includes('world/vista/'),
  vista_manifest_embedded: embedded.has(sha(fs.readFileSync(path.join(dist, 'world/vista/manifest.json')))) };
console.log(JSON.stringify(out, null, 2));
process.exit(embedded.size && !missing.length && out.marker_world_legacy && out.marker_world_vista && out.vista_manifest_embedded ? 0 : 1);
JS
  node "$LOGDIR/verify_release_embedded_20261013.mjs" "$TARGET" "$BD/permagent-app" "$DIST" || die "embedded frontend check failed (see JSON above)"
  note "embedded frontend contains every dist asset and the World markers"
  note "VERIFY OK"
  ;;
# ---------------------------------------------------------------------------------------------
5) # INSTALL to /Applications (rollback copy first). Needs RELEASE_CONFIRM_INSTALL=1.
  ROLLBACK=/Applications/Permagent.app.before-release-$(date +%Y%m%d-%H%M%S)
  cat <<EOF
WILL DO:
  1. refuse if any permagentd (launchd agent $LAUNCHD_LABEL pid: ${LIVE_NOW:-$(live_daemon_pid)}) or permagent-app is still running
  2. ditto $APP -> $ROLLBACK   (new timestamped path; refuses to overwrite ANY existing path)
  3. ditto $BUNDLE -> $APP.staging-20261013, verify it (codesign --deep --strict)
  4. rm -rf $APP; mv staging into place
  5. provenance-check post $APP
EOF
  [ "${RELEASE_CONFIRM_INSTALL:-0}" = 1 ] || { echo "Not installing: set RELEASE_CONFIRM_INSTALL=1 to proceed."; exit 3; }
  mget baseline.shell_sha256 >/dev/null; mget daemon_sha256 >/dev/null   # steps 0 and 1 must have run for real
  LIVE_NOW=$(live_daemon_pid)
  [ -z "$LIVE_NOW" ] || die "installed daemon (pid $LIVE_NOW, launchd $LAUNCHD_LABEL / :3001) is still running: coordinator must bootout first"
  pgrep -x permagentd >/dev/null && die "a permagentd process is still running: $(pgrep -x permagentd | tr '\n' ' ')"
  pgrep -x permagent-app >/dev/null && die "permagent-app is running: quit it first"
  [ -d "$BUNDLE" ] || die "no bundle: run steps 3 and 4"
  codesign --verify --deep --strict "$BUNDLE" || die "bundle fails codesign: rerun step 4"
  if [ -d "$APP" ] && [ "$(sha "$APP/Contents/MacOS/permagentd")" = "$(sha "$BUNDLE/Contents/MacOS/permagentd")" ]; then
    note "installed app already equals this bundle: skipping copy"
  else
    [ -e "$ROLLBACK" ] && die "$ROLLBACK already exists: refusing to overwrite a rollback"
    [ -d "$APP" ] || die "no current $APP to keep as rollback"
    ditto "$APP" "$ROLLBACK"
    codesign --verify --deep --strict "$ROLLBACK" 2>&1 || note "WARN: rollback copy does not verify strictly (the original may not have either)"
    [ "$(sha "$ROLLBACK/Contents/MacOS/permagentd")" = "$(sha "$APP/Contents/MacOS/permagentd")" ] || die "rollback copy differs from current app"
    note "rollback kept: $ROLLBACK"
    mset rollback_path "$ROLLBACK"
    STG=$APP.staging-20261013; rm -rf "$STG"
    ditto "$BUNDLE" "$STG"
    codesign --verify --deep --strict "$STG" || die "staged copy fails codesign"
    rm -rf "$APP"; mv "$STG" "$APP"
  fi
  record installed-permagentd "$APP/Contents/MacOS/permagentd"
  record installed-permagent-app "$APP/Contents/MacOS/permagent-app"
  note "--- provenance-check post (installed)"
  prov post "$APP"
  note "INSTALLED"
  ;;
# ---------------------------------------------------------------------------------------------
6) # POST-INSTALL CHECKS (coordinator has restarted the app/daemon); compares against MANIFEST facts only
  EXPECTED_GIT_SHA=$(mget git_sha9)          # step 1: `git rev-parse --short=9 HEAD` at build time (what build.rs bakes)
  DAEMON_SHA=$(mget daemon_sha256); DEP=$(mget daemon_artefact)
  B_SHELL=$(mget baseline.shell_sha256); B_DAEMON=$(mget baseline.daemon_sha256); B_API=$(mget baseline.api_git_sha)
  B_DEVN=$(mget baseline.device_count); B_DEVM=$(mget baseline.device_mtime); B_TS=$(mget baseline.tailscale_serve_status)
  mget baseline.daemon_pid >/dev/null; mget rollback_path >/dev/null
  code=000
  for i in $(seq 1 24); do
    code=$(curl -s -m 5 -o /dev/null -w '%{http_code}' http://127.0.0.1:3001/status || true)
    [ "$code" = 200 ] && break; sleep 5
  done
  [ "$code" = 200 ] || die "daemon /status on 127.0.0.1:3001 -> $code after 2 min"
  note "daemon /status 200"
  now_api=$(api_git_sha)
  [ -n "$now_api" ] || die "no /api/version git_sha from the installed daemon"
  [ "$now_api" = "$EXPECTED_GIT_SHA" ] || die "installed daemon /api/version git_sha $now_api != manifest git_sha9 $EXPECTED_GIT_SHA"
  note "installed daemon /api/version git_sha $now_api == manifest $EXPECTED_GIT_SHA"
  [ "$now_api" != "$B_API" ] || note "WARN: git_sha equals the baseline's ($B_API): the daemon source did not change"
  # installed files vs baseline and vs the reviewed build
  n_shell=$(sha "$APP/Contents/MacOS/permagent-app"); n_daemon=$(sha "$APP/Contents/MacOS/permagentd")
  [ "$n_shell" != "$B_SHELL" ] || die "installed shell sha equals the baseline's: the app was not replaced"
  [ "$n_daemon" != "$B_DAEMON" ] || die "installed daemon sha equals the baseline's: the app was not replaced"
  [ "$(text_sha "$APP/Contents/MacOS/permagentd")" = "$(text_sha "$DEP")" ] || die "installed permagentd __text != cargo artefact (manifest daemon_sha256 $DAEMON_SHA)"
  note "installed shell and daemon differ from the baseline; installed daemon code == cargo artefact $DAEMON_SHA"
  # running daemon: the launchd agent's pid, and its executable must be inside $APP
  pid=$(live_daemon_pid)
  [ -n "$pid" ] || die "cannot identify the running daemon (launchctl print $LAUNCHD_LABEL / lsof :3001)"
  cmd=$(ps -p "$pid" -o command= || true); note "daemon pid $pid: $cmd"
  case "$cmd" in "$APP/"*) ;; *) die "daemon pid $pid is not the one inside $APP";; esac
  codesign --verify --deep --strict "$APP" || die "installed app fails codesign"
  # unchanged state: devices and tailscale, against the baseline in the manifest
  bad=0
  n_devn=$(device_count); n_devm=$(dev_mtime); n_ts=$(ts_status)
  [ "$n_devn" = "$B_DEVN" ] || { echo "device count CHANGED: $B_DEVN -> $n_devn"; bad=1; }
  [ "$n_devm" = "$B_DEVM" ] || { echo "device registry mtime CHANGED: $B_DEVM -> $n_devm"; bad=1; }
  [ "$n_ts" = "$B_TS" ] || { echo "tailscale serve status CHANGED:"; echo "$n_ts"; bad=1; }
  [ "$bad" = 0 ] || die "state differs from the step-0 baseline (above)"
  note "device count ($n_devn), device registry mtime and tailscale serve status match the baseline"
  note "POST-INSTALL OK"
  ;;
# ---------------------------------------------------------------------------------------------
*) die "unknown step '$STEP' (use 0 1 2 3a 3 4 5 6)";;
esac
