# Cursor World session — 2026-10-07

Worker: Cursor (Composer). Branch: `world/production-20261006` @ b55bf214 (+ uncommitted). No commit. No native build. Disk after work: ~45 GiB free (above 10 GiB guard).

## Planned FAL asset paths (coordinate with Codex)

Drop completed downloads under `ui/command-center/public/world/fal-vista-20261007/`:

| Asset | Runtime path | Source ledger |
|---|---|---|
| Day panorama (primary) | `world/fal-vista-20261007/vista-day-4k.jpg` | candidate A/B/C → chosen 4K JPEG |
| Day panorama (fallback) | `world/fal-vista-20261007/vista-day-2k.jpg` | downscale of chosen candidate |
| Cliff/waterfall card | `world/fal-vista-20261007/cliff-waterfall.png` | LEDGER job index 4 |
| Floating islet + ring card | `world/fal-vista-20261007/floating-islet.png` | LEDGER job index 5 |
| Lake/mist strip (optional) | `world/fal-vista-20261007/lake-mist.png` | reroll if needed |
| Ring arc (optional) | `world/fal-vista-20261007/ring-arc.png` | reroll if needed |
| Vegetation atlas | `world/fal-vista-20261007/botanical.png` | LEDGER job index 3 (future planting pass) |

Manifest: `public/world/fal-vista-20261007/manifest.json`. Constants: `habitatVista.ts`. Sun alignment target: panorama sun at **u ≈ 0.25**, scenic ring at **u ≈ 0.60** (per `ASSET_INTEGRATION_NOTE_2026-10-07.md`).

Until files exist the scene uses **procedural golden-hour sky** (lower-hemisphere landform/water/haze fill — no black void) and **procedural horizon cards** with the same placements.

## Files changed (World scope only)

| File | Change |
|---|---|
| `habitatVista.ts` | **New.** Sun bearing, FAL asset paths, four horizon card placements outside walk bounds, validation helper. |
| `HabitatSky.tsx` | Day: equirect panorama dome (4096→2048→procedural fallback chain) + 2.5D horizon cards with parallax; night: Gaia Milky Way retained; celestial bodies in both modes; visible sun disc in day. |
| `HabitatScene.tsx` | `HabitatLighting`: golden-hour key (`#FFC882`), teal fill/shadow hemisphere, warm fog haze, sun position from `HABITAT_SUN_DIR`. |
| `ForumPostProcessing.tsx` | Day saturation 0.16 → 0.22 (warmer grade without orange filter). |
| `HabitatPlanting.tsx` | Terrace-edge ground cover, moss beds, extra hanging vine strands on rails. |
| `habitatVista.test.ts` | **New.** Asset path contract, card/walk-bound separation, sun bearing. |
| `public/world/fal-vista-20261007/manifest.json` | **New.** Drop-zone manifest for Codex. |

Preserved: existing dirty changes outside World files (calendar lane, Button, etc.) untouched.

## Behavior

### Day
- Warm golden-hour lighting: low NW sun, amber key, cool teal fill/shadows, exponential haze fog (`#C4D2DC`).
- Sky: tries FAL panorama; on missing/failed load falls back to procedural gradient with **continuous lower-hemisphere scenic fill** (terrain, water band, haze — not `#010208` void).
- Four distant horizon cards (cliff/waterfall, lake mist, ring arc, floating islet) outside all `HABITAT_SOLIDS`/walkable samples; slow camera parallax when reduced motion is **off**.
- Gas giant + moon remain visible (self-lit shaders).

### Night
- Unchanged Gaia EDR3 dome + existing night lighting/fog.

### Vegetation
- Additional instanced ground cover and vine detail at terrace rails; no new draw-call pattern (reuses `DetailBuilder` + `InstancedMesh`).

### Unchanged contracts
- Collision/walkability: `habitatFullTraversal.test.ts` still green.
- Hidden-tab pause, reduced-motion card parallax freeze, `GardenAssetBoundary` asset failure isolation.

## Checks run

| Check | Result |
|---|---|
| `npx tsc --noEmit` (ui/command-center) | exit 0 |
| `habitatVista.test.ts` | 4 passed |
| `habitatArrival.test.ts` | 4 passed |
| `habitatRichness.test.ts` | 1 passed |
| `habitatFullTraversal.test.ts` | 1 passed (~73 s) |
| `ForumPostProcessing.test.tsx` | 2 passed |

Not run: full vitest suite, `vite build`, native staging, visual capture.

## Remaining visual acceptance (Codex / Jesse — native app)

1. **Panorama drop:** After Codex places `vista-day-4k.jpg`, confirm seam at wrap, horizon level, sun at u≈0.25 matches scene key light, no mipmap shimmer.
2. **No black void:** Arrival, overview, walk, and orbit views — terrace edge must show continuous landform/water/haze/sky in day mode (compare to `docs/design/solar-forum/north-star.png` and `reports/world-production/review/`).
3. **Horizon cards:** With FAL PNGs loaded, verify ring/waterfall/islet read at wide views; cards must not intersect walk mesh or block routes.
4. **Golden hour:** Day reads warm immediately; white robots stay legible (not orange-washed).
5. **Night regression:** Gaia sky, lantern pools, gas-giant AgX/bloom chain (`probe-black-rect.mjs` if post chain touched further).
6. **Reduced motion:** Cards static; head-bob/camera ambient unchanged; hidden-tab pause still holds.
7. **Performance:** Frame time at 1440×900 with new dome + four cards — watch for fill-rate regression.

## Risks / notes

- FAL panorama jobs in LEDGER are 21:9 PNGs; runtime expects **2:1 equirect JPEG** at the paths above — Codex must convert/crop before drop.
- `lake-mist` and `ring-arc` cards have procedural fallbacks only until PNGs arrive.
- Botanical atlas (job 3) not yet wired into `HabitatPlanting` — deferred to Codex vegetation block.
- Visual verification **not claimed**; procedural fallback is intentionally conservative until FAL assets land.
