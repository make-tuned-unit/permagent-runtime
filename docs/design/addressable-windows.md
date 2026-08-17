# Addressable windows — letting the agent know what is happening where

Status: design, not implemented. Written 2026-08-17.

The agent can already work inside a popped-out browser or terminal window. What
it cannot do is **tell them apart**. This is the design to fix that.

---

## The limitation, precisely

Two surfaces pop out into their own OS windows — `browser` and `terminal`
(`ui/command-center/src/lib/paneWindows.ts:4`) — each labelled
`${kind}-${crypto.randomUUID()}` (`:9-11`), so any number can be open at once.
Chat detaches too, by a different path and under the fixed label `'chat'`
(`ChatDock.tsx:50`), so there is only ever one of those.

The daemon knows about **none** of them. A grep for `webview_windows`,
`WebviewWindow`, `window_label` or `app_handle` across `crates/goose-server/`
returns nothing outside tests: the daemon is window-blind by construction.

That produces four concrete gaps.

**1. Reads and actions go to whoever answers first.** `read_content` emits a
`BrowserContentRequested` event on the global bus and blocks until *a* frontend
POSTs the content back (`routes/browser_content.rs:146-180`). The same
broadcast-and-wait shape is used for `snapshot/read` and `act`
(`routes/browser_act.rs:196-199`). Every window with a `Browser` component
mounted is an equal candidate, because a popped-out window renders that very
component (`PaneWindowApp.tsx:151`). With three browser windows open, which one
the agent reads is a race.

**2. The agent cannot enumerate.** `observe_app` has no window aspect. There is
no tool, no API and no event that answers "how many windows are open, and what
is in them".

**3. The agent cannot address.** No browser tool takes a window argument, so
even if the agent knew three windows existed it could not choose one.

**4. Terminals and browsers are asymmetric.** Terminal PTY sessions are
registered and addressed by session id (`terminal_supervision.rs:375, 409, 435,
467`), so terminal output already has identity and survives being popped out.
Browsers have nothing equivalent. Half the problem is already solved on one side
and not the other.

## Three corrections to the first draft of this design

A full inventory of the tool surface changed three things. Recorded because each
made the design smaller or the scope honest.

**1. The ownership gate already exists — for one of three bridges.**
`act_on_page` does *not* race. `resolveActBinding`
(`useBrowserActBridge.ts:170-201`) returns `ignore` when the requested
`webview_id` is not in this client's `ownedWebviewIds`, and the comment records
why: bug #939, where two windows each executed one act and the loser's fulfil
404'd invisibly. So the precedent is in-tree, was written for exactly this class
of bug, and **content-read and snapshot simply never got it**. This is far less
"design a mechanism" and much more "extend the one that already works to the two
paths that were missed".

**2. A per-target identity already exists — it is just hidden from the model.**
`get_page_snapshot` mints a `webview_id` and stashes it server-side keyed by
agent session (`browser.rs:497, 725-731`), and `act_on_page` sends it on the
wire. The agent never sees it and cannot supply it. So the identity does not
need inventing; it needs **surfacing and accepting**.

**3. Terminals cannot be addressed because they cannot be touched at all.**
`terminal_supervision.rs` registers **zero tools** — no `Tool::new` anywhere in
the file. There is no tool to read a terminal's output, write to one, or list
open terminals. `project_launch` creates a *new* tab and returns a `sup-` id in
prose that nothing accepts back. So terminal addressing is not a gap in this
design's sense; **the tools do not exist yet**, and building them is a separate
piece of work. This design covers browsers, and stops honestly at that line.

Two smaller notes that change details rather than shape:

- **`window` is already taken.** In `app_perception.rs` it means a *time* window
  (`7d/30d/90d/365d/all`, `parse_window` at `:476-491`). A `windows` aspect would
  collide with it in the params. Use **`panes`**.
- **Two label namespaces already exist and are known to be confusable.** Pane
  *windows* are `browser-<uuid>` (`paneWindows.ts:9-11`); child *webviews* are
  `browser-<n>` from a process counter (`ui/desktop/src-tauri/src/browser.rs:225`).
  `main.rs:278-283` flags them as distinct namespaces. The registry must be
  explicit about which it stores — it should key on the **webview id**, since
  that is what the bridges already carry, and record the window label alongside
  it as context.

## What is already right — do not rebuild it

The bridge is **not** the problem and should not be replaced. `PendingBridge<T>`
is a UUID-keyed map of oneshot channels with an RAII slot that releases on drop,
explicitly so a caller disconnecting mid-await cannot leak
(`routes/browser_content.rs:20-56`). Request/response correlation is already
correct and already concurrent-safe.

The missing piece is not transport. It is **addressing**: the request goes out
with no statement of who it is for.

That is a much smaller change than it first appears.

---

## Design

### 1. A window registry in the daemon, fed by the frontend

`WindowRegistry` in `AppState`, in memory:

```rust
struct WindowRecord {
    id: String,            // the Tauri label — NOT a new id
    kind: WindowKind,      // Main | Browser | Terminal | Chat
    title: String,
    subject: Option<String>,   // browser: current URL; terminal: cwd or session id
    terminal_session_id: Option<String>,
    focused: bool,
    registered_at: DateTime<Utc>,
    last_seen_at: DateTime<Utc>,
}
```

**Reuse the Tauri label as the id.** It already exists, is already unique, and
is already what the frontend uses to route redock events. Minting a second
identity would create two id spaces that drift, and drift between two copies of
one fact is the failure mode this codebase keeps getting bitten by.

**Why daemon-side rather than asking Tauri.** Two reasons, and the second is
decisive. The daemon is a separate process from the Tauri shell, so it would
need IPC either way. And the UI is *also* served as a plain web app at `/ui`
(`routes/mod.rs:142`) where there is no Tauri at all — a Tauri-derived window
list would simply not exist in browser mode. A registry the frontend maintains
works in both.

**In memory, never persisted.** Windows do not survive a restart, and a
persisted registry would resurrect ghosts that the agent would then try to
address. Losing the registry on restart is correct behaviour, not a gap.

### 2. Registration lifecycle

| Event | Call |
|---|---|
| window mounts | `POST /api/windows` → `{id, kind, title, subject}` |
| URL / tab / cwd changes | `PATCH /api/windows/{id}` |
| every 15s | `POST /api/windows/{id}/heartbeat` |
| window closes | `DELETE /api/windows/{id}` |

The main window registers too, as `kind: Main`. If it did not, "windows the
agent can see" would exclude the one the user is usually looking at, and the
enumeration would be quietly wrong in the most common case.

**Liveness is three-valued, not two.** A window that has missed heartbeats for
more than 60s is marked `stale` — still listed, flagged — and reaped after
5 minutes. "Present", "not heard from recently" and "gone" are different facts
and the agent should be able to tell them apart. Collapsing stale into gone
would make a briefly-suspended window look closed.

**Re-registration on 404.** If the daemon restarts, a window's heartbeat gets a
404; the frontend re-registers rather than assuming it is still known. Without
this, a daemon restart silently empties the registry while windows stay open.

### 3. Bridges gain an optional target — and refuse to guess

Each bridge request takes `target_window_id: Option<String>` and the emitted
event carries it. Every frontend listener compares against its own label and
**ignores requests not addressed to it**.

Resolution, in order:

- **Target given, window registered** → only that window answers.
- **Target given, unknown id** → `no such window` error, listing live candidates.
  Distinct from a timeout: "that window is gone" and "that window did not
  answer" are different problems with different fixes.
- **No target, exactly one candidate** → answers. Identical to today, so the
  single-window case — which is nearly all of them — is unchanged.
- **No target, several candidates** → **error naming every candidate.**

That last case is the heart of this design. The temptation is to keep taking
whoever answers first, because it "works". It does not work: it silently reads
the wrong page and returns content the agent has no reason to distrust. An
ambiguity error is a worse demo and a better product, and it is the same
empty-vs-broken distinction this codebase already enforces elsewhere — a result
whose provenance is unknown must not be presented as a result.

### 3b. Reuse the existing liveness counter, don't run a parallel one

`events::has_listeners()` over `UI_CLIENTS: AtomicUsize`
(`crates/goose/src/events/mod.rs:99-114`) already counts connected `/events`
sockets, incremented and decremented by an RAII guard on the socket itself.
`navigate` already uses it to 503 when no UI is attached
(`browser_content.rs:222-231`).

That is the correct lifecycle hook: a socket that drops is a window that is gone,
observed by the transport rather than reported by a heartbeat that can lie. The
registry should hang off the **same** guard — register on socket open, drop on
socket close — with heartbeats as a *refinement* for subject changes (URL, cwd),
not as the liveness signal. A second, parallel notion of "is this window alive"
would be a second copy of one fact, which is the failure mode this codebase keeps
paying for.

### 4. `observe_app` gains a `panes` aspect

Following the existing aspect conventions exactly: bounded by `LIST_LIMIT`, with
`returned` / `total` / `truncated`, and a `status` that separates `available`,
`empty` and `unavailable`, so "no windows open" never renders the same as
"could not read the registry".

```json
{
  "surface": "windows", "queried": true, "status": "available",
  "data": {
    "windows": {
      "items": [
        {"id": "main", "kind": "main", "title": "Permagent", "focused": true},
        {"id": "browser-8f3c…", "kind": "browser",
         "subject": "https://example.org/pricing", "focused": false,
         "stale": false},
        {"id": "terminal-1a90…", "kind": "terminal",
         "subject": "~/dev/permagent-runtime",
         "terminal_session_id": "pty-4471", "focused": false, "stale": false}
      ],
      "returned": 3, "total": 3, "limit": 5, "truncated": false
    }
  }
}
```

`subject` is what makes this useful. "Three browser windows" tells the agent
nothing; "one on the pricing page, one on the docs" is what lets it choose.

### 5. Tool surface

`read_browser_content` and `get_page_snapshot` take an **optional `pane`
argument** — the webview id the agent read out of a previous snapshot or the
`panes` aspect. No new per-window tools, no tool multiplication. The default is
the resolution ladder above, so the common single-window case needs no argument
and the ambiguous case fails loudly.

`act_on_page` needs no new argument: it already resolves by ownership. It should
gain the *ambiguity error* though, so "no snapshot identity for this session"
stops being a silent `ignore`.

**Terminals are out of scope and that is a finding, not an omission.** There are
no terminal tools to add an argument to. A `sup-` id already exists, is already
registered before the launch event fires, and already survives pop-out (PTYs
live in the app process, so redocking is enough for the shell to keep running).
The registry should record the window ↔ session mapping now, because it is free
and the ids exist — but the agent cannot use it until terminal read/write tools
exist. Adding those is the follow-on piece.

### 5b. Adding a surface has a checklist, and it is enforced

`OBSERVABLE_SURFACES` is not a loose list. A new surface must be added to the
const (`app_perception.rs:41-60`), paired in `TAB_SURFACES` (`:77-137`) with the
store it reads, named in `SELF_KNOWLEDGE_FEATURE`'s prose (`:144-176`) and in the
`ObserveAppParams.surface` doc comment (`:179-186`) — and
`every_shipped_tab_is_observable_or_exempt` in the daemon crate checks the
pairing against `catalog.yaml`. Any implementation that skips a step fails the
build, which is the intended behaviour and worth knowing before starting rather
than discovering.

---

## Guards

The design is only worth having if the addressing cannot silently regress. Each
of these must be watched go red before it is trusted.

1. **Two registered browser windows, no target** → ambiguity error naming both.
   Mutate the resolution to pick the first candidate: must go red.
2. **Targeted read** → only the addressed window fulfils; the other listener is
   asserted *not* to have been asked. Mutate the frontend's label comparison to
   accept everything: must go red.
3. **Unknown window id** → `no such window`, and specifically *not* a timeout.
   These two failures are confusable and the whole point is to keep them apart.
4. **Closed window** → removed from enumeration; a read against it gives
   `no such window`.
5. **Stale ≠ gone** → a window past the heartbeat threshold is still listed and
   flagged, not dropped.
6. **Floor** — the `windows` aspect never reports zero while the main window is
   registered. Without this, every assertion above passes vacuously on an empty
   registry.

Guard 6 is not ceremony. Most of the tests here iterate over a window list, and
a list-iterating test over an empty list passes having checked nothing.

---

## Scope, as confirmed by the user

Two things are wanted: **send messages to a terminal**, and **review a browser
window I have open**. Window *management* — moving, focusing, resizing — is
explicitly not wanted. That settles the boundary cleanly.

So terminals come back into scope, and the work is larger than the browser half
because the tools genuinely do not exist. The good news is that most of the
machinery does:

| Need | State |
|---|---|
| list open terminals | `list_sessions` exists (`terminal_supervision.rs:636`); `GET /terminal/supervised/sessions` exists with **no MCP caller** |
| read a terminal's output | `session_snapshot` exists (`:619`), fed continuously by `ingest_output` (`:467`) from the Tauri tee |
| write to a terminal | `write_to_pty` exists in the **app** process (`ui/desktop/src-tauri/src/terminal.rs:387`) with **no daemon path to it** |
| identify a terminal | `sup-` id minted before the launch event; `resolve_session_id` maps either id (`:435`) |

So the read side is a tool wrapper over routes that already work. The write side
needs a daemon→app path that does not exist yet, and is the only genuinely new
plumbing in this design.

**Writing to a terminal is the one privileged operation here** and should be
treated as such: it injects keystrokes into a live shell the user is watching. It
must target an explicit session id — never "the current terminal" — and must not
fall back to a guess when the id is unknown. The ambiguity rule applies with more
force here than for reads, because a misdirected read returns the wrong answer
while a misdirected write runs a command in the wrong shell.

## Deliberately not in v1

- **Window management** — focus, move, resize, close. Confirmed not wanted.
- **Chat window unification.** Chat keeps its fixed `'chat'` label and singleton
  behaviour. It registers so it is enumerable, but nothing else changes.
- **Cross-window state sync.** Out of scope; each window keeps owning its tabs.
- **Persisting the registry.** Covered above — deliberate.

## Risks

- **Close/act race.** A window can close between enumeration and use. Mitigated,
  not eliminated, by the distinct `no such window` error — the agent can re-read
  and retry rather than misreading a timeout as a hung page.
- **Heartbeat cost.** 15s per window is negligible, but it is a new periodic
  write path; it should not touch the database, only in-memory state.
- **A window that registers and never listens.** The registry says a window
  exists; the bridge listener is what makes it answerable. If those two diverge
  the agent sees a window it cannot read. Registration should therefore happen
  in the same component that mounts the listener, not in a parent.

## Estimated shape

Around 400–600 lines including tests: a registry module and routes on the Rust
side, an optional field threaded through three existing bridges, a registration
hook plus label comparison in the frontend, and one new `observe_app` aspect.
No new subsystem, no change to the bridge transport.
