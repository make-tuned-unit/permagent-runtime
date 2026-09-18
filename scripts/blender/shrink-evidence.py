#!/usr/bin/env python3
"""Shrink forum evidence renders: PNG -> WebP, and rewrite the docs that cite them.

Blender and Unreal both write PNG; a single 1500x1100 Cycles frame is 1.5-3 MB,
and roughly forty of them had accumulated under assets/world/forum/. Evidence
renders are read by humans in a diff, not sampled by a renderer, so they do not
need a lossless format at full resolution.

This script:
  * converts every PNG under assets/world/forum/ to WebP at max 1400 px on the
    long edge, quality 82, and deletes the PNG;
  * leaves docs/design/solar-forum/north-star.png as a PNG (it is the reference
    art direction image) but downscales it to max 1600 px;
  * rewrites `.png` -> `.webp` for the converted basenames in
    docs/design/solar-forum/**/*.md and docs/design/solar-forum/**/*.json.

Deliberately NOT touched:
  * assets/world/forum/third-party/ — CC0 source textures that Blender reads at
    build time. Converting those would change the shipped materials.
  * scripts/blender/*.py and scripts/unreal/*.py — the renderers keep writing
    PNG (Blender and Unreal cannot write WebP directly). Re-run this script
    after a render pass; it is idempotent.

Run from anywhere:  python3 scripts/blender/shrink-evidence.py [--dry-run]
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
EVIDENCE_DIR = ROOT / 'assets/world/forum'
SKIP_DIRS = {'third-party'}
DOCS_DIR = ROOT / 'docs/design/solar-forum'
NORTH_STAR = DOCS_DIR / 'north-star.png'

WEBP_MAX_EDGE = 1400
WEBP_QUALITY = 82
NORTH_STAR_MAX_EDGE = 1600
UI_ROOT = ROOT / 'ui/command-center'


# --------------------------------------------------------------------------
# Encoder backends, in order of preference. The first one that works is used
# for the whole run so every output is produced by the same encoder.
# --------------------------------------------------------------------------

class PillowBackend:
    """Blender's bundled Python, or any interpreter with Pillow installed."""

    name = 'Pillow'

    def __init__(self) -> None:
        from PIL import Image  # noqa: F401  (import is the availability probe)

    def _resize(self, image, max_edge: int):
        from PIL import Image
        if max(image.size) <= max_edge:
            return image
        scale = max_edge / max(image.size)
        size = (max(1, round(image.width * scale)), max(1, round(image.height * scale)))
        return image.resize(size, Image.LANCZOS)

    def to_webp(self, src: Path, dst: Path) -> None:
        from PIL import Image
        with Image.open(src) as image:
            self._resize(image, WEBP_MAX_EDGE).save(dst, 'WEBP', quality=WEBP_QUALITY, method=6)

    def downscale_png(self, src: Path, dst: Path) -> None:
        from PIL import Image
        with Image.open(src) as image:
            self._resize(image, NORTH_STAR_MAX_EDGE).save(dst, 'PNG', optimize=True)


SHARP_SCRIPT = """
// The helper is written to a temp directory, so `sharp` is required by
// absolute path rather than through node's resolution from that directory.
const [sharpPath, mode, src, dst, maxEdge, quality] = process.argv.slice(2);
const sharp = require(sharpPath);
const pipeline = sharp(src).rotate().resize({
  width: Number(maxEdge), height: Number(maxEdge),
  fit: 'inside', withoutEnlargement: true,
});
(mode === 'webp' ? pipeline.webp({ quality: Number(quality), effort: 6 })
                 : pipeline.png({ compressionLevel: 9 })).toFile(dst)
  .then(() => process.exit(0))
  .catch(error => { console.error(String(error)); process.exit(1); });
"""


class SharpBackend:
    """`sharp`, pinned as a devDependency of ui/command-center."""

    name = 'sharp (ui/command-center/node_modules)'

    def __init__(self) -> None:
        if not (UI_ROOT / 'node_modules/sharp/package.json').exists():
            raise RuntimeError('sharp is not installed; run `npm install` in ui/command-center')
        if shutil.which('node') is None:
            raise RuntimeError('node is not on PATH')
        self._script = Path(tempfile.mkdtemp(prefix='shrink-evidence-')) / 'sharp-resize.cjs'
        self._script.write_text(SHARP_SCRIPT)

    def _run(self, mode: str, src: Path, dst: Path, max_edge: int, quality: int) -> None:
        result = subprocess.run(
            ['node', str(self._script), str(UI_ROOT / 'node_modules/sharp'),
             mode, str(src), str(dst), str(max_edge), str(quality)],
            cwd=str(UI_ROOT), capture_output=True, text=True)
        if result.returncode != 0:
            raise RuntimeError(f'sharp failed on {src}: {result.stderr.strip() or result.stdout.strip()}')

    def to_webp(self, src: Path, dst: Path) -> None:
        self._run('webp', src, dst, WEBP_MAX_EDGE, WEBP_QUALITY)

    def downscale_png(self, src: Path, dst: Path) -> None:
        self._run('png', src, dst, NORTH_STAR_MAX_EDGE, 0)


class CwebpBackend:
    """Last resort: Google's `cwebp`, plus macOS `sips` for the PNG downscale."""

    name = 'cwebp + sips'

    def __init__(self) -> None:
        if shutil.which('cwebp') is None:
            raise RuntimeError('cwebp is not on PATH')
        self._sips = shutil.which('sips')

    def to_webp(self, src: Path, dst: Path) -> None:
        subprocess.run(
            ['cwebp', '-quiet', '-q', str(WEBP_QUALITY), '-resize', str(WEBP_MAX_EDGE), '0',
             str(src), '-o', str(dst)], check=True)

    def downscale_png(self, src: Path, dst: Path) -> None:
        if self._sips is None:
            raise RuntimeError('sips is unavailable; cannot downscale the north-star PNG')
        subprocess.run([self._sips, '-Z', str(NORTH_STAR_MAX_EDGE), str(src), '--out', str(dst)],
                       check=True, capture_output=True)


def pick_backend():
    errors = []
    for factory in (PillowBackend, SharpBackend, CwebpBackend):
        try:
            return factory()
        except Exception as error:  # noqa: BLE001 — probing availability
            errors.append(f'{factory.__name__}: {error}')
    raise SystemExit('No image encoder available.\n  ' + '\n  '.join(errors))


# --------------------------------------------------------------------------

def evidence_pngs() -> list[Path]:
    found = []
    for dirpath, dirnames, filenames in os.walk(EVIDENCE_DIR):
        dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
        for name in sorted(filenames):
            if name.lower().endswith('.png'):
                found.append(Path(dirpath) / name)
    return sorted(found)


def rewrite_references(basenames: set[str], dry_run: bool) -> list[Path]:
    """Point the evidence docs at the WebP files. Only the converted basenames
    are rewritten, so a `.png` that still exists (north-star, third-party) keeps
    its extension."""
    touched = []
    for path in sorted(DOCS_DIR.rglob('*')):
        if path.suffix not in {'.md', '.json'} or not path.is_file():
            continue
        text = path.read_text()
        updated = text
        for name in basenames:
            updated = updated.replace(name, name[:-4] + '.webp')
        if updated != text:
            touched.append(path)
            if not dry_run:
                path.write_text(updated)
    return touched


def main() -> int:
    dry_run = '--dry-run' in sys.argv
    backend = pick_backend()
    print(f'encoder: {backend.name}')

    pngs = evidence_pngs()
    before = sum(p.stat().st_size for p in pngs)
    after = 0
    converted = set()
    for png in pngs:
        webp = png.with_suffix('.webp')
        if dry_run:
            print(f'  would convert {png.relative_to(ROOT)}')
            converted.add(png.name)
            continue
        png_bytes = png.stat().st_size
        backend.to_webp(png, webp)
        webp_bytes = webp.stat().st_size
        after += webp_bytes
        png.unlink()
        converted.add(png.name)
        print(f'  {png.relative_to(ROOT)}: {png_bytes:,} -> {webp_bytes:,} bytes')

    north_star_before = north_star_after = 0
    if NORTH_STAR.exists():
        north_star_before = NORTH_STAR.stat().st_size
        if not dry_run:
            staged = NORTH_STAR.with_suffix('.png.tmp')
            backend.downscale_png(NORTH_STAR, staged)
            # Only keep the downscale if it actually helped; a second run of
            # this script must not slowly grow the reference image.
            if staged.stat().st_size < north_star_before:
                staged.replace(NORTH_STAR)
            else:
                staged.unlink()
        north_star_after = NORTH_STAR.stat().st_size

    touched = rewrite_references(converted, dry_run)

    summary = {
        'encoder': backend.name,
        'converted': len(converted),
        'pngBytesBefore': before,
        'webpBytesAfter': after,
        'northStarBytesBefore': north_star_before,
        'northStarBytesAfter': north_star_after,
        'docsRewritten': [str(p.relative_to(ROOT)) for p in touched],
    }
    print('SHRINK_EVIDENCE ' + json.dumps(summary))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
