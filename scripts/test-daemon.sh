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
# Usage:
#   scripts/test-daemon.sh                    # all daemon lib + integration tests
#   scripts/test-daemon.sh voice::speakable   # filter, as passed to cargo test
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
PROFILE_DIR="${TARGET_DIR:-${CARGO_TARGET_DIR:-$ROOT/target}}/debug"
FILTER="${1:-}"

if [[ "$(uname -s)" != "Darwin" ]]; then
  # Only macOS needs the signing dance; elsewhere plain cargo works.
  exec cargo test -p permagent-daemon --lib --tests ${FILTER:+"$FILTER"}
fi

echo "[test-daemon] building test binary…"
cargo build -p permagent-daemon --tests

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

echo "[test-daemon] running tests${FILTER:+ (filter: $FILTER)}"
exec cargo test -p permagent-daemon --lib --tests ${FILTER:+"$FILTER"}
