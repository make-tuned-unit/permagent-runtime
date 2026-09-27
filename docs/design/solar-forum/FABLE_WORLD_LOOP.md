# Fable 5.1 — continuous World production assignment

Jesse explicitly requested this worker, starting 2026-09-26 at 21:30 America/Halifax, to use the remaining Claude usage allowance on a continuous screenshot → inspect → critique → fix → validate loop. You are the worker: use `claude-fable-5-1`, not a cheaper fallback. Continue productive iterations until the allowance is exhausted or Jesse stops you. A successful intermediate pass is not a reason to stop. Keep checkpoints so usage interruption loses no context. Do not buy credits, enable extra usage, switch to paid API credentials or change billing settings.

## User direction, including latest correction

The World currently feels like Minecraft instead of a high-quality video game. Work towards the original supplied goal image and create an engaging, genuinely interactive world for real agents. Use Blender and Unreal on this Mac to their practical abilities. **Three.js is replaceable**: if it limits the result, select and implement a better runtime. Every user downloading Permagent should experience the world; success is not restricted to the developer machine or an editor render.

Read the neighboring `AI_ENVIRONMENT_RESEARCH_2026-09-26.md`, `WORLD_VISION.md`, `APP_INTEGRATION.md`, `REALISM_REFERENCES.md`, and `RECOVERY.md`. Visually open `north-star.png`; read it as art direction, not a background to paste over the scene. Open the actual current World and capture a baseline. Use Blender modeling/material/rigging/animation/procedural tools and Unreal lighting/materials/foliage/PCG/runtime tools as appropriate. Do not spend the session only writing plans or changing UI colors.

## Ownership and resources

Source checkout: `/Users/j/Documents/dev/permagent-runtime`. QA integration: `/Users/j/Documents/dev/permagent-worktrees/dagJ-lane-qa-int3`, branch `dagj/qa-int3`. Create your own branch/worktree under `/private/tmp/permagent-world-fable` from the latest clean integration checkpoint if available, otherwise the main committed HEAD. Record the exact base and keep changes isolated. Preserve pre-existing main edits. Read relevant repository instructions.

Existing Unreal generated `Content` is untracked (~419MB), and source/world assets may also be ignored. Inventory and copy necessary assets into your lane, retaining provenance; do not assume a Git worktree contains them. Do not symlink mutable Unreal Content into another worker's checkout. You may read existing assets, scripts and source in the main tree. Blender is `/Applications/Blender.app`; UE is `/Users/Shared/Epic Games/UE_5.8`. Use installed tools before adding infrastructure.

QA recovery owns `/private/tmp/qa-ux-target` and the combined release. Read `/private/tmp/qa-ux-recovery/COORDINATION_FROM_QA_WALK.md` and `COORDINATION_TO_QA_WALK.md`. Do not modify, delete or build into that target, release a competing app, or merge into the QA branch. Leave a precise integration handoff and your commit tip. Coordinate heavy builds via the repository guard and process checks. Disk was only ~7GB free at preparation; recheck. Do not delete other work, original assets, build targets or app data to make room. Use sequential Blender/Unreal/build phases and bounded assets. If an expensive phase is resource-blocked, work on a useful independent asset/source task; preserve the blocker rather than repeatedly rerunning it.

## First deliverable: a vertical slice that can ship

Create an excellent furnished garden arrival terrace with appealing animated agents, layered planting, believable materials, water and a grand orbital vista. Choose a representative scene and compare the currently delivered renderer with Unreal. Make a concrete runtime decision with screenshots, performance and packaging evidence. If Unreal is needed, implement a packaged runtime entry from Permagent plus live local agent integration early. Do not leave all integration to the last turn. No requirement to retain Three.js for the main World.

The installed World must start without Blender/Unreal editors or a dev server, find bundled/cooked assets, and support the app's actual distribution platforms through a documented tested path. Detect hardware, provide appropriate quality tiers and honest unsupported-platform behavior. Do not claim universal support from one Mac test. Avoid assuming cloud streaming is free, offline or already provisioned. No new paid infrastructure without authorization.

Retain the essential loop: arrive → approach/select real agent → ask → see acknowledged real job → inspect required approval → see verified result. Never fabricate task progress, connected mesh peers or approvals. Test using isolated fixtures or safe read-only live interactions; do not approve or launch unrelated real tasks for QA. Keep normal app task access usable while World loads or is unavailable.

## Each production iteration

1. Read the last checkpoint and Jesse's new messages if available. Inspect actual screenshots with image-capable tools; rendering files without looking at them does not count.
2. Capture fixed arrival/agent/garden/vista/water/navigation/night cameras plus a camera-motion/interaction run. Early cycles can concentrate on the hero and two diagnostic angles; periodically recapture the full suite. Record camera, settings, asset/build revision and frame-time/memory evidence.
3. Compare reference, baseline and current. Identify the three largest visible gaps, explain the concrete causes and implement the highest-impact changes. Prioritize composition, agent shapes, foliage, materials and light over larger acreage or more primitives.
4. Run the real Metal-rendered scene. `-NullRHI`, successful import, an offline beauty render, an AI-enhanced video or a poster behind the UI cannot satisfy the visual gate. Inspect the actual Permagent World as well as authoring renders.
5. Validate collisions, scale, walkability, camera clipping, state/event bridge, selection, text legibility, day/night, motion stability, hidden-view pause and reduced motion. Compare performance at fixed resolution. Use measured quality tiers; pursue visual excellence within user hardware limits.
6. Keep improvements, revert regressions in your own lane, commit coherent checkpoints, and update `reports/world-fable/CHECKPOINT.md` with screenshots, branch/hash, commands, results, open deficits and exact next step. Maintain asset source/license records and reproducible generation scripts.
7. Continue immediately. Do not stop at “looks good,” a plan, a single camera, a tool success or a proposed future fix. Use remaining allowance on the next weakest visual, interaction or delivery dimension. If necessary rotate to a new context using the durable checkpoint, keeping this mandate.

## Completion and interruptions

The supervisor resumes normal completed turns with this mandate until usage is exhausted. Actual usage denial ends this scheduled run; do not loop errors, purchase more usage or wait for another billing window automatically. Respect existing tool permissions; do not disable approval checks. If authorization, an unavailable model, an unrecoverable tool fault or hardware prevents all meaningful progress, leave a precise blocker and checkpoint rather than misrepresenting success. The user can stop via the supervisor's STOP file. If a stop arrives, save safely and stop.

Final handoff must distinguish: visually inspected evidence; functional tests; package/clean-launch evidence; untested platforms; actual unresolved quality gaps; branch/commit; generated asset locations; and the next productive step. Preserve all work at a usage-limit interruption.
