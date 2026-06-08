#!/bin/bash
# Copy the permagentd binary AND its required dylibs to the Tauri sidecar
# location. Fixes rpaths so the bundled daemon can find its dylibs at
# @executable_path (inside Contents/MacOS/ in the app bundle).
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
SRC="$RELEASE_DIR/permagentd"
BIN_DIR="$TAURI_DIR/binaries"
DST="$BIN_DIR/permagentd-$TRIPLE"

if [ ! -f "$SRC" ]; then
  echo "ERROR: daemon binary not found at $SRC" >&2
  echo "Run: cargo build --release --bin permagentd" >&2
  exit 1
fi

mkdir -p "$BIN_DIR"

# --- Copy the daemon binary ---
cp "$SRC" "$DST"
chmod +x "$DST"
echo "Sidecar copied: $DST ($(du -h "$DST" | cut -f1))"

# --- Bundle required dylibs alongside the binary ---
# The daemon dynamically links libsherpa-onnx-c-api.dylib (STT via sherpa-onnx),
# which in turn depends on libonnxruntime (sherpa-onnx's bundled ONNX Runtime).
# These must travel with the binary in the app bundle.
DYLIBS=()

for dylib in libsherpa-onnx-c-api.dylib libonnxruntime.1.24.4.dylib; do
  src_dylib="$RELEASE_DIR/$dylib"
  if [ -f "$src_dylib" ]; then
    # Tauri externalBin maps binaries/ -> Contents/MacOS/. To place dylibs
    # alongside the daemon, we use Tauri resources (Contents/Resources/).
    # But since externalBin only copies the named binary, we stage dylibs
    # as additional externalBin entries by giving them the triple suffix.
    # Alternative: copy into binaries/ and add to externalBin in tauri.conf.json.
    #
    # Simplest: place in binaries/ without triple suffix. Tauri copies
    # externalBin items to Contents/MacOS/. We add them to tauri.conf.json.
    cp "$src_dylib" "$BIN_DIR/$dylib"
    DYLIBS+=("$dylib")
    echo "Dylib staged: $BIN_DIR/$dylib ($(du -h "$BIN_DIR/$dylib" | cut -f1))"
  else
    echo "WARNING: $dylib not found at $src_dylib — daemon may crash at runtime" >&2
  fi
done

# Also copy the unversioned symlink for onnxruntime
if [ -f "$RELEASE_DIR/libonnxruntime.dylib" ]; then
  cp "$RELEASE_DIR/libonnxruntime.dylib" "$BIN_DIR/libonnxruntime.dylib"
  DYLIBS+=("libonnxruntime.dylib")
fi

# --- Fix rpaths on the daemon binary ---
# In the app bundle, dylibs land in Contents/Frameworks/ (via tauri.conf.json
# macOS.frameworks). The daemon is in Contents/MacOS/. Standard macOS convention:
install_name_tool -add_rpath "@executable_path/../Frameworks" "$DST" 2>/dev/null || true
echo "Added @executable_path/../Frameworks rpath to daemon"

# Fix the sherpa-onnx dylib's internal reference to onnxruntime.
# Both dylibs will be in Contents/Frameworks/, so @loader_path resolves there.
SHERPA_DYLIB="$BIN_DIR/libsherpa-onnx-c-api.dylib"
if [ -f "$SHERPA_DYLIB" ]; then
  install_name_tool -change \
    "@rpath/libonnxruntime.1.24.4.dylib" \
    "@loader_path/libonnxruntime.1.24.4.dylib" \
    "$SHERPA_DYLIB" 2>/dev/null || true
  echo "Fixed sherpa-onnx -> onnxruntime reference to @loader_path"
fi

echo "Done. ${#DYLIBS[@]} dylibs staged alongside daemon."
