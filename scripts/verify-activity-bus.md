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
