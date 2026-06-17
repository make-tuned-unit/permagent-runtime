# W1 Evidence — The Vertical Cave System Blockout

**Scope:** THE_CAVE_vision_bible.md §9 W1 — the vertical cave beneath the crown:
strata chambers (depth = time), the throat (the Brain Archive's descending abyss →
raw rock), the Mouth sightline aperture (geometry only; W4 owns its light),
survey-line footprints, and scaffold-state mount points.

**Status:** gray-geometry blockout. This is the **§9 W1 blockout checkpoint** — a
hard Jesse review gate. No detail pass begins until this blockout is approved.

Captured 2026-06-15 against the uncommitted evidence harness `worldevidence.html`
(mounts the real `WorldSceneContent` + a dev fill light, on its own vite port 5291),
Chrome 149 `headless=new --use-angle=metal` over raw CDP, dpr 2, viewport 1600×1000.
Camera parked via the FROZEN `window.__worldDebug`; perf read from the FROZEN
`shared/perf.ts` via `window.__worldPerf`.

> **Lighting note:** the committed scene deliberately leaves the cave dark — W4 owns
> the cave lighting and the Mouth daylight shaft (bible §9 W4). The review shots use
> an evidence-only fill light so the gray geometry reads; it exists only in the
> uncommitted harness, never in the scene.

## The strata map (areas/strata/strata.ts)

Depth is time (bible §2): the crown floor is y=0; the cave descends in −y; maturity
falls with depth (composed senate stone high → feral raw rock low). 1 unit = 1 metre.

| Stratum | id | y band | radius | maturity |
|---|---|---|---|---|
| The Verge | `verge` | −2 … −7 | 13 | 0.85 |
| Carved Halls | `carved` | −7 … −13 | 15 | 0.60 |
| Rough Work | `roughwork` | −13 … −19 | 16 | 0.35 |
| The Mining | `mining` | −19 … −25 | 14 | 0.12 |
| Bedrock | `bedrock` | −25 … −29 | 10 | 0.00 |

- **The throat** (`THROAT`, centre [0,0,9], biased toward the Brain threshold): a
  clean bored shaft with descending memory shelf-rings up top, giving way to a
  raw-rock funnel + teeth below `rawRockY = −19`. Entered on the abyss bridge.
- **The Mouth** (`MOUTH`, centre [0,64,0]): a blade-slit aperture cut through a rock
  shelf far above the dome, on the oculus sightline. Geometry only.

## The W1 → W2 mount-point module (areas/strata/anchors.ts)

`getMounts(stratum?)`, `getMount(id)`, `getMountsByKind(kind)` → `ScaffoldMount[]`.
Typed construction-state mounts (`survey | scaffold | banked | crane | built`) with
world-space `position`, `facing`, `footprint`, and a `progress` honesty bit. W2's
construction kit dresses each mount; survey-kind mounts drive the glowing footprints.
Distinct from the FROZEN `shared/anchors.ts` agent-seat registry.

## Perf (perf.json)

Whole cave ON vs OFF at an identical inside-cavern pose (both depth chunks loaded):
**+19 draw calls, +6,696 triangles** — well under the bible §8 zone budget (≤100 DC
total). Geometries flat (805–806) across the capture: no leak introduced. fps is
known-broken pending W4's fill-rate remediation (§8); the DC/triangle delta is the
renderer-independent headline.

Lazy depth chunks confirmed: `geometries` climbs 787 (crown only) → 800 (descent
chunk) → 803–806 (Mouth chunk) as the camera descends/looks up; both chunks
code-split in the production build (`CaveInterior-*.js`, `MouthChunk-*.js`).

## Shots

- `fly-00-crown-establishing` … `fly-05-mining-bedrock` — the descent fly-through,
  sinking down the throat axis through the five strata.
- `throat-looking-down` — on the bridge over the descending abyss: shelf-rings →
  raw-rock funnel.
- `mouth-from-deep-looking-up`, `mouth-up-oculus-sightline`, `mouth-aperture-closeup`
  — the looking-up Mouth-aperture sightline.
- `survey-lines-rough-work`, `survey-lines-mining` — the glowing survey-line
  footprints on bare rock where future wings rise.
