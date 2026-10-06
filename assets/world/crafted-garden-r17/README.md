# Crafted commons table — r17 source candidate

The original walnut/brass table reference, exact prompt, FAL recipe, completed result and library registration are retained here. Hunyuan 3.1 Pro PBR/custom 40,000-face output was downloaded with verified HTTPS, exact length, GLB header and SHA256 validation. Original `raw/table.glb` is 47,752,084 bytes, SHA256 `9c1063d9c0cc85eda474c66cf01f6ab91b2f5ea0ad1ccdb218364170af7bc843`, locally retained and Git-ignored.

`processed/table-v1.glb` and packed `.blend` preserve the optimized editable result. Actual front and reverse renders are in `previews/`. Runtime is 1,523,268 bytes, 18,000 triangles, one shared primitive and three embedded 2048² WebP PBR maps. Root normalization preserves the existing 2.688 m diameter and 0.99 m top height. The carved pedestal now meets the floor; the original table independently covers loading/failure.

Raycasts across the usable tabletop measured 0.98715–0.98885 m. Cups and notebook now sit at 0.989 m instead of floating about 4 cm above the old table. The tabletop instrument remains in contact. Collision includes the actual full table height and width. Fifteen targeted tests pass, including real GLB decoding/instancing, transformed mesh bounds, tabletop prop support, terrain coverage, all 26 navigation routes and crowd avoidance. TypeScript passes.

**Native asset acceptance passed in r17.** Actual arrival and seated commons screenshots show the new table and grounded props. Later r18–r20 builds retain it. See `reports/world-production/native-r17-commons.png` and the production checkpoint. The broader north-star beauty goal remains open.

Cost estimate: $0.675 for this completed job; $5.775 combined completed FAL trials against the user's reported $25 purchase. This is not a billing-balance claim. No additional purchase. Account OAuth MCP supports the upload, generation and saved asset; no pasted key required.

Reproduce from the repository root using existing dependencies:

```sh
python3 assets/world/crafted-garden-r17/download_exports.py table
node assets/world/crafted-garden-r17/prepare_runtime.mjs table
```

The download command reuses verified local files and never submits a generation. `mesh-census.json` records geometry counts only: 124 ferns changed from 349,184 to 89,280 submitted triangles, about 74% less fern geometry. It does not claim a native FPS improvement.
