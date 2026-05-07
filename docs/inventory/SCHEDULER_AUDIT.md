# Scheduler Subsystem Audit

## Summary
- **Total HTTP endpoints:** 10
- **Job model fields:** 9
- **Worker personas defined:** Dynamic (loaded from `~/.permagent/agent.yaml`)
- **Active in current daemon:** Yes — routes registered, endpoint responds `{"jobs":[]}`
- **Persistence file:** `~/.permagent/data/schedule.json` (created on first job)
- **Recipe storage:** `~/.permagent/data/scheduled_recipes/`
- **Cron library:** tokio-cron-scheduler (async)
- **Tests:** 3 integration tests
- **Audit date:** 2026-05-07

---

## HTTP API

All endpoints are registered under the `/schedule` prefix. No auth required (same as other daemon routes — runs on localhost only).

### POST /schedule/create
Create a new scheduled job.

**Request:**
```json
{
  "id": "daily-standup",
  "recipe": { "title": "...", "prompt": "...", ... },
  "cron": "0 9 * * 1-5",
  "worker_persona": "analyst"   // optional
}
```

**Response (200):** `ScheduledJob` object

**Errors:**
- 400: Invalid cron expression or malformed request
- 409: Job ID already exists (`SchedulerError::JobIdExists`)
- 500: Internal error

**Notes:** The `recipe` field is the full `Recipe` struct (not a path). The daemon copies the recipe to `~/.permagent/data/scheduled_recipes/{id}.yaml` for persistence. Job ID validated against `[a-zA-Z0-9\-_ ]+`.

### GET /schedule/list
List all scheduled jobs.

**Request:** None

**Response (200):**
```json
{
  "jobs": [
    {
      "id": "daily-standup",
      "source": "~/.permagent/data/scheduled_recipes/daily-standup.yaml",
      "cron": "0 9 * * 1-5",
      "last_run": "2026-05-07T13:00:00Z",
      "currently_running": false,
      "paused": false,
      "current_session_id": null,
      "process_start_time": null,
      "worker_persona": null
    }
  ]
}
```

**Notes:** Internally calls `sync_from_storage()` first to reconcile in-memory state with disk. This handles external file edits gracefully.

### DELETE /schedule/delete/{id}
Remove a scheduled job and its stored recipe file.

**Response:** 204 No Content

**Errors:**
- 404: Job not found

### PUT /schedule/{id}
Update a job's cron expression.

**Request:**
```json
{ "cron": "0 */2 * * *" }
```

**Response (200):** Updated `ScheduledJob`

**Errors:**
- 400: Invalid cron, or job is currently running
- 404: Job not found

**Notes:** Cannot update cron while job is running. No endpoint to update the recipe itself — delete and recreate.

### POST /schedule/{id}/run_now
Trigger immediate execution outside the cron schedule.

**Response (200):**
```json
{ "session_id": "session-uuid-here" }
```

**Errors:**
- 404: Job not found

**Notes:** Creates a new `SessionType::Scheduled` session. Returns session ID immediately; execution runs async in background.

### POST /schedule/{id}/pause
Pause a scheduled job (cron triggers skip execution).

**Response:** 204 No Content

**Errors:**
- 400: Job is currently running (cannot pause mid-execution)
- 404: Job not found

### POST /schedule/{id}/unpause
Resume a paused job.

**Response:** 204 No Content

**Errors:**
- 404: Job not found

### POST /schedule/{id}/kill
Cancel a currently-running job execution.

**Response (200):**
```json
{ "message": "Job killed successfully" }
```

**Errors:**
- 400: Job is not currently running
- 404: Job not found

**Notes:** Uses `CancellationToken.cancel()` which propagates to the agent reply stream. Graceful — in-flight tool calls complete but no new tools are dispatched.

### GET /schedule/{id}/inspect
Get execution info for a running job.

**Response (200):**
```json
{
  "session_id": "session-uuid",
  "process_start_time": "2026-05-07T14:30:00Z",
  "running_duration_seconds": 45
}
```

All fields null if job is not running.

### GET /schedule/{id}/sessions?limit=N
List sessions created by a scheduled job.

**Request query:** `limit` (default varies)

**Response (200):**
```json
[
  {
    "id": "session-uuid",
    "name": "Scheduled job: daily-standup",
    "created_at": "2026-05-07T09:00:00Z",
    "working_dir": "/Users/jesse",
    "schedule_id": "daily-standup",
    "message_count": 12,
    "total_tokens": 4500,
    "input_tokens": 3000,
    "output_tokens": 1500,
    "accumulated_total_tokens": 4500,
    "accumulated_input_tokens": 3000,
    "accumulated_output_tokens": 1500
  }
]
```

---

## Job Model

```rust
pub struct ScheduledJob {
    pub id: String,                              // Unique ID (alphanumeric, -, _, spaces)
    pub source: String,                          // Path to recipe file
    pub cron: String,                            // Cron expression
    pub last_run: Option<DateTime<Utc>>,         // Last execution timestamp
    pub currently_running: bool,                 // Active execution flag
    pub paused: bool,                            // Paused flag
    pub current_session_id: Option<String>,      // Session ID of running execution
    pub process_start_time: Option<DateTime<Utc>>, // Start time of current run
    pub worker_persona: Option<String>,          // Worker key from agent.yaml
}
```

**Required fields on create:** `id`, `recipe` (full Recipe object), `cron`
**Optional fields on create:** `worker_persona`
**Computed fields:** `source` (set by daemon), `last_run`, `currently_running`, `current_session_id`, `process_start_time`

### State Transitions

```
[Created] → paused: false, currently_running: false
    │
    ├─ Cron fires ──────────────────────────→ [Running]
    │   (sets currently_running=true,           │
    │    current_session_id, process_start_time, │
    │    last_run = now)                         │
    │                                            │
    ├─ run_now() ───────────────────────────→ [Running]
    │                                            │
    ├─ pause() ─────────────────────────────→ [Paused]
    │                                            │
    │   [Paused] ──── unpause() ───────────→ [Created]
    │   [Paused] ──── cron fires ──────────→ (skipped, stays paused)
    │                                            │
    │                                    [Running]
    │                                        │
    │                         ┌──────────────┼──────────────┐
    │                         ▼              ▼              ▼
    │                    [Completed]     [Failed]       [Killed]
    │                    (clears running  (clears running  (token.cancel(),
    │                     flags, persists  flags, logs     clears running
    │                     last_run)        error)          flags)
    │                         │              │              │
    └─────────────────────────┴──────────────┴──────────────┘
                              ▼
                         [Created] (back to idle, awaits next cron trigger)
```

### Cron Expression Format

| Format | Example | Description |
|--------|---------|-------------|
| 5-field | `0 9 * * 1-5` | min hour dom month dow |
| 6-field | `0 0 9 * * 1-5` | sec min hour dom month dow |
| Shorthand | `@daily` | Predefined (CLI only) |
| Shorthand | `@hourly` | Predefined (CLI only) |
| Shorthand | `@weekly` | Predefined (CLI only) |

5-field expressions are auto-converted to 6-field by prepending `0` for seconds. Timezone is system local (not configurable per-job).

---

## Worker Persona System

### How It Works
Each scheduled job can optionally reference a `worker_persona` by key. When the job fires, the scheduler looks up the key in the agent config's `workers` map and applies that persona's system prompt to the agent for that execution.

### Configuration
Workers are defined in `~/.permagent/agent.yaml`:
```yaml
workers:
  analyst:
    first_name: "Analyst"
    role: "Data analyst"
    traits: ["analytical", "precise"]
    tone: "professional"
  writer:
    first_name: "Writer"
    role: "Content writer"
    traits: ["creative", "concise"]
    tone: "conversational"
```

### Runtime Behavior
- Workers loaded at daemon startup via `set_agent_config()`
- Stored behind `Arc<RwLock<>>` — hot-reloadable without restart
- If referenced worker key not found, falls back to primary persona with warning log
- Worker applies only to the scheduled execution — doesn't affect interactive sessions

### What a Persona Controls
- System prompt block (injected via `set_persona_block_override()`)
- Agent first name, role, traits, tone
- Does NOT control: model, provider, tools, extensions (those come from the recipe)

---

## Persistence Layer

### Storage Locations
| Data | Path | Format |
|------|------|--------|
| Job definitions | `~/.permagent/data/schedule.json` | JSON array of `ScheduledJob` |
| Recipe copies | `~/.permagent/data/scheduled_recipes/{id}.yaml` | YAML recipe |
| Execution sessions | SQLite via `SessionManager` | `SessionType::Scheduled` |

### schedule.json Format
```json
[
  {
    "id": "daily-standup",
    "source": "/Users/jesse/.permagent/data/scheduled_recipes/daily-standup.yaml",
    "cron": "0 9 * * 1-5",
    "last_run": "2026-05-07T13:00:00.123Z",
    "currently_running": false,
    "paused": false,
    "current_session_id": null,
    "process_start_time": null,
    "worker_persona": null
  }
]
```

### Persistence Points
State is written to disk at these moments:
1. After job added
2. On cron trigger (before execution starts)
3. After job completes (success or failure)
4. On pause/unpause
5. On cron update

### Retention
- Jobs persist until explicitly deleted
- No automatic purge or TTL
- Sessions persist independently in SQLite — deleting a schedule does not delete its sessions
- Recipe files deleted only when `remove_scheduled_job(..., remove_recipe: true)` is called

### Recovery on Restart
- All jobs reloaded from `schedule.json` on daemon start
- Cron timers re-registered with tokio-cron-scheduler
- `currently_running` flag may be stale after crash — next cron trigger will reset it
- No in-flight execution state preserved (the session continues to exist but the agent stream is lost)

---

## Cancellation Behavior

### Mechanism
Each running job has a `CancellationToken` stored in `running_tasks: HashMap<String, CancellationToken>`. Calling `kill_running_job()`:
1. Retrieves the token from `running_tasks`
2. Calls `token.cancel()`
3. The token propagates to `agent.reply()` which checks it during streaming
4. The reply stream stops producing messages
5. Cleanup in `execute_job()` handler clears running flags and persists state

### Graceful vs Hard
- **Graceful:** In-flight tool calls complete (the agent finishes its current tool execution), but no new tools are dispatched
- **No orphan cleanup:** If a tool was writing to a file, that write completes — partial writes are possible for multi-step tools
- **Session preserved:** The session with partial conversation is saved and visible in session history

### Edge Cases
- Killing a non-running job returns 400 error
- Pausing a running job returns 400 error (must kill first, then pause)
- If the CancellationToken is already dropped (race condition), kill is a no-op with success response

---

## Trigger Types

### Cron-Based (Primary)
Standard cron scheduling via tokio-cron-scheduler. Jobs fire at the scheduled time and execute asynchronously.

### Manual (run_now)
`POST /schedule/{id}/run_now` triggers immediate execution. Creates a new session independent of the cron schedule. The cron schedule continues independently.

### Event-Based
**Not implemented.** The scheduler has no event-trigger mechanism. It cannot react to activity events, webhooks, file changes, or other signals. All triggers are time-based (cron) or manual (run_now).

### Dependency-Based
**Not implemented.** No job dependency system. Jobs are independent — job B cannot be configured to run after job A completes.

### Concurrency
**No concurrency guard.** If a cron trigger fires while the previous execution is still running, the scheduler checks `currently_running` and skips the trigger (logs a warning). This prevents overlapping executions of the same job.

---

## Output Handling

### Where Output Goes
1. **Session:** Each execution creates a `SessionType::Scheduled` session with full conversation history. Session name: `"Scheduled job: {job_id}"`. Viewable via `GET /api/sessions` or `GET /schedule/{id}/sessions`.

2. **Brain (Spectral):** Each completed turn is remembered to Brain with:
   - Key: `"scheduled-{job_id}-{turn_idx}"`
   - Source: `"scheduled"`
   - Visibility: `Private`
   - Confidence: `1.0`

3. **Telemetry:** PostHog events emitted (if feature enabled): `schedule_job_started`, `schedule_job_completed`, `scheduler_job_failed`

### Notification
**No notification mechanism.** There is no webhook, email, push notification, or event emission when a job completes or fails. The only way to check is:
- Poll `GET /schedule/{id}/inspect` for running status
- Check `GET /schedule/{id}/sessions` for completed sessions
- Check `last_run` in `GET /schedule/list`

### Error Reporting
- Errors logged to daemon stderr
- No structured error storage per-job
- Failed executions still create sessions (with whatever messages were exchanged before failure)
- `last_run` is updated even on failure (it tracks when the job was triggered, not whether it succeeded)

---

## Existing Frontend

### goose2 Tauri Shell
**None.** No scheduler-related Tauri commands in `ui/goose2/src-tauri/src/commands/`. No schedule UI components in `ui/goose2/src/`.

### Command Center (React)
**Minimal.** The `Session` interface in `ui/command-center/src/lib/api.ts` includes `schedule_id?: string | null` (line 84), indicating sessions know about their parent schedule. But no schedule management UI components exist.

### CLI
**Full CLI surface** in `crates/goose-cli/src/commands/schedule.rs`:
- `permagent schedule add --schedule-id ID --cron EXPR --recipe-source PATH`
- `permagent schedule list`
- `permagent schedule remove --schedule-id ID`
- `permagent schedule sessions --schedule-id ID [--limit N]`
- `permagent schedule run-now --schedule-id ID`
- `permagent schedule cron-help`

---

## Gaps and Open Questions

### 1. No job-level model/provider override
The recipe defines extensions, but the model/provider comes from the daemon's current default. If you want different jobs to use different models, you'd need to change the daemon default between jobs or add model override to `ScheduledJob`.

### 2. No recipe update endpoint
To change a job's recipe, you must delete and recreate the job. A `PUT /schedule/{id}/recipe` endpoint doesn't exist.

### 3. No completion notifications
When a job finishes (success or failure), nothing is emitted to the event bus. The Automate tab would need to poll or the scheduler would need to emit `PermagentEvent::ScheduledJobCompleted`.

### 4. No run history beyond sessions
There's no structured "run history" (start time, end time, status, error message) per job. You can reconstruct this from sessions, but it requires joining session data with schedule data.

### 5. Stale `currently_running` after crash
If the daemon crashes during job execution, `currently_running` stays true in `schedule.json`. The next daemon startup loads this stale state. The cron trigger will skip because it sees the job as running. Recovery requires manual edit of `schedule.json` or adding a startup cleanup.

### 6. No max_runs or expiry
Jobs run indefinitely. There's no way to say "run this 5 times then stop" or "expire after 2026-06-01". The CLI has a `max_runs` concept but it doesn't appear to be wired through.

### 7. Timezone is system-local only
Cron expressions use the system timezone. Users can't specify UTC or a different timezone per job.

### 8. Recipe must be self-contained
The recipe is copied to `scheduled_recipes/` at creation time. If the original recipe references external files (sub-recipes, skill files), those references may break after the copy.

---

## Recommended Permagent Integration Points

### Automate Tab — Primary Surface
The Automate tab in the Command Center sidebar should be the home for scheduler UX. Current state: the Automate workspace exists in the sidebar but has no dedicated content.

**Minimum viable surface:**
1. **Job list view** — call `GET /schedule/list`, show each job with status indicator (running/paused/idle), cron description in human-readable form, last run time
2. **Create job form** — job ID, cron expression (with preset options like "Every morning at 9am"), recipe text/selection, optional worker persona
3. **Job actions** — pause/unpause, run now, kill, delete
4. **Job detail/history** — `GET /schedule/{id}/sessions` showing past executions with token usage and message count

### Chat Integration
When the agent mentions scheduling or the user asks to schedule something, the agent already has a `schedule` tool (via `schedule_tool.rs`). This tool can create, list, and manage jobs. The Automate tab would provide visual management of the same data.

### Activity Awareness
Scheduled job executions should emit activity events so they appear in the inspection panel and contribute to the agent's ambient context. Currently they don't — this requires adding `emit_activity()` calls in `execute_job()`.

### Brain Integration
Already implemented. Scheduled job outputs are remembered to Brain with source `"scheduled"`. The agent can recall past scheduled execution results during interactive sessions.

### Event Bus Integration (Gap to Fill)
The scheduler should emit `PermagentEvent` on job start/complete/fail so:
- The Automate tab can update in real-time via WebSocket
- The activity awareness layer can track scheduled work
- Notifications can be built on top

### Session View
Scheduled sessions already appear in the session list. Adding a filter for `schedule_id` would let the Automate tab show "all sessions from this job" without a separate endpoint.
