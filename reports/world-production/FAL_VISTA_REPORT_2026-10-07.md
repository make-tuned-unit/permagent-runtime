# FAL vista asset lane — 2026-10-07

Status at 00:54 UTC: all eight jobs completed, downloaded through verified HTTPS and inspected. Candidate A and two semantic RGBA cards are delivered for runtime. Botanical crops remain archival. No source integration edits owned by this lane.

Generation endpoint: `fal-ai/nano-banana-pro`, $0.15/image base × six 2K outputs = $0.90 estimate. Official documentation says 4K doubles the rate; these requests used 2K. Two dependent semantic matte jobs used `fal-ai/ideogram/remove-background` at $0.01/image = $0.02 estimate. Total new lane estimate is **$0.92**. Actual final charges are unverified. Lane estimate ceiling is $5; session ceiling is $18. No duplicate or retry submissions.

## Runtime contract

- `ui/command-center/public/world/vista/golden-hour-panorama-4096.jpg`
- `ui/command-center/public/world/vista/golden-hour-panorama-2048.jpg`
- `ui/command-center/public/world/vista/cards/cliff-waterfall.png`
- `ui/command-center/public/world/vista/cards/floating-islet.png`

`lake-mist.png` and `ring-arc.png` are not separately generated in this batch. The floating-islet image includes a ring. The botanical atlas request was accepted before the coordinator reprioritized cards and will be preserved, without further botanical spend.

Three panorama candidate prompts ask for a continuous horizon, complete scenic lower hemisphere, ring, floating landforms, waterfalls and lakes, gas giant and moons, tiny craft, warm sun at image u=0.25, teal shadows. Those are requested properties and remain unverified until images complete and are inspected. Supported output ratio is 21:9, so the 2:1 runtime files are documented derivatives, not a native calibrated equirectangular capture. Wrap seam and pole distortion must be checked before acceptance.

## Panorama inspection and delivery

All three original PNGs are preserved with 4096×2048 and 2048×1024 JPEG derivatives. Candidate B has a visually rich inhabited ring-world but its overhead ring crosses the top and edge boundaries, increasing spherical distortion risk. Candidate C has a beautiful river basin but a flattened horizontal ring reads less clearly as a landmark. Candidate A is selected for its strong self-contained upright ring and clean sky edges.

Request `01a118d4-d655-7e12-b8af-ac822b4df790` produced a 3168×1344 PNG, 6,557,573 bytes. The scene clearly contains a monumental planted ring, cascading waterfalls/lakes, floating garden islands, gas giant/moons, and tiny craft. Warm amber sun and teal water/shadows match the requested palette. No text, foreground railing, people or black void are visible. The lower hemisphere is densely scenic. The style is a detailed cinematic environment illustration with some idealized foliage.

`prepare-panorama.cjs` resamples to 2:1, blends 96 pixels at each meridian edge symmetrically, and converges 60 polar rows. The wrap-centered preview was visually inspected: mountains, haze and sky remain continuous without a hard black seam. This is an environment-map adaptation, not a calibrated equirectangular photograph; spherical shape distortion and native visual performance still need in-app review.

Delivered primary JPEG: 1,323,490 bytes; fallback: 420,835 bytes. Hashes and original acquisition provenance are in the ledger and `candidate-a/derivatives.json`. Sun was generated at actual image u≈0.350, y≈0.470 (not the requested u=0.25). Recommended sphere Y rotation is −2.006 radians to align with world sunlight direction approximately (−0.977, 0.094, 0.191). `world/vista/manifest.json` records this metadata; it is not required by the current runtime.

Receipts, exact inputs, seeds, routing decision IDs and estimates are in `assets/world/fal-vista-20261007/LEDGER.json`. Downloads reuse `assets/world/fal-world-trial-20261006/download_world.py` with verified HTTPS and a 25 GiB disk floor. Signed storage query strings are never committed. Cards must contain actual alpha transparency before delivery.

## Cards, botanical archive and validation

Both generated card originals are visually strong: cliff/waterfall has lush woodland, a pale dome, dangling roots and mist; floating-islet has an inhabited planted ring, ivory spires, tiny craft and waterfalls. The first local luminance mattes were rejected because dark shadows became holes. Two FAL semantic removal jobs produced RGBA replacements, preserving shadows and detailed foliage. Both were composited over light teal and inspected. The runtime card files are 2048×1374, 4,798,584 and 4,166,932 bytes respectively. Actual fully transparent pixel fractions are 48.1% and 58.2%; image metadata verifies four channels and alpha. Soft waterfall mist remains semi-transparent. Exact hashes are in `cards-final.json` and the ledger.

The botanical model returned a white-background grid despite the black-background prompt. Six grid-free crops (vines, canopy, fern, flowers, moss, alternate vines) are archived under `assets/world/fal-vista-20261007/botanical/cutouts/`. They have true alpha, but thin pale edges remain and they are not integrated or shipped in the runtime directory. Existing foliage is the accepted baseline for this pass.

Acquisition scripts, panorama preparation, threshold-matte trials and final semantic-card publishing all exited 0. PNG magic, byte hashes, dimensions, alpha-channel presence and visual composites were checked. No app source was changed and no frontend/native build was run by this lane. Native perspective, frame time, traversal and reduced-motion verification belong to the coordinator. Assets are left uncommitted for the coordinator's shared-worktree commit; no unrelated staged files were touched.

The two semantic matte jobs are `01a118fe-0692-7851-b4e1-34a55d784113` and `01a118fe-06eb-7511-9fe3-0ae4a8d9f573`. All earlier original generation request IDs and canonical queue handles remain in the ledger. No pending generation remains.
