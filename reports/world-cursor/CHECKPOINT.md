# World Cursor loop checkpoint

- Date: 2026-09-27 (ADT)
- Worker route: Cursor CLI (this session)
- Model: unknown/default (not exposed in-tool)
- Branch: `world/fable-loop-20260926`
- Start checkpoint: `eed56db69661` (setup-only baseline; no implementation claimed)

## Coordinator / resource safety

- Read `/private/tmp/qa-ux-recovery/COORDINATOR.md` at start and before each phase.
- STOP gate checks run before implementation and reporting (`STOP_ABSENT` each time).
- Disk check: `df -h` showed ~4.9 GiB free on `/System/Volumes/Data`.
- Followed lightweight-only constraints: no Cargo/Rust builds, no Unreal/Blender heavy work, no large asset generation/copies, no billing/auth changes.

## Visual inspection evidence (real files inspected)

Inspected with image-capable tooling:

- `docs/design/solar-forum/north-star.png`
- `assets/world/forum/browser/v7-vista-day.webp`
- `assets/world/forum/browser/v7-vista-night.webp`
- `assets/world/forum/browser/v3-tower-windows-night.webp`

Notable critique from inspected baseline:

- Major sky/composition occlusion from top-center waypoint chip cloud.
- Visual focus splits between HUD overlays and vista landmarks.
- Scene still reads as overly technical/diagrammatic from the default app framing versus the furnished, foreground-led north-star reference.

## Runtime validation attempts and blockers

Commands attempted:

- `npm run dev -- --host 127.0.0.1 --port 5284` (from `ui/command-center`)
  - Result: failed (`vite: command not found`).
- `browser_navigate http://127.0.0.1:5284/ui/forum.html`
  - Result: `ERR_CONNECTION_REFUSED` (preview not running).
- `browser_navigate http://127.0.0.1:3001/ui/`
  - Result: HTTP response failure (app route not usable as-is in this session).

Validation command after source changes:

- `npm run typecheck`
  - Result: fails in this environment with `TS5102` (`baseUrl` removed), indicating local tooling/version mismatch against this checkout; no trustworthy pass/fail signal for the edited files was produced.

## Source changes made

Goal: remove redundant composition-blocking overlay while preserving navigation and interaction.

1. `ui/command-center/src/components/world/forum/ForumNavigation.tsx`
   - Removed floating in-world waypoint chip cloud (`Html`/`forum-waypoint` buttons).
   - Kept camera framing/orbit behavior and district/agent focus logic intact.
2. `ui/command-center/src/components/world/forum/ForumView.tsx`
   - Updated `ForumNavigation` callsite to match the simplified API.

Effect: World place selection now relies on the existing left `Places` rail and district cards instead of duplicating top-of-frame chip overlays, opening the vista and reducing immediate HUD clutter.

## Remaining deficits / packaging gaps

- No fresh live runtime screenshot capture from this exact revision due preview/runtime access blockers above.
- No heavy validation phase (by instruction): no Unreal runtime pass, no packaged build verification, no native app clean-launch test.
- Existing visual deficits remain (materials/foliage/agent appeal vs north-star target) and need iterative source+runtime evidence loops once preview/runtime path is available.

## Next action

1. Re-establish a runnable lightweight preview path for this checkout without violating coordinator headroom constraints.
2. Capture new day/night baseline screenshots from this commit.
3. Continue highest-impact visual loop on materials/planting/agent readability with measured before/after captures and commit in bounded increments.

## 2026-09-27 source follow-up

The preview blocker and TypeScript mismatch above describe the earlier Cursor environment, not this follow-up. A preview at port 5284 and Cursor screenshot files were available for root's separate visual review. This follow-up did not drive the browser or claim a new live screenshot.

- Day lighting and postprocessing are tuned in source; night grade is unchanged. The default vista and celestial body sizes/positions were revised. Vite dedupes React, Three, and R3F packages; Vitest inlines Zustand to resolve React through that rule in the worktree.
- The in-app celestial probe now identifies the gas giant and moon by their stable mesh names and reads their live radii/positions. It no longer silently misses them after the size change. The harness itself was syntax checked but not run against the live daemon.
- The Sovereign Passport was moved off the World canvas into the Meet detail for the sovereign orchestrator. It is unmounted when the desk is closed, another agent is selected, or another tab is active. Legacy dismissed-passport storage is ignored because it no longer represents a dismissible canvas card. Existing identity data and polling remain in the Passport component.
- Removed the obsolete `onDistrict` compatibility prop from navigation and tests; updated camera/sky comments for the current values.
- Validation: all 442 frontend test files and 3,492 tests passed; `npm run typecheck`, `npm run build`, `node --check scripts/verify-world-in-app-v2.mjs`, and `git diff --check` passed. Focused Passport visibility test covers canvas absence, sovereign detail presence, closed desk, other agent, and other tab.

Remaining limitation: no new real app or packaged runtime verification in this follow-up. Cursor's untracked screenshots and `.playwright-mcp/` are preserved outside this source commit for root's review. The visual result remains a low-poly architecture study compared with the furnished north-star reference; source checks do not establish visual parity.
