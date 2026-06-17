# W4 — THE CAVE detail pass (atmosphere, light & camera)

Lane W4 re-spec against `THE_CAVE_vision_bible.md` §9. Built against the bible
geography and **staged** where W1's vertical strata + Mouth aperture have not yet
merged into `feature/world-v2` (the dependency is noted per file and in the PR).

Method (same as Phase 0 baseline, bible §10, and the prior W4 package):
Chrome `--headless=new --use-angle=metal` (real GPU, ANGLE Metal / Apple M1),
1600×1000 @ deviceScaleFactor 1.5 (dpr cap, bible §8), `shared/perf.ts`
`window.__worldPerf` probe, medians over reads 2s apart after a 9s settle. Driven
over CDP by `w4cave-capture.mjs` (UNTRACKED, like the prior lane's harness).

The capture drives the **DEV-only** `window.__worldSignals` / `__worldDev` /
`__worldTurn` harnesses, which write the **same real-signal stores** the daemon
writes (`atmosphere/worldSignals` ← `/api/brain/graph` + `/events`). They stand in
for a live Brain during capture; they do not bypass the honesty boundary. The
real binding shape is recorded verbatim in `capture-logs/capture-summary.json`
(`signals_dayone`, `signals_flowing`).

---

## The real signals bound (honesty law, bible §8)

| World element | Bound to (REAL, read-only) | Dormant when |
|---|---|---|
| **Spring presence + size** | `/api/brain/graph` `memories.length` (memory count) | `memoryCount === 0` → dry stone, no pool |
| **River + crown pools/groves** | same memory count (coarse log map, §4 "slow") | `memoryCount === 0` → no river, no groves |
| **River brightness / ripple** | `/events` `memory_added` (rainfall) + `librarian_describe_*` / `task_*` (recall proxy) | no events → channel settles to base |
| **Shadows-on-the-wall silhouettes** | `/api/brain/graph` `entities[]` (real entities, type-shaped) | empty Brain → bare wall, no silhouettes |

The world already consumes both transports — this lane adds **NO daemon code and
NO new endpoints**. `worldSignals.ts` polls `/api/brain/graph` (60s, matching
`useBrainData`) and listens on the `/events` WebSocket (the exact pattern in
`agents/stateSources.tsx`).

### Gaps filed (NOT faked, per §8)

1. **No `recall` event exists on `/events`.** "Recall is the river running" (§3)
   is currently approximated from `librarian_describe_*` + `task_*` activity.
   **Proposed daemon follow-up:** emit a `recall` event (snake_case) on `/events`
   when the Brain runs `recall_cascade`, so the river binds to the literal signal.
2. **No `entity_added` event.** The silhouette set refreshes on the 60s graph
   poll rather than instantly. **Proposed follow-up:** an `entity_added` event so
   a new first-fragment shadow appears the moment it's learned.

Both are read-only/event-only proposals; no daemon code was written in this lane.

---

## What landed (all in `atmosphere/` + `camera/`, staged against geography)

- **`atmosphere/MouthShaft.tsx`** — the Mouth's daylight: a distant pale-cold
  blade + halo far above the crown on the Antechamber sightline, a depth-graded
  fake-volumetric throat (bright near the aperture → dark in the depths), and the
  world's single cold key light. `cave-02` shows the blade above the dome.
- **`atmosphere/Water.tsx`** — spring (presence = memory count), river (flow =
  recall/rainfall), crown pools + groves revealed by Brain maturity. `cave-01`
  (dormant, empty Brain) vs `cave-05/06/07` (flowing, 48 entities / 140 memories).
- **`atmosphere/ShadowsOnTheWall.tsx`** — the day-one wall + a faint type-shaped
  silhouette per real entity; exports `TURN_CAMERA` for the turn. `cave-03` shows
  real silhouettes (a near-empty Brain casts few).
- **`atmosphere/worldSignals.ts`** — the read-only signal boundary (above).
- **`camera/TurnFraming.tsx`** — the §2 "Turn": Shift+T parks at the day-one
  wall frame (Mouth behind the lens), Shift+T again sweeps 180° to reveal the
  Mouth. reduceMotion → instant cut.
- **`atmosphere/ambience.ts`** — added `getVeinOpacity(i, now)`; **re-wired
  `areas/hall/HallStructure.tsx`** (W1 file, minimal diff, flagged for W1) so the
  colonnade veins brighten with the live working-agent count (§7). `cave-08`.

Tour-mode downward extension is deferred to W1's strata landing (noted in
`camera/TourMode.tsx`).

---

## Light census delta (the 32→12 trajectory)

The 32→12 scene-wide point-light reduction is the **cross-lane integration job**
(bible §8 item 4); this lane's mandate was to **REDUCE, not add**.

| | point lights (atmosphere's contribution) | shadow casters added |
|---|---|---|
| Before this pass | 1 (oculus uplight) | 0 |
| After this pass | 1 (Mouth cold key — **replaces** the oculus uplight) | 0 |

**Net atmosphere point-light delta: 0.** The Mouth's cold key light *replaces* the
old warm oculus uplight (the world's light now comes from the Mouth, §2 re-mean),
so the count does not grow. **No new shadow caster** — the 1-caster law stands
(the warm key directional). Live scene census from the capture
(`capture-summary.json`): `{ point: 26, directional: 2, spot: 0, shadowCasters: 1 }`
— identical with reduceMotion on.

---

## FPS table (full stack, this pass)

| State | fps (median) | notes |
|---|---|---|
| Dormant (empty Brain) | 9.8 | water + groves not rendered (honest) |
| Busy (3 working, veins reactive) | 9.7 | reactive veins add 0 draw calls (§7) |
| Full stack (water flowing + shadows + Mouth) | 8.9 | all cave systems mounted |

Same fill-rate-bound regime as the prior W4 package (the legacy ~1,340-DC
submission + the carved-out geometry leak dominate, bible §8 item 4 — all lanes,
integration pass). The cave systems are additive-blended quads, instanced
pools/groves (1 draw call each via `shared/instancing`), and ≤ 8 capped
silhouettes — they add no measurable cost over the existing bottleneck.

---

## reduceMotion pairs (bible §8)

`cave-10-reducemotion-t0` vs `cave-11-reducemotion-t6` are identical 6s apart:
with `permagent-reduce-motion=true` the water surface is static (no ripple/scroll),
the shadow silhouettes do not drift, the Mouth shaft holds constant opacity, and
the reactive veins are pinned to idle (`getVeinOpacity` reads the frozen ambience
level). The turn renders as an instant cut, not a sweep.

---

## Files

| Screenshot | Shows |
|---|---|
| `cave-01-dormant-overview.jpg` | Empty Brain: no spring/river/groves (honest dormancy) |
| `cave-02-mouth-shaft-dormant.jpg` | The Mouth's pale-cold daylight blade above the crown |
| `cave-03-shadows-on-wall-dayone.jpg` | Real entities as shadow silhouettes on the wall |
| `cave-04-the-turn-revealed.jpg` | The turn → overview (Mouth discovered) |
| `cave-05-flowing-overview.jpg` | Matured Brain: spring + river + crown groves |
| `cave-06-rainfall-river-running.jpg` | After `memory_added` pulses (rainfall → river brighter) |
| `cave-07-spring-and-pools.jpg` | Close on the spring pool + running channel |
| `cave-08-veins-busy.jpg` | Colonnade veins brightened by working agents (§7) |
| `cave-09-fullstack.jpg` | Full stack settled (census + fps read here) |
| `cave-10/11-reducemotion-t0/t6.jpg` | reduceMotion: water/shadows/shaft static (identical) |
| `capture-logs/capture-summary.json` | fps medians, light census, real signal-store shape |
