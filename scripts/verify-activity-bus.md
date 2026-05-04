# Activity Bus Verification — Phase 2

*Date: 2026-05-04. Daemon binary: target/release/permagentd.*

## Phase 2 Surfaces Wired

| Surface | Events | File | Line(s) | Method |
|---------|--------|------|---------|--------|
| Chat (legacy /reply) | ChatTurnStarted, ChatTurnCompleted | `crates/goose-server/src/routes/reply.rs` | 226, 592 | Direct emit_activity() |
| Chat (SSE /sessions/{id}/reply) | ChatTurnStarted, ChatTurnCompleted | `crates/goose-server/src/routes/session_events.rs` | 337, 762 | Direct emit_activity() |
| Browser | BrowserSessionStarted, BrowserNavigated, BrowserSessionEnded | `ui/command-center/src/components/browser/Browser.tsx` | 52, 186, 325, 366 | Tauri invoke → POST /activity/emit |
| Terminal | TerminalCommandStarted | `ui/command-center/src/components/terminal/Terminal.tsx` | 199 | Tauri invoke → POST /activity/emit |

## Phase 2 Surfaces Deferred

| Surface | Reason |
|---------|--------|
| TerminalCommandCompleted | PTY exit is detected (`pty_exit` event at Terminal.tsx:169) but does not carry exit_code, stdout_summary, or duration_ms. The PTY streaming model needs redesign to capture command boundaries and output (Phase 2.5). Currently only TerminalCommandStarted fires on Enter. |
| BrowserFormSubmitted | Requires injecting a JS shim into web pages to intercept form submissions. Fragile and browser-dependent. Deferred to Phase 2.5 when Rust-owned browser surfaces are built. |
| ProjectSelected / ProjectOpened | No project selection lifecycle event exists. Projects.rs is CRUD-only. The frontend manages project state in React. A `select_project` Tauri command could be added, but the UI has no clear "select" action — projects are implicit via workspace/session context. |
| FileOpened / FileEdited | No file viewer surface exists in the command-center UI. File operations happen through the agent's developer extension (tracked via TaskLogger). |
| SkillExecuted | skill_executions table exists but no execution code path. Skill execution code path must be built before SkillExecuted events can fire. Phase 3+ dependency. |
| IntegrationTokenRefreshed | OAuth token refresh happens inside the Gmail MCP extension (Python) or via provider-side opaque refresh. No daemon-side hook point for "token was refreshed." The integrations.rs route handles OAuth connect/callback but not token refresh lifecycle. |

## POST /activity/emit Smoke Tests

### Test 1 — Valid emit (200)
```json
{"accepted":true,"event_id":"1e359bbd-5285-458a-a65a-398eb244846e"}
```

### Test 2 — No auth header (401)
```json
{"error":"missing Authorization: Bearer <token> header"}
HTTP: 401
```

### Test 3 — Wrong token (401)
```json
{"error":"invalid token"}
HTTP: 401
```

### Test 4 — Malformed JSON (400)
```
Failed to parse the request body as JSON: expected ident at line 1 column 2
HTTP: 400
```

### Test 5 — Stale timestamp (400)
```json
{"error":"timestamp out of range (age=200075010s, max=60s)"}
HTTP: 400
```

### Test 6 — Wrong tier gets overwritten
Sent `chat_turn_started` with `tier: "always"`. Server returned 200.
Stored event has `tier: "ephemeral"` (canonical tier enforced server-side).

### Test 7 — Rate limiting (200 sequential events)
200 responses: 200, 429 responses: 0.
Sequential curl calls (~10ms each) spread over ~2s, well under the
100/s threshold. Rate limiter correctly allows all events within budget.

### Test 8 — Recent events visible
```json
[
  {
    "event_id": "1009ae6d-...",
    "event_type": "chat_turn_started",
    "source_surface": "chat",
    "timestamp": "2026-05-04T16:24:54Z",
    "payload": {},
    "tier": "ephemeral"
  }
]
```

## Auth Configuration

- Token file: `~/.permagent/secrets/daemon_token.json`
- Permissions: `-rw-------` (0600)
- Generated on first daemon startup if absent
- Shared between daemon and Tauri shell (both read same file)

## Canonicalization Helper Tests (13/13 passed)

```
test identity::canonical::tests::basic_person ... ok              → person:jesse-sharratt
test identity::canonical::tests::project_with_special_chars ... ok → project:get-ladle
test identity::canonical::tests::did_scheme ... ok                 → did:chitin:henry-malcolm
test identity::canonical::tests::empty_input ... ok                → EmptyAfterNormalization
test identity::canonical::tests::underscores_to_hyphens ... ok     → project:my-cool-project
test identity::canonical::tests::mixed_case ... ok                 → org:atlas-atlantic
test identity::canonical::tests::already_canonical_is_idempotent   → person:jesse-sharratt
test identity::canonical::tests::unicode_stripped_in_v1 ... ok     → person:rene-descartes
test identity::canonical::tests::repeated_hyphens_collapsed ... ok → project:my-project
test identity::canonical::tests::leading_trailing_hyphens ... ok   → project:leading-trailing
test identity::canonical::tests::all_special_chars_empty ... ok    → EmptyAfterNormalization
test identity::canonical::tests::agent_prefix ... ok               → agent:henry
```

## Build Status

- `cargo build --release`: PASS (no new warnings)
- `npm run build` (command-center): PASS (3.62s)
- `cargo test --package permagent --lib events`: 12 passed
- `cargo test --package permagent --lib identity`: 13 passed
