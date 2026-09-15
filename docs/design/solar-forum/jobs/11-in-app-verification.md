# In-app verification of the shipping World route

Date: 2026-09-14. Branch: `feat/solarpunk-forum` (clone
`recovery/solar-forum`, HEAD `c2ab4087`).

This exercises the release gates listed in `jobs/03-report.md` and
`APP_INTEGRATION.md` **inside the real Command Center app**, against the live
local daemon (`/Applications/Permagent.app/Contents/MacOS/permagentd agent
--host 127.0.0.1 --port 3001`, launchd). It is not a unit test and not the
standalone forum page: every number below comes from the app's own workspace
route, its own API client and its own conversation.

No daemon settings, credentials, workers, billing or identity were changed.
Every prompt sent is prefixed `[world-e2e-test]`.

## Commands run

```text
# 1. dev server (from ui/command-center)
npm run dev -- --host 127.0.0.1 --port 5284

# 2. new harness, all gates in one pass (from ui/command-center)
node scripts/verify-world-in-app.mjs
GATES=route,legacy,ask node scripts/verify-world-in-app.mjs     # subset form
FORUM_SOFTWARE_GL=1 node scripts/verify-world-in-app.mjs        # software-GL form

# 3. WKWebView (the engine Tauri uses on macOS), from the repo root
swiftc -O -o /tmp/WorldWebKitInAppEvidence \
  ui/command-center/scripts/WorldWebKitInAppEvidence.swift
SNAPSHOT_PATH=assets/world/forum/browser/in-app-world-webkit.webp \
  /tmp/WorldWebKitInAppEvidence
```

New files (evidence only, no `src/` changes):

- `ui/command-center/scripts/verify-world-in-app.mjs`
- `ui/command-center/scripts/WorldWebKitInAppEvidence.swift` (a copy of
  `scripts/blender/WorldWebKitEvidence.swift`; the original is untouched)
- `docs/design/solar-forum/in-app-report.json` (machine-readable run record)
- `assets/world/forum/browser/in-app-*.png`

## Serving and proxy check (step 1)

```text
GET http://127.0.0.1:5284/ui/        -> 200   (the real app, not forum.html)
GET http://127.0.0.1:5284/status     -> 200   (proxied to the daemon)
GET http://127.0.0.1:5284/api/status -> 404   (no such daemon route; /status is the health route)
GET http://127.0.0.1:5284/config     -> 401   (proxy works; the route requires a bearer token)
```

`/api/status` — the endpoint named in the task — **does not exist** on the
daemon. `api.getHealth()` uses `/status` (`src/lib/api.ts:1094`), which is
unauthenticated and returns 200 through the vite proxy. Everything else is
fail-closed 401 (`crates/goose-server/src/middleware/auth.rs`).

Authentication is the app's own browser path, not an invented credential: the
browser build reads the daemon bearer token from
`localStorage['permagent-daemon-token']` (`src/lib/api.ts:23,76-92`), captured
from a `#token=` pairing fragment. The harness writes that same key with
`context.addInitScript` before any page script runs, reading the value at
runtime from the daemon's own `~/.permagent/secrets/daemon_token.json`
(`crates/goose-server/src/state.rs:1354-1381`). The token is never printed and
never written to an evidence file (see the redaction note under Bugs).

## Gate results

| # | Gate | Result | Evidence |
|---|------|--------|----------|
| — | `world` tool renders the forum in the app | **PASS** | `in-app-world.webp`, GLB 200, 80.9 FPS |
| — | rollback lever (`permagent.world.legacy`) | **PASS** | `in-app-world-legacy.webp` |
| a | query from the forum ask control streams into the existing conversation | **PASS** | `in-app-ask.webp` |
| b | job brief → persisted goal | **NOT-EXERCISED** (no goal was created) | `in-app-job-cancel.webp` |
| b | cancellation via the app's cancel control | **PASS** | `in-app-job-cancel.webp` |
| c | pending approval | **PASS** (rendered, not acted on) | `in-app-approvals.webp` |
| d | reconnect | **PASS** | `in-app-reconnect-server-stopped.webp`, `in-app-reconnect-recovered.webp` |
| e | sovereign name = configured orchestrator name | **PASS** | `in-app-sovereign.webp` |
| f | skills panel + saved-skills list matches the API | **PASS** | `in-app-skills-panel.webp` |
| g | workspace switch away/back, no key or pointer-lock leak | **PASS** | `in-app-other-workspace.webp`, `in-app-world-return.webp` |
| 4 | WKWebView render/FPS | **NOT-EXERCISED** — window is occluded in this session | `in-app-world-webkit.webp` |

### Route — the app's `world` tool renders `ForumAppView`

Workspaces returned by the daemon: `Home, Projects, People, Build, Grow,
Finance, Automate, World, Brain`; only `World` hosts the `world` tool. Clicking
its sidebar row renders the forum.

```json
"glb": { "url": "/ui/world/solar-forum.glb?v=c4fcaa378a8cf02f", "status": 200 },
"perf": { "fps": 80.9, "calls": 203, "triangles": 2258284, "geometries": 125,
          "textures": 35, "programs": 35, "dpr": 1 },
"renderer": "ANGLE (Apple, ANGLE Metal Renderer: Apple M4, Unspecified Version)",
"sampling": { "supported": true, "width": 1022, "height": 972,
              "rgbVariance": 7510.43, "alphaIgnored": true },
"legacyFlagInStorage": null
```

Visible WebGL canvas 1022×972 inside the panel, distributed RGB samples from a
canvas-clipped PNG are non-uniform (variance 7510, alpha ignored), 14 district
buttons, `Day / follows system appearance`. The GLB request is the app's own
`/ui/`-based URL, so the `BASE_URL` contract holds on the app route.

### Rollback lever

With `localStorage['permagent.world.legacy'] = '1'` and a reload, the World
panel renders **zero** `.forum-shell` elements and the legacy `WorldView`
instead (`THE FORUM / OPENS WHEN THE MESH DOES … GETTING AROUND`). Removing the
key and reloading brings the forum back (`forumBackAfterClear: true`). The flag
was cleared at the end of the run; it is not left set.

### (a) Query from the forum ask control — PASS

Type `Query`, brief `[world-e2e-test] reply with the single word PONG`,
criteria `a one-word reply`, button label `Ask Henry`. Transcript tail:

```text
You 01:44 PM
Question from the World forum. Selected agent for context: Henry. This is a
request to the orchestrator, not a claim that this specialist has been directly
assigned. … [world-e2e-test] reply with the single word PONG …
Agent 01:44 PM
PONG
```

Network (redacted query strings):

```json
POST /api/sessions                      200
GET  /api/sessions/20260914_11          200
GET  /api/sessions/20260914_11/cost     500     <-- see Bugs
GET  /sessions/20260914_11/events?token=REDACTED 200
POST /sessions/20260914_11/reply        200
```

Connection indicator in the forum conversation: `connected`. The reply arrived
on the app's own authenticated SSE channel, in the app's own conversation
surface — not a forum-private transport.

### (b) Job brief and cancellation — cancellation PASS, persisted goal NOT-EXERCISED

Type `Job`, brief `[world-e2e-test] Do not use any tools and do not create
files. Just print the numbers 1 to 60, one per line, nothing else.` Button label
`Ask to run job · Henry`.

- Streaming was observed (`Stop reply` rendered), then clicked. Network:
  `POST /sessions/20260914_11/cancel -> 200`. Afterwards the `Stop reply`
  control is gone (`stopVisibleAfterCancel: 0`) and the connection indicator is
  back to `connected` with no `· reply in progress`.
- **No goal was persisted.** `/api/goals/active` returned `{"count":0,"goals":[]}`
  before and after, and the forum's own feed read `0 active jobs reported` both
  times. A job brief is a request to the orchestrator; on this daemon it was
  answered conversationally and created no goal, so "a representative job with
  persisted goal and artifact" is **not** satisfied by this run. An earlier
  identical attempt with a one-word answer (`ACK`, session `20260914_5`) also
  produced no goal. Gate (b)'s goal/artifact half still needs a brief the
  orchestrator actually turns into work.

### (c) Approval — PASS (rendered, nothing approved)

The daemon had 6 pending decisions during the run
(`GET /api/decisions -> 200`, `summary.total_pending: 6`): one `risk_gate`
tier 1 (`IREN looks stretched — consider taking the gain`) and five
`council_action` tier 2. The forum conversation rendered the shared
`ChatPendingDecisions` dock (`[data-testid="chat-pending-decisions"]` present,
`From your Decision Inbox · Open the Inbox · 3 more waiting`), i.e. the same
approval surface the chat uses, inside the forum.

Nothing was approved, rejected or discarded.

### (d) Reconnect — PASS

Two mechanisms were tried; only the second is a real drop.

1. **Route interception / offline emulation is insufficient.** Aborting
   `**/sessions/*/events*` does nothing to an EventSource that is already open —
   it is not re-issued. Taking the context offline does block the page
   (`fetch('/status')` → `ERR Failed to fetch`) but the already-established
   stream socket is not torn down, and the indicator stayed `connected`
   throughout. Recorded as `offlineEmulation` in the JSON, not as a pass.
2. **Killing the dev server is a real drop**, because the SSE goes through the
   vite proxy. Result:

```text
statusBeforeServerStop   "connected"
devServerPidsKilled      2        devServerPortClosed  true
statusAfterServerStop    "disconnected"
forum still rendering while disconnected: canvas 1022x972, visible
devServerBackUp          true
statusAfterViteReload    "disconnected"   (vite's HMR client reloads the page)
statusAfterServerRestart "connected"
```

The forum kept rendering while the daemon channel was gone, the app reported
`disconnected` honestly rather than pretending, and after the transport
returned the World route came back on its own (the forum was already visible
without re-clicking) and the conversation reconnected when used.

### (e) Sovereign name — PASS

```text
GET /api/agent/identity -> 200  { "first_name": "Henry", "last_name": null }
forum note   : "Henry / Your sovereign orchestrator · leads your team / Find Henry"
find button  : "Find Henry"
roster option: "Henry · Sovereign orchestrator"
```

The name was **not** renamed and restored — it was only read. A rename test was
deliberately skipped to avoid mutating the live sovereign identity.

### (f) Skills — PASS

```text
GET /permagent/skills -> 200, 37 skills
forum "CAPABILITIES": "2 registered workers" (Henry, Librarian), "37 saved skills",
                      "0 skill proposals from observed work"
first six forum rows == first six API rows, in order
```

`Open Skills` from the forum opens the app's real Skills overlay
(`SKILLS LIBRARY / 37`, with per-skill run counts) — the forum's
`onNavigate('skills')` → `setActivePanel('skills')` seam, not a forum-local
list.

### (g) Workspace switch, remount, key and pointer-lock leak — PASS

Walk mode was entered first, so a leaked listener would be observable
(`forum-controls` read `WASD walk · Shift faster · Mouse / drag to look · E meet
nearby agent · Esc overview`).

```text
perf ticks while visible (3.5s window)        : 3
switch to workspace "Home"
  .forum-shell bounding box                   : [0, 0]
  document.pointerLockElement                 : null
  forum-controls                              : "Drag to orbit · Scroll to explore · …"  (walk mode exited)
type W A S D W W on the other workspace
  perf ticks while hidden (3s window)         : 0
switch back to "World"
  perf ticks after return (3.5s window)       : 4
  canvas                                      : 1022x972, visible
  canvas RGB sampling                         : variance 7619.59, non-blank
```

The hidden panel's frameloop is genuinely stopped (`window.__worldPerf` is
republished by `PerfSampler` only when frames run; zero republishes across the
WASD window means no frames, so nothing can have moved), pointer lock is
released, and walk mode is left automatically. Returning remounts a rendering,
non-blank forum.

## WKWebView (step 4) — NOT-EXERCISED, with a precise reason

`ui/command-center/scripts/WorldWebKitInAppEvidence.swift` is a copy of
`scripts/blender/WorldWebKitEvidence.swift` retargeted at
`http://127.0.0.1:5284/ui/`, injecting the same localStorage credential the
browser build uses and driving the app's own sidebar to the World workspace.
Verbatim output:

```text
TOKEN_INJECTED yes
SCREEN_MAX_FPS 30
SCREENS 1 OCCLUSION occluded ACTIVE false
TARGET http://127.0.0.1:5284/ui/
WORLD_TAB clicked after 4s
OCCLUSION_AFTER_LOAD occluded
WEBKIT_EVIDENCE {"engine":"WKWebView standalone (in-app route)","forumShells":1,
  "forumVisible":true,"brand":"The Solar Forum","sovereign":"henry","perf":null,
  "glb":[],"canvas":{"css":[862,972],"buffer":[1293,1458],"renderer":"Apple GPU"},
  "bodyText":"…World…HENRY GalleryBuildBrainAutomateMeshReading grove…"}
WEBKIT_PERF_SECOND {"perf":null,"hidden":true,"visibility":"hidden","rafTicks":0,
  "glbHead":"200","resourceEntries":250,"glbEntries":0,
  "canvasClientRect":[862,972],"worldBox":[862,972]}
SNAPSHOT assets/world/forum/browser/in-app-world-webkit.webp
```

What this does prove in WebKit: the app boots, authenticates by the same
localStorage token path, the World workspace resolves to the forum
(`forumShells: 1`, brand `The Solar Forum`, sovereign `henry`), the forum
canvas is laid out at 862×972 CSS / 1293×1458 backing pixels, a WebGL context
is created on `Apple GPU`, and `world/solar-forum.glb` is reachable
(`HEAD -> 200`).

What it does **not** prove: rendering or FPS. `NSWindow.occlusionState` never
contains `.visible` in this session (`OCCLUSION occluded`, `NSApp.isActive
false`, one 30 Hz screen — a remote/virtual display), so WKWebView marks the
document `visibilityState: "hidden"` and never fires `requestAnimationFrame`
(`rafTicks: 0`). `ForumView` correctly gates its frameloop on
`!document.hidden`, so it renders nothing, `window.__worldPerf` is never
published, and the snapshot's 3D area is black. That black area is expected
under this condition and is **not** evidence of a WebKit render failure — but
neither is it evidence of success. `glb: []` is also inconclusive: the
resource-timing buffer is full at its 250-entry default under a vite dev
server (`resourceEntries: 250`).

To close this gate, the same binary must run with a real, unoccluded
foreground window (an interactive local GUI session), or the check must be done
in the packaged Tauri app.

## Console errors and HTTP failures observed

- 5 × uncaught `TypeError: Cannot read properties of undefined (reading
  'transformCallback')` on every page load (see Bug 1).
- `GET /api/sessions/<new id>/cost -> 500` (see Bug 2).
- `GET /api/coding-sessions/harness-runs -> 404` (repeatedly, polled) and
  `GET /api/browser/history -> 404` (see Bug 3).
- WebGL warnings `GL_INVALID_FRAMEBUFFER_OPERATION: … Attachment has zero size`
  while the panel is still 0×0 during mount; they stop once the panel is laid
  out. No shader compile/link failures, no context loss.
- No page errors originate from any `world/forum/*` module.

## Bugs and findings

**1 (P2, pre-existing, not forum) — five uncaught TypeErrors per load in
browser mode, from the Browser pane.**
`getTauriApi()` (`ui/command-center/src/components/browser/Browser.tsx:46-60`)
resolves the dynamic `@tauri-apps/api/*` imports successfully in a plain
browser, so `api` is non-null even though `window.__TAURI_INTERNALS__` is not
defined. The five `api.listen(...)` calls at
`ui/command-center/src/components/browser/Browser.tsx:636, 654, 681, 710, 722`
then throw `Cannot read properties of undefined (reading 'transformCallback')`.
The same file already has the right guard at
`ui/command-center/src/components/browser/Browser.tsx:354`
(`if (detached || !('__TAURI_INTERNALS__' in window)) return;`). Not fixed here.

**2 (P2, daemon) — `GET /api/sessions/{id}/cost` 500s for a freshly created
session.** Reproduced outside the browser:

```text
GET /api/sessions/20260914_11/cost -> 500 (empty body)   # created minutes earlier
GET /api/sessions/20260914_1/cost  -> 200 {"own":1.964…,"childrenTotal":-0.0,"perChild":[]}
```

Surfaced by the forum's ask path because the app requests the cost rollup right
after creating the session. Server-side; this is the installed daemon, which
may predate the branch.

**3 (P3, version skew) — two app endpoints 404 against the installed daemon:**
`/api/coding-sessions/harness-runs` (polled, so it repeats) and
`/api/browser/history`. Expected when the UI branch is newer than the installed
`permagentd`; worth confirming they exist in the daemon this UI ships with.

**4 (P3, evidence hygiene, by design) — the master daemon token rides the SSE
URL.** `getStreamToken()` (`ui/command-center/src/lib/streamToken.ts:3-14`)
falls back to the daemon/device token unless the build-time flag
`PERMAGENT_SHORTLIVED_STREAM_TOKEN=1` is set, so `/sessions/{id}/events?token=…`
carries the master credential in a query string. That is documented and
deliberate (`middleware/auth.rs`, and the access log strips query strings), but
any harness that records network URLs will capture a live credential —
`verify-world-in-app.mjs` redacts `?token=` before writing the report, and the
default-off short-lived token is worth turning on for desktop builds.

**5 (P3, UX, area under concurrent edit) — district nav overflows the panel.**
`.forum-districts` is a non-wrapping absolutely positioned flex row with 14
buttons. Measured in the app: at 1600×1000 `scrollWidth 1159 > clientWidth 964`
and `Debating theatre` and `Harbor` are past the right edge; at 1280×1000 six
districts are. `overflow-x: auto` makes them reachable by horizontal scroll, but
there is no visible affordance. Also,
`ui/command-center/src/components/world/forum/forum.css:318-319` are two
contradictory rules in sequence — line 318 (`flex-wrap: wrap; width: fit-content`)
is immediately overridden by line 319 (`flex-wrap: nowrap; max-width; overflow-x`),
so line 318 is dead. `ForumNavigation.tsx` and `forum.css` are being edited by
another worker right now; reported, not touched.

**6 (P2, performance, environment-dependent) — without GPU acceleration the
forum is not usable, and it takes the whole tab with it.** Under swiftshader
(`FORUM_SOFTWARE_GL=1`) the same scene renders at **0.8 FPS** and
`requestAnimationFrame` gaps grow to 5–16 seconds, which starves the main
thread: every Playwright actionability wait (visible/enabled/**stable**) times
out, i.e. ordinary DOM interaction in the right-hand agent desk becomes
unreliable for a real user too. On the same machine with ANGLE/Metal the scene
runs at 77–97 FPS with the same 2.26M triangles / 203 draw calls. Worth a
fallback (reduced geometry or a static image) when `WEBGL_debug_renderer_info`
reports a software renderer.

**7 (harness note, not a product bug) — the forum chunk needs > 20 s to mount
cold under software GL**, and the World panel shows the Suspense `Loading...`
fallback meanwhile. The harness now waits up to 90 s.

## Live-daemon state created by this verification

Nothing was deleted, renamed, approved, or configured. The runs did create
real, clearly labelled chat sessions on the daemon (each two messages, all
prefixed `[world-e2e-test]`): `20260914_3`, `20260914_4`, `20260914_5`,
`20260914_7`, `20260914_8`, `20260914_9`, `20260914_11`. The streaming turn in
the job gate was cancelled through the app. The sessions were **left in place**
rather than deleted, because deleting conversation data is irreversible and was
not explicitly authorised; they can be removed with
`DELETE /api/sessions/{id}`.

The dev server started for this work was stopped at the end of the run.

## What this does and does not close

Closed against the app, on a real daemon: the shipping route, the rollback
lever, a streamed query, cancellation, approval rendering, reconnect, the
configured sovereign name, the skills list and the Skills overlay seam, and
workspace switching without render or input leaks.

Still open from `APP_INTEGRATION.md`'s list:

- a representative job that actually persists a goal and an artifact (gate b);
- a skill proposal and promotion with source evidence (the registry currently
  reports `0 skill proposals`, so there was nothing to promote);
- a renamed sovereign (deliberately not exercised on the live identity);
- a newly registered worker (deliberately not exercised — no worker was
  registered or deleted);
- WKWebView/Tauri render and FPS (blocked by window occlusion in this session).
