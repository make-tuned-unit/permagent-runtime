# App integration audit

Audit date: 2026-09-11

Scope: browser forum adapter and its existing Permagent runtime seams. No daemon,
native Unreal, generated asset, or production component changes were made.

## Findings

### P0 — forum patrol mutates shared live worker motion state — fixed in this audit

`ForumInhabitants` uses the same module-level records consumed by
`AgentCharacterV2` and the existing World locomotion (`motion.ts`). On mount,
`ForumInhabitants.tsx:16-24` snapshots each record, then calls `ensureMotion`
and immediately assigns `x/y/z`, `walking:false`, `engaged:'none'`,
`ringLock:null`, and `queue:[]`. That can teleport a worker, clear an active
path, clear engagement, and erase a ring lock while the forum is mounted. The
cleanup restores only the initial snapshot (`:24`), so any real state changes
that occur while the forum is mounted can also be lost when it unmounts. The
per-frame loop repeats `m.walking=false` at `:30` before checking the live HUD
state at `:31-33`.

This violated the requirement that ambient movement not overwrite actual worker
state. The fix keeps forum patrol records in an isolated `ForumMotionState`
store (`ForumInhabitants.tsx:12-32`) and passes those records to the forum
avatars through the optional motion override (`AgentCharacterV2.tsx:92-116`,
`ForumAgents.tsx:16-22`). Forum mount/unmount no longer snapshots, resets, or
restores records owned by `agents/motion.ts`; the real World locomotion path is
unchanged. The patrol loop also pauses its isolated record when the real HUD
source reports `working` (`ForumInhabitants.tsx:35-58`).

The follow-up position audit found two remaining forum visual reads of the
shared motion store. `ForumNavigation.tsx:17-24` now focuses the sovereign
using the isolated forum position, and `AgentCharacterV2.tsx:363-374` accepts
the same resolver for hovered and working peer look-at targets. Real daemon
state remains shared, while forum avatar positioning and camera targeting use
the isolated visual layer.

### P1 — the new forum is not wired into the shipping World workspace

`WorkspaceRenderer.tsx:11` lazy-loads `../world/WorldView`, and
`WorkspaceRenderer.tsx:27` maps the `world` tool to that component. There are
no imports or route references to `ForumAppView`; its only references are the
adapter and its test. Therefore the browser app currently renders the original
World view. The adapter's navigation, goal detail, skills-panel, and agent
settings callbacks are tested, but are not production-reachable until the
release gate is intentionally changed and exercised in the actual app.

### P1 — saved skills can remain stale after the real registration event

`ForumCapabilities.tsx:15-16` refreshes its authenticated registry snapshot
only for `skill_proposed` and `task_completed`, plus the 30-second poll at `:14`.
The runtime event union includes `skill_saved` (`lib/store.ts:104`), which is the
event emitted after a reusable skill is actually registered. A skill promoted or
registered through the shared conversation can therefore succeed while the
forum continues to show the old skill list for up to 30 seconds. Subscribe to
`skill_saved` as well (and retain the authenticated endpoint as the source of
truth); do not infer registration from the avatar or proposal count.

### P2 — browser integration is substantially real, but the selected specialist
is context only

`forumQuery.ts:7-27` validates the brief, blocks concurrent streaming, obtains
the existing session, loads its history, connects the authenticated SSE channel
before sending, and labels query/job/capability requests explicitly. It also
states that selecting a specialist is not direct assignment. The existing
conversation surface renders pending approvals and the store's cancellation
action (`ForumConversation.tsx:1-21`). This is the correct shared pipeline, but
the specialist selection is presentation/context, not a dispatch target; job
ownership and evidence still depend on the orchestrator's normal runtime.

`forumRuntime.ts:11-22` correctly keeps rejected agent/skill/proposal feeds as
`null` and reports them unavailable. `ForumCapabilities.tsx:20-24` consequently
does not turn an outage into an empty successful registry. The registered-agent
manage action passes the registry id into the existing settings seam via
`ForumAppView.tsx:7-11`.

### P2 — native Unreal has no equivalent live bridge

The browser adapter uses the existing authenticated HTTP/SSE/session stores.
The current Unreal implementation has scenery, skeletal bodies, ambient
sequences, and an identity snapshot only. As documented in
`APP_INTEGRATION.md`, it has no live daemon/event/approval bridge. Native Unreal
visual or movement checks therefore cannot substantiate browser jobs, skills,
approvals, cancellation, or worker status until that bridge is implemented.

## Verified behavior

The user-named sovereign is taken from `useOrchestratorName()` and used for the
Henry option, portrait, and agent rig (`ForumView.tsx:64-65, 96, 132-137`;
`ForumAgents.tsx:14-21`). Existing tests cover name changes and finding the
sovereign. Job/capability briefs are sent through `askFromForum`; exporting a
brief is explicitly marked draft-only (`ForumView.tsx:97-101, 141-147`). Active
jobs come from `/api/goals/active` and open the existing goal detail seam
(`goalActivity.ts:47-70`; `ForumView.tsx:149`).

Commands run:

```text
cd ui/command-center
npm test -- --run src/components/world/forum/ForumAppView.test.tsx src/components/world/forum/ForumView.test.tsx src/components/world/forum/forumQuery.test.ts src/components/world/forum/forumRuntime.test.ts src/components/world/forum/patrolMotion.test.ts
```

Result: 5 test files and 16 tests passed. The run emitted existing React `act`
deprecation and duplicate-Three warnings only.

```text
cd ui/command-center
npm run typecheck
```

Result: passed with exit code 0.

The existing focused tests already cover adapter navigation, sovereign naming,
session/event-channel ordering, concurrent-send guards, failed history/session
handling, job/capability intent text, unavailable registry feeds, and waypoint
continuity. The new `ForumInhabitants.contract.test.ts` covers shared worker
path/engagement survival, isolation of forum records, and pausing ambient walk
while a daemon-backed worker reports `working`.

The focused run after the fix passed 7 files and 20 tests. `npm run typecheck`
was also run; it is currently blocked by unrelated pre-existing
`ForumSurfaceMaterials.ts` and `ForumSurfaceMaterials.test.ts` errors from the
parallel runtime-surfaces work (including Three.js type mismatches). The new
forum motion files introduce no reported type errors.

The refreshed textured export was also checked with the geometry-only route
harness:

```text
cd ui/command-center
npx vitest run src/components/world/forum/forumWalkRoutes.test.ts
```

Result: 12 route/geometry tests passed. The test registers a named GLTFLoader
plugin that returns a neutral `THREE.Texture`, avoiding Node's browser-only
`self`/image decode path while preserving the exported mesh transforms,
vertices, indices, and collision/BVH checks. Texture and shader behavior remain
covered by the browser smoke harness; `forumAssets.test.ts` passed unchanged
(13 tests) and did not need the geometry-only loader shim.

## Release gates

Before switching `WorkspaceRenderer` to `ForumAppView`, fix the remaining
`skill_saved` refresh gap, then test in the actual browser
app with a persisted goal/artifact, approval, cancellation, reconnect, skill
proposal and promotion with source evidence, renamed sovereign, newly registered
worker, and transitions between World and other workspaces. Keep native Unreal
claims separate until its live daemon bridge and interactive validation exist.
