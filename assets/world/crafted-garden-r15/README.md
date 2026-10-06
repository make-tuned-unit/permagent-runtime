# Crafted garden materials and architecture — r15/r16

Original image references and exact prompts live in `references/`. Hunyuan 3.1 Pro PBR/custom 40,000-face outputs were downloaded over verified HTTPS, checked against provider lengths and GLB headers, and registered in the FAL library. Raw originals remain locally in ignored `raw/`; receipts and SHA256 values are retained. Re-running `download_exports.py bookcase column fern` verifies/reuses the originals without submitting jobs.

## Acceptance

- **Bookcase:** accepted after actual front/back GLB inspection and r15 native commons view. 1,804,516 bytes, 18,000 triangles, one primitive, 2048² embedded PBR maps. Three archive bays.
- **Column:** accepted after actual front/back inspection and r15 native view. 634,752 bytes, 12,000 triangles, one primitive, 1024² embedded PBR maps. Two front archive columns.
- **FAL fern:** rejected. The provider fused the leaflets into a solid disk and left a detached fragment. Original, optimized model, packed Blender file and both diagnostic views are retained as evidence; the model is never referenced by runtime.
- **Runtime fern:** curved instanced alpha-masked fronds from an original generated botanical image. Reuses one geometry/material for every placement, independently falls back to existing fern assets if loading fails.
- **Laurel foliage:** an original alpha-masked leaf supplies vein detail on the existing deterministic instanced planting. The previous solid leaf geometry remains its failure/loading fallback.

## Surfaces and support

CC0 photographed stone and timber maps come from [Monastery Stone Floor](https://polyhaven.com/a/monastery_stone_floor) by Amal Kumar and [Wood Floor](https://polyhaven.com/a/wood_floor) by Dimitrios Savva, under [Poly Haven CC0](https://polyhaven.com/license). `surface-provenance.json` records official metadata, source MD5/length verification, downloaded SHA256 values and runtime WebP hashes. Maps retain metre-scale world projection across merged geometry. The second pass lightens/desaturates the paving response and restores cool fill after r15 native shadows hid too much detail.

r16 replaces disconnected ellipsoid supports and stepped corner tiles with a shared rounded perimeter, clipped continuous paving/collision surface, stone cornice and tapered closed foundation. All apparent garden destinations remain on the same accessible footprint. Existing live robots and voice services remain intact.

## Verification and current state

Five actual-export checks pass, including embedded map decoding, PBR/UV/bounds, instancing, table contact and transformed collision containment. After the terrain change, all 26 arrival/station/seat routes, destination rejection and crowd avoidance pass. Two terrain tests verify rounded footprint coverage, exact elevation and a finite hull beneath the garden. Full TypeScript/Vite/native builds passed for r15 (34.1 s) and r16 (32.1 s); 638 dist assets plus both CSP-transformed HTML entries are verified in each build.

r15 was signed, staged, launched and visually reviewed at the commons. `reports/world-production/native-r15-commons.png` is the actual native screenshot. Lighting was too dark, triggering r16. Native control then lost visibility of every app window, including Finder. r16 is built/embedded-verified but has **not yet been staged or visually accepted**. The current review package ledger still describes r15. No performance claim is made for r15/r16; no production app installation occurred.

## Cost

Three new FAL jobs estimated at $0.675 each: $2.025 this folder, $5.10 combined completed trials through r15. The user authorized remaining credits from a reported $25 purchase. These are configuration-based estimates, not an independently verified billing balance. The completed table refinement is tracked separately under `../crafted-garden-r17/`.
