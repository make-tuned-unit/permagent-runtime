# W5 round 2 receipt — 2026-10-12

**Blocked at host tool approval: no generated assets. Round-two spend $0.00 / $5.00; cap not reached.**

The configured FAL MCP's read-only account, discovery, schema and price calls worked. All five generation attempts returned exactly:

```text
MCP tool call requires approval, but approval policy is never
```

The block occurred before provider submission. No request IDs, output URLs, original files, hashes, byte counts or actual pixel dimensions exist for these attempts; ledger fields are null. This is not a new FAL HTTP/content-policy error. No permission escalation or alternate source was used.

| Request | Prepared model / seed | Outcome |
|---|---|---|
| Neutral leather, 512 px | Z-Image Turbo Tiling / 61012201 | Blocked; refused round-one prompt not reused |
| Walnut, 1024 px, 1 m | Z-Image Turbo Tiling / 61012202 | One further attempt, blocked |
| Fixed-layout foliage atlas, 1024 RGBA | Ideogram v3 Transparent / 61012203 | Blocked |
| Grey olive bark, 1024 px, 0.9 m | Z-Image Turbo Tiling / 61012204 | Blocked |
| Warm limestone, 1024 px, 1 m | Z-Image Turbo Tiling / 61012205 | Blocked |

Each failure was recorded and work moved to the next requested item. No second candidates or retries. The optional water-line decal was deferred under the shared generation blocker. No provider jobs or new inference charges occurred. The $0.25 preparation reservation was released; round one's $0.60 conservative exposure is unchanged.

Z-Image exposes actual both-axis tiling controls, used in the prepared recipes. Walnut's required 2×2 composite and visual inspection could not run without an output. No new tileability, alpha, layout or runtime acceptance is claimed. The existing `inspect_tiles.py` method is documented in the report for a future authorized run.

W4's atlas layout is preserved: top-left heart-leaf vine; top-right radial narrow grey-green cluster; bottom-left narrow olive-like sprig; bottom-right broad laurel-like sprig. Every 512² tile requires a bottom-centre root and 4-texel transparent gutters. Latest user instruction sets bark to 1024 rather than the receipt's 512. Keep all authored W3/W4 assets and the prior weave/walnut/brass decisions.

## Files and provenance

- `LEDGER.json`: five appended schema-v1 request records with exact model/prompt/params/seed, null originals, rights, $0 costs, error evidence and W3/W4 derivative proposals. Original four item records unchanged; appended round summaries distinguish totals.
- `REPORT.md`, `RIGHTS.md`: appended round-two outcome, method, commercial-use evidence and limits.
- `REQUESTS-round2.json`: exact recipes, with safety checks enabled and no prompt expansion.
- `evidence/round2-preflight.json`: account, discovery, live schemas and pricing.
- `evidence/round2-submission-errors.json`: exact five tool failures and attempted arguments.
- `evidence/round2-scope.json`: W4 specification verbatim, source hashes and size precedence.

Both selected endpoints carry FAL's commercial-use label: [Z-Image tiling](https://fal.ai/models/fal-ai/z-image/turbo/tiling), [Ideogram Transparent](https://fal.ai/models/fal-ai/ideogram/v3/generate-transparent). [Service terms](https://fal.ai/legal/terms-of-service) and [API terms](https://fal.ai/legal/api-services) apply. No generated output exists; these are preflight rights findings only.

All work is inside `assets/world/realism-20261012/`. No other-lane file was modified or staged. No cargo/npm/Blender/builds, commit or push. Coordinator commits by path.

## Staging

`git add assets/world/realism-20261012/` succeeded. The repository local exclude rule `evidence/` initially skipped the three new evidence JSON files; they were explicitly staged using `git add -f` on those three exact W5 paths. **8 W5 files staged** (3 modified, 5 added), all within this directory. No commit attempted. JSON validation passed, and all four historical ledger item records exactly match HEAD. Five new blocked records have null job/image fields and $0 costs. No seam/alpha check was possible without new images.
