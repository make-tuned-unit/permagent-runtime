#!/bin/bash
# Generate the macOS + iOS app icons from ONE Icon Composer source.
#
# WHY THIS IS NOT A RESIZE SCRIPT ANY MORE
#
# macOS 26 (Tahoe) replaced the Big Sur icon convention. Under Big Sur an app
# baked its own rounded-squircle at 824x824 inside a 1024 canvas and the system
# drew it as-is. Under Tahoe the SYSTEM owns the tile: it draws the shape, the
# material, the shadow and the specular highlight, and the app supplies layers
# plus a background fill. Shipping a Big Sur-style icon on Tahoe means the
# system treatment lands on artwork that is ALREADY inset, so it renders
# visibly smaller than every native neighbour — reported 2026-08-04 as the icon
# being "considerably shorter" than the rest of the Dock, and correctly so.
#
# That is also why a plain full-bleed 1024 PNG looked closer to right before:
# full-bleed is what the modern system wants to mask. `Permagent.icon` is the
# supported way to say that, and it is the SAME source for both platforms —
# actool renders macOS and iOS from it, legacy fallbacks included.
#
# Source of truth:  assets/Permagent.icon   (Icon Composer package)
#   icon.json          — background fill + layer groups
#   Assets/Mobius.png  — the glyph ALONE, transparent, no background
#
# The glyph is extracted from public/PermagentIcon.png, which stays the master
# and is never rewritten. Do not reintroduce wordmark scaling, and do not bake a
# squircle or a margin into the artwork — the system does both now.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
MASTER="$ROOT/public/PermagentIcon.png"
ICONSRC="$ROOT/assets/Permagent.icon"
OUT="$ROOT/ui/desktop/src-tauri/icons"
IOS_DIR="$ROOT/ios/PermagentMobile/PermagentMobile"

if [ ! -f "$MASTER" ]; then
  echo "ERROR: master icon not found at $MASTER" >&2
  exit 1
fi
if [ ! -f "$ICONSRC/icon.json" ]; then
  echo "ERROR: $ICONSRC/icon.json missing — it defines the icon, it is not a build artifact" >&2
  exit 1
fi

mkdir -p "$ICONSRC/Assets" "$OUT"

# ── 1. Extract the glyph from the master ────────────────────────────────────
# The master is a flat known field (#0B1220) plus an additive glow. Recovering
# the glyph as `src - field`, unpremultiplied, keeps the glow's falloff exactly
# and composites correctly over whatever background icon.json specifies — a
# hard alpha threshold would leave a visible cut-out fringe around the ribbon.
MASTER="$MASTER" ICONSRC="$ICONSRC" python3 <<'PYEOF'
import os
from PIL import Image

src = Image.open(os.environ["MASTER"]).convert("RGB")
FIELD = (11, 18, 32)  # tokens.ts color.bg — the master's flat background
w, h = src.size
out = Image.new("RGBA", (w, h), (0, 0, 0, 0))
sp, op = src.load(), out.load()
for y in range(h):
    for x in range(w):
        r, g, b = sp[x, y]
        dr, dg, db = max(0, r - FIELD[0]), max(0, g - FIELD[1]), max(0, b - FIELD[2])
        a = max(dr, dg, db)
        if a:
            op[x, y] = (min(255, dr * 255 // a), min(255, dg * 255 // a),
                        min(255, db * 255 // a), a)
out.save(os.path.join(os.environ["ICONSRC"], "Assets", "Mobius.png"))
bbox = out.split()[-1].getbbox()
print(f"  glyph {bbox[2]-bbox[0]}x{bbox[3]-bbox[1]} extracted (transparent, no field)")
PYEOF

# ── 2. Compile for macOS ────────────────────────────────────────────────────
# actool emits the modern Assets.car AND a Permagent.icns pre-rendered with the
# system treatment. The app ships the .icns: Tauri's bundler has no asset-catalog
# support, and injecting the .car after bundling would break the ad-hoc
# signature and force a re-sign — which changes the embedded daemon's cdhash and
# walks straight into the Keychain-ACL boot hang. The .icns is the same render,
# so the Dock looks right; what it gives up is the dynamic dark/tinted variants,
# which are worth revisiting only when Tauri supports catalogs natively.
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
if ! xcrun actool --output-format human-readable-text --notices --warnings \
      --app-icon Permagent --compile "$STAGE" \
      --minimum-deployment-target 26.0 --platform macosx \
      --output-partial-info-plist "$STAGE/partial.plist" \
      "$ICONSRC" > "$STAGE/actool.log" 2>&1; then
  cat "$STAGE/actool.log" >&2
  exit 1
fi
if [ ! -f "$STAGE/Permagent.icns" ]; then
  echo "ERROR: actool produced no .icns — log follows:" >&2
  cat "$STAGE/actool.log" >&2
  exit 1
fi
cp "$STAGE/Permagent.icns" "$OUT/icon.icns"
echo "  macOS icon.icns (system-rendered from Permagent.icon)"

# ── 3. The PNG sizes tauri.conf.json still lists, rasterised from that icns ──
# They must come from the SAME render, or the Dock and the About panel disagree.
OUT="$OUT" MASTER="$MASTER" python3 <<'PYEOF'
import io, os, struct
from PIL import Image

out_dir = os.environ["OUT"]
data = open(os.path.join(out_dir, "icon.icns"), "rb").read()
_, total = struct.unpack(">4sI", data[:8])
off, best = 8, None
while off < total:
    kind, ln = struct.unpack(">4sI", data[off:off + 8])
    try:
        im = Image.open(io.BytesIO(data[off + 8:off + ln])).convert("RGBA")
        if best is None or im.size[0] > best.size[0]:
            best = im
    except Exception:
        pass
    off += ln
if best is None:
    raise SystemExit("could not read any representation out of icon.icns")

for name, sz in {"16x16.png": 16, "32x32.png": 32, "64x64.png": 64,
                 "128x128.png": 128, "128x128@2x.png": 256, "256x256.png": 256,
                 "256x256@2x.png": 512, "512x512.png": 512,
                 "512x512@2x.png": 1024, "icon.png": 1024}.items():
    best.resize((sz, sz), Image.LANCZOS).save(os.path.join(out_dir, name), "PNG")

# Windows draws no tile of its own, so the .ico keeps the full-bleed master.
Image.open(os.environ["MASTER"]).convert("RGBA").save(
    os.path.join(out_dir, "icon.ico"),
    sizes=[(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
)

body = best.split()[-1].point(lambda p: 255 if p >= 200 else 0).getbbox()
ratio = (body[3] - body[1]) / best.size[1]
# Tahoe's own margin, measured off native apps: System Settings and Xcode both
# sit at 0.797. A Big Sur-style icon lands at 0.805 and a raw full-bleed at
# 1.000; both are wrong here, in opposite directions.
if not 0.780 <= ratio <= 0.815:
    raise SystemExit(
        f"rendered body ratio {ratio:.3f} is outside the Tahoe band (0.78-0.815). "
        f"Native apps measure 0.797. Something changed in how actool renders "
        f"this icon — check icon.json before shipping."
    )
print(f"  PNG sizes + .ico rasterised (body ratio {ratio:.3f}, native apps 0.797)")
PYEOF

# ── 4. iOS consumes the SAME package ────────────────────────────────────────
# Xcode 26 compiles .icon directly and back-fills the legacy PNG sizes for the
# 17.0 deployment floor, so one source covers 17 through 26.
if [ -d "$IOS_DIR" ]; then
  rm -rf "$IOS_DIR/Permagent.icon"
  cp -R "$ICONSRC" "$IOS_DIR/Permagent.icon"
  echo "  iOS Permagent.icon (same source; Xcode renders it per OS version)"
fi

echo "Done. One Icon Composer source -> macOS .icns + PNGs + .ico, iOS .icon."
