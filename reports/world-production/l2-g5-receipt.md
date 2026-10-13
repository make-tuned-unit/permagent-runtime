# L2 responsiveness — G5 follow-up, 2026-10-13

Source changes are left in the working tree for coordinator staging by path. Review patch: `reports/world-production/l2-g5.patch` (base `8c66cfe127cc808bd4967a9e978ea1b241f6095b`, r32). No Git index writes, lock attempts, commits, cargo, Tauri, or builds were run. The coordinator advanced HEAD on other paths during this task; this patch excludes those changes.

Read evidence: `../wq-reports/qa/native-gate-20261013.md`, G5; `../wq-reports/qa/native-timing-20261013-r32-run1.json`; and `l2-receipt-20261012.md`, item 2. r32's native first World entry dropped its follow-up and two subsequent switches; later switches reached 98–176 ms. The earlier visibility deferral did not break up initialization. This receipt does **not** claim a new native pass.

## Measurement first

Before changing production code, ran `ForumStartup.trace.test.tsx` against r32. The original harness is preserved as `l2-g5-baseline-test.txt`, raw measurements as `l2-g5-before.json`, runner output as `l2-g5-before.log` (1 file / 1 test passed). `performance.now()` surrounds the real terrain and collision functions; React Profiler measures the real architecture, planting, and water render work. Textures are empty Three textures; optional crafted GLBs are stubbed. The large blockers reproduced without WebGL: **141.471 ms architecture** and **788.067 ms planting**, both synchronous inside render's `useMemo`. Terrain, source construction, and every collision BVH also ran synchronously at initial mount.

The final trace starts the 120 ms follow-up timer **before** the lazy World import, retaining a clickable host control while the module is pending. It then exercises actual `ForumView → HabitatScene → procedural builders`, the actual startup queue, and `ForumEnvironment` with GPU doubles. It records render spans, import dispatch, Canvas allocation proxy, terrain/source/BVH work, geometry slices, and PMREM scheduling. See `l2-g5-after.json`, `l2-g5-simulated.json`, and `l2-g5-final-trace.log`.

| First-entry step | Before (ms) | After (ms) / interpretation |
|---|---:|---|
| Lazy ForumView module | 1226.478 wall | 1178.131 wall; dispatch 0.050; asynchronous, includes Vite |
| ForumView initial React render | not separately isolated | 25.066 synchronous; later updates max 7.036 |
| Canvas / WebGL creation | unavailable in jsdom | DOM/GPU double 0.058; real context cost unmeasured |
| Terrain | 20.856 synchronous | 16.543 total; 1 slice(s), max 16.543 |
| Collision-source geometry | 16.933 synchronous | 13.641 total; 1 slice(s), max 13.641 |
| Collision BVHs | 11.013 whole registration | 5.870 total; 211 slice(s), max 3.654 (BVH construction only; cloning/publish overhead in combined HabitatScene span) |
| HabitatScene terrain + collision | 48.802 sum of separate synchronous calls | 40.949 total; 4 slice(s), max 16.583 |
| Architecture | 141.471 synchronous render | 135.888 total; 16 slice(s), max 14.300 |
| Planting | 788.067 synchronous render | 1020.911 total; 104 slice(s), max 15.236 |
| Water geometry | 2.244 synchronous render | unchanged builder; included in final React render spans |
| Environment / PMREM | GPU cost unavailable | 0.131 total; 2 slice(s), max 0.067; GPU double, after first frame |
| Shipped hero GLB | supplementary, not a before capture | JSON parse 0.224 sync; parser dispatch 1.146 sync; geometry parse promise 4.960 wall |
| Texture decode / upload | browser cost unavailable | Sharp image decode 12.683 async wall (surrogate); bitmap-loader and one-upload-per-task behavior tested with doubles |

Final captured cold-entry click handled at **123.776 ms**, requested at 120 ms; core readiness at **4964.804 ms**, including the lazy import. Largest recorded synchronous application span **25.066 ms**. Deterministic acceptance: **101 slices, maximum 12 ms simulated work, click handled at 120 ms while initialization is still pending**. The target is ≤50 ms per simulated slice. Total geometry CPU time is not expected to shrink; scheduling restores input/paint opportunities between pieces.


The baseline import measurement is asynchronous wall time, including Vite transformation and module loading. It must not be read as a 1.2 s synchronous evaluation. The final import has a smaller synchronous dependency graph because architecture and planting are separate lazy modules. The baseline did not separately time the entire ForumView shell render or actual WebGL Canvas creation. WebGL context creation, shader compilation, PMREM execution, browser image decode/upload, and browser worker runtime cannot be timed by jsdom; baseline zero placeholders explicitly mean **unavailable**, not zero work. Final GPU-double measurements likewise are scheduling evidence only.

## Changes

- Added one shared rAF-then-task startup queue, with an 8 ms CPU budget between generator checkpoints and a bounded iteration count. Every builder shares the queue, so independent components do not each monopolize one frame. A resolved Promise alone is never the yield boundary. Optional diagnostics record `performance.now()` samples without retaining production traces.
- Yield before lazy import dispatch, Canvas creation, core scene, actors/decor, and postprocessing. Architecture and planting are separately imported and build outside React render. Preserve HUD/navigation and the existing external loading/slow/retry status while core geometry and collisions prepare. A first active frame plus completed core preparation is required to clear loading.
- Converted architecture and planting to resumable generators, including per-loop checkpoints, matrix population, and geometry merges. Merge chunks are bounded at approximately 32,768 vertices (a single input primitive can cross that threshold). The authored placements/materials are preserved; this can add draw calls compared with one enormous mesh per material. Disposal covers completed and interrupted builds.
- Terrain and collision-source creation get separate slices; BVHs are built one mesh at a time. Publish the collider set atomically after completion. Partial builds pause across hidden workspaces, then resume without restarting. StrictMode effect replay and interrupted cleanup are covered.
- Defer environment preparation until after the first active frame. Generator construction and capture run on separate turns; retain cached day/night targets and prior lighting while preparing. Missing-bake scene capture uses a 64-pixel PMREM face instead of the default 256, reducing fallback GPU work. `PMREMGenerator.fromScene/fromCubemap` themselves remain indivisible GPU API calls; **native ≤50 ms for those calls is not established**. Baked faces retain their own resolution.
- Use a single lazily created, same-origin module worker for browser meshopt decompression. Submission copies/transfer are queued separately. This avoids blob-worker CSP requirements and does not silently fall back to main-thread decoding when worker startup fails. CPU-only asset tests / environments without Worker retain the existing decoder. The worker is shared for the page lifetime by the GLTF resource cache. The current 5,889,960-byte hero kit is uncompressed; `l2-g5-assets.json` measures its unchanged CPU geometry parser separately. The browser still constructs GLTF scene objects on the main thread; the worker handles meshopt compression, not all GLTF parsing.
- Added a shared sky/material texture loader using asynchronous `ImageBitmapLoader` / `createImageBitmap`, with `HTMLImageElement.decode()` fallback. Queue GPU uploads one texture per turn and suspend texture consumers until their uploads finish. Sky preparation and rendering share the same loader/cache. Colors, mipmaps and samplers are finalized before upload; consumers borrow those cache-owned textures without cloning or setting needsUpdate during first render, which would trigger a second upload. Canvas visibility pauses pending uploads; final unmount releases them, including StrictMode replay handling.
- Existing lifecycle, lighting, loading, routing, hierarchy, label-stacking and agent-focus assertions remain. Fixtures now advance actual startup turns rather than assuming immediate Canvas mounting. Added tests for bounded simulated work/input, real cold route startup, worker dispatch, asynchronous image decode, individual texture uploads, partial-build ownership, and first-frame PMREM scheduling.

## Gates

| Gate | Final result | Evidence |
|---|---|---|
| Required full serial suite | **exit 0; 538 files, 4386 tests passed**; 342.63 s | `l2-g5-full-serial.log`, `l2-g5-full-serial.exit` |
| TypeScript | **exit 0, no diagnostics** | `l2-g5-tsc.log`, `l2-g5-tsc.exit` |
| Final saved CPU/input + decode trace | **3 files / 5 tests passed** | `l2-g5-final-trace.log`, `l2-g5-after.json`, `l2-g5-assets.json`, `l2-g5-simulated.json` |
| Whitespace / review patch | `git diff --check` and `git apply --reverse --check reports/world-production/l2-g5.patch` pass | Read-only checks; no index lock or index writes |

The full run began at 01:41:47 local with coordinator HEAD `a5da3b8086c7c2cb1ff41288922b8a3e98c65fc9` plus this lane's working-tree changes. It also exercised the new texture sampler/upload checks and real shipped-GLB measurement. In that full run, the cold-import click arrived at **182.177 ms**, the largest measured application span was **25.487 ms**, and the strict simulated maximum was still **12 ms** with input handled at **120 ms**. This lane adds **9 tests across 5 new test files and the existing environment-readiness file**. Counts reflect the actual concurrently advancing checkout, above the supplied 4363+ baseline.

L2 lifecycle assertions remain unchanged in meaning: 3 retained canvases/context proxies, 4 lifecycle listeners, zero hidden input listeners, zero hidden timers, zero hidden invalidations across the leave/return and page-hide cycles. Lighting remains 2 environment builds for day→night→day, zero premature disposals, and both targets released on owner unmount. These remain jsdom proxies, not a native GPU lifetime capture.


Exact gate commands, cwd `ui/command-center`:

```sh
node --no-concurrent-sparkplug node_modules/vitest/vitest.mjs run --config ../../reports/world-production/vitest.serial-forks.config.ts
npx tsc --noEmit
```

Focused commands use the same serial Vitest prefix and the file filters named in their logs. The final trace command adds `L2_G5_TRACE=1` to save its JSON; ordinary suite runs do not overwrite the captured evidence. Vitest still prints the existing nonfatal sandbox HMR listen denial and library warnings.

The first full run was 534 files passed / 2 failed, 4379 tests passed / 4 failed (4383 total), exit 1. All four failures assumed immediate module/Canvas creation (`WorkspaceRenderer.test.tsx`, `worldLabelStacking.test.tsx`). Their startup waits were corrected without weakening route or stacking assertions. That run is preserved as `l2-g5-full-serial-first.log` / `.exit`. An additional texture test initially waited only three microtasks for a fetch/blob/decode chain; it now waits for the actual decode callback. The second full run had 536 files / 4384 tests pass and one failure: an extra 220 ms wall-clock assertion in the cold-import test saw input at 246.720 ms while Vite was transforming modules. That run is `l2-g5-full-serial-second.log` / `.exit`. The real route test now records the cold-import latency and checks delivery within the existing 500 ms ceiling; the strict ≤50 ms simulated-step assertion and exact 120 ms simulated click remain. The 246.720 ms sample is not hidden by the faster standalone capture above. No claim is made that browser module evaluation is preemptible or that Vite wall time measures its duration.

## Handoff and limits

Coordinator should stage **only** the paths listed in `l2-g5-paths.txt`, or apply `l2-g5.patch`. This patch contains the source/tests, before/after/simulated/asset JSON, baseline harness, and receipt; ignored `.log` evidence remains alongside them in this worktree; final `.exit` records are included. No changes were made to `forum/data/`, `forum/places/`, `assets/`, `scripts/blender/`, wizard, or Build files. Other lanes' existing edits and commits are excluded.

This refactors the default Habitat first-entry path. The legacy rollback World and the previous campus retain their own scene-assembly internals, apart from the shared loader/environment changes. The next native staged build still needs the assigned 20-switch re-capture with 120 ms follow-ups, actual GPU/texture/decode timings, and visual review of the lower-resolution fallback environment / extra merge batches. No native latency, native first-usable budget, GPU memory, renderer upload ceiling, or native worker-bundle validation is claimed. Awaiting an async API or scheduling an indivisible browser/GPU call does not preempt that call internally. The proven jsdom CPU blockers are split; the native gate belongs to the coordinator.

Execution-location deviation: two temporary transformation scripts were initially created in `/tmp`, contrary to the requested worktree-only file location. They were moved into this worktree and then removed as temporary tooling; neither remains in `/tmp`. All deliverable/source changes are in this worktree. No approval or index-lock attempt was made.
