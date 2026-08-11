#!/usr/bin/env bash
# Build the system-audio capture sidecar (ScreenCaptureKit) and stage it for
# bundling. See src-tauri/audiocap/main.swift for why capture is a Swift binary
# rather than inline Rust.
#
# macOS only. On other platforms this is a deliberate no-op: system-audio
# capture is unavailable there, `system_audio_available` returns false, and the
# meeting recorder offers mic-only rather than a toggle that cannot work.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="$ROOT/src-tauri/audiocap/main.swift"
OUT="$ROOT/src-tauri/audiocap/permagent-audiocap"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "[build-audiocap] not macOS — skipping system-audio sidecar"
  exit 0
fi

if ! command -v swiftc >/dev/null 2>&1; then
  # Not fatal: the app still builds and runs, just without the far side of a
  # meeting. Failing the whole desktop build over an optional capture helper
  # would be a worse trade.
  echo "[build-audiocap] swiftc not found — skipping (meeting capture will be mic-only)"
  exit 0
fi

echo "[build-audiocap] compiling $SRC"
swiftc -O -o "$OUT" "$SRC" \
  -framework ScreenCaptureKit -framework AVFoundation -framework CoreMedia

# Ad-hoc sign so the binary is launchable from inside the bundle; the app's own
# signing pass re-signs it in place during packaging.
codesign --force --sign - "$OUT" >/dev/null 2>&1 || true

echo "[build-audiocap] built $(du -h "$OUT" | cut -f1) → $OUT"
