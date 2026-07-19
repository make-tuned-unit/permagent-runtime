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
# Pinned SHA-256 of the model — MUST match the "base" entry in
# crates/goose/src/dictation/whisper.rs. The bundled model ships inside the
# signed .app, so whatever we stage here is what every user runs: verify the
# download, never just its size.
MODEL_SHA256="7073e51db7ab02b38cc4fceeac39adc2d7a19beb98badf66aa708f4f0ac71aa9"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TAURI_DIR="$SCRIPT_DIR/../src-tauri"
OUT="$TAURI_DIR/resources/whisper"
DEST="$OUT/$MODEL_FILE"

# Portable SHA-256 (macOS `shasum -a 256`, Linux `sha256sum`).
sha256_of() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    sha256sum "$1" | awk '{print $1}'
  fi
}

mkdir -p "$OUT"

if [ -f "$DEST" ]; then
  if [ "$(sha256_of "$DEST")" = "$MODEL_SHA256" ]; then
    echo "Whisper model already staged and verified: $DEST ($(du -sh "$DEST" | cut -f1))"
    exit 0
  fi
  echo "Staged whisper model failed SHA-256 verification — re-downloading" >&2
  rm -f "$DEST"
fi

echo "Downloading $MODEL_URL"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
curl -fsSL "$MODEL_URL" -o "$TMP/$MODEL_FILE"

GOT_SHA256="$(sha256_of "$TMP/$MODEL_FILE")"
if [ "$GOT_SHA256" != "$MODEL_SHA256" ]; then
  echo "ERROR: whisper model SHA-256 mismatch" >&2
  echo "  expected: $MODEL_SHA256" >&2
  echo "  got:      $GOT_SHA256" >&2
  echo "Refusing to bundle an unverified model." >&2
  exit 1
fi

mv "$TMP/$MODEL_FILE" "$DEST"
echo "Whisper model staged and verified: $DEST ($(du -sh "$DEST" | cut -f1))"
