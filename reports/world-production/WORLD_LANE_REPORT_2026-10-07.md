# World lane report — 2026-10-07

## Block 1 — arrival and hero presentation

Status: complete.

Files changed:

- `ui/command-center/src/components/world/forum/habitatLayout.ts`
- `ui/command-center/src/components/world/forum/HabitatScene.tsx`
- `ui/command-center/src/components/world/forum/HeroGardenKit.tsx`
- `ui/command-center/src/components/world/forum/habitatArrival.test.ts`

Changes: the compact arrival camera is now eye-height and beside the host/table, the overview target is `[0, -3, -12]`, and the authored reverse-arrival framing contract is recorded. Hero robot surface copies use physical ceramic materials with clearcoat/roughness tuning, dark faceplate treatment, and camera-aware head pitch/yaw tracking while preserving reduced-motion behavior and live status/identity bindings.

Verification: the arrival test checks host distance, eye height, reverse planting depth, and the complete compact footprint at 1280×800 and 1440×900 aspect ratios. Focused hero, arrival, and ForumView tests passed. Full frontend TypeScript check passed. No live browser/native visual review was available in this lane.

Open defects: visual acceptance, actual close-up shell/hand readability, reverse-view inspection, and measured frame-time performance remain open for coordinator review.

Commit: unavailable: `git add`/`git commit` could not create `/Users/j/Documents/dev/permagent-runtime/.git/worktrees/wq-world/index.lock` (`Operation not permitted`) in this sandbox. The source remains uncommitted.

## Block 2 — complete playability

Status: complete in source/tests; live runtime play verification remains open.

Files changed:

- `ui/command-center/src/components/world/forum/HabitatLearning.tsx`
- `ui/command-center/src/components/world/forum/HabitatLearning.test.tsx`
- `ui/command-center/src/components/world/forum/ForumView.tsx`
- `ui/command-center/src/components/world/forum/ForumWalk.tsx`
- `ui/command-center/src/components/world/forum/forum.css`
- `ui/command-center/src/components/world/forum/habitatLayout.ts`
- `ui/command-center/src/components/world/forum/habitatFullTraversal.test.ts`

Changes: the Evidence desk now exposes Arrival table, Garden archive, and Water workshop demonstrations with choices, immediate corrective feedback, stated sources/assumptions, revisit controls, and no daemon/task/approval dispatch. Walk range prompts name the nearby agent or Garden archive and only appear within interaction range; the archive trigger uses the authored archive position. A bounded traversal test covers all agent stations, both observatory bridge ends, the observatory court, and every bench/planter/canopy approach in both directions back to arrival.

Checks: `HabitatLearning.test.tsx`, `EvidenceLesson.test.tsx`, `ForumView.test.tsx`, `habitatMotion.test.ts`, and `habitatObservatoryWing.test.ts` passed (21 tests; the traversal portion took 52.46 s). The complete traversal test passed 1 test in 133.82 s with route progress and collision assertions. Full frontend TypeScript check passed. No live keyboard, hidden-tab, reduced-motion, corrupt-asset, or native/browser review was available.

Open defects: actual in-app WASD/arrow, Escape from every live state, prompt positioning, and full-land visual/runtime review remain coordinator gates.

Commit: unavailable for the same worktree index permission error above.

## Block 3 — visible richness

Status: complete in source/tests; visual acceptance remains open.

Files changed:

- `ui/command-center/src/components/world/forum/HabitatPlanting.tsx`
- `ui/command-center/src/components/world/forum/HabitatArchitecture.tsx`
- `ui/command-center/src/components/world/forum/habitatRichness.test.ts`

Changes: explicit five-form planting metadata and clustered flowering accents were added while retaining the shared instanced leaf mesh and merged static detail materials; the arrival table gained a page-edge and brass pencil detail. Existing layered water normals/foam/cascade and restrained reflection remained unchanged in this pass.

Checks: `habitatRichness.test.ts`, `habitatWaterFlow.test.ts`, and `staticGardenInstances.test.ts` passed (9 tests). Full frontend TypeScript check passed. No draw-call measurement or live visual review was performed.

Open defects: final rendered foliage silhouettes, alpha edges, water movement, draw submissions, frame-time tails, and camera/nameplate review remain open.

Commit: unavailable for the same worktree index permission error above.

## Correction 1 — camera clearance and nameplates

Status: complete in source and frontend checks.

Files changed:

- `ui/command-center/src/components/world/forum/habitatLayout.ts`
- `ui/command-center/src/components/world/forum/habitatArrival.test.ts`
- `ui/command-center/src/components/world/forum/HabitatNameplate.tsx`
- `ui/command-center/src/components/world/forum/HeroGardenKit.tsx`
- `ui/command-center/src/components/world/forum/HabitatOrrery.tsx`
- `ui/command-center/src/components/world/forum/forum.css`

Exact camera values:

- Arrival: position `[-0.5, 1.60, 5.5]`, target `[-1.1, 1.30, -1.5]`, FOV `72`.
- Reverse arrival: position `[-1.5, 1.60, 4.5]`, target `[-1.5, 2.0, 10.0]`, FOV `80`.
- Overview remains position `[40, 43, 37]`, target `[0, -3, -12]`, FOV `48`; the compact footprint-fit assertions remain in place.

Arrival body positions tested against the runtime station/home data: orchestrator station `[-1.1, 0, 3.1]`; librarian `[-10, 3.4, -19]`; reader `[-6, 3.4, -17]`; watcher `[11, 3.4, -23]`; steward `[10, 3.4, -17]`; strix `[12, 0, -3]`; financier `[-11, 0, -3]`; forecaster `[3, 3.4, -24]`; council `[-1, 3.4, -19]`; polybot `[11, 0, 7]`; picker `[-10, 0, 8]`; growth measurement `[10, 0, -8]`; and the orchestrator table / “Sit with” seat `[-3, 0, 1.1]`. Every tested body is at least `2.0 m` horizontal distance from the arrival camera. Every `HABITAT_SOLIDS` collider is at least `0.6 m` horizontal distance from it. Arrival eye height is `1.60 m`.

Nameplate clamp values: drei `Html` `distanceFactor={4}`; minimum camera distance `1.5 m`; fade band `0.35 m` (fully hidden at or inside `1.5 m`); maximum label width `140 px`. The clamp is used by agent and overview nameplates.

Checks: `habitatArrival.test.ts`, `ForumView.test.tsx`, and `habitatMotion.test.ts` passed together (16 tests). `npx tsc --noEmit` passed with exit code 0. `npx vite build` passed (`3127 modules transformed`, `34.17 s`); the existing dynamic-import and large-chunk warnings were emitted.

Commit: `8aa3ca13` (`fix(world): correct habitat arrival framing`) succeeded on `wq/world` with the required co-author trailer.

## Lane constraints

Only frontend source/tests and allowed frontend checks are being used. No cargo, Blender, Unreal, native build, paid generation, push, or live-config change was performed. Free disk was 26 GiB at start. Final build result is recorded below.

## Final checks

Completed: correction focused tests passed (16 tests); `npx tsc --noEmit` passed (exit code 0); `npx vite build` passed (`3127 modules transformed`, `34.17 s`); `git diff --check` passed; final free disk was 29 GiB. Build emitted the existing dynamic-import and large-chunk warnings. No cargo/Blender/Unreal/native/paid-generation process was run.
