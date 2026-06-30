# Build-Tab Webview & Render Lifecycle — Root-Cause Diagnosis + Fix Design

**Status:** Design-only (no code). Companion to `PHASE_2_5_TAURI_REFACTOR.md`.
**Scope:** Six Build-tab render/lifecycle bugs — #517, #548, #550, #551, #553
(browser-webview flavor) and #555 (terminal-pane flavor).
**Author note:** This doc diagnoses the *class*. The six bugs are acceptance
criteria, not six work items. Do **not** patch them individually.

> Relationship to `PHASE_2_5_TAURI_REFACTOR.md`: that draft scopes Phase 2.5 as
> *activity-emission ownership* (moving `emit_activity` from TS into Rust
> surface commands). That is a **different concern** and remains valid. This doc
> corrects the framing that "Phase 2.5" is one thing: the render/compositing
> lifecycle below is the higher-priority half and is what #551 (CRITICAL) and
> the cluster actually need. Treat them as two tracks under one phase.

---

## 1. The architecture as-built (verified)

The Build tab (`ui/command-center/src/components/build/BuildView.tsx:74-154`)
is a horizontal split: a **Terminal** pane (xterm.js, real DOM) on the left and
a **Browser** pane on the right, with a sidebar elsewhere in the app shell.

The decisive fact is **how the browser pane is rendered**:

```
window.add_child(builder, position, size)   // browser.rs:280
```

The in-app browser is a **native child WKWebView** attached to the main window
via Tauri's multi-webview API (`WebviewBuilder` → `add_child`). It is **not** an
iframe, not a DOM node, not React-rendered. It is a second OS-level WebKit
surface that macOS composites **on top of** the main app's WebView.

React never owns the browser's pixels. It owns a **placeholder `<div>**
(`Browser.tsx:651`) and continuously pushes that div's
`getBoundingClientRect()` to Rust so the native webview can be repositioned to
sit over it:

```
// Browser.tsx:143-197 — bounds are PUSHED, not composited
syncBounds(): rect = container.getBoundingClientRect()
  → invoke('update_browser_bounds', {x,y,width,height})   // active tab
  → invoke('hide_browser', {webviewId})                   // inactive / hidden
driven by: ResizeObserver + window 'resize' + setInterval(syncBounds, 500)
```

```
// browser.rs:310-340 — Rust applies them to the native surface
update_browser_bounds → webview.set_position() + set_size()
hide_browser          → set_position(-10000,-10000)   // parked offscreen
```

So the model is: **two independently-composited native WebKit surfaces, kept in
visual alignment by a 500 ms polling loop that pushes DOM rectangles across the
JS→Rust boundary.** Everything below follows from that single fact.

Supporting facts (verified):
- **No drag-drop handler is configured** on the browser `WebviewBuilder`
  (`browser.rs:213-281` — no `.with_drag_and_drop(...)`). The native WebView
  uses the OS default file-drop hit-test over its whole rectangle.
- **Reload tears nothing down.** The Reload menu item runs
  `window.location.reload()` on the **main** window only
  (`menu.rs:79-87`). Child webviews are never destroyed; React unmount only
  *hides* them offscreen (`Browser.tsx:128-141`).
- **A native z-order workaround already exists** but is partial:
  `reorder_chat_above_main` (`main.rs:102-131`) issues
  `orderWindow:NSWindowAbove` to lift the **chat window** above the main window
  on focus. It does **not** address the collapsed chat *widget* (a DOM element
  inside the main webview, `ChatLauncher.tsx:97-115`, `position:fixed;
  zIndex:9999`), which lives *below* the native browser surface in the
  compositor and so cannot be lifted by any CSS z-index.
- **The terminal uses xterm's DOM renderer** (chosen deliberately —
  `Terminal.tsx:98-100`, WebGL/Canvas rejected for Unicode/box-drawing bugs).
  PTY bytes are written unconditionally (`term.write`, `Terminal.tsx:184`); the
  Rust reader thread emits `pty_data` with no focus/visibility gate
  (`terminal.rs:125-182`). There is **no `term.refresh()` force-flush** and **no
  `visibilitychange`/focus listener** anywhere in the terminal path.

---

## 2. Root-cause: ONE root, with a partially-separable terminal sub-fix

**The root is the native sibling-webview compositing model.** Because the
browser is a second native WebKit surface rather than content inside the main
webview's DOM, it has its own compositor layer, its own event hit-test, its own
lifecycle, and — critically — it changes the **occlusion state** of the main
webview underneath it. Every one of the six symptoms is a direct consequence of
"the browser is native and out-of-DOM."

The terminal-render staleness (#555, and the terminal half of #517) shares this
root through the **occlusion-throttling** mechanism, but also has an
**independent latent bug** (the DOM renderer never force-flushes after a missed
frame). So the honest finding is: **one root, plus one small separable
hardening.** Not six bugs; not two unrelated subsystems.

### Why each symptom falls out of the root

| Bug | Mechanism (all trace to "browser is a native out-of-DOM surface") |
|-----|----|
| **#553** chat widget covered | Native browser surface composites **above** the main webview's DOM. The collapsed widget is a DOM element (`zIndex:9999`) *inside* the main webview — a different compositing stack. CSS z-index cannot cross stacks, so the native surface always wins. |
| **#550** drop-zone captures whole tab | No `.with_drag_and_drop` scoping (`browser.rs:213`), so the native WebView OS-hit-tests file drops over its **entire rectangle**, swallowing them before any DOM/sibling pane sees them. The React `DropZone` (`DropZone.tsx`) is *also* mis-scoped (wraps the whole workspace → "send to chat"), compounding it. No drop-to-terminal path exists at all. |
| **#548** reload orphans browser | `window.location.reload()` resets the **main** webview's React tree but the **child** webview is a sibling native surface with independent lifetime — never destroyed (`menu.rs:81`). The new React tree has no handle to it; it renders over the reloading shell as an orphan → force-quit. |
| **#551** CC pane fails to render | The terminal pane (DOM/xterm in the **main** webview) stops painting while the browser (separate native surface) keeps painting — because the main webview is being **occlusion-throttled** (see #517). Processes are alive (detached); only the main-webview *render* is starved. |
| **#517** terminal+sidebar blank, browser survives | macOS marks the main WebView **occluded/backgrounded** when focus moves or the child surface is forward, and WebKit throttles that view's timers + `requestAnimationFrame`. The browser is a *separate* WebView with its own (un-throttled) paint, so it survives. Terminal (rAF-driven xterm DOM renderer) and sidebar (React paint) freeze until focus-return un-throttles the main view. This is the signature that names the mechanism: **only the main webview throttles; the native child does not.** |
| **#555** answered prompt stays on screen | Same throttling, less severe: a single main-webview frame is dropped, so xterm's escape sequences (clear/redraw the prompt) update the buffer but never flush to the DOM renderer. PTY accepted the input (CC advanced); only the **render** is stale. The independent latent bug: xterm's DOM renderer has no forced re-flush, so a dropped frame is never recovered until an unrelated repaint (resize/click). |

### Dependency map (what fixing the root resolves)

```
                ┌──────────────────────────────────────────────┐
                │  ROOT: browser = native sibling WebKit surface │
                │  (add_child, out of the main webview's DOM)    │
                └──────────────────────────────────────────────┘
                  │            │             │            │
       compositing│     event  │    lifecycle│   occlusion │
        (z-order)  │   hit-test │   (reload)  │  throttling │
                  ▼            ▼             ▼            ▼
                #553         #550          #548      #517 / #551
                                                         │
                                              (same throttling,
                                               foreground frame-drop)
                                                         ▼
                                                       #555
                                                         │
                                          + independent latent:
                                          xterm DOM renderer never
                                          force-flushes  → SUB-FIX T
```

**Finding: ONE root (R) + ONE separable hardening (T).**
- **R** — replace/contain the native sibling-webview model → structurally kills
  #553, #550, #548, #551, and the browser-half of #517.
- **T** — make the terminal render self-heal on focus/visibility regain → kills
  #555 and the terminal-half of #517. Largely subsumed once R removes the
  occlusion source, but kept because rAF throttling on plain window-background
  is independent of the browser and would still strand a backgrounded terminal.

---

## 3. The corrected compositing / lifecycle / render model

Three properties the system must hold, none of which the current model holds:

1. **Bounded surface.** The browser surface must never composite, hit-test, or
   paint outside its pane rectangle, and must yield z-order to designated app
   chrome (the collapsed chat widget).
2. **Owned lifecycle.** Every native surface has an explicit
   create/show/hide/**destroy** owner that is driven by, and stays consistent
   with, the React tree across reload, tab-switch, and workspace-switch — no
   orphan can outlive its placeholder.
3. **Throttle-immune render.** The main webview's terminal and React panes must
   keep rendering across focus/occlusion changes, or self-heal immediately on
   regain — never present a stale or blank frame.

### Options for R (the browser surface)

**Option A — Move the browser into the DOM (iframe/webview tag).**
Eliminates the root entirely; everything composites and hit-tests in one stack.
**Rejected:** Tauri has no Electron-style `<webview>` tag, and an `<iframe>`
cannot load arbitrary third-party sites (X-Frame-Options/CSP/frame-ancestors
block most real pages, no cookie isolation, no devtools). This is *why* the
native child exists. Loss of browser fidelity is unacceptable for the Build-tab
browser. Not viable as the primary path.

**Option B — Keep the native webview, add a managed lifecycle/compositing
layer (RECOMMENDED).** Treat the native browser surface as a first-class
*managed layer* with an explicit Rust-side owner (call it `NativeSurfaceManager`)
responsible for: bounds (already partly there), **scoped drag-drop**, **explicit
teardown on reload/unmount**, **z-order vs app chrome**, and **occlusion
control**. This is the same shape as the existing `reorder_chat_above_main`
seam (`main.rs:102`), generalized from a one-off into the owner of all native
surfaces. Keeps full browser fidelity; makes the surface obey the three
properties.

**Option C — Browser as its own top-level window.** Sidesteps in-window
compositing but breaks the single-pane Build-tab UX (the browser is supposed to
sit *beside* the terminal) and re-creates the cross-window z-order problems
#477/#487 already fought. Rejected for the Build tab.

**Recommendation: Option B**, with the surface manager owning the four
mechanisms below, plus sub-fix **T** on the terminal.

### How Option B + T satisfies each property

- **Bounded — z-order (#553):** Render the collapsed chat widget as its own
  **native overlay** (a tiny always-on-top child surface or the existing chat
  *window* seam) ordered above the browser child via the same
  `orderWindow:NSWindowAbove` mechanism already proven in `main.rs:131`.
  Alternative (cheaper): **constrain the browser bounds** so its rectangle never
  intersects the widget's corner — `update_browser_bounds` already controls the
  rect; subtract the widget's reserved corner. Decision point D2.
- **Bounded — events (#550):** Configure the browser `WebviewBuilder` with an
  explicit drag-drop handler (`.with_drag_and_drop` / wry drag events) scoped to
  the pane, so file drops over the browser are interpreted by *us*, not the OS
  default full-rect swallow. Re-scope the React `DropZone` to the pane it
  belongs to (not the whole workspace). Separately decide whether to **wire a
  drop-to-CC-terminal path** (net-new; does not exist today) — Decision D4.
- **Owned lifecycle (#548, #551):** The surface manager registers a
  **window-reload / before-unload hook** that destroys (`close_browser`, which
  already exists at `browser.rs:343`) every child surface before the main
  webview reloads, and a deterministic re-create on the new tree. No surface may
  outlive the React placeholder that owns it. This also removes the
  desync that lets #551 present a half-rendered pane.
- **Throttle-immune (#517, #551 render):** Stop the main webview from being
  occlusion-throttled while the native child is forward. Two levers, use both:
  (a) hold an **App-Nap / activity assertion**
  (`NSProcessInfo beginActivityWithOptions:`) so the process isn't background-
  throttled; (b) on `focus`/occlusion-regain, **force a repaint** of the main
  webview's panes.
- **Sub-fix T (#555, #517 terminal):** Add a focus/`visibilitychange` listener
  in the terminal path that calls `term.refresh(0, rows-1)` (and re-runs the
  guarded `fit()`) on regain, so a dropped frame self-heals immediately instead
  of waiting for an unrelated repaint. This is the one piece that is independent
  of R and worth keeping regardless.

---

## 4. Acceptance criteria — each bug → how the design makes it impossible

| Bug | Resolved by | Acceptance check |
|-----|-------------|------------------|
| **#553** widget covered | Native overlay for widget **or** bounds-subtract its corner (D2) | Collapsed chat widget visible above the browser on the Build tab; never covered while browsing. |
| **#550** drop over-scope | Scoped browser drag-drop + re-scoped `DropZone` (+ optional drop-to-terminal D4) | Dragging a file over the terminal pane targets the terminal; the browser's drop overlay never spans the whole tab. |
| **#548** reload orphan | Reload hook destroys child surfaces before `location.reload()`; deterministic re-create | Right-click→Reload with a page loaded returns to a clean shell; no orphaned browser surface; no force-quit. |
| **#551** CC pane blank | Owned lifecycle (no desync) + throttle-immune main webview | Build tab always renders the running CC/terminal pane; never display-only loss while processes live. |
| **#517** blank on focus | App-Nap assertion + force-repaint on regain (browser-half) + sub-fix T (terminal-half) | Fullscreen → switch away → back: terminal + sidebar stay painted; no blank-then-reload. |
| **#555** stale prompt | Sub-fix T: `term.refresh()` on focus/visibility regain | Answering a CC prompt advances the terminal's rendered state immediately; no stale prompt. |

---

## 5. Build sequence (after Jesse's rulings)

Each slice is independently shippable and independently testable; ordered by
leverage (highest-impact / unblock-the-core-workflow first).

1. **S1 — Throttle-immune render (R-occlusion + T).** App-Nap activity
   assertion + force-repaint on focus regain + `term.refresh()` listener.
   Smallest change, resolves the two highest-pain items (#517, #551 render,
   #555) and the core-workflow CRITICAL. *No surface-model change.*
2. **S2 — Owned lifecycle / reload teardown.** Surface-manager teardown hook on
   reload + unmount; kills #548 and the #551 desync. Reuses existing
   `close_browser`.
3. **S3 — Scoped events.** Browser drag-drop handler + `DropZone` re-scope;
   kills #550’s over-capture. (Drop-to-terminal wiring gated on D4.)
4. **S4 — Z-order chrome.** Widget native overlay or bounds-subtract; kills
   #553. Reuses the `reorder_chat_above_main` seam.

S1 is the recommended first cut: it is the lowest-risk change and directly
restores supervision of running CC sessions (the #551 CRITICAL).

---

## 6. Decision points for Jesse's ruling

- **D1 — Browser surface model.** Confirm **Option B** (keep native child,
  add managed lifecycle/compositing owner). A and C are rejected above; confirm
  or redirect. *This is the load-bearing ruling — everything else assumes B.*
- **D2 — #553 z-fix.** (a) widget as its own native overlay (robust, reuses the
  proven `orderWindow` seam) vs (b) constrain browser bounds to spare the
  widget's corner (cheaper, pure rect math, no new native surface).
  Recommendation: **(b)** for v1, escalate to (a) only if the widget needs to
  sit *over* live browser content.
- **D3 — Occlusion mitigation appetite.** Frontend force-repaint + `term.refresh`
  alone (pure TS, no native risk), or also add the native **App-Nap activity
  assertion** (`NSProcessInfo`, small unsafe objc shim like the existing
  reorder). Recommendation: **both** — TS self-heal is necessary; the assertion
  prevents the throttle in the first place.
- **D4 — Drop-to-CC-terminal.** #550 has a second ask: there is **no** path to
  drop a file onto a running CC session today. Scope it into S3 (net-new: a
  terminal-pane drop target that injects the file path into the PTY) or split it
  to a follow-up issue. Recommendation: **split** — #550’s *bug* is the
  over-capture (S3); drop-to-terminal is a *feature*.
- **D5 — Phase 2.5 framing.** Confirm this render/compositing track and the
  existing activity-emission track in `PHASE_2_5_TAURI_REFACTOR.md` are two
  tracks under one phase, and that this track is the higher priority (it owns
  the #551 CRITICAL).

---

## 7. Hard constraints honored

- **Design-only.** No code in this change. No edits to browser/webview/terminal
  source (the `feat/henry-browser-control` worktree #469 is open there — left
  untouched).
- Disjoint from CRM bridge (#554) and Steward (#552) — not touched.
- All architecture claims above are verified against the named file:line in the
  current `origin/main` worktree.
