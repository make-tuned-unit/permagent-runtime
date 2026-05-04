# Activity Bus Verification

*Date: 2026-05-04. Daemon binary: target/release/permagentd (built from this commit).*

## Test Procedure

1. Started daemon: `./target/release/permagentd agent`
2. Created session `20260504_1` via `POST /api/sessions`
3. Sent chat message "Say hello in one word." via legacy `/reply` endpoint
4. Sent chat message "Say goodbye in one word." via `/sessions/{id}/reply` SSE endpoint
5. Queried `GET /activity/recent?limit=10`

## Captured Events

```json
[
    {
        "event_id": "019df07c-d948-7b11-8683-1940013b390c",
        "event_type": "chat_turn_started",
        "source_surface": "chat",
        "timestamp": "2026-05-04T00:56:47.176962Z",
        "session_id": "20260504_1",
        "payload": {},
        "tier": "ephemeral"
    },
    {
        "event_id": "019df07c-dc83-7422-b3ce-baf9bc77c92b",
        "event_type": "chat_turn_completed",
        "source_surface": "chat",
        "timestamp": "2026-05-04T00:56:48.003683Z",
        "session_id": "20260504_1",
        "payload": {
            "duration_ms": 826,
            "input_tokens": 5353,
            "output_tokens": 5
        },
        "tier": "always"
    },
    {
        "event_id": "019df07d-3c50-78c2-8553-8d4f9928a71c",
        "event_type": "chat_turn_started",
        "source_surface": "chat",
        "timestamp": "2026-05-04T00:57:12.528525Z",
        "session_id": "20260504_1",
        "payload": {},
        "tier": "ephemeral"
    },
    {
        "event_id": "019df07d-3ec8-7e30-989c-4630943d3200",
        "event_type": "chat_turn_completed",
        "source_surface": "chat",
        "timestamp": "2026-05-04T00:57:13.160986Z",
        "session_id": "20260504_1",
        "payload": {
            "duration_ms": 632,
            "input_tokens": 5380,
            "output_tokens": 6
        },
        "tier": "always"
    }
]
```

## Verification Summary

| Check | Result |
|-------|--------|
| `/activity/recent` endpoint responds | PASS |
| ChatTurnStarted emitted on request | PASS |
| ChatTurnCompleted emitted on finish | PASS |
| Legacy `/reply` endpoint emits | PASS |
| SSE `/sessions/{id}/reply` endpoint emits | PASS |
| session_id populated | PASS |
| tier correctly assigned (started=ephemeral, completed=always) | PASS |
| payload contains duration_ms, input_tokens, output_tokens | PASS |
| Events forwarded to WebSocket bus (via PermagentEventType::Activity) | PASS (code verified) |
| Ring buffer limits to 500 | PASS (unit test) |
| `cargo build --release` | PASS |
| `npm run build` (command-center) | PASS |

## Surfaces Wired

| Surface | Events | File | Status |
|---------|--------|------|--------|
| Chat (legacy /reply) | ChatTurnStarted, ChatTurnCompleted | `crates/goose-server/src/routes/reply.rs:226,592` | WIRED |
| Chat (SSE /sessions/{id}/reply) | ChatTurnStarted, ChatTurnCompleted | `crates/goose-server/src/routes/session_events.rs:337,762` | WIRED |

## Surfaces Deferred

| Surface | Reason | Notes |
|---------|--------|-------|
| Embedded Browser | Frontend-only (Tauri webview) | Browser.tsx exists but navigation events are Tauri IPC, not daemon HTTP. Requires Tauri command bridge (spec item 4). No daemon endpoint exists to receive browser events today. |
| Embedded Terminal | Frontend-only (xterm.js) | Terminal.tsx uses Tauri IPC for PTY. No daemon-side terminal command tracking. Shell commands executed by the agent's developer extension ARE tracked via TaskLogger, but terminal UI commands are not. |
| Project Picker | No project selection event in daemon | UI sends session creation requests, but there's no explicit "project selected" action in the daemon API. Projects are implicit via working_dir on session creation. |
| File Viewer | Does not exist | No file viewer surface in the command-center UI. File operations happen through the agent's developer extension (tracked via TaskLogger). |
| Integrations Panel | No token refresh event path | The integrations route handles OAuth connect/callback but has no explicit token refresh endpoint that could emit an event. Token refresh happens inside the spectral/MCP client opaquely. |
| Skills Engine | No execution tracking | skill_executions table exists but nothing writes to it. SkillProposed and SkillSaved events already emit on the existing bus. SkillExecuted would need the execution write path first. |
| Tauri command bridge | Tauri app not part of this build | The `emit_activity_event` Tauri command would go in `ui/goose2/src-tauri/src/commands/`. Deferred because the command-center UI (web) is the primary interface; Tauri wraps it optionally. |
