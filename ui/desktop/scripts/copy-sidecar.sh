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
#
# The onnxruntime version is NEVER hardcoded here. A sherpa-onnx bump changes
# which libonnxruntime.<version>.dylib it links against, and target/release/
# keeps the stale copies from previous builds — so a hardcoded name silently
# bundles the wrong ONNX Runtime and the daemon dies at launch with
# "Library not loaded: @rpath/libonnxruntime.<version>.dylib". (That shipped:
# the script pinned 1.24.4 after sherpa moved to 1.27.0.) Instead, ask the
# sherpa dylib itself what it needs, and stage that exact file under the
# stable name libonnxruntime.dylib so tauri.conf.json never needs a version.
DYLIBS=()
STABLE_ONNX="libonnxruntime.dylib"

SRC_SHERPA="$RELEASE_DIR/libsherpa-onnx-c-api.dylib"
if [ ! -f "$SRC_SHERPA" ]; then
  echo "ERROR: libsherpa-onnx-c-api.dylib not found at $SRC_SHERPA" >&2
  exit 1
fi

# Tauri externalBin maps binaries/ -> Contents/MacOS/, and tauri.conf.json's
# macOS.frameworks list places these in Contents/Frameworks/.
cp "$SRC_SHERPA" "$BIN_DIR/libsherpa-onnx-c-api.dylib"
DYLIBS+=("libsherpa-onnx-c-api.dylib")
echo "Dylib staged: $BIN_DIR/libsherpa-onnx-c-api.dylib ($(du -h "$BIN_DIR/libsherpa-onnx-c-api.dylib" | cut -f1))"

# Ask the linker record which onnxruntime sherpa actually wants.
ONNX_REF=$(otool -L "$SRC_SHERPA" | grep -o 'libonnxruntime[^[:space:]]*\.dylib' | head -1)
if [ -z "$ONNX_REF" ]; then
  echo "ERROR: could not determine the libonnxruntime version sherpa-onnx links against" >&2
  exit 1
fi
echo "sherpa-onnx requires: $ONNX_REF"

if [ ! -f "$RELEASE_DIR/$ONNX_REF" ]; then
  echo "ERROR: $ONNX_REF not found at $RELEASE_DIR/$ONNX_REF." >&2
  echo "       The daemon cannot launch without it. Rebuild the daemon so cargo" >&2
  echo "       re-stages the matching ONNX Runtime, then re-run this script." >&2
  exit 1
fi

# Stage under the stable, version-free name and make the dylib's own install
# name agree, so the referrer below resolves it.
cp "$RELEASE_DIR/$ONNX_REF" "$BIN_DIR/$STABLE_ONNX"
install_name_tool -id "@loader_path/$STABLE_ONNX" "$BIN_DIR/$STABLE_ONNX"
DYLIBS+=("$STABLE_ONNX")
echo "Dylib staged: $BIN_DIR/$STABLE_ONNX (from $ONNX_REF, $(du -h "$BIN_DIR/$STABLE_ONNX" | cut -f1))"

# --- Fix rpaths on the daemon binary ---
# In the app bundle, dylibs land in Contents/Frameworks/ (via tauri.conf.json
# macOS.frameworks). The daemon is in Contents/MacOS/. Standard macOS convention:
install_name_tool -add_rpath "@executable_path/../Frameworks" "$DST" 2>/dev/null || true
echo "Added @executable_path/../Frameworks rpath to daemon"

# Point sherpa-onnx at the staged copy. Both dylibs sit in Contents/Frameworks/,
# so @loader_path resolves there regardless of rpath.
SHERPA_DYLIB="$BIN_DIR/libsherpa-onnx-c-api.dylib"
install_name_tool -change \
  "@rpath/$ONNX_REF" \
  "@loader_path/$STABLE_ONNX" \
  "$SHERPA_DYLIB"
echo "Fixed sherpa-onnx -> $STABLE_ONNX reference to @loader_path"

# Verify the rewrite actually landed: a stale reference here is invisible until
# the daemon fails to launch on a user's machine.
if otool -L "$SHERPA_DYLIB" | grep -q "@rpath/libonnxruntime"; then
  echo "ERROR: libsherpa-onnx-c-api.dylib still carries an unresolved @rpath onnxruntime reference:" >&2
  otool -L "$SHERPA_DYLIB" | grep "libonnxruntime" >&2
  exit 1
fi

# install_name_tool invalidates the code signature; an unsigned dylib in a
# signed bundle gets the whole process SIGKILLed at exec with no diagnostic.
for dylib in "${DYLIBS[@]}"; do
  codesign --force --sign - "$BIN_DIR/$dylib" 2>/dev/null || true
done
codesign --force --sign - "$DST" 2>/dev/null || true

echo "Done. ${#DYLIBS[@]} dylibs staged alongside daemon."
