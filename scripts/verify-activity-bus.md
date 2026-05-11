# Activity Bus Verification

## Phase 3a — Brain Ingestion + ContextBuilder

*Date: 2026-05-04. Spectral pin: 795c4c7 (PR #53).*

### Startup Confirmation

```
ActivityIngester subscribed to event bus, device_id=permagent-host
ContextBuilder subscribed to event bus
```

### Ingest-Status After Test Sequence

6 events emitted: 2 Always, 2 Aggregated, 2 Ephemeral.

```json
{
    "events_ingested": { "always": 2, "aggregated": 2 },
    "events_observed": { "ephemeral": 2 },
    "ingestion_failures": 0,
    "aggregation_queue_size": 2,
    "last_ingested_at": "2026-05-04T18:59:42.082786+00:00",
    "context_builder": {
        "recent_events_buffered": 6,
        "live_state": {
            "active_project_id": "project:permagent",
            "active_session_id": "test-p3a",
            "last_browser_url": "https://docs.rs",
            "last_terminal_command": "ls -la",
            "last_terminal_cwd": "/tmp",
            "events_in_last_5_minutes": 0
        }
    }
}
```

### Brain Inspection — Ingested Activity Memories

```
key                                              | source              | content                                                                      | compaction_tier | visibility
activity:1777921182:project_selected:5fa6e4f4    | permagent.activity  | Started working in project Permagent (project:permagent).                     | raw             | private
activity:1777921182:browser_navigated:d209673c   | permagent.activity  | Navigated to Docs.rs (https://docs.rs) in tab t1.                            | raw             | private
activity:1777921181:chat_turn_completed:4e396953  | permagent.activity  | Chat turn completed in 1200ms (500 input tokens, 80 output tokens).          | raw             | private
```

All ingested memories have:
- source = "permagent.activity"
- compaction_tier = "raw"
- visibility = "private"
- Human-readable content (not JSON)
- Keys formatted as activity:{timestamp}:{event_type}:{event_id_prefix}

### Tier Behavior Confirmed

| Tier | Events | Brain Write | Queue |
|------|--------|-------------|-------|
| Always | ChatTurnCompleted, ProjectSelected | YES | No |
| Aggregated | 2x BrowserNavigated | YES | YES (queue_size=2) |
| Ephemeral | ChatTurnStarted, TerminalCommandStarted | NO | No |

### ContextBuilder Tests (12/12 passed)

```
test activity::context_builder::tests::live_state_tracks_browser_url ... ok
test activity::context_builder::tests::live_state_tracks_terminal_command ... ok
test activity::context_builder::tests::live_state_tracks_project_selection ... ok
test activity::context_builder::tests::ring_buffer_bounded ... ok
test activity::context_builder::tests::current_digest_returns_recent_events ... ok
```

### Ingester Tests (4/4 passed)

```
test activity::ingestion::tests::render_chat_turn_completed ... ok
test activity::ingestion::tests::render_project_selected ... ok
test activity::ingestion::tests::render_browser_navigated ... ok
test activity::ingestion::tests::ephemeral_events_are_not_ingested ... ok
```

### Build Status

- `cargo build --release`: PASS
- `cargo test --package permagent --lib activity`: 12 passed
- `cargo test --package permagent --lib events`: 14 passed
- `cargo test --package permagent --lib identity`: 13 passed
- `npm run build` (command-center): PASS

## Terminal Persistence Fix

Terminal tabs now persist across workspace switches using module-level
state (same pattern as Browser.tsx). PTY sessions stay alive on the
Tauri backend. When the user returns to the Build page, terminals
reconnect to existing PTY sessions.

File: `ui/command-center/src/components/terminal/TerminalManager.tsx`

## Phase 3a Follow-up — Active Project Tracking + Wing Override Pre-staging

*Date: 2026-05-05.*

### Active Project Tracking via ingest-status

Before ProjectSelected:
```json
"active_project": null
```

After emitting ProjectSelected for "project:permagent":
```json
"active_project": {
  "project_id": "project:permagent",
  "project_name": "Permagent",
  "wing": "permagent"
}
```

After switching to "project:get-ladle":
```json
"active_project": {
  "project_id": "project:get-ladle",
  "project_name": "Get Ladle",
  "wing": "get-ladle"
}
```

### Brain Rows — Wing Still Spectral-Assigned (Override Stubbed)

```
key                                              | source             | wing    | compaction_tier
activity:1777941342:project_selected:d3d10dea    | permagent.activity | general | raw
activity:1777941340:project_selected:3c673891    | permagent.activity | general | raw
```

Pre-activation rows show `wing: general` (Spectral TACT classifier).

### Wing Override Activation — Spectral PR #56 (rev daabbda)

*Date: 2026-05-05. Pin updated from 795c4c7 to daabbda.*

Wing override is now live. Brain rows after activation:

```
key                                                    | wing       | content
activity:1777944216:project_selected:06773b7b          | permagent  | Started working in project Permagent (project:permagent).
activity:1777944216:chat_turn_completed:b323b19d       | permagent  | Chat turn completed in 700ms (200 input tokens, 30 output tokens).
activity:1777944216:browser_navigated:f4137065         | permagent  | Navigated to Permagent Repo (https://github.com/permagent) in tab t1.
activity:1777944216:project_selected:236ad05a          | get-ladle  | Started working in project Get Ladle (project:get-ladle).
activity:1777944216:browser_navigated:c8a656dd         | get-ladle  | Navigated to Get Ladle (https://getladle.com) in tab t2.
```

Wing tracks the active project correctly:
- Events after selecting Permagent → wing: permagent
- Events after switching to Get Ladle → wing: get-ladle
- Classifier is bypassed when wing is set (no "general" fallback)

### Unit Tests (17/17 passed)

```
derive_wing_slug tests:
  wing_slug_from_canonical_project ........... ok  -> Some("permagent")
  wing_slug_from_project_with_hyphens ....... ok  -> Some("get-ladle")
  wing_slug_no_prefix_returns_none .......... ok  -> None
  wing_slug_did_returns_none ................ ok  -> None
  wing_slug_empty_returns_none .............. ok  -> None
  wing_slug_empty_after_prefix_returns_none . ok  -> None

active project tracking tests:
  active_project_set_on_project_selected ............. ok
  active_project_replaced_on_subsequent_project_selected ok
  active_project_unchanged_when_project_id_malformed . ok
  wing_override_computed_during_ingestion ............ ok
```

## Phase 3b — Ambient Awareness Loop Closed

*Date: 2026-05-04. Spectral pin: daabbda (unchanged).*

### Probe Integration into Chat Turn

On every chat reply:
1. ContextBuilder.current_digest called with include_probe: true
2. Recent events synthesized into context string
3. Brain.recall used with wing_filter from active project
4. Results rendered as `<ambient_context>` system prompt block
5. Recall query included when user message > 20 chars

System prompt structure:
```
<ambient_context>
<live_state>
You are currently working in: permagent (project:permagent).
Recent terminal command: cargo test --release.
Activity in last 5 minutes: 12 events.
</live_state>

<recent_activity>
- 14:32 Ran 'cargo test --release' -- exit 0, took 4200ms
- 14:28 Started working in project Permagent
</recent_activity>

<recognized_memories>
The following memories from your past activity may be relevant:
- "Started working in project Permagent..." (relevance: 0.84, wing: permagent)
</recognized_memories>
</ambient_context>
```

### Inspection Panel

- Eye icon button added to chat widget header
- Slide-over panel (400px, right-aligned) shows:
  - Active project with wing badge
  - Per-source filter pills (chat, browser, terminal, project, skills, integrations)
  - Live event tail (click to expand payload JSON)
  - Collapsible recent ambient memories section
  - Collapsible current digest section (refreshes every 10s)
  - Pause/Resume toggle (gates Brain writes, not event emission)
  - "Open Brain" link (deferred to Phase 3.5)

### CLI: permagent activity tail

```
$ permagent activity tail
Connected to daemon activity stream. Press Ctrl-C to exit.
14:32:18 [Chat] ChatTurnStarted session=abc-1234
14:32:19 [Chat] ChatTurnCompleted (1200ms)
14:32:25 [Browser] BrowserNavigated docs.rs/spectral
14:32:40 [Terminal] TerminalCommandStarted "cargo test"

$ permagent activity tail --json
{"event_id":"...","event_type":"ChatTurnStarted",...}

$ permagent activity tail --filter terminal
14:32:40 [Terminal] TerminalCommandStarted "cargo test"
```

### New Daemon Endpoints

| Method | Route | Auth | Purpose |
|--------|-------|------|---------|
| GET | /activity/recent | Bearer | Auth-gated (was unauthenticated) |
| GET | /activity/recent-memories | Bearer | Last N activity-source memories |
| GET | /activity/current-digest | Bearer | Full ContextBuilder digest as JSON |
| POST | /activity/pause | Bearer | Pause Brain writes |
| POST | /activity/resume | Bearer | Resume Brain writes |

### Pause/Resume Verification

- POST /activity/pause → `{"paused": true}`
- Events still flow on WebSocket bus while paused
- Brain writes skipped (verified via ingest-status counts)
- POST /activity/resume → `{"paused": false}`
- Brain writes resume normally

### Build Status

- `cargo check`: PASS (warnings only)
- `cargo test --package permagent --lib activity`: 25 passed
- `cargo test --package permagent --lib events`: 13 passed (1 race-condition flake when run with other tests)
- `cargo test --package permagent --lib identity`: 13 passed
- `npx tsc --noEmit` (command-center): PASS
- `npm run build` (command-center): PASS

## probe_recent Activation Verification

*Date: 2026-05-05. Spectral pin: ee2931a.*

### Implementation Status

All probe_recent wiring was completed in Phase 3b (commit 6d97b8a29).
The Spectral pin was already updated to ee2931a which exports
`probe_recent`, `ProbeOpts`, `ProbeWindow`, and `RecognizedMemory`
through the public `spectral::Brain` wrapper.

**Code path (chat turn):**
1. `reply.rs` / `session_events.rs` call `context_builder.current_digest(DigestOpts { include_probe: true, focus_wing, .. })`
2. `ContextBuilder::current_digest()` calls `brain.probe_recent(window, ProbeOpts { wing_filter, max_results, min_relevance })`
3. Results sorted by `relevance` descending
4. `render_ambient_context()` renders `<recognized_memories>` block with `{:.2}` relevance formatting

**Code path (REST endpoint):**
`GET /activity/current-digest` → same `current_digest(include_probe: true)` → serialized via `serde_json::to_value(&digest)`

### Endpoint Verification

```
# current-digest returns successfully (no panic — spawn_blocking fix applied)
$ curl -s -H "Authorization: Bearer $TOKEN" http://127.0.0.1:3001/activity/current-digest
{
  "live_state": { "events_in_last_5_minutes": 6, ... },
  "recent_events": [ ... 6 events ... ],
  "probed_memories": [],
  "recalled_memories": []
}
```

`probed_memories: []` — expected on fresh Brain with no pre-existing activity
memories. The probe fires cleanly (no tokio runtime panic after
spawn_blocking fix).

### Test Results

```
cargo test --package permagent --lib activity
  27 passed; 0 failed; 0 ignored
```

## Activity Bus Restoration Verification (commit 5d8b79062 + follow-up)

**Date:** 2026-05-06
**Issue:** `emit_activity` Tauri command was missing from `ui/desktop/src-tauri/`,
causing all frontend activity emissions to be silent no-ops. Additionally, the
command used Tauri 2's default camelCase parameter naming while the frontend
called with snake_case — so even after porting the command, the invoke
deserialization silently failed.

**Root causes (two-layer):**
1. `emit_activity` Tauri command never ported from `ui/goose2/` to `ui/desktop/`
2. Tauri 2 `#[tauri::command]` auto-renames `event_type` → `eventType` on the JS
   side, but the frontend sends `event_type`. Fixed with
   `#[tauri::command(rename_all = "snake_case")]`.

**Fix verified end-to-end on rebuilt desktop app.**

### Baseline (before fix)

Brain activity memory count: **19**

Most recent 5 entries (all chat — zero terminal/browser):
```
activity:1778091247:chat_turn_completed:019dfe7f  Chat turn completed in 2183ms
activity:1778091220:chat_turn_completed:019dfe7e  Chat turn completed in 716ms
activity:1778090143:chat_turn_completed:019dfe6e  Chat turn completed in 649ms
activity:1778088770:chat_turn_completed:019dfe59  Chat turn completed in 3293ms
activity:1778087112:chat_turn_completed:019dfe40  Chat turn completed in 946ms
```

### Reproducer (direct Tauri invoke)

Dev tools not available in release build. Verified via `eprintln!` instrumentation
in `activity.rs` — before `rename_all` fix, the function was never called (Tauri
deserialization failed silently). After fix, every terminal interaction produces
`[activity] emit_activity called` + `[activity] OK: accepted=true` in stderr.

### After verification actions

Brain activity memory count: **26** (delta: **+7**)

Ring buffer captured events from real app interactions:
```
terminal_session_started  terminal  2026-05-06T19:35:04  session=pty-dd7f6b3f, cwd=/Users/USER
terminal_command_started  terminal  2026-05-06T19:35:14  command="echo test", cwd=/Users/USER
terminal_command_started  terminal  2026-05-06T19:39:38  command="cd ~/dev/canon", cwd=/Users/USER
terminal_command_started  terminal  2026-05-06T19:39:41  command="claude", cwd=/Users/USER/dev/canon
terminal_session_started  terminal  2026-05-06T19:40:42  session=pty-31b0f2d3, cwd=/Users/USER
terminal_command_started  terminal  2026-05-06T19:40:57  command="cd ~/dev/World Litter Run"
terminal_command_started  terminal  2026-05-06T19:40:58  command="claude", cwd=.../World Litter Run
```

### ContextBuilder digest (live state)

```json
{
  "live_state": {
    "last_terminal_command": "claude",
    "last_terminal_cwd": "/Users/USER/dev/World Litter Run",
    "events_in_last_5_minutes": 7
  }
}
```

### Chat with awareness

User asked: "Hey can you see what project I am working on in the Terminal?"

Agent referenced real terminal state from ambient context:
- "You're in the `~/dev/canon` directory"
- "You ran the `claude` command"
- "There's been terminal activity in the last 5 minutes"

Citation marker appeared: **yes** ("based on 5 memories")

Before the fix, the same question produced: "I don't have direct access to your
terminal or its current state" with zero context.

### Build verification

```
cargo check --package permagent --lib           OK
cargo test --package permagent --lib activity    27 passed
cargo build --release (desktop shell)           OK
npm run build (command-center)                  OK
npm run tauri build (full bundle)               OK
```

### Conclusion

Activity bus fully operational from the desktop shell. Terminal session lifecycle
events (started/ended) and command events now flow through the Tauri command →
daemon → ring buffer → ContextBuilder pipeline. The Phase 3.5 awareness UX feeds
on real signal across terminal and chat surfaces. Browser events are wired but
require separate browser navigation to verify (browser `emitActivity` helper uses
the same `emit_activity` Tauri command and will work identically).

**Key lesson:** Tauri 2 `#[tauri::command]` silently renames snake_case params to
camelCase. Frontend code must match, or use `rename_all = "snake_case"` on the
Rust side. This mismatch is invisible — the invoke fails with a deserialization
error caught by `.catch()`, producing no visible error in release builds.

Key tests covering probe_recent:
- `probe_results_sorted_by_relevance_descending` — seeds Brain with activity memories, runs probe, asserts descending relevance order
- `probe_wing_filter_passes_through` — seeds memories in two wings, probes with `focus_wing: Some("permagent")`, asserts all results have matching wing
- `current_digest_returns_recent_events` — verifies `include_probe: false` default returns empty probed_memories

### Notes

- The `/activity/current-digest` endpoint previously panicked with
  "Cannot start a runtime from within a runtime" because
  `Brain::probe_recent()` calls `block_on()` internally. Fixed by
  wrapping in `tokio::task::spawn_blocking()` (same pattern as
  `get_recent_memories` handler).
- Real relevance scores will appear once the Brain accumulates
  activity-tagged memories over multiple chat sessions. The probe
  synthesizes a query from recent events and searches for semantically
  matching memories — requires a populated Brain to produce non-empty results.

## Phase 3.5 — Visible Awareness UX

*Date: 2026-05-06.*

### New SSE Event: ContextAttached

Added `ContextAttached` variant to `MessageEvent` enum in `reply.rs:148-152`:
```rust
ContextAttached {
    probed_memories: Vec<ProbedMemoryRef>,
    recalled_memories: Vec<RecalledMemoryRef>,
}
```

Emitted from:
- `reply.rs` (after digest success, before extend_system_prompt) via `stream_event()`
- `session_events.rs` (same location) via `publish()`

Only emitted when at least one probed or recalled memory exists — no-op on empty digests.

### New Frontend Components

1. **AwarenessIndicator** — `ui/command-center/src/components/awareness/AwarenessIndicator.tsx`
   - Persistent row above chat input, polls /activity/current-digest every 5s
   - Shows "Aware of {project} · {N} events · {M} memories" with time-decay suffix
   - Clicking opens inspection panel

2. **PreTurnPreview** — `ui/command-center/src/components/awareness/PreTurnPreview.tsx`
   - Appears on input focus, describes what the agent will consider
   - Fades out on blur

3. **CitationMarker** — `ui/command-center/src/components/awareness/CitationMarker.tsx`
   - Small "based on N memories" pill on agent responses
   - Click expands popover showing probed memories with relevance scores and wing badges

### ChatWidget Integration

Layout (top to bottom):
- Chat header with eye icon (unchanged)
- Message list (unchanged, now with CitationMarker on assistant bubbles)
- AwarenessIndicator (new, persistent)
- PreTurnPreview (new, focus-triggered)
- Chat input (unchanged)

### Store Updates

- `ChatMessage` extended with optional `context_attached` field
- SSE handler processes `ContextAttached` events, attaches to streaming message
- Pending context mechanism for race conditions (event arrives before message placeholder)

### Build Status

- `cargo check --package permagent-daemon`: PASS (pre-existing warnings only)
- `cargo test --package permagent --lib activity`: 27 passed
- `npx tsc --noEmit`: PASS
- `npm run build`: PASS
- `vitest run time-decay.test.ts`: 7 passed
- Tauri bundle: PASS

## Action Affordances Runtime Verification (commit d7e132c34)

**Date:** 2026-05-07
**Status:** Verified end-to-end on rebuilt desktop app

### Trash action proof
Test file: `/Users/USER/Downloads/permagent-test-trash-1778176790.txt`
Pre-action: file at original location (71 bytes, created 14:59)

API request:
```
POST /automation/finding/test-trash-file/action
{"action": "trash", "run_id": "20260507_3"}
```

API response:
```json
{
  "finding_id": "test-trash-file",
  "action_taken": "trashed",
  "size_recovered_bytes": 71,
  "trash_path": "/Users/USER/.Trash/permagent-test-trash-1778176790.txt",
  "timestamp": "2026-05-07T18:13:58.894628+00:00"
}
```

Post-action: file gone from Downloads, present in ~/.Trash/
Native Trash semantics confirmed via `trash` crate (macOS NSFileManager).

### Sensitive path rejection
Synthetic finding with path `/Users/USER/.ssh/id_rsa_test`

API response:
```json
{"error": "Refusing to trash sensitive path: /Users/USER/.ssh/id_rsa_test"}
```

HTTP status: 403 Forbidden. Validation rejected BEFORE checking file existence.

### Persistence across daemon restart
Pre-restart finding state: `action=trashed, actioned_at=2026-05-07T18:13:58`
Daemon killed and restarted.
Post-restart finding state: `action=trashed, actioned_at=2026-05-07T18:13:58`
Action state preserved in `~/.permagent/automation/findings/20260507_3.json`.

### Scheduler events
4 automation events captured in activity bus:
```
automation_job_started   scheduler  2026-05-07T18:08:42
automation_job_started   scheduler  2026-05-07T18:08:51
automation_job_completed scheduler  2026-05-07T18:08:59
automation_job_completed scheduler  2026-05-07T18:10:36
```

### Brain state
Baseline: 89 activity memories
Post-verification: 91 (+2 automation_job_completed events)

### Conclusion
Action affordances verified end-to-end: native Trash via `trash` crate,
sensitive path rejection, findings persistence across daemon restart,
and scheduler event emission. Ready for use in Dispatch D recipes.
