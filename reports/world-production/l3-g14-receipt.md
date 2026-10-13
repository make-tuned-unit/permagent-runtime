# L3 / G14 — explicit World Overview travel — 2026-10-13

Starting candidate: r33, `ec93999c2a9f40bd9f840a0e6c36ba3fcd25f5c6`. Changes are left unstaged in the shared worktree. Delivery patch: `reports/world-production/l3-g14.patch`.

## Root cause and reproduction

The failing path is `ForumView.showOverview()` → `overview=true` / incremented `overviewRevision` → `HabitatNavigation`'s view effect → `transition.current` → its `useFrame` callback. The retained-state guard correctly recognizes an explicit command, including a repeated Overview after manual orbit. Narrow aspect alone also computes the correct destination. The regression fixture verified those cases against the original production code before changing it.

Two timing defects prevented that destination from being reached on schedule:

1. In `HabitatScene.tsx`, the aspect-change effect admitted an automatic refit with `changed=false`, then replaced the pending transition with `elapsed:0`. Every new measurement restarted the entire 0.85-second journey from its current pose. Repeated measurements could keep the view near arrival; returning from a hidden canvas at a different aspect also restarted the remaining journey.
2. The frame callback advanced travel with `Math.min(dt,.05)`. At five slow frames spanning 0.85 seconds, it counted only 0.25 seconds. An individual 0.85-second frame counted only 50 ms. This is an interpolated camera command, so a physics-style delta cap made its duration depend on rendering throughput.

Both timing behaviors predate L3b; arrival-first entry exposes the animated arrival → Overview path instead of starting already at Overview. The fix leaves L3b's entry and retention rules intact. It does not remove the intentional manual-orbit cancellation or the keyboard/reduced-motion snap behavior.

Before the production fix, the first regression run had **7 failures / 12 passes**: three slow-frame cases, two mid-travel resize cases, one repeated-resize case, and one hidden-return/new-aspect case failed. For example, the repeated-resize test ended with camera X `5.991860610868482` instead of the fitted `38.268965517241384` after 0.85 seconds. Evidence: `l3-g14-repro.log`.

The tests use the real `ForumView` HUD button, `HabitatNavigation`, `PerspectiveCamera`, and `three-stdlib` OrbitControls. The GPU renderer and scene decoration are omitted; canvas dimensions and frame deltas are injected. They cover 1024×640 and 1280×800 canvases (both aspect 1.6), plus representative shell canvases 784×612 and 464×612 (aspects approximately 1.281 and 0.758). Narrow dimensions are explicit fixture inputs, not claimed measurements from native screenshots. The supplied `wq-reports/qa/shots-gate-r33-20261013/` images were not present in this checkout; this receipt does not claim a new native capture or establish the native frame/resize sequence from those images.

## Change

`HabitatNavigation` now preserves elapsed command time when an aspect change retargets ongoing travel. It starts a remaining segment from the current camera/target/FOV and reaches the newly fitted destination by the original deadline. An actual new command still receives its own 0.85-second duration. Frame updates count the full visible frame delta; hidden frames still return before touching travel state.

The frozen Overview contract is unchanged: position `[30,26,42]`, target `[-3,-4,-16]`, FOV `50`, with position fitted as `target + (position - target) * max(1, 1.6 / max(.25, canvasAspect))`. Assertions calculate this contract independently of `habitatOverviewCamera`. Arrival stays `[0,1.6,6]` → `[-1.1,1.3,-1.5]`, FOV `72`; retained orbit/place state, walk feet/look, table framing, clipping and orbit limits remain unchanged.

Only these application/test files are included in the patch:

- `ui/command-center/src/components/world/forum/HabitatScene.tsx`
- `ui/command-center/src/components/world/forum/ForumView.overview.test.tsx` (new)

## Test names

The new file contains **25 cases**:

- `HUD Overview reaches the fitted contract within 0.85 s at canvas {width}x{height} from {state}` — 16 cases: four canvas sizes × arrival, retained manual arrival, retained manual overview, retained place. Also verifies the highlighted HUD action, retained leave/return state, and stable framing at 4.5 seconds.
- `HUD Overview meets the 0.85 s deadline with slow frames at canvas %sx%s` — 3 cases.
- `resize during HUD Overview preserves its original deadline (retained=%s)` — 2 cases, including successive aspect changes and no immediate pose jump on resize.
- `repeated resize observations cannot strand HUD Overview near arrival`
- `pauses Overview while hidden and resumes the remaining travel with a newly fitted aspect`
- `finishes Overview on a delayed frame instead of advancing only 50 ms`
- `a repeated explicit Overview gets a fresh deadline after resize retargeting`

Existing regression coverage remains green in the focused run: arrival without aspect fitting; manual and selected-place retention; keyboard and reduced-motion behavior; hidden transition suspension; table/clipping/orbit contracts; HUD entry/desk hierarchy; World lifecycle; and `ForumWalk.visibility.test.tsx`'s preserved feet/look and released hidden input.

## Gates

Commands run from `ui/command-center` with logs saved under `reports/world-production/`.

| Gate | Result | Evidence |
| --- | --- | --- |
| Red reproduction before production fix | Exit 1; 7 failed / 12 passed (19 cases) | `l3-g14-repro.log` |
| Focused navigation/entry/lifecycle/walk/contracts | Exit 0; 6 files / 95 tests passed (initial 19-case regression file) | `l3-g14-focused.log` |
| `npx tsc --noEmit` | Exit 0; no diagnostics | `l3-g14-tsc.log` |
| Full serial suite | Exit 0; **543 files / 4471 tests passed**, 361.99 s; **4437 baseline + 34**, including all **25** new G14 tests and concurrent lane additions | `l3-g14-serial.log` |
| Scoped `git diff --check` | Exit 0 | `HabitatScene.tsx` |
| `git apply --reverse --check reports/world-production/l3-g14.patch` | Exit 0, read-only validation | Both delivered files |

Full serial command: `node --no-concurrent-sparkplug node_modules/vitest/vitest.mjs run --config ../../reports/world-production/vitest.serial-forks.config.ts`. The npm cache was directed inside the worktree for the TypeScript command. No build, cargo, Tauri, commit, staging, index lock, or write outside the worktree was performed. No changes were made to `forum/data/`, `forum/places/`, or other workers' lanes. Existing nonfatal Three.js, jsdom/React and mocked daemon warnings were not suppressed or changed.

Gated source SHA-256 values:

```text
a83d1950fd1dafa179485c45260963bb9996400042727ae0cf45973550540a21  HabitatScene.tsx
5562f21eb8ebac924ba8a6bf82a094d9791b928406c5eb330b55531eeee61bf4  ForumView.overview.test.tsx
```

Both source hashes were rechecked unchanged after the full gate. Patch SHA-256: `aa237543cabdf15d0cd2a7284b14c7775452d633fc5565dad21a7f9baa2ac7f5`; reverse-apply validation passed again after the gate.

Shared-worktree HEAD was `e2d08d53e1626b6ef568bcfdeccb7080358a07fd` when both required gates started and `f1c1b0e75aa90b7030e8b3e4f978c864b4df59c7` at closeout. Other workers continued committing and editing their own lanes during the run; they were not staged, reverted, or included in this patch. Native GPU recapture and packaging remain outside this task.
