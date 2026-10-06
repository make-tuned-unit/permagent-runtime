# Asset checkpoint and next action — 2026-10-06

Status: **documented drafts; visual acceptance incomplete**. Parent explicitly froze further asset geometry edits/renders/downloads pending runtime visual review or supplied resources. No asset heavy process remains active.

Current kit v2: `exports/hero-kit.glb`, 4,214,312 bytes, 124,094 exported triangles, 64 mesh primitives/draws. SHA256 `99a1ad1632e4a8adc8820767f6b684321a37b945692d3aa019e1c9d3a7f57139`. All individual and mono exports total 8,436,236 bytes. `validation.json` contains measured exports, embedded-buffer checks, normals, roots/pivots, double-sided foliage and payload ceiling checks.

Editable sources: `source/hero-kit.blend` is the v1 complete library; `source/robot-v2.blend` supersedes its robot. The mono kit includes robot v2. Source scripts and README document reconstruction. Texture-map enrichment remains unfinished and is excluded from current reproduction.

Inspected versioned diagnostics: `previews/robot-v2-export-diagnostic-512.png` and `previews/foliage-v1-export-diagnostic-512.png`. These are CPU Cycles four-sample 512px renders of the actual individual GLB exports, not runtime acceptance. Robot construction improved but remains intermediate. Canopy is too sparse with repeated horizontal leaf plates; shrub/fern lack volume. Do not label this kit hero-approved.

## Botanical exemplar candidate

Official asset URLs:

- https://polyhaven.com/a/potted_plant_01 — approximately 96K triangles, 1.4m tall per official page; authored by Rico Cilliers.
- https://polyhaven.com/a/potted_plant_04 — small zebra haworthia candidate; authored by James Ray Cock. Parent research records approximately 6K triangles and .3m height, not yet independently measured locally.
- https://polyhaven.com/license — official CC0 license information.
- https://polyhaven.com/our-api — official public API information; default endpoints free for personal, academic and commercial use without login/API key.
- https://api.polyhaven.com/files/potted_plant_01 — official file metadata endpoint linked from asset page.
- https://api.polyhaven.com/files/potted_plant_04 — official candidate file metadata endpoint.

Authorized acquisition scope: **one** self-contained reference plant, **1K glTF plus every buffer/image dependency**, **at most 15 MB total downloaded**. No 8K bundles (advertised 264.31 MB for plant01, 27.72 MB for plant04), no tree_small_02 (parent identifies roughly 5M triangles), no paid use/API bypass/unknown-size downloads. No import/render authorized during integration tests.

Blocker: direct local `curl` requests to `api.polyhaven.com` failed DNS resolution (`curl: (6) Could not resolve host: api.polyhaven.com`). The web tool could read official asset/license/API pages but returned the file endpoint as application/json with no readable payload. Therefore 1K URLs and complete dependency byte totals have **not** been verified. No candidate asset was downloaded. Do not chase alternate routes or proxies.

Next authorized action when access/resources are supplied: read official file metadata, sum 1K glTF and dependency byte sizes, acquire only if complete total is at most 15 MB, preserve source URLs/license/authorship/hashes and actual byte counts. Wait for explicit parent clearance before Blender import, geometry edits, or new diagnostic renders. Integrating worker may copy the existing kit/source scripts into its branch and run parent-approved focused runtime checks.
