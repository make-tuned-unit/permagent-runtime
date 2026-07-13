#!/bin/bash
# Bundle the local Whisper dictation model into the Tauri app so voice dictation
# works in the shipped .app with ZERO setup — no first-run download, works fully
# offline. The mic must never 503 on a fresh install.
#
# Produces (git-ignored, ~78MB):
#   src-tauri/resources/whisper/whisper-base-q8_0.gguf
#     → bundled at Contents/Resources/whisper/whisper-base-q8_0.gguf
#
# On first run the daemon copies this into ~/.permagent/models/ and sets
# LOCAL_WHISPER_MODEL=base (see routes/dictation.rs provision_handler), after
# which the existing loader (dictation::whisper) finds it by filename.
#
# Model: whisper "base", q8_0 GGUF — the size/quality sweet spot for dictation.
# Keep the URL + filename in sync with WHISPER_MODELS "base" in
# crates/goose/src/dictation/whisper.rs.
set -euo pipefail

MODEL_URL="https://huggingface.co/oxide-lab/whisper-base-GGUF/resolve/main/whisper-base-q8_0.gguf"
MODEL_FILE="whisper-base-q8_0.gguf"
# Size floor (bytes) — guards against a truncated download or an HTML error page
# saved as the model. The real file is ~78MB; 60MB is a safe lower bound.
MIN_BYTES=$((60 * 1024 * 1024))

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TAURI_DIR="$SCRIPT_DIR/../src-tauri"
OUT="$TAURI_DIR/resources/whisper"
DEST="$OUT/$MODEL_FILE"

# Portable file size (macOS `stat -f%z`, Linux `stat -c%s`).
filesize() { stat -f%z "$1" 2>/dev/null || stat -c%s "$1" 2>/dev/null || echo 0; }

mkdir -p "$OUT"

if [ -f "$DEST" ] && [ "$(filesize "$DEST")" -ge "$MIN_BYTES" ]; then
  echo "Whisper model already staged: $DEST ($(du -sh "$DEST" | cut -f1))"
  exit 0
fi

echo "Downloading $MODEL_URL"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
curl -fsSL "$MODEL_URL" -o "$TMP/$MODEL_FILE"

SIZE="$(filesize "$TMP/$MODEL_FILE")"
if [ "$SIZE" -lt "$MIN_BYTES" ]; then
  echo "ERROR: downloaded model is only $SIZE bytes (< $MIN_BYTES) — likely a truncated or error response" >&2
  exit 1
fi

mv "$TMP/$MODEL_FILE" "$DEST"
echo "Whisper model staged: $DEST ($(du -sh "$DEST" | cut -f1))"
