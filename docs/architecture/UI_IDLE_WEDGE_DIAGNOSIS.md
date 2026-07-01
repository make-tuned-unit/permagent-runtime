# UI Idle-Wedge Diagnosis — the Tauri app hangs after ~1 h idle (#562)

**Status:** Diagnosis + capture instrumentation. **No behavioral fix this round**
— the root block class is narrowed to two candidates that a single captured
main-thread stack will disambiguate; the watchdog below captures it.
**Date:** 2026-07-01
**Scope:** The **desktop UI process** (`permagent-app` crate / `Permagent.app`)
and its webviews. The daemon (`permagentd`) is **confirmed healthy** during the
wedge and is out of scope here — see the boundary note below.
**Companions:** `DURABILITY_AUDIT.md` (#560, daemon side of "weeks-untouched"),
`WEBVIEW_LIFECYCLE.md` (#556/#558, the render-throttle cluster). This doc is the
**UI-process** third leg.

---

## The observation (repro 2026-07-01)

Permagent left idle ~1 h. On return the Tauri **UI app is hung**:

- **Window cannot be dragged** — the native title bar does not respond.
- Terminal pane + parts of the app shell are **blank**; the in-app **browser
  pane still shows its last frame**.
- **`permagentd` answered HTTP 200 on `/api/people`** while the UI was frozen.
- Both processes were **alive at 0 % CPU** — *blocked*, not crashed, not spinning.
- Recovery: `pkill Permagent` (the app, **not** `permagentd`) + relaunch. The
  daemon runs straight through.

---

## The critical unknown — resolved

> When the UI wedged, was the daemon still responsive?

**Resolved: yes.** `permagentd` served HTTP 200 during the freeze. This is the
single most important fact and it re-scopes the whole issue:

- The **remote-access bar is intact** — a phone over Tailscale reaches a live
  daemon regardless of the desktop hang. The daemon durability findings in #560
  (F1 half-dead task, F2 scheduler wedge, F4 WAL) are **not** implicated here.
- The failure is **UI-process-local**: something inside `Permagent.app` blocked
  its own main thread. This is a **UI-durability** failure ("walk up to the Mac
  after time away and the desktop app is a frozen husk"), not a daemon one.

The watchdog in §5 re-confirms this **automatically on every future wedge** (it
curls the daemon at the moment of capture), so we never again rely on a single
inconclusive manual curl.

---

## 1. Why this is a MAIN-THREAD wedge, not a blank-pane (the decisive inference)

This is **not** another instance of the render cluster (#517/#548/#551/#558).
Those are blank *panes* that **self-heal on focus regain**. #562 is different in
kind, and the difference pins the layer:

**On macOS, WKWebView runs web content in a separate `WebContent` process.** JS
executing in the React app runs *there*, not in the app process. Therefore:

- A hung **JS / WebContent** thread blanks the React panes but **cannot freeze
  window dragging** — the app process's main (AppKit) run loop keeps servicing
  the title bar. That is the #517 cluster.
- **The window will not drag ⇒ the app process's main thread is blocked.** Only
  a block on the AppKit main run loop freezes native window management. This is
  a strictly *lower* layer than the render cluster.

Corroborating the same conclusion:

- The **#558 repaint-on-regain self-heal cannot fire** (`repaintOnRegain.ts`
  listens for `focus`/`visibilitychange`). Those handlers run on the main run
  loop — which is wedged — so the very mechanism that fixes the blank-pane bugs
  is dead here. The blank panes in #562 are a *symptom* of the main-thread wedge,
  not the #517 throttle.
- The **browser pane still painting** is consistent: it is a **second, separate
  `WebContent` process** (`window.add_child`, a native child WKWebView —
  `WEBVIEW_LIFECYCLE.md` §1). It holds its last composited frame; it simply is
  not being re-driven. Its liveness says nothing about the app main thread.

So: **the diagnosis is a block on the UI process's AppKit main thread.** Everything
below ranks *what* it is blocked on. The confirming evidence is one stack trace of
that thread while wedged — which §5 captures.

---

## 2. What runs on the main thread during idle (the pressure map)

The main thread only wedges if it is *doing* something when idle. Two verified
idle-active sources keep posting main-thread work indefinitely, with **no user
interaction required**:

### 2a. The 500 ms browser-bounds pump (primary suspect)

`Browser.tsx` runs `setInterval(syncBounds, 500)` (plus a `ResizeObserver` and a
`resize` listener). Every 500 ms it pushes the browser div's rectangle across the
JS→Rust boundary:

```
syncBounds()  →  invoke('update_browser_bounds', {x,y,w,h})   // active tab
              →  invoke('hide_browser', {webviewId})          // hidden
```

and Rust applies it to the **native child webview** (`browser.rs:310-340`):

```
update_browser_bounds → webview.set_position() + set_size()
```

`set_position` / `set_size` on an `NSView`/`WKWebView` are **main-thread-only**
AppKit operations — Tauri dispatches them to the main thread and the JS `invoke`
awaits the reply. So **every 500 ms, forever, even fully idle**, the app main
thread performs a geometry mutation on a *second* WebContent surface, driven by
JS in the *first* WebContent surface. That is a continuous cross-process coupling
through the main thread — precisely the kind of work that deadlocks when the OS
starts throttling/suspending the participants.

### 2b. The event/poll fan-out (secondary pressure)

The World view mounts a heavy, idle-active timer set, all of which post
main-thread work (webview `emit`s, `invoke` replies):

- **Three independent `/events` WebSocket** consumers: `stateSources.tsx:84`,
  `worldSignals.ts:158`, `LibrarianHUD.tsx:60`.
- **1 s polls**: `HenryHUD.tsx:105`, `LibrarianHUD.tsx:149`, `stateSources.tsx`
  (`HENRY_POLL_MS`).
- **`requestAnimationFrame` loop**: `WorldHUD.tsx:27`.
- Plus 5 s / 8 s / 15 s polls elsewhere (`AwarenessIndicator`, `SettingsView`,
  `ProjectsView`, `useDashboard`).

Each daemon `emit` to the webview and each `invoke` reply is a main-thread hop.
Backlogged while the app is occluded/App-Napped, they must all drain on return.

---

## 3. Ranked block candidates

| # | Candidate | Mechanism | Main-thread stack signature | Likelihood |
|---|---|---|---|---|
| **C1** | **App-Nap / occlusion cross-surface stall via the 500 ms pump** | After ~1 h idle+occluded, macOS App-Naps the app and throttles/suspends both `WebContent` processes. A `set_size`/`set_position` (or `emit`) that synchronously round-trips to the throttled/suspended browser `WebContent` blocks the app main thread. Window drag freezes; browser holds last frame; #558 regain-repaint can't run. | Main thread in a **WebKit synchronous XPC/`mach_msg`** wait (e.g. inside `-[WKWebView setFrame:]` → `WebPageProxy` sync send), *not* the benign `CFRunLoop` event wait. | **Highest** |
| **C2** | **Cross-process WebKit IPC deadlock, pump-independent** | Any main-thread webview op that synchronously waits on a wedged `WebContent`: an `emit` to a napped main webview, or `get_page_content`'s `eval` path. Same class as C1 without the 500 ms trigger. | Main thread in WebKit sync IPC `mach_msg`; differs from C1 only in the *caller* frame. | **High** |
| **C3** | **Resource / fd / WebSocket leak over the idle hour** | 3× `/events` WS + RAF + polls never torn down; a reconnect storm or fd growth degrades the process until it stalls. | Main thread NOT cleanly blocked — would show CPU churn or allocation, and fd/conn counts trending up. | **Lower** (0 % CPU "cleanly blocked" argues against exhaustion) |
| **C4** | **Rust `std::sync::Mutex` deadlock** (`browser.rs`/`terminal.rs`) | A `BrowserSessions`/`PtySessions` lock held across a main-thread-dispatched op. No code path currently shows a lock held across a main-thread wait, but cheap to rule out. | Main thread in **`__psynch_mutexwait`** with a Permagent Rust frame holding it. | **Low** |

**Disambiguation is entirely in the stack.** The four candidates have four
different main-thread signatures. One capture decides it:

- WebKit sync IPC `mach_msg` under a `set*`/`emit` frame → **C1/C2** (fix = UI
  lifecycle: suspend the pump when hidden + hold an App-Nap assertion).
- `__psynch_mutexwait` under a Permagent Rust frame → **C4** (fix = the specific
  lock).
- Benign `CFRunLoop`/`mach_msg_trap` event wait with everything idle → a **pure
  App-Nap suspension**. *But the reported wedge does not self-heal on drag
  attempts*, which a pure suspension would — so this is the least likely and, if
  seen, points at the OS napping a process that must stay awake (fix = App-Nap
  assertion regardless).

---

## 4. Cross-reference to #560 and #556

- **#560 (daemon durability):** its Part A external sampler is the right *shape*;
  #562 shows the sampler must also cover the **UI process** and add **wedge
  capture** (stack, not just counters). This doc's watchdog is that extension.
  #560's daemon findings are **not** the cause here (daemon stayed 200).
- **#556 (`WEBVIEW_LIFECYCLE.md`):** #562 sits **under** the render cluster. The
  doc already flagged the native-App-Nap assertion as a *"conditional
  follow-up, NOT part of [the S1] slice."* #562 is the condition that calls it in.
  The 500 ms bounds pump it documents is this doc's primary suspect (C1).

---

## 5. Instrumentation — capture the wedged stack (this round's deliverable)

Root confirmation needs the **main-thread stack while wedged**. The wedge takes
~1 h to appear and the app is unresponsive when it does, so capture must be
**automatic and external** (zero UI-process code, so it ships now and cannot
itself add to the hang). Ship: `scripts/ui-wedge-watchdog.sh` + a launchd
template. It mirrors #560's "Part A ships first" discipline.

Each tick (default 30 s) the watchdog:

1. Finds the GUI process (`permagent-app` / `Permagent.app` MacOS binary) and
   its `WebContent` children; records `ps` (pid, %cpu, rss, state, etime), open
   fd count, and TCP conns to `:3001` — one self-rotating JSONL line
   (`~/.permagent/logs/ui-durability-probe.jsonl`, capped, so the probe never
   becomes #560-F5).
2. **Wedge test:** takes a fast `sample <gui-pid> 1`. If the **main thread**
   (thread 0) is in a **non-idle** wait — anything other than the benign
   `CFRunLoop` `mach_msg` event wait — for **two consecutive ticks**, it declares
   a wedge.
3. **On wedge, captures the root-cause evidence** into
   `~/.permagent/logs/ui-wedge-<ts>/`:
   - `sample <gui-pid> 10` — full user-space stacks (the money shot: thread 0).
   - `spindump <gui-pid> 10` (best-effort; needs privileges) — kernel + all
     threads + "unresponsive" attribution.
   - `sample` of each `WebContent` child — is the browser/main content process
     also blocked (C1/C2) or fine?
   - `curl -m 3 -s -o /dev/null -w "%{http_code}" localhost:3001/api/people` —
     **re-confirms the daemon boundary automatically**, every wedge.
   - the last N probe lines, for the fd/conn trend into the wedge (C3).
4. Fires **once per wedge episode** (a captured flag clears when the main thread
   returns to the idle wait or the pid changes), so it never spams.

Detection is sound for the *reported* failure specifically: a wedge that will not
drag is a **real block with a non-idle stack** (not a plain nap, which drag would
wake), so thread 0 will show a distinctive frame the heuristic catches.

**Companion signal (designed, not built — needs a disk-safe Rust round):** a
**main-thread heartbeat** inside the app — `app.run_on_main_thread(...)` on a
repeating timer touching `~/.permagent/logs/ui-heartbeat`. Only a *main-thread*
heartbeat proves the run loop drains; a **JS/async-command heartbeat is useless
here** because an async Tauri command runs on a Tokio worker and would keep the
heartbeat fresh *while the main thread is wedged*. With the heartbeat present the
watchdog gets a crisp, zero-heuristic trigger (stale mtime → capture). It is
deferred only because it requires a `cargo` build the current disk (≈8.6 GiB free)
cannot safely run; the watchdog's stack heuristic covers the reported wedge in the
meantime.

---

## 6. Fix — identified candidates, NOT built this round (evidence-gated)

The mandate is *no blind fix*. The stack from §5 selects among:

- **If C1/C2 (WebKit sync IPC — expected):**
  1. **Suspend the 500 ms bounds pump when the document is hidden/occluded**
     (`document.hidden` / `visibilitychange`) so the app stops driving a
     throttled child surface from the main thread while backgrounded. This reuses
     the exact regain seam #558 already wired (`repaintOnRegain.ts`), pure-TS,
     low-risk. Likely the highest-leverage single change.
  2. **Hold an App-Nap assertion** for the "weeks-untouched, reachable" posture:
     `NSProcessInfo -beginActivityWithOptions:` (`.userInitiated |
     .idleSystemSleepDisabled`-style, minus display sleep) so macOS does not
     suspend the run loop while the daemon-backed app must stay reachable. This
     is the #556-deferred native assertion. **Battery trade-off ⇒ Jesse ruling
     (§7).**
- **If C4 (Rust mutex):** fix the specific lock ordering in `browser.rs` /
  `terminal.rs`; do not add the App-Nap assertion for its own sake.

Both fixes touch `cargo`/frontend gates and should land in a **disk-safe build
round after** the watchdog has captured one real stack.

---

## 7. Decision points for Jesse (Tier-2 — not auto-buildable)

1. **App-Nap posture.** Should the UI take an App-Nap-preventing activity
   assertion so the desktop app survives idle for the same "weeks-untouched"
   bar as the daemon — accepting the **battery/energy cost** of never napping —
   or should the app be *allowed* to nap and instead be made **nap-safe** (pump
   suspended when hidden, clean resume) so napping is harmless? (C1 fix shape.)
2. **UI supervision / auto-relaunch.** Mirror of #560 decision 2: should the
   watchdog, on a confirmed wedge, **auto-`pkill` + relaunch** the GUI (daemon
   untouched), or only capture + notify and leave recovery manual? Auto-relaunch
   makes "walk up after a week" self-healing; it also risks masking the bug.
3. **Probe/watchdog cadence & retention.** 30 s tick / capped JSONL /
   per-episode capture is a starting point; confirm before it ships as a
   standing launchd agent (it writes to the same near-full disk it watches — the
   #560-F5 lesson).
4. **Fold into one probe or two?** #560 proposed a daemon durability probe;
   #562 needs a UI one. Ship as **one** `~/.permagent/logs/*-durability-probe`
   agent covering both processes, or keep them separate?

---

## Honest bottom line

- **Confirmed:** the daemon survives the idle wedge (remote bar intact); the
  failure is a **block on the UI process's AppKit main thread** (proven by
  window-drag-freeze + WKWebView's multi-process model), not the #517 render
  cluster.
- **Narrowed, not yet proven:** the block is almost certainly a **cross-process
  WebKit IPC stall under App-Nap**, driven by the idle-active 500 ms browser-
  bounds pump (C1). A single captured thread-0 stack decides C1/C2 vs. a Rust
  mutex (C4) vs. a pure nap.
- **This round ships the capture, not a guess:** an external watchdog that grabs
  that stack (and re-confirms the daemon) on the next wedge. The fix is
  identified per-branch and gated on that stack — and on a disk-safe build round.
