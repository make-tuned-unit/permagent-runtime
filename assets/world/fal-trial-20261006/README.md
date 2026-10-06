# First FAL World asset trial — 6 October 2026

Two references, two 3D models per reference, four completed jobs. Estimated FAL generation cost **US$1.95** from current published endpoint pricing, below the announced US$10 first-round cap. This is a price calculation, not a billing-ledger balance. No generation after the four comparisons.

| Asset/model | Estimated cost | Outcome |
| --- | ---: | --- |
| Planter / TRELLIS.2 | $0.30 | Repaired, optimized and integrated at the eleven existing planter positions |
| Instrument / TRELLIS.2 | $0.30 | Held: wavy panels, soft gauge and rear surface defects |
| Planter / Hunyuan 3.1 Pro | $0.675 | Completed and saved in FAL library; GLB download pending |
| Instrument / Hunyuan 3.1 Pro | $0.675 | Completed and saved in FAL library; GLB download pending |

Hunyuan previews look more detailed, but that is not a full-mesh quality verdict. Inspect their reverse sides and optimize them before changing the runtime selection. The recommendation tool misclassified the task; the model comparison instead used the live catalog and official endpoint schemas. `jobs.json` and `instrument-jobs.json` preserve original submission receipts, while `*Result.json` preserves completion responses. `library-receipts.json` identifies all four registered FAL library assets. Source URLs may expire; never resubmit a completed job merely to recover its file.

## Runtime selection

`ui/command-center/public/world/garden-crafted/planter-v1.glb` is the repaired TRELLIS planter: **821,176 bytes**, **6,503 triangles**, two primitive batches shared by eleven instances, embedded 1024² color and metal/roughness textures. The original hero-kit planter remains the loading/error fallback. Ferns, foliage, character rigs and layout/collision coordinates are unchanged.

The raw export had disconnected, jagged soil triangles. A deterministic mesh repair removes the internal fragments and adds a level soil cap. The stone bowl and brass ribs retain the generated material. Normalization places the base at Y=0 and fits a conservative radial envelope within the existing 1.1 m collision footprint. The fern stem enters the soil. The irregular underside of the generated rim remains visible in an empty-pot close-up; this is a restrained static prop, not a perfect manufactured product model.

`processed/planter-v1.blend` is an editable, packed Blender source of the accepted mesh. `processed/planter-v1.glb` matches the runtime export. `previews/planter-v1-{front,back}.png` are renders of the final GLB itself. `previews/hero-kit-v3b-{front,back}.png` provide a baseline comparison. References and exact image prompts are under `references/`; reference images were made with built-in image generation, not an additional FAL image job.

## Reproduce and verify

Run from the repository root with the lane's installed dependencies:

```sh
node assets/world/fal-trial-20261006/prepare_runtime.mjs planter trellis
/Applications/Blender.app/Contents/MacOS/Blender --background --factory-startup --python-exit-code 1 --python assets/world/fal-trial-20261006/render_diagnostic.py -- assets/world/fal-trial-20261006/processed/planter-v1.glb
npm --prefix ui/command-center test -- --run src/components/world/forum/craftedGardenAssets.test.ts src/components/world/forum/staticGardenInstances.test.ts src/components/world/forum/habitatMotion.test.ts
```

The preparation script writes only trial outputs; copy a visually accepted export into `public/world/garden-crafted/` and update the URL hash when promoting a changed asset. Hunyuan preparation is available once its existing completed GLB is acquired. The TRELLIS soil repair is specific to this axis-aligned reference/export and must not be applied blindly to other geometry.

Eight targeted tests pass, including actual embedded texture decoding, generated mesh bounds, collision containment, batching, all 26 navigation routes and crowd avoidance. Native build/package/visual status is recorded in `reports/world-production/CHECKPOINT.md` and `review.json`. A mesh census is not an FPS benchmark; a new native performance sample remains pending.

## Acquisition limits

The CLI cannot resolve `v3b.fal.media`. Safari downloaded the two TRELLIS exports earlier; those files were copied into this workspace through Finder. Subsequent Chrome access failed because the browser could not verify its required administrative policy. No security controls were bypassed. The two Hunyuan outputs remain available by their exact completed-job links and saved library record IDs. Download them after access recovers, without spending more credits.

Pricing sources: [TRELLIS.2](https://fal.ai/models/fal-ai/trellis-2), [Hunyuan 3.1 Pro](https://fal.ai/models/fal-ai/hunyuan-3d/v3.1/pro/image-to-3d). TRELLIS uses 1024 resolution. Each Hunyuan estimate includes base generation, PBR and custom 40,000-face output.
