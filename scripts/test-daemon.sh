#!/bin/bash
# Run permagent-daemon unit tests on macOS.
#
# Why this script exists
# ----------------------
# `cargo test -p permagent-daemon --lib` dies with SIGKILL and no output on
# macOS — even with a test name that matches nothing, so it is the test BINARY
# failing to start, not any test. Two separate causes stack up:
#
#   1. No rpath. The binary needs libsherpa-onnx-c-api.dylib, which cargo leaves
#      in target/<profile>/ but does not add to the binary's search path:
#
#        dyld: Library not loaded: @rpath/libsherpa-onnx-c-api.dylib
#              Reason: no LC_RPATH's found
#
#   2. Unsigned dylibs. Once dyld CAN find them, macOS kills the process for an
#      invalid signature — SIGKILL, exit 137, zero diagnostic output. This is
#      the same failure class ui/desktop/scripts/copy-sidecar.sh warns about
#      ("an unsigned dylib gets the whole process SIGKILLed at exec with no
#      diagnostic"); cargo re-links the dylibs unsigned on every rebuild, so
#      signing has to happen after the build, every time.
#
# Scope: `--lib --tests`, matching what CI runs for this package.
#
# It ran `--lib` alone until 2026-08-14, while already BUILDING `--tests` two
# lines above — so the integration suites in crates/goose-server/tests/ were
# compiled here and then never executed. A green run reported "589 passed" and
# read as full daemon coverage; the tests/ directory was not in it, and a
# librarian change that broke hardening_tests passed this gate and failed CI.
# A gate whose scope is narrower than it appears is worse than no gate.
#
# Profile
# -------
# Debug by default, matching CI. A `--release` argument (or
# TEST_DAEMON_PROFILE=release) runs the same gate against the release profile
# instead — which is what the disk doctrine needs when target/debug has been
# reclaimed and only the warm release tree exists: building a whole second
# debug tree to run one filter costs ~100 GB this machine does not have.
# The profile decides BOTH the cargo flag and the directory the dylibs are
# signed in; hardcoding "/debug" while cargo built into "/release" signed zero
# dylibs and pointed DYLD_LIBRARY_PATH at nothing, which is the exact silent
# SIGKILL this script exists to prevent.
#
# Usage:
#   scripts/test-daemon.sh                          # debug, lib + integration
#   scripts/test-daemon.sh voice::speakable         # debug, filtered
#   scripts/test-daemon.sh --release                # release, all
#   scripts/test-daemon.sh --release voice::speak   # release, filtered
#   TEST_DAEMON_PROFILE=release scripts/test-daemon.sh voice::speak
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# Cargo's target dir can be redirected by CARGO_TARGET_DIR *or* by a
# `[build] target-dir` in any .cargo/config.toml cargo merges from an ancestor
# directory — which is how the git worktrees here share one warm target instead
# of growing a ~40 GB copy each. Guessing "$ROOT/target" silently signed zero
# dylibs and pointed DYLD_LIBRARY_PATH at a directory that did not exist, so the
# test binary was SIGKILLed with an invalid signature and no hint why. Ask cargo
# where it actually builds; fall back to the guess only if that fails.
TARGET_DIR="$(cargo metadata --no-deps --format-version 1 2>/dev/null \
  | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"

# Profile first, filter second. The profile may arrive as an argument or in the
# environment; anything else in $1 is a cargo test filter, exactly as before.
PROFILE="${TEST_DAEMON_PROFILE:-debug}"
case "${1:-}" in
  --release|release) PROFILE="release"; shift ;;
  --debug|debug)     PROFILE="debug";   shift ;;
esac
case "$PROFILE" in
  debug)   CARGO_PROFILE_FLAG=() ;;
  release) CARGO_PROFILE_FLAG=(--release) ;;
  *) echo "[test-daemon] unknown profile '$PROFILE' (use debug or release)" >&2; exit 2 ;;
esac

PROFILE_DIR="${TARGET_DIR:-${CARGO_TARGET_DIR:-$ROOT/target}}/$PROFILE"
FILTER="${1:-}"

if [[ "$(uname -s)" != "Darwin" ]]; then
  # Only macOS needs the signing dance; elsewhere plain cargo works.
  exec cargo test -p permagent-daemon "${CARGO_PROFILE_FLAG[@]:-}" --lib --tests ${FILTER:+"$FILTER"}
fi

echo "[test-daemon] building test binary ($PROFILE)…"
cargo build -p permagent-daemon "${CARGO_PROFILE_FLAG[@]:-}" --tests

echo "[test-daemon] ad-hoc signing dylibs in $PROFILE_DIR"
shopt -s nullglob
signed=0
for dylib in "$PROFILE_DIR"/libsherpa-onnx-c-api.dylib "$PROFILE_DIR"/libonnxruntime*.dylib; do
  codesign --force --sign - "$dylib" >/dev/null 2>&1 && signed=$((signed + 1))
done
shopt -u nullglob
if [[ "$signed" -eq 0 ]]; then
  echo "[test-daemon] WARN: no dylibs found in $PROFILE_DIR — build may not have run" >&2
fi
echo "[test-daemon] signed $signed dylib(s)"

# DYLD_LIBRARY_PATH substitutes for the missing rpath. It is exported for the
# test binary only; nothing here changes how the app itself is built or shipped.
export DYLD_LIBRARY_PATH="$PROFILE_DIR${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}"

echo "[test-daemon] running tests in $PROFILE${FILTER:+ (filter: $FILTER)}"
exec cargo test -p permagent-daemon "${CARGO_PROFILE_FLAG[@]:-}" --lib --tests ${FILTER:+"$FILTER"}
