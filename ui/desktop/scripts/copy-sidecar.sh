#!/bin/bash
# Copy the permagentd binary to the Tauri sidecar location with the
# architecture-specific name that Tauri expects.
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

SRC="$ROOT/target/release/permagentd"
DST="$TAURI_DIR/binaries/permagentd-$TRIPLE"

if [ ! -f "$SRC" ]; then
  echo "ERROR: daemon binary not found at $SRC" >&2
  echo "Run: cargo build --release --bin permagentd" >&2
  exit 1
fi

cp "$SRC" "$DST"
chmod +x "$DST"
echo "Sidecar copied: $DST ($(du -h "$DST" | cut -f1))"
