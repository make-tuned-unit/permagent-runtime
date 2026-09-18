# World integration contract

World uses the existing Permagent runtime. Rendering and ambient movement do not
create job state, grant permissions, or prove a capability exists.

The browser integration component is `ForumAppView`. It passes the real workspace
navigation, goal-detail action, Skills overlay and agent-settings action into the
forum.

## Shipping route and rollback

`WorkspaceRenderer` maps the `world` tool to a single `lazy()` boundary that
selects `ForumAppView` by default, so the Solar Forum is the World the app
renders. Both Worlds stay behind that boundary: neither Three.js bundle is
fetched until a World panel is actually rendered (the build emits
`assets/ForumAppView-*.js` and its own `ForumAppView-*.css`, reached only
through a dynamic import from the app chunk).

The one escape hatch is `world/forum/worldRoute.ts`. `shouldUseLegacyWorld()`
returns true only when `localStorage['permagent.world.legacy'] === '1'`; any
other value, an absent key, or a storage access that throws keeps the shipping
route. Rolling back is therefore: run
`localStorage.setItem('permagent.world.legacy', '1')` in the app and reload —
no rebuild, no redeploy. Removing the key and reloading returns to the forum.
The flag is read once, when the World panel first renders; it has no UI, and it
is a rollback lever rather than a supported second mode.

Panel behaviour: the forum fills its parent, not the viewport (no `100vw`/`dvh`
units and no minimum height in `forum.css`), so it resizes with the panel. The
app mounts every workspace at once and hides inactive ones with `display:none`,
so `ForumView` gates both the canvas frameloop and walk mode on a ResizeObserver
reading of its own container: a hidden or zero-size panel stops rendering and
leaves walk mode, which unmounts `ForumWalk` and releases its window key
listeners and pointer lock. `ForumWalk` additionally ignores keys once focus
moves to an element outside the forum shell. Every forum asset
(`solar-forum.glb`, `forum-bird.glb`, `forum-characters/*`,
`observatory-vault.glb`) is loaded as `${import.meta.env.BASE_URL}world/…`, so
the same code resolves under the browser's `/ui/` base and the Tauri app's `/`;
they live in `ui/command-center/public/world/` and ship in `dist/world/`.

## MESH gate and Agora

The World leads to the MESH. The Sovereign walks out through a monumental ring
on the gate court at the north-west (polar angle 135 degrees, radius 56 m) and
arrives in the **Mesh Agora** — the antechamber of `WORLD_VIEW_BIBLE.md` §3 A5.

**What triggers it.** Three ways in, all of them the same state change in
`ForumView` (`agoraOpen`), and none of them a new tool, tab or panel:

- Walking through the ring. `ForumWalk` tracks the walker every frame and asks
  `forum/meshPortal.ts` whether the step just taken crossed the ring plane
  (`crossedGatePlane`). The test is on the point of intersection, so a step
  that passes wide of the 4.6 m ring or outside the 0–6 m height band is not a
  crossing. An outward crossing releases pointer lock and opens the Agora
  without leaving walk mode; walking back in through the ring returns to the
  forum.
- `E · Enter the MESH`, offered while the walker is standing in the exedra
  beyond the ring (`inExedra`) — the way in for someone who stepped round the
  side of the gate rather than through it. Same key as meeting an agent.
- Selecting the **Mesh gate** district, which focuses `gateApproachPoint()`,
  and then either selecting it again or pressing **Enter the MESH** in the
  agent desk. This is the path for anyone who never enters walk mode.

Escape and the **Return to the Forum** button both leave, putting the walker
back down on the spur six metres in front of the ring. Leaving the World panel
closes the Agora with it.

**What it reads.** `MeshAgora` reads `world/shared/meshStatus.ts` —
`useMeshStatus()` — and nothing else. Offline renders the dormant ring and an
engraved `MESH: NOT CONNECTED`; `connecting` renders `MESH: CONNECTING`;
`connected` lights the ring's horizonBlue channel and chevrons, mounts the
event-horizon shader inside the ring, and prints `MESH: CONNECTED · N peers`
with the peer count the contract supplied. There is no daemon call, no
endpoint, no socket and no polling behind any of it.

**What it never fakes.** Mesh is Permagent's peer network and there is no live
Mesh backend today, so `useMeshStatus()` returns `{ state: 'offline' }` and the
room says so. No simulated peers, no ambient traffic, no "connecting" animation
that is not a real connecting state, and no count that is not the count the
contract returned. When Mesh ships, real state plugs into that one module and
the room updates without touching the Agora.

**How it renders.** The Agora is a scene branch of the forum's single Canvas,
not a second Canvas: `ForumView` hides the forum group (`visible={false}` —
zero draw calls, GLB never re-fetched, collision BVH never rebuilt) and mounts
`MeshAgora` in place of `ForumLighting`. Measured in the browser: 22 draw calls
and ~1.8k triangles in the Agora against 214 and 2.26M in the forum. The event
horizon respects `getReduceMotion()` by freezing rather than disappearing.

## Implemented connections

- Configured sovereign identity and name-change revisions use the same API as Chat.
  Startup failures retry. Henry's status poll cannot pile up overlapping requests.
- Existing World event subscriptions provide actual agent activity and task events.
  Avatar walking uses an isolated forum motion store and pauses during reported work.
  Mounting/unmounting World preserves live worker paths and engagement. Find and
  peer look-at resolve the visible forum pose without changing daemon state.
- Queries, jobs and capability-development requests enter the existing conversation.
  Its session history, authenticated event channel, streaming, execution trace,
  pending approvals and cancellation remain the app's own implementations.
- Active project goals open the existing goal detail. Registered workers open their
  real settings using the registry ID, including workers without a bespoke avatar.
- Saved skills and repetition-derived skill proposals come from the real endpoints,
  refresh after work/proposal/skill-saved events and poll as a backstop. A failed fetch is shown
  as unavailable, not an empty list of capabilities.
- Capability development asks the sovereign to inspect prior conversation, use
  available tools, implement and test the gap, and register evidence-backed reusable
  skills within existing runtime permissions. No skill is silently installed by
  opening World. Newly registered skills appear from the backend registry.

## Evidence and remaining release gates

`runtime-contract.json` records authenticated GET checks against the local daemon.
The first cold status request timed out; a subsequent check returned all six
contracts successfully, including status in 0.435 seconds. This is not a write-path
or end-to-end gameplay test.

Unit checks cover waypoint continuity, shared-session connection before sending,
job/capability intent, identity recovery/rename, registry errors, and the app adapter's
real navigation/detail/settings actions. The browser build and exported geometry
checks are separate from Unreal's native movement evaluation.

The route switch is covered by unit tests only: `worldRoute.test.ts` pins the
flag semantics (including a throwing storage), and `WorkspaceRenderer.test.tsx`
pins that a `world` panel renders the forum by default and the legacy WorldView
under the flag. `npm run typecheck`, `npx vitest run src/components/world
src/components/workspaces` and `npm run build` pass.

**These in-app gates are still required and have not been run.** A unit test
that the route selects `ForumAppView` is not evidence that the forum works in
the app. Still to be exercised in the running app, with the rollback flag as the
fallback if any of them fails: a representative job with persisted goal and
artifact, an approval, cancellation, reconnect, a skill proposal/promotion with
source evidence, a renamed sovereign, a new registered worker, and transitions
between the app's World and other workspaces — including resizing the World
panel, switching away and back, and confirming the hidden panel stops rendering
and does not capture keyboard input. Newly registered workers appear in the live
registry; automatic bespoke 3D bodies for arbitrary new workers remain a
separate extension.

Unreal currently has skeletal bodies, ambient sequences and an identity snapshot.
It does **not** yet have the browser's live daemon/event/approval bridge. That bridge
and interactive gameplay validation are mandatory if Unreal is the app's runtime.
Native scenery or animated bodies alone do not satisfy this integration contract.
