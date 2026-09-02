# World View Bible — v2

Status: **APPROVED 2026-06-10 (Jesse, creative director) with amendments — see §9**
Branch: `feature/world-v2` · Swarm: World View v2 (lanes W1–W4)
Source audit: full read of `ui/command-center/src/components/world/**` at `cb28713e7`.

This document is law for the World V2 swarm. Lanes do not deviate from it; amendments
go through the coordinator and Jesse.

---

## 0. What exists today (audit summary)

The world is further along than "a hall with agents." Current scene, all inside
`WorldScene.tsx`/`WorldFurniture.tsx` (no zone modularity, nothing lazy-loaded except
WorldView itself):

- **Floating obsidian platform** (r=20) in a starfield (2,000 pts) over a wireframe void grid,
  exponential fog (`#0A0E1A`, density 0.012).
- **Rotunda**: marble floor r=15 with animated circuit inlays, 8 columns with pulsing
  cyan veins, hemispherical dome (r=16) with oculus + volumetric light shaft, 200 dust motes,
  3 rotating orbital arcs (the signature dynamic element).
- **Mezzanine library** at y=10: ring floor, spiral staircase (30 steps), continuous
  bookshelf wall with **~400 individually-meshed books**, 6 desks, 2 armchairs.
- **Four station corners** at r≈10: workbench (z=-10), library pedestal (x=+10),
  observatory (z=+10), forum (x=-10) with a fully built **Stargate** (shader event horizon,
  particle drift, stochastic electrical arcs).
- **5 cyborg agents** (~65 meshes each): Henry (crowned), Aria, Felix, Nova, the Librarian
  (mezzanine-locked). Idle sway, walk bob, facing rotation, drei Html name tooltips.
- **Camera**: orbit (damped, auto-rotate after 5s) + third-person follow with WASD.
- **Post**: Bloom (intensity 0.8, threshold 0.4, LARGE kernel) + Noise (0.12).

**Critical findings**

1. **Agent state is 100% simulated.** ~~`useAgentStates.ts` is a local wander loop with a
   TODO to wire a WebSocket.~~ **CORRECTED 2026-09-01 — this has been false for some time
   and an audit that trusted it would be wrong.** `agent_state_changed` exists and six
   emitters use it: `agent_state_tick.rs` (Henry), `steward_sweep.rs`, `strix.rs`,
   `finance.rs`, `forecaster.rs`, `picker_close_scan.rs` (which announces under the
   `financier` id — a known misattribution). Nine world sources are live end-to-end
   (orb states, goal dispatch, the decision inbox, Librarian curation, Brain memory).
   What remains is **charted silence, never a false claim**: each roster entry declares
   its own `wire` — `daemon` (something reports it), `sim` (nothing does; the §4 clamp
   holds it to idle/available), or `static` (nothing does YET — a fixed pose and a HUD
   that says what it is waiting for). The Council, Polybot and the Picker are `static`
   today. → See §9 stop item S1.
2. **Issue #87 confirmed** (since resolved): Henry's trim was hardcoded `#00D9FF`,
   identical to the then-environment accent `neonCyan` — and the global token cyan
   is `#00D5FF`, a third near-duplicate. Resolved: Henry's trim is `#F0E6D0` (§9 D2),
   and #193 normalized `neonCyan` to the canonical global `#00D5FF` (tokens
   `NEON_ACCENT`).
3. **No instancing anywhere.** Estimated ~500–550 draw calls at full population
   (books alone ~400 meshes; 5 agents ≈ 300 meshes). The 300-DC budget is achievable
   almost entirely by instancing books, columns, railing posts, and agent parts.
4. **`reduceMotion` is not respected** — `getReduceMotion()` exists in tokens.ts, unused
   in world/. V2 makes it law (§8).
5. Known per-frame allocations: `new THREE.Vector3()` per frame in WorldCamera
   third-person mode; arc geometry churn in Stargate. Fixed in place by the owning lanes.
6. **Measured baseline is ~4 fps** (see §10) — not the assumed "roughly 60." The scene is
   severely fill-rate bound: the identical scene at a 300px-tall buffer runs 38–41 fps with
   the same 1,340 draw calls. Post-processing passes + `antialias: true` + shadows at
   dpr 2 (2784×1944 buffer) dominate the frame.
7. **1,340 actual draw calls for only 71k triangles** (~53 tris/call) — the audit's
   ~500-DC estimate was optimistic; batching/instancing upside is even larger.
8. **Geometry leak:** `gl.info.memory.geometries` grows ~+40/s unbounded (reached 4,498
   in 40s) — something allocates BufferGeometries continuously without disposing. Prime
   suspect: the Stargate electrical-arc respawner (new geometry per arc every 0.3–0.5s).
   Owning lane confirms and fixes.
9. **All workspaces render simultaneously** in `WorkspaceRenderer` — the World canvas
   mounts (and renders) even when another tab is visible, and mounts before layout gives
   it size (`GL_INVALID_FRAMEBUFFER_OPERATION` spam at startup). The canvas must pause
   (`frameloop='never'` or unmount) when the World tab is not visible.
10. `shadows="soft"` is deprecated in three 0.184 and silently falls back to PCF.

---

## 1. Visual language — "cyber classical"

The identity: **a classical marble institution that runs on light.** Greco-Roman mass and
proportion; intelligence expressed as engraved, glowing circuitry in the stone. Deep space
outside, warm scholarship inside.

### Materials — the three-tier rule
Every surface belongs to exactly one tier:

| Tier | Material | Rules |
|---|---|---|
| **Stone** (structure) | Marble `#E8E4DD` rough 0.35 / dark stone `#2A2A3E` / obsidian platform | Matte. Never emissive. Receives shadows. This is 80% of any view. |
| **Metal** (mechanism) | Gunmetal `#555B6E`, worn bronze `#A08555` | Mid metalness (0.6–0.8), low emissive or none. Hinges, frames, instruments, agent bodies. |
| **Light** (intelligence) | Emissive trim, holo surfaces, particles | The only emissive tier. Always *engraved into* stone/metal (channels, inlays, seams) — never free-floating neon signage. Emissive intensity ≤ 2.0 except deliberate focal points (event horizon, crown). |

Corollaries: no PBR texture maps in v2 (geometry + flat materials only — keeps the look
graphic and the memory flat); transparency only for holo surfaces, glow rings, and the
cape; additive blending reserved for particles and arcs.

### Light
- One warm key (directional, `#FFF0D4`, shadows), one cool fill (`#8EC8E8`, no shadows),
  near-black ambient (0.08). This is the established formula — areas reuse it, re-aimed.
- Point lights are **budgeted accents** (≤ 12 total scene-wide, distance-capped, decay 2).
  Emissive materials carry the "lit by circuitry" feeling for free; prefer them over lights.
- Exactly **one shadow-casting light** in the whole scene. Shadow map 2048 at baseline;
  drops to 1024 if the budget demands (§8).

### Atmosphere
- FogExp2 `#0A0E1A` density 0.012 everywhere; areas may bias density ±0.004 for mood
  (the Antechamber runs densest).
- Scale: **1 unit = 1 meter.** Agents ≈ 2.4u tall. Doorways ≥ 3u tall — institutional, not
  domestic. Ceilings of side areas 6–9u (the rotunda dome at 18u stays the vertical climax).

### Silhouette rules for props (W2 law)
1. A prop must read at 25u distance in fog as a **single dark shape with at most one light
   accent**. If it needs two glows to read, it's two props.
2. Big simple masses, chamfered by geometry (cylinders/boxes/lathes); detail lives in the
   engraved Light-tier lines, not in poly count.
3. Every prop family shares one stone material + one metal material + the semantic emissive.
   New materials require a bible amendment.

---

## 2. Palette

Derived from `src/styles/tokens.ts` and `world/constants.ts`. The world keeps its own
constants (3D needs differ from UI chrome) but **semantic roles must visually match the
product**: cyan = intelligence, violet = memory.

### Semantic (HUD COLOR LAW — gray idle, amber working, cyan available, red error)
| Role | Hex | Use |
|---|---|---|
| `state.idle` | `#8A94A6` (token textMuted) | Agent trim/aura when idle |
| `state.working` | `#FFB347` (established neonAmber) | Agent at a station, busy-room ambience |
| `state.available` | `#00D5FF` (world neonCyan = global `NEON_ACCENT`, #193) | Agent alert/ready, steady-state |
| `state.error` | `#FF5D5D` | Error slump/flash. *Not* token danger `#FFB4A2` (too soft to read in fog at distance); HUD panels may keep `#FFB4A2`. |

**No green anywhere in the world.** `success`-green is a UI-chrome token only.

### Environment
| Name | Hex | Use |
|---|---|---|
| `deepVoid` | `#0A0E1A` | Background, fog |
| `marble` | `#E8E4DD` | Primary stone |
| `marbleVein` | `#8B7E6F` | Veining, Librarian trim |
| `darkStone` | `#2A2A3E` | Secondary stone, prop bases |
| `gunmetal` | `#555B6E` | Agent bodies, mechanisms |
| `bronze` | `#A08555` | Worn metal accents, chevrons |
| `neonCyan` | `#00D5FF` (global `NEON_ACCENT`, #193) | Intelligence accent (circuits, available-state) |
| `violet` | `#A855CC` (token purpleBright) | **Memory accent — new.** Owns the Brain Archive area + memory-themed props. Currently absent from the world; v2 introduces it as the second emissive identity. |
| `neonAmber` | `#FFB347` | Working-state, warm library light |
| `horizonBlue` | `#5599FF` | Stargate/Mesh family only |

### Per-agent trim (toga/visor)
Henry **`#F0E6D0` warm white-gold** (resolves issue #87 — see §9 D2), Aria `#FFB347`,
Felix `#FF6B9D`, Nova `#A78BFA`, Librarian `#8B7E6F`. Agent trim is *identity*; state is
expressed by the §5 state channels, never by repainting identity trim.

All of the above ships as `world/shared/palette.ts` (single source; lanes import, never
inline hex).

---

## 3. Area map — main hall + 5 zones

Zones are **rooms off the rotunda**, replacing today's open station corners. Each is
lazy-loaded (`React.lazy` + suspense inside the scene graph), reached through a threshold
readable from the hall center, themed to a product tab. The existing station pedestals
become threshold markers. The mezzanine library remains part of the main hall.

**Naming law (creative director):** the area map doubles as the marketing
feature-showcase structure. Zone display labels (plaques, HUD tooltips, docs) use the
product tab name **exactly** where one exists: **Build** (A1), **Brain** (A2),
**Automate** (A4), **Mesh** (A5, future). The Lab (A3) covers Skills + Trace until those
have a single tab name.

Layout (plan view, rotunda r=15, thresholds punched through the colonnade line):

```
                    LAB (z=-24)
                 ┌──────┴──────┐
        AUTOMATE │   ROTUNDA   │ WORKBENCH
        (x=-24) ─┤  + mezz lib ├─ (x=+24)
                 └──────┬──────┘
                  BRAIN ARCHIVE (z=+24)
   ANTECHAMBER: behind the Stargate, NW diagonal (x=-19, z=-19)
```

**A1 — The Workbench Wing** (tab: Build/Terminal · landmark: a 6u bronze gantry crane arm
reaching over the threshold). A working bay: long stone benches in rows, holo terminals,
tool racks, cable runs in floor channels. This is where agents physically sit to work —
densest concentration of W2 anchor points. Mood: the brightest, warmest zone; amber task
lights over each station.

**A2 — The Brain Archive** (tab: Memory · landmark: a 7u rotating violet **memory core** —
a lathe-turned obelisk with orbiting index rings, visible down the south threshold). The
deep stacks under the mezzanine's discipline: shelf canyons (instanced), specimen plinths
holding glowing memory shards, a consultation table. The only violet-dominant zone —
stepping from cyan hall to violet archive is the strongest mood shift in the product.

**A3 — The Lab** (tab: Skills/Trace · landmark: a 5u armillary-style **experiment orrery**
suspended in the threshold sightline — the existing ArmillarySphere promoted and enlarged).
Absorbs today's observatory corner. Benches of glass instruments, a trace-wall (tall holo
panel streaming faint glyphs), the telescope relocated here. Mood: cool, precise; cyan
instrument glow against dark stone.

**A4 — The Automate Hall** (tab: Automate · landmark: a wall-mounted 6u **horologium** —
a clockface of concentric bronze rings with cyan tick lights). A narrow gallery of
scheduler boards: stone steles engraved with glowing timetable grids, a long planning
table. Mood: rhythmic, quiet.

The gallery is **bound to the real scheduler** (2026-09-01): one stele per registered job,
its grid coloured by that job's real last outcome (`/api/job-health`), refreshed on
`schedule_changed` and a 60s backstop. No jobs, or no answer from either source, renders an
empty gallery — never a reassuring one. The horologium keeps its slow clock pulse, which is
true whatever the scheduler is doing; the **amber tick is reserved for a run genuinely in
flight** (`currently_running` off `/schedule/list`) and is otherwise still.

**A5 — The Forum Antechamber** (tab: future Mesh · landmark: the existing **Stargate**
itself, relocated to frame the NW threshold). Through the gate's doorway: a small liminal
chamber, the cyber-classical language at its most austere — bare dark stone, densest fog
(+0.004), a single cold downlight. Centerpiece: an **inactive portal ring** (dormant
sibling of the Stargate, no event horizon, chevrons unlit) above an engraved plaque reading
`MESH: NOT CONNECTED`, flanked by dormant Chitin sigils (unlit engraved Light-tier channels).
**No fake activity, no simulated peers** — the room honestly renders Mesh's offline state
via the `meshStatus` contract (§6). Henry can walk in and back (W3). Marketing note: this
room is the "coming soon" shot.

Thresholds: each is a portal frame in the colonnade — two columns + lintel + a floor seam
of the area's accent color that leads the eye in. The zone interior renders only when
loaded; an unloaded zone shows fog and the landmark's distant silhouette (a cheap imposter
allowed: ≤ 3 meshes).

---

## 4. Agent character spec (W3 law)

Body language per HUD state, on top of existing identity (trim color, crown for Henry):

| State | Color channel | Posture & motion |
|---|---|---|
| **idle** | gray `#8A94A6` | Standing, slow ambient sway (existing), occasional weight shift. Visor dim (emissive 0.3). |
| **working** | amber `#FFB347` | **Seated/engaged at a W2 anchor point**, leaning in; hands toward the work surface; visor bright; small periodic head nods. Walk to station uses existing locomotion. |
| **available** | cyan `#00D5FF` | Alert stance: straightened spine, head up, visor at full; aura ring at feet breathing slowly. |
| **error** | red `#FF5D5D` | Distinct silhouette: shoulders dropped, head bowed 20° (the "slump"), visor flickering red at 2Hz for 3s then steady dim red. |

- **State channels** = visor + joint glow rings + cape circuit lines + feet aura. Identity
  trim (toga edge) never changes color.
- Transitions tween over 0.8s (color lerp + posture blend). No teleporting between
  postures.
- Locomotion: straight-line + ease (existing system) between hall and anchor points, plus
  through the Antechamber threshold for Henry. **No pathfinding engine** — scope-creep
  fence stands.
- Name labels: existing drei Html tooltips kept; always-on small label only when the
  camera is within 18u.
- Henry: white-gold trim `#F0E6D0` + crown. Crown gems pick up the *state* color — the
  one sanctioned identity/state crossover, so Henry's state reads from across the room.
- Scale: 2.4u tall, unchanged. Seated height ≈ 1.7u — W2 seat anchors assume this.

### Sim-state rule (creative director amendment, LAW)
Agents whose state `source === 'sim'` may only display **idle (gray)** and
**available (cyan)** — never amber working or red error. Amber and red are claims that
real work is happening or really failing; a simulation making that claim is a lie the
user can see. Today: Henry and the Librarian (real daemon sources) show all four states;
Aria/Felix/Nova wander as gray/cyan ambient life. The clamp is **enforced in
`shared/agentStatus.ts`** so it reaches the pixels, not just the types. When the daemon
`AgentStateChanged` follow-up ships, sim agents graduate to the full state range
automatically through the same boundary — no W3 code change.

### Manual character control — puppeting RELAXED (Jesse's ruling, 2026-06-20, LAW)
The earlier "no per-user puppeting" stance is **lifted**. When zoomed into an agent
(third-person), arrow keys / WASD **drive that agent**, overriding its autonomous walk
*while the keys are held*; on release the behavior layer resumes autonomy at the new
position. This is an **intended feature, not a regression** — a future audit must not
flag puppeting as a bug.

Reasoning: the World View is a **place you inhabit and walk through**, not just an
orbiting view. You can take the wheel
and explore as a character. Implemented via `nudgeAgent()` in `agents/motion.ts`
(clears the autonomous path so the nudge sticks, faces travel direction, honors the
Librarian ring lock); wired through `WorldView`'s `onMoveAgent` → `WorldCamera`
third-person handler. The autonomy law (§4 locomotion, sim-state clamp) otherwise stands:
manual control is a transient override, never a new state source.

---

## 5. Module layout & lane ownership

Target layout under `ui/command-center/src/components/world/`:

```
world/
  WorldView.tsx          # entry: Canvas + HUD mounts (coordinator-owned, thin)
  WorldScene.tsx         # thin composition: hall + zone mounts (W1)
  areas/                 # W1 — hall geometry, 5 zones, thresholds, lazy mounting
  props/                 # W2 — instanced prop library, anchor registry
  agents/                # W3 — CyborgCharacter, WorldCharacters, useAgentStates → here
  atmosphere/            # W4 — lights, fog, starfield, dust, post, reactive ambience
  camera/                # W4 — WorldCamera, tour mode
  shared/                # Phase 0 — FROZEN after approval (see §6)
  hud/                   # existing HUD files move here untouched; HenryIdentityTab FROZEN
```

Migration of existing monoliths (lane owns the move):
- `WorldScene.tsx` interior geometry → W1 `areas/hall/*`; the file itself becomes
  composition-only and is owned by W1.
- `WorldFurniture.tsx` → W2 `props/*`, instanced in the process. Deleted when empty.
- `CyborgCharacter.tsx`, `WorldCharacters.tsx`, `useAgentStates.ts` → W3 `agents/`.
- Lighting/fog/starfield/dust/post from `WorldScene.tsx` + `WorldPostProcessing.tsx`
  → W4 `atmosphere/`; `WorldCamera.tsx` → W4 `camera/`. (Sequencing makes this safe:
  W1 lands the skeleton split first; W3/W4 rebase.)
- `Stargate.tsx` → W1 `areas/antechamber/` (it becomes the threshold landmark).

Nothing outside `world/` except read-only imports of `styles/tokens.ts` and the events
client. HUD color semantics and the frozen `HenryIdentityTab.tsx` stand.

---

## 6. Shared contracts (`world/shared/`) — written in Phase 0, frozen after

**Frozen-amendment policy (creative director, 2026-06-11):** frozen modules may receive
**bug fixes only** — coordinator-authored, commit message prefixed `FROZEN AMENDMENT:`,
all consuming lanes notified before they next capture evidence. Interface changes remain
forbidden. (Precedent: the PerfSampler render-loop-takeover fix, found by Lane W2.) This covers bug-fixes to EXISTING frozen modules only; a NEW shared module requires coordinator blessing before a lane adds it (e.g. shared/tendingBank.ts, blessed 2026-06-15).

**Lane setup verification (standing rule, both swarms):** every lane's first action is to
paste `pwd` and `git worktree list` output as evidence of correct setup, and the
coordinator verifies it BEFORE the lane's first commit. The main checkout at
`~/dev/permagent-runtime` is the operator's live working tree — lanes never work there.

**`palette.ts`** — §2 as typed constants.

**`anchors.ts`** — the W2→W3 interface. W2 publishes early, then freezes:
```ts
export type AnchorKind = 'seat' | 'stand' | 'lean';
export interface AgentAnchor {
  id: string;                  // e.g. 'workbench.bench2.seatL'
  areaId: AreaId;
  kind: AnchorKind;
  position: [number, number, number];  // world-space
  facing: number;              // Y rotation the agent assumes when engaged
}
export function getAnchors(areaId?: AreaId): AgentAnchor[];
export function claimAnchor(id: string, agentId: string): boolean;  // simple in-memory lock
export function releaseAnchor(id: string): void;
```

**`meshStatus.ts`** — the Antechamber contract (amendment). The room's status surface
reads ONLY from here; when Mesh ships, real state plugs into this one module:
```ts
export type MeshStatus =
  | { state: 'offline' }                                  // today's constant
  | { state: 'connecting' }
  | { state: 'connected'; peerCount: number };
export function useMeshStatus(): MeshStatus;              // returns { state: 'offline' }
```
No daemon code, no new endpoints, no WebSocket additions behind it today.

**`agentStatus.ts`** — the W3 state source boundary. Maps real daemon signals to the HUD
state machine; simulation only fills documented gaps (§9 S1):
```ts
export type AgentHudState = 'idle' | 'working' | 'available' | 'error';
export interface AgentRuntimeState {
  id: string; name: string; hudState: AgentHudState;
  // Honesty bit — evidence recordings of working/error must show 'daemon'.
  // 'static' = a real seat whose emitter has not shipped: never animated.
  source: 'daemon' | 'sim' | 'static';
}
export function useAgentRuntimeStates(): AgentRuntimeState[];
```
Real inputs wired today: `/api/agents` roster; Henry `/api/henry/status`
(`idle→available`, `in_conversation|tool_call→working`); Librarian `LibrarianDescribe*`
(`describing→working`, `error event→error`, else `available`).

**`instancing.ts`** — helper for budget law: `<InstancedProp geometry material count …/>`
wrapper + a debug counter that reports instances-per-draw-call for evidence screenshots.

**`perf.ts`** — the probe used for all lane evidence: samples fps / `gl.info.render.calls`
/ triangles once per second to an on-screen dev overlay (toggle `~`, extends the existing
FPS counter). One shared measurement method = comparable numbers across lanes.

---

## 7. Camera & ambience (W4 scope notes)

- Orbit stays primary; constraints as today. Tour mode: a slow drift on a precomputed
  spline through hall → each threshold → antechamber, cut off by any user input;
  respects `reduceMotion` (renders as a static establishing shot instead).
- Activity-reactive ambience: room responds to the count of `working` agents — column-vein
  emissive intensity, dust/particle rate, orbital-arc speed scale gently (≤ 1.5× idle
  values). Driven by `useAgentRuntimeStates()` only.
- Post-processing: Bloom is the first thing cut (§8). Noise is cheap, stays.

---

## 8. Performance budget (LAW)

Baseline numbers measured from the running scene are recorded in §10. **The baseline is
~4 fps / 1,340 draw calls — v2 is a performance remediation as much as a content
expansion.** Hitting 60fps is not achievable by instancing alone; the fill-rate ceiling
must come down first. Mandatory remediations, owned as noted:

1. **dpr cap at 1.5** (Canvas `dpr={[1, 1.5]}`) and **`antialias: false` whenever
   post-processing is on** (the post chain already controls AA) — W4.
2. **Pause rendering when the World tab is hidden** (`frameloop` gating in WorldView /
   visibility from the workspace store) — W4, wiring in WorldView with coordinator review.
3. **Geometry leak — CARVED OUT of v2** (creative director): it is a live bug on main.
   The coordinator fixes it as a standalone hotfix PR to main (minimal diff; evidence:
   geometry count flat over 5 minutes, before/after counters). `feature/world-v2`
   rebases over it after merge.
4. **Instancing/batching** to take 1,340 DC under 300 — all lanes per ownership.
5. Re-measure after 1–3 land; if 60fps is still out of reach with post on, Bloom is cut
   per the standing rule.

Budget for the final combined v2 scene, all 5 zones loaded, 5 agents, post on:

| Metric | Budget | Notes |
|---|---|---|
| Frame rate | 60fps on Apple Silicon baseline | measured via `shared/perf.ts` |
| Draw calls | **< 300** | per-category: hall ≤ 80 · all zones ≤ 100 · agents ≤ 60 (≤ 12/agent via instanced parts or merged geometry) · atmosphere+post ≤ 60 |
| Triangles | **< 500k** | |
| Point lights | ≤ 12 scene-wide; 1 shadow caster | |
| Per-frame allocations in `useFrame` | **0** | existing violations fixed by owning lane |
| InstancedMesh | MANDATORY for any prop placed > 2× | books, posts, columns, shelves, steles |
| Zones | lazy-loaded; unloaded zone ≤ 3 imposter meshes | |
| `reduceMotion` | static fallback for particles, arcs, tour, reactive ambience | |

Every lane PR includes before/after fps + draw calls + triangles from `shared/perf.ts`
on the running scene. Budget exceeded and not recoverable by instancing/LOD → stop,
propose cuts (Bloom first, then shadow map 2048→1024, then starfield density).

---

## 9. Decisions & stop items — RESOLVED 2026-06-10

All items below approved by Jesse. Amendments incorporated: sim-state rule (§4),
zone naming law (§3), leak carve-out (§8), **W1 blockout checkpoint** — W1's blockout
milestone is a Jesse review gate: all five zones in gray geometry + fly-through recording
BEFORE any detail pass begins. The checkpoint applies to W1 only.

- **D1 — Area themes & layout (§3).** Observatory is absorbed into the Lab; Antechamber
  is the fifth area (per amendment) rather than an addition.
- **D2 — Issue #87:** recommend **warm white-gold `#F0E6D0`** for Henry (king/orchestrator
  reading, matches the existing gold crown; electric blue `#4DA8FF` is the runner-up but
  stays in the crowded cool-cyan family). Applies to `togaTrimColor` and the HenryHUD trim
  constant (HUD side is a visual-only change to a non-frozen file).
- **D3 — Violet as the Brain Archive's signature (§2, §3).** Introduces the product's
  "violet memory" into the world for the first time.
- **S1 — /events gap (stop condition, pre-triggered):** no daemon event stream
  distinguishes per-agent working/available/error. W3 binds the real signals that exist
  (Henry status poll, Librarian events, agent roster) through `shared/agentStatus.ts` and
  marks everything else `source: 'sim'`. **Proposed daemon follow-up issue:** emit agent
  lifecycle events (`AgentStateChanged { id, state }`) on `/events`. No daemon code in
  this swarm. W3's evidence requirement is met for Henry + Librarian with real events;
  Aria/Felix/Nova run on the sim until the follow-up ships. Jesse to confirm this
  evidence standard.
- **D4 — Henry trim vs HUD:** `HenryHUD.tsx` trim constant update (D2) touches a HUD file
  outside lane geometry ownership — visual-only, flagged here per standing rule 2.

## 10. Measured baseline (Phase 0 capture, 2026-06-10)

**Method:** branch `feature/world-v2` (== `origin/main` @ `cb28713e7`), `vite` dev build,
Google Chrome 149 driven over CDP, real GPU (`ANGLE Metal Renderer: Apple M1` — verified
not SwiftShader; headed and headless+Metal runs statistically identical). Daemon token
injected so the real World workspace loaded. Viewport 1600×1000 @ deviceScaleFactor 2;
canvas 1392×972 CSS px → **2784×1944 drawing buffer (dpr 2)**. Probe: temporary
`useFrame`/`useThree` component sampling `gl.info` once per second
(`autoReset=false`, manual reset). Display compositor cap: 61 Hz.

**Samples** (headed, after 15s settle, 2s apart):

| # | fps | draw calls | triangles | geometries | textures | programs | dpr |
|---|-----|-----------|-----------|------------|----------|----------|-----|
| 1 | 4.0 | 1340 | 71,297 | 1,478 | 20 | 16 | 2 |
| 2 | 3.7 | 1340 | 71,297 | 1,558 | 20 | 16 | 2 |
| 3 | 3.8 | 1340 | 71,297 | 1,638 | 20 | 16 | 2 |
| 4 | 3.5 | 1340 | 71,297 | 1,638 | 20 | 16 | 2 |
| 5 | 3.6 | 1340 | 71,297 | 1,718 | 20 | 16 | 2 |

**Reference experiment:** same scene squashed to a 1600×150 viewport (300px-tall buffer):
38–41 fps at the same 1,340 draw calls → fill-rate bound, not geometry bound.

**Leak:** `geometries` climbs ~+40/s monotonically (4,498 after 40s); textures/programs flat.

**Console at startup:** `GL_INVALID_FRAMEBUFFER_OPERATION … Attachment has zero size`
(canvas renders before layout sizes it / mounts hidden), `PCFSoftShadowMap has been
deprecated` (three 0.184), polling 404 spam, Tauri `transformCallback` exceptions
(expected outside Tauri).

**Caveat:** numbers are from Chrome/Blink; the shipped app runs WKWebView, where absolute
fps may differ — but draw calls, triangles, the leak, and the fill-rate conclusion are
renderer-independent. Lane evidence uses `shared/perf.ts` in the same dev-build setup for
comparability.

**Screenshots:** `docs/design/world-v2-baseline/01-default-orbit.jpg`, `02-settled.jpg`,
`03-late.jpg` (downscaled evidence copies; auto-rotate barely progressed between shots —
consistent with ~4 fps).
