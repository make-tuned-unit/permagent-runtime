#!/usr/bin/env bash
# Build the Apple Foundation Models sidecar and leave it where the provider
# looks for it. See crates/goose/applefm/main.swift for why on-device inference
# is a Swift binary rather than inline Rust.
#
# macOS only. Everywhere else this is a deliberate no-op: the provider probes
# for the binary at runtime, reports `unavailable` when it is missing, and the
# caller falls back to whatever backend it was using before. A missing sidecar
# is never a build break and never an error surfaced to a user.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="$ROOT/crates/goose/applefm/main.swift"
OUT="$ROOT/crates/goose/applefm/permagent-applefm"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "[build-apple-fm] not macOS — skipping on-device inference sidecar"
  exit 0
fi

if ! command -v swiftc >/dev/null 2>&1; then
  # Not fatal. Bulk work keeps going to whichever backend it used before;
  # failing the whole build over an optional cost optimisation would be a
  # worse trade.
  echo "[build-apple-fm] swiftc not found — skipping (bulk work stays on the current backend)"
  exit 0
fi

# FoundationModels shipped in the macOS 26 SDK. An older SDK cannot compile
# this, and that is fine — same no-op as above. `xcrun` is guarded because
# /usr/bin/swiftc exists as a stub even where the developer tools are not
# installed, so passing the check above is not proof of a working toolchain.
SDK_PATH="$(xcrun --show-sdk-path 2>/dev/null || true)"
if [[ -z "$SDK_PATH" || ! -d "$SDK_PATH/System/Library/Frameworks/FoundationModels.framework" ]]; then
  echo "[build-apple-fm] no SDK with FoundationModels — skipping"
  exit 0
fi

echo "[build-apple-fm] compiling $SRC"
# Weak-linked so the binary still launches on a pre-26 system, where it answers
# every request with `os_too_old` instead of dying on first use.
#
# A compile failure is printed but not propagated. The requirement this serves
# is that no configuration of this optional helper can break the build: without
# the binary the provider reports `sidecar_missing` and every caller falls back.
# The error text is still on screen for whoever just edited main.swift.
if ! swiftc -O -parse-as-library -o "$OUT" "$SRC" \
  -Xlinker -weak_framework -Xlinker FoundationModels; then
  echo "[build-apple-fm] !! compile FAILED — on-device inference will be unavailable"
  echo "[build-apple-fm]    bulk work stays on the current backend; not failing the build"
  exit 0
fi

# Ad-hoc sign so the binary is launchable from inside a packaged bundle; the
# app's own signing pass re-signs it in place during packaging.
codesign --force --sign - "$OUT" >/dev/null 2>&1 || true

echo "[build-apple-fm] built $(du -h "$OUT" | cut -f1) → $OUT"
