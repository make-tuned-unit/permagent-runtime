#!/bin/bash
# Generate macOS app icons from the Permagent logo.
# Produces all required sizes for .icns and places them in ui/desktop/src-tauri/icons/
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
LOGO="$ROOT/public/PermagentLogo.png"
OUT="$ROOT/ui/desktop/src-tauri/icons"

if [ ! -f "$LOGO" ]; then
  echo "ERROR: Logo not found at $LOGO" >&2
  exit 1
fi

mkdir -p "$OUT"

python3 << 'PYEOF'
import sys, os
from PIL import Image, ImageDraw

logo_path = sys.argv[1] if len(sys.argv) > 1 else os.environ.get("LOGO")
out_dir = sys.argv[2] if len(sys.argv) > 2 else os.environ.get("OUT")

BG_COLOR = (11, 18, 32)  # #0B1220
CORNER_RATIO = 0.225      # macOS Big Sur+ icon corner radius

logo = Image.open(logo_path).convert("RGBA")

def make_rounded_mask(size, radius):
    """Create a rounded-rectangle alpha mask."""
    mask = Image.new("L", (size, size), 0)
    draw = ImageDraw.Draw(mask)
    draw.rounded_rectangle([0, 0, size - 1, size - 1], radius=radius, fill=255)
    return mask

def make_icon(size):
    """Compose logo on dark rounded-rect background."""
    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    radius = int(size * CORNER_RATIO)

    # Draw dark background with rounded corners
    bg = Image.new("RGBA", (size, size), BG_COLOR + (255,))
    mask = make_rounded_mask(size, radius)
    canvas.paste(bg, (0, 0), mask)

    # Scale logo to ~65% of canvas width, preserving aspect ratio
    logo_w = int(size * 0.65)
    logo_h = int(logo_w * logo.height / logo.width)
    resized = logo.resize((logo_w, logo_h), Image.LANCZOS)

    # Center on canvas
    x = (size - logo_w) // 2
    y = (size - logo_h) // 2
    canvas.paste(resized, (x, y), resized)

    return canvas

# macOS .icns required sizes
sizes = {
    "16x16.png": 16,
    "32x32.png": 32,
    "64x64.png": 64,        # 32x32@2x
    "128x128.png": 128,
    "128x128@2x.png": 256,
    "256x256.png": 256,
    "256x256@2x.png": 512,
    "512x512.png": 512,
    "512x512@2x.png": 1024,
    "icon.png": 1024,       # App Store / web use
}

for name, sz in sizes.items():
    icon = make_icon(sz)
    path = os.path.join(out_dir, name)
    icon.save(path, "PNG")
    print(f"  {name} ({sz}x{sz})")

# Build .iconset for iconutil
iconset = os.path.join(out_dir, "AppIcon.iconset")
os.makedirs(iconset, exist_ok=True)

iconset_map = {
    "icon_16x16.png": 16,
    "icon_16x16@2x.png": 32,
    "icon_32x32.png": 32,
    "icon_32x32@2x.png": 64,
    "icon_128x128.png": 128,
    "icon_128x128@2x.png": 256,
    "icon_256x256.png": 256,
    "icon_256x256@2x.png": 512,
    "icon_512x512.png": 512,
    "icon_512x512@2x.png": 1024,
}

for name, sz in iconset_map.items():
    icon = make_icon(sz)
    icon.save(os.path.join(iconset, name), "PNG")

print("  Icon PNGs generated.")
PYEOF

# Convert iconset to .icns
ICONSET="$OUT/AppIcon.iconset"
if [ -d "$ICONSET" ]; then
  iconutil -c icns "$ICONSET" -o "$OUT/icon.icns"
  rm -rf "$ICONSET"
  echo "  icon.icns created"
fi

# Generate a simple .ico (Windows placeholder — 256x256 PNG inside ICO)
python3 -c "
from PIL import Image
import os
icon = Image.open(os.path.join('$OUT', '256x256.png'))
icon.save(os.path.join('$OUT', 'icon.ico'), format='ICO', sizes=[(256, 256)])
print('  icon.ico created')
"

echo "Done. Icons at $OUT"
ls -la "$OUT"
