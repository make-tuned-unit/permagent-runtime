# World lane brief — 2026-10-07 (branch `wq/world`, worktree `/Users/j/Documents/dev/permagent-worktrees/wq-world`)

Read `reports/world-production/LANE_RULES.md` first; its rules are binding. Then read `reports/world-production/CHECKPOINT.md` (the r23–r28 history, latest sections first), `docs/design/solar-forum/WORLD_PRODUCTION_PLAN_2026-10-06.md` (art direction, budgets, play loop) and `docs/design/solar-forum/WORLD_REVIEW_2026-10-06.md` (what the reviewer rejected and why). Open `docs/design/solar-forum/north-star.png` and `docs/design/solar-forum/compact-habitat-concept-2026-10-06.png` as art direction.

The user's standing correction: the whole landscape must be walkable; it may be smaller than the reference but must be as richly detailed. The user's words today: "make the World View richly detailed". Astra (lead) ruled the order below. Do not expand acreage; deepen what exists.

## Where the code is

`ui/command-center/src/components/world/forum/` — `ForumView.tsx` (shell, HUD, desk), `HabitatScene.tsx`, `HabitatArchitecture.tsx`, `HabitatSky.tsx`, `HabitatWater.tsx` + `habitatWaterMaterial.ts`, `HeroGardenKit.tsx` (robots, furniture, foliage placement), `habitatLayout.ts` / `habitatTerrace.ts` (placements + shared collision data), `habitatMotion.ts` (walk, routes, A*), `habitatSocial.ts` (ambient schedules), `HabitatAgentDock.tsx`, `HabitatNavigation.tsx`, `EvidenceLesson` (the one existing lesson), `forum.css`. Tests sit beside them (`*.test.ts(x)`); `habitatMotion.test.ts` and the production path tests are the route-coverage proof. Assets: `ui/command-center/public/world/` (hero kit GLB, sandstone maps, Gaia sky) with provenance under `assets/world/*`.

## Ordered work (stop and report after each numbered block; commit each block)

### 1. Arrival and hero presentation
- Arrival camera at human eye height (~1.6 m) beside the host robot and the round worktable, 2–4 m from the host, with a framed view down the main route and the water beyond. Compose three depth layers (foreground table/robot/planter, mid garden steps/water, far observatory wing + sky). The current arrival still reads as an elevated overview with small figures.
- Apply the proposed overview camera target `[0, -3, -12]` (currently `[2, 1, -20]`), verifying the whole footprint fits the frame at 1280×800 and 1440×900.
- Reverse view: when the player turns around at the arrival, the view must be as composed as the front (no empty paving, no backs of props).
- Robot close-up: at 1 m the hero robot must show shell seams, a dark readable faceplate, eyes, segmented hands, joint detail. Improve the materials (clearcoat ceramic, subtle roughness variation) and the idle/head-tracking animation. No "toy ellipsoid" look.
- Lighting: warm key / cool fill, open shadow detail, no crushed night shadows, no cyan globe dominating, practical lights as warm pools at night. Keep the existing Metal black-rectangle fixes and the `gentle` post-processing branch.
- Keep "Overview" and the normal app navigation reachable.

### 2. Complete playability
- Implement the three learning interactions from the production plan's "Playability and learning" table (arrival table: plan vs constraints; garden archive: two claims + sources → choose → feedback; water workshop: change a bounded illustrative setting → predict → inspect). Each is clearly labelled as a demonstration, never submits a real task or approval, cites its evidence, offers immediate explanation and a revisit. Reuse the existing `EvidenceLesson` pattern and the desk "Evidence" tab; add an accessible non-spatial entry (the desk) for each.
- Route coverage: a test that walks every destination in both directions including BOTH bridge ends and the observatory court, returns home, and asserts no obstruction from every agent station and every bench/planter/tree collider. Then fix every obstruction it finds.
- In-world: interaction range is visible (prompt appears only in range, with the key and the target named), Escape returns to the shell from every state, collisions recover (no stuck-on-a-planter), WASD + arrows both work, hidden tab pauses, `prefers-reduced-motion` removes head-bob and ambient camera motion.

### 3. Visible richness (no acreage)
- Foliage: at least five readable forms (canopy tree, broadleaf shrub, fern groundcover, climbing vine, flowering accent) with clustered distribution and clear walk corridors; no repeated spherical crowns, no uniform equal-size scatter. Improve the v3b canopy/shrub silhouettes (fuller crowns, varied leaf orientation), add vine and flowering accent forms. Runtime instancing for every repeated static placement; check draw submissions do not grow.
- Furniture and surfaces: tactile stone/wood/ceramic with edge highlights, drainage/rails/troughs where sensible, a furnished arrival table (instrument, cups, a field notebook), cushions. Use the existing sandstone maps and the FAL furniture kit; author new props in code (`HeroGardenKit.tsx`) — no Blender in this pass.
- Water: layered normal motion, shoreline contact, restrained reflection, thin cascade with foam that survives movement; keep the r25 highlight restraint.
- Framing polish: nameplates must not cover faces; the agent dock stays compact; the chat pill never covers World controls.

## Constraints
- Real agent identity and activity stay authoritative; ambient behaviour never invents task progress. Never crowd every roster member into the arrival terrace.
- Performance budget: p95 frame time ≤ 20 ms at 1440×900 on an M4 is the target; do not add post-processing passes; prefer instancing and material sharing; keep the hero kit ≤ 40 MB.
- No paid asset generation (no FAL/Meshy), no Blender/Unreal, no native builds. Frontend checks only (tsc, focused vitest, `npx vite build` at the end).
- The coordinator will review visually in Chrome and natively; do not self-score as accepted.

## Report
For each block: files, commits, tests run with exit codes, what changed visually and how you verified it, open defects. Write it to `reports/world-production/WORLD_LANE_REPORT_2026-10-07.md` and keep it updated.
