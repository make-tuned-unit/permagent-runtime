# FAL World assets — 6 October 2026

**Downloads recovered and r14 visually verified in Permagent World Review.** Five completed object jobs cost an estimated **US$2.625**: four comparisons plus the selected bench. The separate panorama/world experiment adds $0.45 submitted, bringing the combined estimate to **$3.075** against the user's $25 purchase. These are endpoint price calculations, not a billing balance.

| Asset/model | Estimated cost | Outcome |
| --- | ---: | --- |
| Planter / TRELLIS.2 | $0.30 | Repaired r13 version retained; superseded by clearer Hunyuan relief |
| Instrument / TRELLIS.2 | $0.30 | Held: wavy panels, soft gauge, rear defects |
| Planter / Hunyuan 3.1 Pro | $0.675 | Downloaded, optimized, installed at 11 planters |
| Instrument / Hunyuan 3.1 Pro | $0.675 | Downloaded, optimized, installed on commons table |
| Bench / Hunyuan 3.1 Pro | $0.675 | Downloaded, optimized, installed at 5 benches |

## Working acquisition

`download_exports.py` downloads the existing completed outputs without creating jobs. The local system resolver fails for public names. The helper first tries normal DNS, then fetches a fresh DNS answer over verified HTTPS and supplies it to curl for that request only. TLS hostname/certificate validation stays enabled; system DNS and browser security settings are unchanged. The actual 51.2 MB planter, 46.6 MB instrument and 43.3 MB bench downloads succeeded. Each has a verified GLB header, exact expected length and SHA256 in `download-receipts.json`; reruns verify/reuse local files.

```sh
python3 assets/world/fal-trial-20261006/download_exports.py
```

Original Hunyuan files remain in `raw/`, locally preserved and gitignored to avoid another 141 MB repository copy. Completed-job URLs and checksums are committed, along with compact runtime GLBs and packed editable Blender files. Source URLs may expire; retrieve the existing job/library asset before considering any new generation. `upload_reference.py` similarly supports FAL's signed upload receipt without an API key; it deletes the private receipt after a successful upload. Signed upload URLs must not be committed.

## Delivered assets

| Runtime file | Bytes | Triangles | Shared primitive batches | Placements |
| --- | ---: | ---: | ---: | ---: |
| `planter-v2.glb` | 792,940 | 12,000 | 1 | 11 |
| `instrument-v1.glb` | 886,884 | 17,998 | 1 | 1 |
| `bench-v1.glb` | 821,192 | 18,000 | 1 | 5 |

These live in `ui/command-center/public/world/garden-crafted/`. Each includes 1024² WebP base-color, normal and metal/roughness maps. The provider's excessive specular-color factor was removed for standard PBR response; instrument normals were softened. Objects are normalized to the existing placement bounds. The instrument now meets the measured tabletop; rotated benches now have matching collision/navigation footprints. Original hero-kit objects independently cover loading or failure.

All three final exports were rendered from front and reverse before native review. The planter's continuous botanical relief and the bench's wood/brass/stone construction are accepted for this draft. The instrument rear is intact; some soft/wavy panel detail remains. These static props do not imply production-perfect manufactured geometry or a finished landscape.

`processed/*-hunyuan-v1.blend` are packed editable sources. Matching GLBs and `previews/*-hunyuan-v1-{front,back}.png` preserve reviewable exports. `references/` contains original images and exact prompts, generated with built-in image generation rather than extra FAL jobs. Earlier TRELLIS exports and repair scripts remain available for comparison.

## Reproduce and verify

Run from the repository root with existing dependencies:

```sh
node assets/world/fal-trial-20261006/prepare_runtime.mjs planter hunyuan
node assets/world/fal-trial-20261006/prepare_runtime.mjs instrument hunyuan
node assets/world/fal-trial-20261006/prepare_runtime.mjs bench hunyuan
/Applications/Blender.app/Contents/MacOS/Blender --background --factory-startup --python-exit-code 1 --python assets/world/fal-trial-20261006/render_diagnostic.py -- assets/world/fal-trial-20261006/processed/bench-hunyuan-v1.glb
npm --prefix ui/command-center test -- --run src/components/world/forum/craftedGardenAssets.test.ts src/components/world/forum/staticGardenInstances.test.ts src/components/world/forum/habitatMotion.test.ts
```

Preparation writes trial outputs only; promotion requires copying an accepted GLB and changing its runtime URL hash. Heavy operations run sequentially, retaining the 10 GiB free-disk floor.

Ten targeted tests pass: actual GLB/embedded textures/PBR, bounds, instancing, bench collision containment, instrument/table contact, all 26 navigation routes and crowd avoidance. Full TypeScript/Vite/native build and exact embedded-asset/package checks passed. Native screenshots are `reports/world-production/native-r14-{commons,reader}.png`. Installed `/Applications/Permagent.app` remains unchanged; the updated review app is separately staged and running.

Matched native seated samples at an inspector-reduced 1204×260 CSS viewport held 30 FPS in both r13 and r14. Calls changed 442→428; submitted triangles 7,754,005→8,067,031. This is not a full-size frame-time benchmark or a speedup claim. Details: `reports/world-production/native-r14-render-samples.json`.

`jobs.json`, `instrument-jobs.json`, `bench-job.json`, `*Result.json` and library receipts preserve provenance. Selection follows actual export comparison, not catalog rank. Pricing: [TRELLIS.2](https://fal.ai/models/fal-ai/trellis-2), [Hunyuan 3.1 Pro](https://fal.ai/models/fal-ai/hunyuan-3d/v3.1/pro/image-to-3d). Hunyuan estimates include PBR and custom 40,000-face output. The separate whole-world experiment is under `../fal-world-trial-20261006/`.
