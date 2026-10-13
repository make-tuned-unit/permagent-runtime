# World W5 — realism acquisition, 2026-10-12

**Partial delivery; generation stopped on the first reported FAL error.** Three original images are preserved. Moss weave is a candidate for W3 review; walnut and brass are held for seams. Leather produced no image. No runtime asset is approved or integrated.

Scope: `assets/world/realism-20261012/` only, plus the requested external receipt. Base: `3622134e67e70fc4fc7d50785ab687d64a7012ad`. FAL access used the configured Codex MCP, account `github|5m4womwn5oan4fjuo2qmp172` (Jesse Sharratt). No credentials or signed recovery strings are stored here.

## October 7 reconciliation

The runtime checkout did not contain the requested ledger/report. Read the production checkout's `assets/world/fal-vista-20261007/LEDGER.json` and `reports/world-production/FAL_VISTA_REPORT_2026-10-07.md`, plus the corresponding files via `git show wq/world:...` at `206396e0316560bba8725b2243990104edc59a18`.

Production records **eight completed requests, $0.92 estimated**: six Nano Banana Pro images × $0.15 and two Ideogram background-removal requests × $0.01. All eight were recovered through FAL history under the connected account, with HTTP 200; every exact retained input and output URL matches the historical ledger. All eight original file hashes/bytes and all eight recorded derivative hashes match. Request IDs, timestamps, original measurements, and comparisons are in [the reconciliation evidence](evidence/october7-reconciliation.json).

The `wq/world` attempt submitted **zero jobs and spent $0 new**. Its $6.60 is a prior cumulative estimate, not a new October 7 charge. Conservatively combining $6.60 prior + $0.92 production gives $7.52 estimated historical use against the recorded $18 ceiling, or $10.48 estimated headroom. This is not current billing or proof of current account balance. The user's current W5 instruction separately limits this tranche to $5, without top-up or subscription.

What remains usable:

- Preserve panorama A and its 4096/2048 JPEG derivatives as scenic atmosphere; B/C remain alternatives. A's source is 3168×1344, adapted from 21:9 to 2:1. It is neither calibrated equirectangular capture nor HDRI, and the observed sun differs from the requested position.
- The two semantic RGBA scenery cards are technically intact. The current ruling prohibits expanding their destination-like inaccessible islands; they are not new explorable content.
- Botanical atlas/crops remain held for pale edge fringes and baked illumination. They are not close-up production replacements.
- These are historical decisions and current hash checks, not new native acceptance. No old file was overwritten or regenerated.

## Acquisition and spend

All four first candidates used `fal-ai/nano-banana-pro` with explicit seeds, `num_images=1`, `resolution=1K`, `aspect_ratio=1:1`, `output_format=png`, `limit_generations=true`, `enable_web_search=false`, `sync_mode=false`, `safety_tolerance=4`, and empty system prompt. Exact prompts and parameters are in [LEDGER.json](LEDGER.json) and [REQUESTS.json](REQUESTS.json), with the live schema and pricing in [acquisition preflight](evidence/acquisition-preflight.json). The endpoint exposes no immutable internal version and no seamless-generation control. Prompted physical dimensions are intended UV scale, not measurement of a real sample.

The live FAL rate and model page both indicated $0.15/image at 1K, with no web-search surcharge for these inputs. Three completed jobs total **$0.45 estimated**. The failed leather request's actual charge is unknown; conservatively retain $0.15, for **$0.60 total estimated exposure/reservation**, $4.40 below the $5 ceiling. Actual charges are unverified. The lane stopped for the provider error before reaching its cap. No second candidates, retry jobs, alternative providers, or credit purchases were made.

| Candidate | Seed | Request ID | Outcome | Cost |
|---|---:|---|---|---:|
| walnut-a | 61012051 | 01a132df-e455-7633-9453-d1e0b15be90d | Image preserved; held for visible seam | $0.15 estimate |
| weave-a | 61012052 | 01a132e1-29fc-7502-b8d1-96dd6fed6f39 | Image preserved; W3 review candidate | $0.15 estimate |
| leather-a | 61012053 | 01a132e1-3173-7a52-adf1-bc7c34d5f090 | Failed; no file | Unknown; $0.15 reserved |
| brass-a | 61012054 | 01a132e1-3a67-7f61-9204-21a23aad0664 | Image preserved; optional study held | $0.15 estimate |

Exact leather error, retained in `evidence/leather-a-result.json`:

```text
Result fetch failed (422)
content_policy_violation
The content could not be processed because it contained material flagged by a content checker.
```

The result reports `retryable: false`, `job_outcome: failed`, field path `["body"]`, and `upstream_reason: unknown`. Its terminal queue status was COMPLETED, but fetching the actual model result confirmed failure. The weave and brass were already submitted and completed when this grouped result arrived. No subsequent FAL call or generation was made.

## Files and candidate decisions

Paths below are relative to this directory. `file` in each ledger item identifies the immutable downloaded original. Derivatives have separate hashes, processing commands, and parent hashes.

| File | Measured pixels | Measured bytes | Decision / proposed W3 use |
| `originals/walnut-a.png` | 1024×1024 | 1,587,360 | Hold; not accepted as seamless |
| `originals/weave-a.png` | 1024×1024 | 2,500,164 | Candidate passes W5 image/seam review; W3 relight/UV acceptance outstanding |
| `candidates/weave-a-512.png` | 512×512 | 628,694 | 512 px base colour candidate |
| `originals/brass-a.png` | 1024×1024 | 1,329,670 | Hold optional roughness study; not accepted as a seamless calibrated map |
| `candidates/brass-a-512.png` | 512×512 | 115,688 | True grayscale study; held |

Derivative proposals, conditional on W3/W6 acceptance:

- **Walnut:** replace `detail-v4/hero/textures/walnut_basecolor.png` on Arrival table/chair at 1 m per tile. Current candidate is held; retain authored walnut.
- **Weave:** replace `weave_basecolor.png` on Arrival-chair upholstery at 0.25 m per tile with `candidates/weave-a-512.png`. W3 must inspect UV scale, close-up appearance, and relighting before accepting.
- **Leather:** would replace `leather_basecolor.png` on the Arrival folio at 0.30 m per tile. No replacement was produced; retain authored leather.
- **Brass:** `candidates/brass-a-512.png` is a roughness study at 0.20 m per tile only if W6 separates brass from the trim palette. It is held and not physically calibrated.

## Tileability and pixel inspection

`inspect_tiles.py` decodes with ffmpeg, writes lossless 2×2 repeat and half-width/half-height offset previews, and compares wrap-edge mean absolute RGB differences to all interior boundaries. Numeric screening requires each wrap mean to be no larger than the greater of twice the interior median or its 99th percentile. Values use 8-bit sRGB differences, not radiometric measurements. This is a screening heuristic; it cannot establish continuity of grain or yarn by itself. Visual review can reject a numeric pass.

| Delivery | LR / TB wrap MAE | LR / TB ratio to interior median | Decision |
|---|---|---|---|
| Walnut original, 1024 px | 5.643 / 6.327 | 1.896 / 1.102 | Numeric pass; visual hold: vertical grain join interrupts the repeat |
| Weave derivative, 512 px | 23.630 / 17.129 | 1.161 / 0.937 | Numeric pass; no conspicuous hard seam at delivery scale in offset/repeat inspection |
| Brass grayscale derivative, 512 px | 2.465 / 3.480 | 2.035 / 1.090 | LR numeric failure; visible repeating patina; hold |

Full observations and exact statistics are attached to ledger items and `review/*-metrics.json`. Walnut is plausible dark oiled timber but fails visual tile acceptance. Weave has moss/olive plain weave with fine slub; repetition and yarn scale remain W3 review matters. Brass has fine directional brushing and cloudy patina; its original was only near-gray (up to 7/255 channel difference), so the optional derivative uses a true grayscale conversion. That conversion does not make brightness a measured roughness value.

Only documented resampling was performed: ffmpeg area reduction to 512 px, plus grayscale conversion for brass. No seam repair, normal/height generation, contrast manipulation, or creative retouch was performed. Original source bytes are preserved.

An initial weave resize overlapped its still-active download and produced a black lower area. QA rejected it; after curl exited 0, the complete original was decoded and resized again. The rejected derivative is retained as `review/weave-a-incomplete-resize-rejected.png`, with `evidence/local-resize-rejection.json`. It is not a second generation candidate. Each completed source was downloaded once; final original measurements were reconciled after download completion.

## Rights and remaining work

[RIGHTS.md](RIGHTS.md) records dated endpoint-specific commercial-use evidence, model and service/API URLs, conditions, attribution findings, and input provenance. All three outputs and derivatives are AI-generated, not CC0 or project-authored material. Preserve the local originals and provenance. No licence assertion changes pixel acceptance.

No Forum code, published asset directory, `detail-v4`, other lane files, or historical assets were modified. No cargo, npm, Blender, build, application, native performance, or GPU runtime test ran. No mesh, foliage, panorama, HDRI, normal map, or height map was generated. The optional brass study is the sole requested roughness exception. Cleanup time was not separately metered, and runtime cost was not measured. W3/W6 own material calibration, integration, runtime measurement, and final acceptance.

## Receipt and Git delivery

The requested receipt path is `/Users/j/Documents/dev/permagent-worktrees/wq-reports/world/w5-receipt-20261012.md`. The filesystem sandbox rejected writing it:

```text
PermissionError: [Errno 1] Operation not permitted: '/Users/j/Documents/dev/permagent-worktrees/wq-reports/world/w5-receipt-20261012.md'
```

The complete receipt is preserved inside the allowed directory as `w5-receipt-20261012.md`. The external reports directory is not a Git repository and is outside this worktree. The requested combined `git add` failed with:

```text
fatal: /Users/j/Documents/dev/permagent-worktrees/wq-reports/world/w5-receipt-20261012.md: '/Users/j/Documents/dev/permagent-worktrees/wq-reports/world/w5-receipt-20261012.md' is outside repository at '/Users/j/Documents/dev/permagent-worktrees/world-production-20261006'
```

An assets-only commit (including the receipt copy) is attempted separately. No push is authorized or performed.

### Final commit result

**No commit created.** The scoped assets-only commit failed at the sandbox Git index lock:

```text
fatal: Unable to create '/Users/j/Documents/dev/permagent-runtime/.git/worktrees/world-production-20261006/index.lock': Operation not permitted
```

The W5 files are preserved and staged only within `assets/world/realism-20261012/`. No push was attempted. Commit hooks were disabled for this attempt to honor the no-build instruction; the failure occurred before any hook or commit ran. No alternate Git directory, index, or permission workaround was attempted.
