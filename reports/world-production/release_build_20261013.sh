#!/bin/bash
# Permagent release 2026-10-13 -- step-numbered, idempotent, run ONE step at a time:
#   reports/world-production/release_build_20261013.sh <0|1|2|3a|3|4|5|6>
# See reports/world-production/release-20261013-plan.md for what each step does.
# Set RELEASE_DRY_RUN=1 to print only (no log file, no manifest, no baseline write).
# This script never stops the live daemon, never pushes, and never touches git state.
set -euo pipefail

STEP="${1:-}"
LANE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
EXPECTED_TIP=43e487d1
TARGET=/Volumes/PermagentBuilds/permagent/targets/world-production-20261006
BUILDS=/Volumes/PermagentBuilds/permagent
LOGDIR=$BUILDS/logs
RUNBUILD=$BUILDS/run-build
LOCKFILE=$BUILDS/world-production-20261006.lock
MANIFEST=$LANE/reports/world-production/release-20261013-manifest.json
BASELINE=$LOGDIR/release-20261013-baseline.json
PROV=/Users/j/Documents/dev/permagent-worktrees/dagJ-reports/release/provenance-check.sh
TAURI_DIR=$LANE/ui/desktop/src-tauri
SCRIPTS=$LANE/ui/desktop/scripts
DIST=$LANE/ui/command-center/dist
DAEMON_SHA=a73a4ac48d294af8acf44f8fb6d6fcf459adeb5f5974ebce06b741b1fa217f58
AUDIOCAP_SHA=03ffdfa5acd042b9d78488d04dd0d3f40d216195088e817ca7be6fdd1a32ceaf
CLI_SHA_BUILT=f5173c846915b762ea57d47a436401259022c76d259f432798281ce5c751d61a
BUNDLE=$TARGET/release/bundle/macos/Permagent.app
APP=/Applications/Permagent.app
ROLLBACK=/Applications/Permagent.app.before-release-20261013
LIVE_PID=1439
TS_BIN=/Applications/Tailscale.app/Contents/MacOS/Tailscale
DEVICE_FILE=$HOME/.permagent/secrets/device_tokens.json
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
d = json.load(open(m)) if os.path.exists(m) else {"release": "20261013", "tip": "43e487d1", "artefacts": {}}
d["artefacts"][label] = {"step": step, "path": path, "sha256": h, "bytes": int(size),
                         "recorded": time.strftime("%Y-%m-%dT%H:%M:%S%z")}
json.dump(d, open(m, "w"), indent=2); open(m, "a").write("\n")
PY
}

# Provenance script has MAIN hard-coded to the primary checkout; run a copy pointed at this lane.
prov() {
  local tmp="$LOGDIR/provenance-check-lane.sh"
  [ "${RELEASE_DRY_RUN:-0}" = 1 ] && tmp=$(mktemp /tmp/prov-lane.XXXXXX)
  sed "s#^MAIN=.*#MAIN=$LANE#" "$PROV" > "$tmp"
  /bin/zsh "$tmp" "$@"
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
  head=$(git rev-parse --short=8 HEAD)
  if [ "$head" != "$EXPECTED_TIP" ]; then
    git merge-base --is-ancestor "$EXPECTED_TIP" HEAD || die "HEAD $head does not contain tip $EXPECTED_TIP"
    extra=$(git diff --name-only "$EXPECTED_TIP" HEAD | grep -v -E '^reports/world-production/(release_build_20261013\.sh|release-20261013-.*)$' || true)
    [ -z "$extra" ] || die "commits after $EXPECTED_TIP touch more than the release script/plan: $extra"
    note "HEAD $head = tip $EXPECTED_TIP + release-prep commit(s) only: OK"
  else note "HEAD = $EXPECTED_TIP: OK"; fi
  # "Clean" = nothing that can ship differs: no tracked change or untracked file outside
  # reports/ (evidence JSON churn there is not compiled/bundled), except the audiocap helper
  # IF it already equals the reviewed binary (step 1 stages it over the tracked file).
  dirty=$(git status --porcelain --untracked-files=all | grep -v -E '^.. reports/' || true)
  ac=ui/desktop/src-tauri/audiocap/permagent-audiocap
  if echo "$dirty" | grep -q "$ac" && [ "$(sha "$LANE/$ac")" = "$AUDIOCAP_SHA" ]; then
    dirty=$(echo "$dirty" | grep -v "$ac" || true); note "audiocap helper already staged (reviewed sha): allowed"
  fi
  [ -z "$dirty" ] || die "tree not clean outside reports/: $dirty"
  note "tree clean outside reports/ (reports/ changes: $(git status --porcelain | grep -c ' reports/' || true), non-shipping)"
  [ -d "$BUILDS" ] && mount | grep -q '/Volumes/PermagentBuilds ' || die "PermagentBuilds volume not mounted"
  [ "$(sha "$TARGET/release/permagentd")" = "$DAEMON_SHA" ] || die "permagentd sha != $DAEMON_SHA"
  note "permagentd sha OK"
  [ "$(sha "$TARGET/release/permagent")" = "$CLI_SHA_BUILT" ] || die "CLI sha != $CLI_SHA_BUILT (rebuilt? update CLI_SHA_BUILT after review)"
  note "permagent CLI sha OK"
  [ "$(sha "$TARGET/review-audiocap/permagent-audiocap")" = "$AUDIOCAP_SHA" ] || die "audiocap helper sha != $AUDIOCAP_SHA"
  note "audiocap helper sha OK"
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
  # Baseline for step 6 (written once, by the first real preflight).
  if [ "${RELEASE_DRY_RUN:-0}" != 1 ] && [ ! -f "$BASELINE" ]; then
    /usr/bin/python3 - "$BASELINE" "$DEVICE_FILE" "$("$TS_BIN" serve status 2>&1 | /usr/bin/python3 -c 'import sys,json;print(json.dumps(sys.stdin.read()))')" <<'PY'
import json, os, sys
b, dev, ts = sys.argv[1:]
st = os.stat(dev) if os.path.exists(dev) else None
json.dump({"tailscale_serve_status": json.loads(ts),
           "device_file": dev, "device_mtime": st.st_mtime if st else None, "device_size": st.st_size if st else None}, open(b, "w"), indent=2)
PY
    note "baseline written: $BASELINE"
  fi
  note "PREFLIGHT OK"
  ;;
# ---------------------------------------------------------------------------------------------
1) # STAGE sidecars, CLI, mcp-runtime, whisper, audiocap  (all outputs are gitignored except audiocap)
  cd "$LANE"
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
  # build-audiocap.sh WOULD COMPILE (swiftc -O main.swift, then ad hoc codesign) into the TRACKED
  # file ui/desktop/src-tauri/audiocap/permagent-audiocap. That yields a new, unreviewed binary, so
  # it is deliberately NOT run. Stage the reviewed helper over the tracked path instead.
  # (The committed binary is 4f5f0dab..., not the reviewed 03ffdfa5...; restore after install with
  #  git checkout -- ui/desktop/src-tauri/audiocap/permagent-audiocap.)
  cp "$TARGET/review-audiocap/permagent-audiocap" "$TAURI_DIR/audiocap/permagent-audiocap"
  chmod +x "$TAURI_DIR/audiocap/permagent-audiocap"
  [ "$(sha "$TAURI_DIR/audiocap/permagent-audiocap")" = "$AUDIOCAP_SHA" ] || die "staged audiocap sha mismatch"
  note "reviewed audiocap helper staged over tracked path (build-audiocap.sh not run)"
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
  # whole-file sha CANNOT equal a73a4ac4... Compare __TEXT,__text instead, and show the built sha.
  BD=$BUNDLE/Contents/MacOS
  [ "$(text_sha "$BD/permagentd")" = "$(text_sha "$TARGET/release/permagentd")" ] || die "bundled permagentd code differs from the reviewed build ($DAEMON_SHA)"
  note "bundled permagentd __text == reviewed build $DAEMON_SHA"
  [ "$(text_sha "$BD/permagent")" = "$(text_sha "$TARGET/release/permagent")" ] || die "bundled CLI code differs from built CLI ($CLI_SHA_BUILT)"
  note "bundled permagent CLI __text == built CLI"
  [ "$(sha "$BUNDLE/Contents/Resources/permagent-audiocap")" = "$AUDIOCAP_SHA" ] \
    && note "audiocap helper in bundle byte-identical to reviewed (unsigned-by-Tauri path)" \
    || { [ "$(text_sha "$BUNDLE/Contents/Resources/permagent-audiocap")" = "$(text_sha "$TARGET/review-audiocap/permagent-audiocap")" ] \
         && note "audiocap helper in bundle: same code as reviewed, re-signed" || die "audiocap helper in bundle differs from reviewed $AUDIOCAP_SHA"; }
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
  cat <<EOF
WILL DO:
  1. refuse if permagentd pid $LIVE_PID (or permagent-app) is still running
  2. ditto $APP -> $ROLLBACK   (kept as rollback; refuses to overwrite an existing rollback)
  3. ditto $BUNDLE -> $APP.staging-20261013, verify it (codesign --deep --strict)
  4. rm -rf $APP; mv staging into place
  5. provenance-check post $APP
EOF
  [ "${RELEASE_CONFIRM_INSTALL:-0}" = 1 ] || { echo "Not installing: set RELEASE_CONFIRM_INSTALL=1 to proceed."; exit 3; }
  pgrep -x permagentd | grep -qx "$LIVE_PID" && die "permagentd pid $LIVE_PID is still running: coordinator must stop it first"
  pgrep -x permagent-app >/dev/null && die "permagent-app is running: quit it first"
  [ -d "$BUNDLE" ] || die "no bundle: run steps 3 and 4"
  codesign --verify --deep --strict "$BUNDLE" || die "bundle fails codesign: rerun step 4"
  if [ -d "$APP" ] && [ "$(sha "$APP/Contents/MacOS/permagentd")" = "$(sha "$BUNDLE/Contents/MacOS/permagentd")" ]; then
    note "installed app already equals this bundle: skipping copy"
  else
    [ -e "$ROLLBACK" ] && die "$ROLLBACK already exists: refusing to overwrite the rollback"
    [ -d "$APP" ] || die "no current $APP to keep as rollback"
    ditto "$APP" "$ROLLBACK"
    codesign --verify --deep --strict "$ROLLBACK" 2>&1 || note "WARN: rollback copy does not verify strictly (the original may not have either)"
    [ "$(sha "$ROLLBACK/Contents/MacOS/permagentd")" = "$(sha "$APP/Contents/MacOS/permagentd")" ] || die "rollback copy differs from current app"
    note "rollback kept: $ROLLBACK"
    STG=$APP.staging-20261013; rm -rf "$STG"
    ditto "$BUNDLE" "$STG"
    codesign --verify --deep --strict "$STG" || die "staged copy fails codesign"
    rm -rf "$APP"; mv "$STG" "$APP"
  fi
  record installed-permagentd "$APP/Contents/MacOS/permagentd"
  record installed-permagent-app "$APP/Contents/MacOS/permagent-app"
  note "--- provenance-check post (installed)"
  prov post "$APP"
  note "INSTALLED. Restore the tracked audiocap file when done: git checkout -- ui/desktop/src-tauri/audiocap/permagent-audiocap"
  ;;
# ---------------------------------------------------------------------------------------------
6) # POST-INSTALL CHECKS (coordinator has restarted the app/daemon)
  [ -f "$BASELINE" ] || die "baseline missing ($BASELINE): step 0 was never run for real"
  code=000
  for i in $(seq 1 24); do
    code=$(curl -s -m 5 -o /dev/null -w '%{http_code}' http://127.0.0.1:3001/status || true)
    [ "$code" = 200 ] && break; sleep 5
  done
  [ "$code" = 200 ] || die "daemon /status on 127.0.0.1:3001 -> $code after 2 min"
  note "daemon /status 200"
  for p in $(pgrep -x permagentd); do
    cmd=$(ps -p "$p" -o command= || true); note "permagentd pid $p: $cmd"
  done
  pgrep -x permagentd | while read -r p; do ps -p "$p" -o command= | grep -q "^$APP/" && echo ok; done | grep -q ok \
    || die "no running permagentd is the one inside $APP"
  codesign --verify --deep --strict "$APP" || die "installed app fails codesign"
  /usr/bin/python3 - "$BASELINE" "$("$TS_BIN" serve status 2>&1 | /usr/bin/python3 -c 'import sys,json;print(json.dumps(sys.stdin.read()))')" <<'PY'
import json, os, sys
b = json.load(open(sys.argv[1])); now = json.loads(sys.argv[2]); bad = []
if now != b["tailscale_serve_status"]: bad.append("tailscale serve status CHANGED:\n" + now)
dev = b["device_file"]; st = os.stat(dev) if os.path.exists(dev) else None
if (st.st_mtime if st else None) != b["device_mtime"]: bad.append("device registry mtime CHANGED: %s -> %s" % (b["device_mtime"], st.st_mtime if st else None))
print("\n".join(bad) or "tailscale serve status unchanged; device registry mtime unchanged")
sys.exit(1 if bad else 0)
PY
  note "POST-INSTALL OK"
  ;;
*) die "unknown step '$STEP' (use 0 1 2 3a 3 4 5 6)";;
esac
