#!/bin/bash
# Stage the `permagent` CLI as a Tauri sidecar, alongside the daemon.
#
# Why this exists: the Build tab's "Permagent" button runs
# `permagent run --recipe permagent-coding --interactive` in a project
# terminal. That command has been byte-identical since the button was added,
# and no build ever produced a `permagent` binary for the bundle — Contents/
# MacOS/ held only permagent-app and permagentd. The button therefore only
# worked on a machine that already had a hand-built CLI on PATH. This script
# is the delivery half of the fix; terminal.rs is the other half (it puts
# Contents/MacOS on the spawned shell's PATH so the shell can find what we
# stage here).
#
# Tauri strips the target triple when it bundles an externalBin, so the staged
# name must carry it: binaries/permagent-<triple> lands as Contents/MacOS/
# permagent.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TAURI_DIR="$SCRIPT_DIR/../src-tauri"
ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

ARCH="$(uname -m)"
case "$ARCH" in
  arm64)  TRIPLE="aarch64-apple-darwin" ;;
  x86_64) TRIPLE="x86_64-apple-darwin" ;;
  *)      echo "ERROR: unsupported arch $ARCH" >&2; exit 1 ;;
esac

RELEASE_DIR="$ROOT/target/release"
SRC="$RELEASE_DIR/permagent"
BIN_DIR="$TAURI_DIR/binaries"
DST="$BIN_DIR/permagent-$TRIPLE"

if [ ! -f "$SRC" ]; then
  echo "ERROR: CLI binary not found at $SRC" >&2
  echo "Run: cargo build --release -p permagent-cli --bin permagent" >&2
  exit 1
fi

mkdir -p "$BIN_DIR"

# Unlike the daemon, the CLI links nothing but system frameworks (verified with
# `otool -L`: AppKit, CoreFoundation, Vision, Metal, libSystem …). There is no
# sherpa-onnx / onnxruntime pair to travel with it, so no dylib staging and no
# rpath rewriting — nothing here rewrites the Mach-O, so whatever signature the
# source carries survives the copy intact and needs no re-signing here.
#
# That source signature is now the Developer ID one, not cargo's ad-hoc one:
# `npm run build:cli` runs scripts/sign-dev-binaries.sh straight after cargo,
# to keep the designated requirement stable across rebuilds so the keychain
# grant sticks (docs/operations/DEV_SIGNING.md). It makes no difference to what
# ships. Tauri re-signs every externalBin with the bundle identity, the bundle
# entitlements and the hardened runtime as the last step of `tauri build`, so
# the staged copy's signature is replaced wholesale either way.
cp "$SRC" "$DST"
chmod +x "$DST"
echo "CLI sidecar copied: $DST ($(du -h "$DST" | cut -f1))"

# A sidecar Tauri cannot execute is the same defect this script fixes, one
# level down — so prove the staged file actually runs before calling it staged.
if ! "$DST" --version >/dev/null 2>&1; then
  echo "ERROR: staged CLI at $DST does not execute (--version failed)" >&2
  exit 1
fi
echo "Staged CLI reports: $("$DST" --version 2>&1 | tr -d '\n')"
