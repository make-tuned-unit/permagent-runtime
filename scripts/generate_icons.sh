#!/bin/bash
# Generate macOS + iOS app icons from the square Permagent icon source.
#
# Source is public/PermagentIcon.png: a full-bleed 1024×1024 Mobius on the brand
# background (#0B1220), already framed so the symbol fills ~80% of the width.
# Earlier this script scaled the WIDE wordmark (1280×621) to 65% of the tile,
# which left the logo small and adrift in a large square — the "wrong size
# square" this replaces. Keep the source square and full-bleed; do not
# reintroduce wordmark scaling here.
#
#   - macOS: rounded-squircle with transparent corners (macOS does not mask
#     app icons itself; the shape must be baked in).
#   - iOS:   full-bleed square, RGB with NO alpha (the App Store rejects icons
#     that carry a transparency channel).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ICON="$ROOT/public/PermagentIcon.png"
OUT="$ROOT/ui/desktop/src-tauri/icons"
IOS="$ROOT/ios/PermagentMobile/PermagentMobile/Assets.xcassets/AppIcon.appiconset/AppIcon.png"

if [ ! -f "$ICON" ]; then
  echo "ERROR: Square icon source not found at $ICON" >&2
  exit 1
fi

mkdir -p "$OUT"

ICON="$ICON" OUT="$OUT" IOS="$IOS" python3 << 'PYEOF'
import os
from PIL import Image, ImageDraw

icon_path = os.environ["ICON"]
out_dir = os.environ["OUT"]
ios_path = os.environ["IOS"]

CORNER_RATIO = 0.2237  # Big Sur+ squircle approximation
master = Image.open(icon_path).convert("RGBA")

def rounded(size):
    base = master.resize((size, size), Image.LANCZOS)
    mask = Image.new("L", (size, size), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        [0, 0, size - 1, size - 1], radius=int(size * CORNER_RATIO), fill=255
    )
    out = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    out.paste(base, (0, 0), mask)
    return out

# macOS sized PNGs
sizes = {
    "16x16.png": 16, "32x32.png": 32, "64x64.png": 64, "128x128.png": 128,
    "128x128@2x.png": 256, "256x256.png": 256, "256x256@2x.png": 512,
    "512x512.png": 512, "512x512@2x.png": 1024, "icon.png": 1024,
}
for name, sz in sizes.items():
    rounded(sz).save(os.path.join(out_dir, name), "PNG")
    print(f"  {name} ({sz}x{sz})")

# .iconset for iconutil
iconset = os.path.join(out_dir, "AppIcon.iconset")
os.makedirs(iconset, exist_ok=True)
for name, sz in {
    "icon_16x16.png": 16, "icon_16x16@2x.png": 32, "icon_32x32.png": 32,
    "icon_32x32@2x.png": 64, "icon_128x128.png": 128, "icon_128x128@2x.png": 256,
    "icon_256x256.png": 256, "icon_256x256@2x.png": 512, "icon_512x512.png": 512,
    "icon_512x512@2x.png": 1024,
}.items():
    rounded(sz).save(os.path.join(iconset, name), "PNG")

# Windows .ico
Image.open(os.path.join(out_dir, "icon.png")).convert("RGBA").save(
    os.path.join(out_dir, "icon.ico"),
    sizes=[(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
)

# iOS: full-bleed square, NO alpha.
if os.path.isdir(os.path.dirname(ios_path)):
    master.convert("RGB").resize((1024, 1024), Image.LANCZOS).save(ios_path, "PNG")
    print("  iOS AppIcon.png (1024x1024, no alpha)")
PYEOF

# Compose the .icns from the iconset, then discard the scratch iconset.
iconutil -c icns -o "$OUT/icon.icns" "$OUT/AppIcon.iconset"
rm -rf "$OUT/AppIcon.iconset"
echo "Done. macOS icons + .icns + .ico and iOS AppIcon regenerated."
