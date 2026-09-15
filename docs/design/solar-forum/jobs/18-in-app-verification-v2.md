# In-app verification of the new World export (v2)

Date: 2026-09-15. Branch: `feat/solarpunk-forum` (clone `recovery/solar-forum`).
The run was made at HEAD `10ae27f3`. A concurrent worker has since committed
`281c5571` ("Map every forum material slot to a native Unreal material"), which
touches only `scripts/unreal/`, `assets/world/forum/unreal/` and the Unreal JSON
records — no file under `ui/command-center/` and not the GLB (still 14,406,784
bytes, sha-256 `bf06eb363adbe7d7…`), so every number below still describes the
current tree.

Job 11 (`jobs/11-in-app-verification.md`) verified the shipping World route
against the *old* export. The export has since been replaced: a 14.4 MB
meshopt-compressed GLB with a lagoon, towers, a ridge and groves, the MESH gate
court, a gas-giant sunrise sky with bloom, and an emissive night pass. This pass
re-runs what could regress and measures what only exists now, **inside the real
Command Center app** against the live local daemon.

Nothing was renamed, registered, approved or configured. Every prompt sent is
prefixed `[world-e2e-test]`. No file under `ui/command-center/src` was changed.

## Commands run

```text
# 1. dev server (from ui/command-center), stopped at the end of this job
npm run dev -- --host 127.0.0.1 --port 5284

# 2. new harness, all gates in one pass (from ui/command-center)
node scripts/verify-world-in-app-v2.mjs
GATES=load,mesh node scripts/verify-world-in-app-v2.mjs     # subset form (merges)

# 3. WKWebView (the engine Tauri uses on macOS), from the repo root
swiftc -O -o /tmp/WorldWebKitInAppEvidence \
  ui/command-center/scripts/WorldWebKitInAppEvidence.swift
# (writes PNG; step 4 converts it to .webp)
SNAPSHOT_PATH=assets/world/forum/browser/v2-world-webkit.png \
  /tmp/WorldWebKitInAppEvidence

# 4. evidence shrink (PNG -> WebP, doc links rewritten), from the clone root
python3 scripts/blender/shrink-evidence.py
```

New files (evidence and harness only):

- `ui/command-center/scripts/verify-world-in-app-v2.mjs`
- `docs/design/solar-forum/in-app-report-v2.json` (machine-readable run record)
- `assets/world/forum/browser/v2-*.webp` (written as PNG, shrunk by step 4)

## Environment actually measured

| | |
| --- | --- |
| App | `http://127.0.0.1:5284/ui/` (vite dev server proxying to the daemon) |
| Daemon | `127.0.0.1:3001` (launchd), `GET /status` 200 |
| Browser | **Google Chrome 152.0.7977.83**, headless, via Playwright 1.62.1 `channel: 'chrome'` |
| GPU | `ANGLE (Apple, ANGLE Metal Renderer: Apple M4, Unspecified Version)`, WebGL 2.0 |
| Flags | `--use-angle=metal --enable-gpu --ignore-gpu-blocklist --disable-frame-rate-limit --disable-gpu-vsync` |
| Viewport | 1280 × 1000; forum canvas 702 × 972 CSS = 702 × 972 drawing buffer (dpr 1) |
| Run | `2026-09-15T02:53:16Z` → `02:59:44Z` |

Two environment notes that change how the numbers compare with job 11:

- **Playwright's bundled Chromium is not installed in this clone** (only
  `webkit-2359` and `ffmpeg-1011` are under `~/Library/Caches/ms-playwright`),
  and the bundled headless build is `chrome-headless-shell`, which has no GPU
  path worth measuring. The harness therefore drives the installed Google
  Chrome (`BROWSER_CHANNEL=''` falls back after `npx playwright install
  chromium`).
- **Frame-rate limiting must be disabled or every reading is 60.0.** The first
  run of this harness reported a suspiciously flat `median 60.0` because Chrome
  pins the frameloop to the display. With vsync off the same scene reports
  139 FPS in overview. Job 11's 77–97 FPS figures were uncapped; the numbers
  below are too.

Walker position is read **without touching `src/`**. `ForumWalk` keeps the
walker's feet in a React ref and publishes nothing, so the harness installs a
minimal React DevTools hook stub in a Playwright init script; react-three-fiber
calls `injectIntoDevTools`, which hands over its fiber roots, and every
r3f-created THREE object carries `__r3f.root` — the store holding the live
camera. Read-only: nothing in the page is mutated
(`verify-world-in-app-v2.mjs`, `__forumProbe` / `__forumCensus` /
`__celestialProbe` / `__emissiveProbe` / `__namedMeshProbe`).

## Results

| # | Check | Result | Evidence |
| --- | --- | --- | --- |
| 1 | GLB 200 at the new revision, meshopt-decoded, no `EXT_meshopt` console errors | **PASS** | `v2-world-overview.webp`, 172 meshes / 929,544 triangles decoded, 0 meshopt console lines |
| 1 | FPS + draw calls on ANGLE/Metal, overview | **PASS** | 139.2 FPS median, 274 calls, 2,670,135 tris/frame |
| 1 | FPS + draw calls, walk mode | **PASS** | 169.1 FPS median, 246 calls, 2,569,743 tris/frame |
| 2 | Mesh gate district focus reaches the gate approach | **PASS** | walker at `(-35.355, 0, -35.355)`, signed distance to ring plane `-6.000` |
| 2 | Walk outward through the ring plane with the real geometry | **PASS** | `-5.520 → +0.357` over 13 samples, lateral ≤ 0.036 m, no stall |
| 2 | Agora opens with the honest plaque | **PASS** | `MESH: NOT CONNECTED`, `data-state="offline"`, 22 calls / 1,807 tris |
| 2 | Escape returns the walker to the gate approach | **PASS** | back at `(-35.355, 0, -35.355)`, signed `-6.000` |
| 2 | Non-walk path: district reselect opens, **Return to the Forum** closes | **PASS** | `agoraOpenedByDistrictReselect: true`, `returnedByButton: true` |
| 3 | `prefers-color-scheme: dark` gives the night scene | **PASS** | `Night / follows system appearance`, `v2-world-night.webp`, variance 3971 |
| 3 | Night FPS (bloom cost) | **PASS** (measured) | overview 143.6 → 98.7 (−44.9); walk 153.5 → 109.5 (−44.0) |
| 3 | Gas giant / moon visible at night | **FAIL** | both project into frame; neither reads against the sky (contrast 1.09, 0.82, 0.70) |
| 3 | Emissive surfaces read as lit at night | **PASS** (as a class) | 1,010 / 7,930 sampled emissive pixels brighter at night; lanterns 233 → 246 |
| 3 | Emissive **window strips** read as lit from the commons overlook | **NOT-EXERCISED** | the material is 5 boxes 108 m away at the market hall; 0 vertices project into either commons view |
| 4 | Ask: forum send path into the existing conversation | **PASS** | session POST 200, SSE `connected`, `/reply` 200 |
| 4 | Ask: the agent actually replies `PONG` | **FAIL** | empty `Agent` turn; an earlier run returned `Network error: Could not connect to 127.0.0.1:8081` |
| 4 | Skills panel opens, saved-skills list matches the API | **PASS** | API 37, forum "37 saved skills", panel opened |
| 4 | Switch away / back, no render or WASD leak | **PASS** | hidden box 0×0, 0 perf ticks, camera moved 0.000 m under 6 WASD presses |
| 5 | WKWebView render / FPS | **NOT-EXERCISED** | window occluded again; `hidden: true`, `rafTicks: 0` |

### 1. Load — the new export in the app's World tool

```json
"glb": { "url": "/ui/world/solar-forum.glb?v=bf06eb363adbe7d7", "status": 200 },
"glbResourceTiming": { "durationMs": 318, "transferSize": 14407084,
                       "encodedBodySize": 14406784, "startTimeMs": 4241 },
"perf": { "fps": { "min": 134.4, "median": 139.2, "max": 140.3 },
          "calls": 274, "triangles": 2670135, "geometries": 148,
          "textures": 45, "programs": 35, "dpr": 1 },
"sceneCensus": { "meshes": 172, "triangles": 929544, "vertices": 1996360,
                 "emissiveMaterials": 49 },
"meshoptConsole": []
```

- **Bytes.** `ui/command-center/public/world/solar-forum.glb` is **14,406,784
  bytes**; its sha-256 begins `bf06eb363adbe7d7`, which is exactly
  `FORUM_ASSET_REVISION` (`src/components/world/forum/assetRevision.ts:1`), and
  that is the `?v=` the app requested. The dev server serves it uncompressed
  (`transferSize` 14,407,084 ≈ body + headers).
- **Meshopt.** The scene really decoded: 172 meshes and **929,544 authored
  triangles** are in the live `scene` graph, and **no console message** matched
  `meshopt | EXT_meshopt | KHR_mesh_quantization | decoder`. The 2,670,135
  triangles the renderer reports per frame are the same geometry drawn through
  the shadow pass and the post chain.
- **Load time.** Warm dev server: GLB request starts 4.24 s after navigation,
  transfers in 318 ms, first published perf sample 5.31 s after navigation.
  Cold (first run of the session, while vite transformed the forum chunk for the
  first time) the same path took **55.7 s** to the first perf sample; that is a
  dev-server compile cost, not a runtime one.
- **Draw calls in walk mode are lower than in overview** (246 vs 274) because
  `ForumNavigation` — `OrbitControls` plus fourteen drei `<Html>` district chips
  (`ForumNavigation.tsx:39-40`) — is unmounted while walking.

![World overview](../../../assets/world/forum/browser/v2-world-overview.webp)

### 2. MESH walk-through against the real geometry

Selecting **Mesh gate** put the location card at `MESH GATE / The gate to the
MESH` and moved the overview camera to the approach. Entering walk mode from
that district placed the walker at exactly `gateApproachPoint()`:

```json
"walkerAtGateDistrictStart": { "feet": { "x": -35.355, "y": 0, "z": -35.355 },
  "gate": { "signedDistance": -6, "lateralOffset": 0, "polarRadius": 50 } }
```

The harness then turned with the product's own `ArrowLeft` look key until the
view faced the outward walk direction (`yaw 0.780`, target `π/4 = 0.785`) and
held `W` for 2.30 s. Thirteen samples of the live camera:

```text
signedDistance: -5.520 -5.026 -4.518 -4.021 -3.531 -3.036 -2.525
                -2.019 -1.509 -1.014 -0.510 -0.007 +0.357
lateralOffset:  <= 0.036 m throughout   (ring radius 4.6 m)
feet.y:         0.038 -> 0.721 m        (the three-step dais, top y = 0.72)
```

The walker **was not blocked**: it climbed the dais and crossed the ring plane
on the court axis. At the crossing the Agora opened:

```json
"agora": { "plaquePresent": true, "plaqueText": "MESH: NOT CONNECTED",
           "plaqueState": "offline",
           "location": "MESH AGORA\n\nThe antechamber beyond the gate" },
"agora.perf": { "fps": { "median": 326.5 }, "calls": 22, "triangles": 1807 }
```

22 draw calls and 1,807 triangles against 246 / 2.57 M in the forum — the
figures `APP_INTEGRATION.md` claims for the hidden-forum branch, confirmed in
the app. `Escape` closed the Agora and put the walker back on the spur at
`(-35.355, 0, -35.355)`, signed distance `-6.000`. Separately, selecting the
**Mesh gate** district twice from the overview opened the Agora, and the
**Return to the Forum** button closed it.

| before | at | after |
| --- | --- | --- |
| ![Gate approach](../../../assets/world/forum/browser/v2-mesh-approach.webp) | ![Mesh Agora](../../../assets/world/forum/browser/v2-mesh-agora.webp) | ![Back at the approach](../../../assets/world/forum/browser/v2-mesh-return.webp) |

### 3. Night appearance

`page.emulateMedia({ colorScheme: 'dark' })` + reload flips the badge to
`Night / follows system appearance`, and the night sky (galactic shader dome,
14,000-point starfield) renders; canvas RGB variance 3,971.

**Frame cost.** Same camera, same frame, day vs night:

| | FPS median | draw calls | triangles/frame |
| --- | --- | --- | --- |
| Day, overview | 143.6 | 274 | 2,670,135 |
| Night, overview | **98.7** (−44.9, −31 %) | 277 | 2,676,601 |
| Day, walk | 153.5 | 266 | 2,667,007 |
| Night, walk | **109.5** (−44.0, −29 %) | 269 | 2,673,473 |

Night costs about **30 % of the frame** for only three extra draw calls, so the
cost is fill/shading — the bloom chain (`ForumPostProcessing.tsx:28-30`) with
many more surfaces over its `luminanceThreshold` plus the nine night point
lights (`ForumLighting.tsx:37-43`).

**Celestial bodies — present, but not visible.** The harness locates them by
geometry (sphere radius 140 and 46), projects them through the live camera and
samples the rendered disc against the sky immediately around it:

| body | vantage | screen (canvas px) | radius px | disc mean | surround mean | contrast |
| --- | --- | --- | --- | --- | --- | --- |
| moon | commons, walk | (338, 494) | 46 | 12.48 | 15.13 | **0.82** |
| gas giant | commons, walk | (326, 540) | 167 | 29.98 | 27.54 | **1.09** |
| gas giant | gate court, walk | (353, 538) | 178 | 17.33 | 24.89 | **0.70** |

All three are inside the frustum, and none reads as a body. The screenshots show
why for the gas giant: from the commons the crosshair sits on the new ridge, and
from the gate court on the exedra. The moon is not occluded by anything visible
and still reads *darker* than the starfield behind it. See bug 3.

![Night sky from the commons](../../../assets/world/forum/browser/v2-world-night-sky.webp)

**Emissives.** From the commons overlook (the default overview camera), 31 of
the 49 emissive meshes project into the frame; 7,930 of their vertices were
sampled in the day render and the night render of that identical frame:

```json
"allEmissivePixels": { "points": 7930,
  "day":   { "median": 74.84, "p90": 161.42, "mean": 95.65 },
  "night": { "median": 42.65, "p90": 158.27, "mean": 71.10 },
  "pointsBrighterAtNight": 1010 }
```

Most emissive-flagged surfaces simply darken with the lighting. The ones that
actually read as lit at night:

| material | emissive intensity | day median | night median | points brighter at night |
| --- | --- | --- | --- | --- |
| Forum Lantern Amber | 8 | 233.2 | **245.9** | 80 / 300 |
| (unnamed) | 1.6 | 50.0 | **80.6** | 273 / 300 |
| (unnamed) | 0.5 | 69.3 | **108.3** | 284 / 300 |
| Forum Strand Amber | 4 | 121.3 | 111.8 | 6 / 300 |
| Forum Hologram | 4.95 | 87.1 | 79.9 | 0 / 300 |

Whole-frame, eye level at the commons: **4,390 pixels (0.71 %) are ≥ 25 luma
brighter at night than in day**, mean delta +60.6, max +195.1 — the lanterns and
strand lights visibly hold their radiance while everything else drops.

![Commons at night, eye level](../../../assets/world/forum/browser/v2-commons-night-walk.webp)

**Window strips — not where the brief assumes.** The material named
`Forum Warm Window Emission` is on exactly **one mesh of 120 vertices**, world
bounds `(-106, 7, 32) … (-99, 7, 34)` — i.e. five small boxes on the market hall,
about 108 m from the commons. **Zero** of its vertices project into the commons
overlook frame or the commons eye-level frame, so "the window strips read as lit
from the commons overlook" cannot be true as stated. Going to look at them from
the Maker hall walk start (30.7 m away) put the camera behind hall geometry, and
the sampled disc read 27.7 against a 25.9 surround, so they are **not confirmed
lit** either. The reason there are only five is a dead branch in the exporter —
see bug 4.

### 4. Job-11 gates that could regress

**Ask.** The forum's send path is intact:

```text
POST /api/sessions                    200
GET  /api/sessions/20260915_4         200
GET  /sessions/20260915_4/events      200   (eyebrow: "connected")
POST /sessions/20260915_4/reply       200
```

The **agent** then produced an empty turn (`Agent 11:56 PM`, no body). An
earlier run in this session produced the explicit cause in the transcript:
`Network error: Could not connect to 127.0.0.1:8081 — check your network
connection and try again.` Nothing is listening on `127.0.0.1:8081` on this
machine. So this is a daemon/model-transport condition, not a forum defect — but
it means **no end-to-end agent reply was obtained in this pass** (bug 2).

**Skills.** `GET /permagent/skills` → 200 with 37 skills; the forum's live
capabilities section reads `37 saved skills` / `2 registered workers`
(`matches: true`); **Open Skills** reached the app's Skills panel.

**Switch away and back.** Pointer lock is held by the forum canvas during walk
mode, so the sidebar cannot be clicked at all — the harness uses the app's own
`⌘1` workspace shortcut, which `ForumWalk` deliberately ignores as a walk key
(`ForumWalk.tsx:58`). After the switch:

```json
"hiddenState": { "forumBox": [[0, 0]], "pointerLock": null,
  "controls": ["Drag to orbit · Scroll to explore · Select a place or walk from this place"] },
"perfTicksWhileHiddenAfterWASD": 0,
"cameraMovedWhileHidden": 0,
"perfTicksAfterReturn": 3,
"perfAfterReturn": { "fps": { "median": 135.3 }, "calls": 274 }
```

The hidden panel collapses to 0×0, drops walk mode, releases pointer lock, stops
rendering entirely (0 perf ticks over 3 s), and six `WASD` presses on the other
workspace moved the camera **0.000 m**. Returning to World resumes at 135.3 FPS
with a non-blank canvas (variance 4,377). No obstruction was recorded anywhere
in the agent desk this run (`"obstructions": []`).

### 5. WKWebView

```text
SCREEN_MAX_FPS 30
SCREENS 1 OCCLUSION occluded ACTIVE false
WORLD_TAB clicked after 4s
OCCLUSION_AFTER_LOAD occluded
WEBKIT_EVIDENCE {"forumShells":1,"forumVisible":true,"brand":"The Solar Forum",
  "sovereign":"henry","perf":null,"glb":[],
  "canvas":{"css":[862,972],"buffer":[1293,1458],"renderer":"Apple GPU"}}
WEBKIT_PERF_SECOND {"perf":null,"hidden":true,"visibility":"hidden","rafTicks":0,
  "glbHead":"200","glbEntries":0,"canvasClientRect":[862,972]}
```

Same outcome as job 11, now with the precise reason. The window is created but
never becomes visible in this session (`OCCLUSION occluded`, `ACTIVE false`), so
WebKit reports `document.hidden === true`; `ForumView` gates the canvas on
exactly that (`ForumView.tsx:99` reads `document.hidden`, `:179`
`frameloop={onPanel && pageVisible ? 'always' : 'never'}`), so
`requestAnimationFrame` never ran (`rafTicks: 0`), `window.__worldPerf` stayed
null and the GLB was never requested — while a `HEAD` on it from the same page
returns 200. The app shell, the HUD, the district chips and the agent desk all
render correctly at 862 × 972 with a dpr-1.5 buffer on `Apple GPU`.

**WKWebView render and FPS for the new export remain unmeasured.** Producing
them needs a session where the window can actually be foregrounded.

![WKWebView](../../../assets/world/forum/browser/v2-world-webkit.webp)

## Console and network during the run

- **0** console messages mentioning meshopt, `EXT_meshopt_compression` or
  `KHR_mesh_quantization`.
- **115 console errors**, all `Failed to load resource … 404`, from three
  endpoints: `GET /api/coding-sessions/harness-runs` ×103 (polled),
  `GET /api/browser/history` ×8, `POST /api/browser/snapshot/{id}` ×4.
- `GET /api/sessions/{id}/cost` → **500** ×2 (job-11 bug 2, unchanged).
- **20** uncaught `Cannot read properties of undefined (reading
  'transformCallback')` (job-11 bug 1, unchanged).
- ~1,000 `GL_INVALID_FRAMEBUFFER_OPERATION … Attachment has zero size` warnings
  across four WebGL contexts while a panel is still 0×0 during mount; they stop
  once the panel is laid out. Same class as job 11.
- **New:** 39 × `THREE.WebGLShadowMap: PCFSoftShadowMap has been deprecated.
  Using PCFShadowMap instead.` (bug 5).
- No page error originates from a `world/forum/*` module.

## Bugs and findings

**1 (P1, harness, fixed here) — job 11's harness writes the live daemon token
into its report.** `ui/command-center/scripts/verify-world-in-app.mjs:168`
records failed-request URLs verbatim:

```js
page.on('requestfailed', r => requestFailures.push({ url: r.url(), … }));
```

The response handler redacts `?token=`, this one does not, and
`getStreamToken()` (`ui/command-center/src/lib/streamToken.ts:3-14`) puts the
**master daemon token** in the SSE query string. An aborted `EventSource` is
routine (it happens on every reload), so the credential lands in the committed
JSON. Reproduced: the first v2 report contained a full 64-character live token.
`verify-world-in-app-v2.mjs` now routes every recorded URL through one
`scrubUrl`, and the committed `in-app-report-v2.json` was verified
token-free before this doc was written. The job-11 report
(`docs/design/solar-forum/in-app-report.json`) is clean — the leak did not
happen there — but the code path is still live and should be fixed in the job-11
harness too.

**2 (P1, environment/daemon) — no agent reply could be obtained; the daemon
cannot reach `127.0.0.1:8081`.** The forum's own path is provably fine (session
created, event stream `connected`, `POST /reply` 200); the reply itself comes
back as `Network error: Could not connect to 127.0.0.1:8081` or as an empty
`Agent` turn. Nothing is listening on that port on this machine. Until that
service is up, the end-to-end query gate cannot be closed by anyone.

  Related, and the reason this was not caught earlier: **job 11's ask gate
  cannot fail.** `verify-world-in-app.mjs:448` waits for `/PONG/i` *anywhere* in
  the transcript, and the transcript contains the echoed prompt "reply with the
  single word PONG". Job 11's **PASS** for gate (a) is therefore not evidence
  that an agent answered. v2 only looks at text after the prompt.

**3 (P2, forum, night) — neither celestial body is visible at night.** Measured
above: the moon reads 12.48 against a 15.13 sky (contrast 0.82) and the gas
giant 17.33 against 24.89 from the gate court (0.70) — both *darker* than the
sky they are supposed to hang in. Two contributing causes, both worth checking
before a fix:

  - `MOON_FRAG` lights the moon from `uSun = SUNRISE_DIR`
    (`ForumSky.tsx:103, 136-145, 155`), which is a westerly bearing, while
    `MOON_POS` is north-north-east (`ForumSky.tsx:105`) — the face turned to the
    viewer is the unlit one, floored at `0.12`.
  - Both bodies are drawn with `depthWrite={false}`
    (`ForumSky.tsx:162, 166`), and so is the night dome at radius 4800
    (`ForumSky.tsx:48-50`). With no depth writes, which of the two shader
    programs survives is decided by three's opaque sort, not by distance. The
    sampled moon disc is *flat* (mean 12.48, max 170.75 from a single star,
    essentially no limb shading), which is what an overpainted sphere looks
    like rather than a dark-but-shaded one.

  Separately, and by design rather than by bug: the gas giant sits only ~14°
  above the horizon on the same north-west bearing as the gate
  (`ForumSky.tsx:104`), so the new ridge closes it off from the commons
  entirely. If it is meant to be part of the skyline, it needs to be higher or
  the ridge lower.

**4 (P2, exporter) — the night window strips are dead code except for five
boxes.** `scripts/blender/forum_towers.py` places the warm window strips in
three loops:

```python
360:    for _level in range(4,int(_height),6):
361:        if _level % 12 == 0:              # spires  — never true
369:    for _level in range(4,int(_height),6):
370:        if _level % 12 == 0:              # habitats — never true
373: for _bay in range(5):                    # market hall — the only one that runs
```

`range(4, h, 6)` yields `4, 10, 16, 22, 28, …`, i.e. `4 + 6k`, whose residue mod
12 alternates between 4 and 10 — **never 0**, for every tower height in the file
(44, 38, 34, 29 …). So the eight spire rings and the four habitat strips are
never created, and only the five unconditional market-hall strips at
`forum_towers.py:373-376` exist. That matches the browser exactly: one mesh, 120
vertices (5 boxes × 24), world bounds `(-106, 7, 32) … (-99, 7, 34)`.
`forum_towers.py:378` then reports
`job16_triangle_delta_estimate = len(_spires)*12*6*2 + 4*4*6`, an estimate that
assumes both dead loops fired, so the manifest over-reports what shipped.
Not fixed here — `scripts/blender/` is outside this job's write scope.

**5 (P3, forum, new since job 11) — 39 shadow-map deprecation warnings per
load.** The forum Canvas passes bare `shadows`
(`ui/command-center/src/components/world/forum/ForumView.tsx:179`), whose r3f
default is `PCFSoftShadowMap`; on three 0.184 that constant is deprecated and
silently downgraded to `PCFShadowMap`. Shadows therefore are **not** the soft
variant the scene was authored against, and every mount logs the warning.
Pass an explicit `shadows="percentage"`/`shadows={{ type: PCFShadowMap }}` (or
accept the downgrade) to settle it.

**6 (P3, product behaviour, worth knowing) — while walking, the app's sidebar is
unreachable by mouse.** Walk mode takes pointer lock on the canvas
(`ForumView.tsx:201-209`), so a click on "Home" never reaches the DOM; only
`Escape` or a keyboard shortcut gets you out. `ForumWalk` already ignores
modified keys (`ForumWalk.tsx:58`), so `⌘1 … ⌘9` work, and that is what a real
user has. Not a defect, but it means any test that clicks the sidebar while
walking is silently testing nothing — job 11's remount gate did exactly that and
still reported a pass.

**7 (P3, performance) — overview is the expensive mode, not walk mode.**
Overview runs 139 FPS at 274 draw calls; walk mode runs 169 FPS at 246. The
geometry difference is 4 %, so most of the gap is `ForumNavigation` —
`OrbitControls` plus fourteen drei `<Html>` chips
(`ForumNavigation.tsx:39-40`), each of which writes `style.transform` every
frame. Worth a look if the overview ever has to run on weaker hardware; this is
an observation from two measurements, not an isolated experiment.

**8 (job-11 bug 5, not reproduced here) — district nav overflow.** At
1280 × 1000 `.forum-districts` wraps to three rows via the narrow-width rule at
`ui/command-center/src/components/world/forum/forum.css:278-282` and nothing
overflows. Job 11 measured the overflow at 1600 × 1000, which this pass did not
re-test, so treat it as open at wide widths.

## Live-daemon state created by this verification

Nothing was deleted, renamed, approved, registered or configured. The runs did
create real, clearly labelled chat sessions on the daemon, each two messages and
every prompt prefixed `[world-e2e-test]`: `20260915_1` … `20260915_4`. They were
**left in place** rather than deleted, because deleting conversation data is
irreversible and was not authorised; they can be removed with
`DELETE /api/sessions/{id}`.

The dev server started for this work was stopped at the end of the run.

## What this closes and what it does not

Closed against the real app on a live daemon, with the new export: the GLB
revision and meshopt decode path, FPS and draw calls in both camera modes, the
whole MESH portal flow including a physical walk out through the ring and the
return, the night appearance switch and its frame cost, the skills seam, and
workspace switching without render or input leaks.

Still open:

- an end-to-end agent reply (blocked by bug 2 — the daemon's model transport);
- the gas giant and the moon actually appearing in the sky (bug 3);
- the window strips the night pass was supposed to add (bug 4), and with them any
  claim that "emissive window strips read as lit" — what does read as lit at
  night is the lantern/strand set, measured above;
- WKWebView render and FPS for the new export (occluded window, as in job 11);
- everything job 11 left open that this pass did not touch: a job that persists a
  goal and an artifact, an approval, a skill proposal and promotion, a renamed
  sovereign, and a newly registered worker.

## Fixes after v2 verification

Date: 2026-09-15, same branch, in-app code only. Scope: `ui/command-center/`
(`src/components/world/forum/`, `scripts/`) and this document. Nothing under
`scripts/blender/` or `scripts/unreal/` was touched, so bug 4 stays open as
written above.

Gates run from `ui/command-center`, all green:

```text
npm run typecheck                      # tsc --noEmit, clean
npx vitest run src/components/world    # 43 files, 278 tests passed
npm run build                          # tsc + vite build, built in 6.5 s
```

### Bug 3 — the celestial bodies (`ForumSky.tsx`)

Three separate causes, all addressed:

- **Draw order.** New exported `SKY_RENDER_ORDER = { dome: -1000, stars: -900,
  bodies: -100 }`. three sorts by `renderOrder` *before* distance, so the
  radius-4800 dome and the 14,000-point starfield are now pinned behind the two
  bodies, and the world's own geometry stays at the default `0` — which is what
  still lets a tower or the ridge legitimately occlude a body. The bodies are
  **opaque and depth-writing** now (they were `depthWrite={false}`, which is
  what let the dome overpaint them); writing depth is also what stops the
  transparent starfield — drawn after every opaque object — from speckling
  through the moon's disc, the "flat disc with a 170-luma star in it" the
  measurement above recorded.
- **Self-lit materials.** `uSun` is gone from both shaders. Each body gets a
  `uLight` derived from its own position (`selfLight()`: back towards the forum,
  lifted 0.55 for a terminator across the lower limb), so the face turned to the
  viewer is always the modelled one, in day and night alike. The gas giant keeps
  its banding over a `0.78 + 0.46 * shade` floor with an all-round limb glow (the
  old glow was scaled by a `sunSide` term); the moon keeps its maria, a higher
  floor, and its own faint `uGlow` halo instead of borrowed sunlight.
- **Elevation and bearing.** `GAS_GIANT_POS` moves to **28° elevation on
  Blender polar 250°** (three.js `(-271.7, 422.5, 746.8)`, i.e. x<0/z>0 — the
  south-westerly quarter over the lagoon and the harbour approach). The ridge
  mesh spans Blender 100–215° only (`forum_landform.py:_add_ridge_and_waterfalls`,
  crest 55–78 m at radius 128–180, ≈26° of elevation from the commons), so this
  bearing has no ridge geometry on it at all. The moon is unchanged
  (Blender 59.0°, 25.3° elevation): it was never occluded, only overpainted and
  unlit.
- Both meshes and their materials are now **named** (`Forum gas giant`,
  `Forum moon`), so the harness can find them with `__namedMeshProbe` as well as
  by sphere radius. The radii are unchanged (140 / 46), so `__celestialProbe`'s
  existing radius heuristic still works.

New test `ForumSky.test.tsx` (jsdom, 5 tests): draw order and depth state for
both bodies; no `uSun` uniform and a `uLight` on the lit side of each body's
viewer-facing point; the gas giant's elevation (26–30°) and bearing (off the
ridge arc, x<0/z>0); the moon smaller, pale and off the ridge; and a mount test
asserting both bodies are present in the **night** scene and in the day scene.

### Bug 5 — shadow-map deprecation (`ForumView.tsx`)

`shadows` → `shadows={{ type: PCFShadowMap }}`. r3f's boolean branch sets
`PCFSoftShadowMap`, which three 0.184 warns about and downgrades to
`PCFShadowMap` on every shadow render; asking for `PCFShadowMap` outright is the
same shadows the scene was already getting, with no deprecation path left. The
sun light's own shadow configuration (`ForumLighting.tsx:32-35`) is untouched.
Covered by a new case in `ForumView.test.tsx`, which now captures the `Canvas`
props. **Note for a future job, outside this scope:**
`src/components/world/WorldView.tsx:499` still passes
`{ type: THREE.PCFSoftShadowMap }`, so the legacy World route keeps the warning.

### Bug 1 — token in the job-11 harness (`scripts/verify-world-in-app.mjs`)

`scrubUrl` ported from `verify-world-in-app-v2.mjs` and applied to **every**
recorded URL: `requestfailed` (the actual leak), `response`, the GLB record, and
— beyond v2 — console text and page-error text via `scrubText`. `flush()` also
runs a last-resort backstop over the whole serialised report (`split(token)` plus
a `token=` query scrub), so no gate can write the credential by another route.

### Bug 2 — the ask gate could not fail (same file)

The `/PONG/i`-anywhere wait at the old `:448` is replaced by v2's logic: find the
last echo of the prompt in the transcript and require `\bPONG\b` *after* it. The
gate now records `agentRepliedPong`, `replyAfterPrompt` and
`replyLooksLikeTransportError`, and records an explicit `failure` when no reply
distinct from the echoed prompt contains PONG — so the run summary reports
FAILURE instead of "recorded". The two reconnect-gate PING prompts only seed the
stream and assert on connection status, so they needed no change.

### Bug 7 — overview draw calls: left alone, and the attribution corrected

Not changed, and on inspection the stated cause does not hold. drei's `<Html>`
creates **no WebGL object** unless `occlude` is set (drei 9's `web/Html.js`),
so the fourteen chips cannot account for draw calls at all;
and their per-frame work is already epsilon-gated — with `transform` off (which
is the case here) the DOM write is skipped entirely unless the projected
position or the zoom actually moved, so "each writes `style.transform` every
frame" is not what the current drei does either. The 274 → 246 delta is far more
likely frustum culling: walk mode puts the camera at eye level inside the forum
and drops ~100,000 triangles with those 28 calls, which is the shape of culling
rather than of chip overhead. Consolidating or thinning the chips would change
navigation behaviour for no measured gain, so nothing was changed. If overview
cost is ever revisited, the experiment to run is toggling `OrbitControls` and the
chips independently and reading `gl.info.render.calls`.

### What still needs a browser re-check

All of this is unit-tested and type-checked, but only a real GPU run can close
these:

- **Contrast.** Re-run `node scripts/verify-world-in-app-v2.mjs` (gates
  `night`) and confirm the disc/surround contrast for both bodies is now
  comfortably above 1, at night and in day, from the commons and the gate court.
  The numbers to beat are moon 0.82 and gas giant 0.70 / 1.09.
- **Occlusion.** Confirm the gas giant clears the ridge from the commons
  overlook at its new bearing, and that the world still occludes both bodies
  correctly where it should (walk behind the market hall and check the moon is
  hidden, not painted over the roof) — the depth-write change is the part most
  worth eyeballing.
- **Starfield.** Confirm no stars punch through the moon's disc at night.
- **Shadows.** Confirm the 39 `PCFSoftShadowMap has been deprecated` console
  lines are gone and that shadows still render (the shadow pass is unchanged,
  but the console count is the evidence).
- **Frame cost.** Re-measure day/night FPS and draw calls; the bodies are now
  opaque and depth-writing, which should be neutral-to-cheaper, but it is
  unmeasured.
- The job-11 harness changes were not executed (running it needs the dev server
  and the live daemon); `node --check` passes on the file.
