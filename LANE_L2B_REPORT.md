# Lane L2b report: chat can live-query terminal harness runs

Branch `lane/L2b`, based on `4dd923fc`. Edit-only: no cargo was run. `rustfmt --edition 2021 --check` passes on every touched `.rs` file.

## Commits

| # | sha | What |
|---|-----|------|
| 1 | `79fac0ed` | `crates/goose/src/harness_runs.rs` (bounded read-only SQL) + `lib.rs` registration. Includes its tests. |
| 2 | `d7c15149` | `harness_watch` platform extension, `pe/mod.rs` registration, self-knowledge inventory case, prompt snapshots. Includes its tests. |
| 3 | — | Optional Live Status line: **skipped**, see below. |
| 4 | — | The tests landed with the code they cover, in commits 1 and 2. There is no separate commit. |

## Changes (file:line)

### `crates/goose/src/harness_runs.rs` (new)

**Constants and liveness**
- `:43` `LEASE_LIVE_WINDOW_SECS = 45`, with a TODO to switch to L2a's constant.
- Limits: `MAX_RUNS=20`, `MAX_GOALS_PER_RUN=50`, `MAX_JOURNAL_ROWS=20`, `MAX_ATTENTION_ITEMS=50`.
- `LOST_RUN_HORIZON_SECS=72h`: how long a stale, non-exited lease stays visible.
- `DEFAULT_RECENT_ENDED=30m`. `SELECTOR_ENDED_WINDOW=24h` applies only when an explicit selector is passed.
- `:111` `classify_lease`: `exited` → Ended. Otherwise the heartbeat must be ≤45 s old to count as Live, else Lost. An unparseable heartbeat reads as Lost.

**Run join** (`:193` `run_cte!`)
- `run_sessions` is a recursive CTE over `sessions.parent_session_id`, starting from the coordinator.
- `run_goals` is the set of goal cards where any of these holds:
  - `json_extract(metadata_json,'$.dispatched_lifecycle') = lease.lifecycle_id`
  - `created_by_session_id = lease.session_id`
  - `worker_session_id IN run_sessions`
- These keys were checked against the real metadata:
  - `dispatched_lifecycle`: orch ~8127
  - `created_by_session_id`: orch ~4534
  - `worker_session_id`: orch ~8524
  - `execution_receipt`
  - `dispatch_evidence.verdict.status`
  - `dispatch_evidence.landing.{outcome,landed_head}`
  - `needs_human_attention` / `_reason`
  - `abandoned_work`
  - `last_error`
  - `attempt_count`

**Reads**
- `:249` `fetch_leases`: SQL window using `julianday`, live and lost first, `LIMIT 20`. A missing `harness_process_leases` table reads as "no runs", so a DB that predates the table doesn't error.
- `:357` `list_runs(pool, now, include_ended_within)` and `:371` `summarize_run`. Each run gets:
  - repo display, e.g. `alpha (alpha-wt)` when `repo_root` ≠ cwd
  - age and heartbeat age
  - title: lease `session_name`, then `sessions.name`/`description`, then the snapshot `promptTitle`
  - goal counts: by state, done, in_progress, parked, `waiting_on_you` (distinct goals that are parked-and-unfinished or have an open decision)
  - open decisions count
  - spend
- Spend (`:461`) is `SUM(cost_ledger.cost_usd)` over `run_sessions` plus the `worker_session_id`s on the run's goals.
- `:701` `run_status(pool, selector, now)` returns `RunSelection::{Resolved, NoRuns, Choose{reason, choices}}`.
  - Selector matching is tiered (`:644`): exact lease/session id, then an id prefix of at least 4 chars, then exact session name, then a case-insensitive fragment of name, cwd or repo_root.
  - If several runs match and exactly one is live, the live one wins.
  - With no selector, the tool resolves only when exactly one run is live. Otherwise it returns the choices.
- `:831` `goal_rows` returns per-goal title, state, attempt, worker key/session id/name, receipt state and heartbeat age, last tool name and age, `last_error` (≤200 chars), park flag and reason, `abandoned_work`, verify verdict, pending gate (`kind: headline`), review state, landed short sha (12 chars) or non-landed outcome, and open-decision count. It fetches `LIMIT 51` so it can report `goals_truncated`.
- `:988` `last_tool_request`: the newest of the worker's last 20 assistant messages. It reads only `json_extract(value,'$.toolCall.value.name')` through `json_each`, never text or arguments.
- `:1010` `journal_rows`: `activity_journal` rows with `ref_kind='goal'` in `run_goals`, or `ref_kind='decision'` for those goals' decisions. `LIMIT 20`. It returns title, kind, actor and ts; `detail` is never read.
- `:1090` `attention`, over live, lost and runs ended in the last 30 m. It reports:
  - a lost run that still has in-progress goals ("terminal exited unexpectedly …")
  - open decisions on run goals; gate kinds are reported as PendingGate
  - parked goals
  - the coordinator's snapshot `pendingGate`
  - open project-level decisions (`goal_id IS NULL`) for the lease's `project_id`, each project reported once
  - capped at 50 items
- Text is always cut by chars (`truncate_chars`), never by bytes.

**Tests**
- `:1293` `test_support` is the seeded fixture shared with the extension tests.
- `:1717` `tests`.

### `crates/goose/src/agents/platform_extensions/harness_watch.rs` (new)
- `:55` `get_tools()` defines `harness_runs` (no args), `harness_run_status {run?: string}` and `harness_attention` (no args).
- `:164`–`:364` are pure renderers. `:421`–`:444` are pool-in handlers returning text plus `structured_content`.
- `:453` `McpClientTrait`: `call_tool` reads `context.session_manager.pool_clone()`, which is this process's own DB. It makes no network or model calls.

### Shared files (minimal)
- `crates/goose/src/lib.rs:58`: `pub mod harness_runs;`
- `crates/goose/src/agents/platform_extensions/mod.rs:28`: `pub mod harness_watch;`
- `crates/goose/src/agents/platform_extensions/mod.rs:823-843`: the registry entry. It is `default_enabled: true` and not hidden. The description names all three tools, and the description and why-it-matters contain no other snake_case tokens (phantom guard).
- `crates/goose/src/agents/self_knowledge/mod.rs:2238,2316-2317`: an `extension_tool_inventories()` case. It is required by the meta-check in `tool_descriptions_name_every_callable_tool`.
- Prompt snapshots were hand-edited because cargo could not run:
  - One line, `- **Harness Watch** [active] — …`, was inserted between Forecaster (`forecast`) and Decision Inbox (`inbox`) in `all_platform_extensions`, `basic`, `one_extension` and `typical_setup`.
  - `family_prompt_size_table.snap`: the unscoped body grows by 1110 bytes (77245 → 78355), and every family total and `div_ceil(4)` token count moves with it. The scoped (CLI) table is unchanged.
  - **If an insta byte mismatch shows up, regenerate with `INSTA_UPDATE=always` on those 5 tests.** Only these snapshots can drift.
- The coding recipe (`crates/goose-cli/src/recipes/builtin/permagent-coding.yaml`) is untouched, and a test asserts it never names `harness_watch`.

## Item 3 skipped: Live Status line
The Live Status block renders from `SelfKnowledgeBuilder` fields. Adding a line means:
- a new builder field, which touches about 20 explicit struct literals in self_knowledge tests (`agent_briefings: None` sites)
- a `PromptManager` builder method
- a query in `reply_parts.rs`

That is broad shared-file churn plus a volatile-prompt change that hand-edited snapshots can't verify, so I skipped it. Recommended follow-up once integrated: add `harness_runs_line: Option<String>` to the builder and fill it in `reply_parts.rs` from `harness_runs::list_runs(..)`. It would render only when a run is live or lost, e.g. `Terminal harness runs: 2 live (alpha, beta), 1 needs you`.

## Gate filters
```
cargo test -p permagent --lib harness_runs
cargo test -p permagent --lib harness_watch
cargo test -p permagent --lib self_knowledge::tests::tool_descriptions_name_every_callable_tool
cargo test -p permagent --lib self_knowledge::tests::descriptor_prose_names_only_real_tools
cargo test -p permagent --lib prompt_manager::tests
cargo test -p permagent --lib platform_extensions::required_secret_tests
cargo test -p permagent --lib identity_name_guard
cargo clippy -p permagent --lib --tests -- -D warnings
```
Test names:
- **harness_runs:**
  - `the_live_window_boundary_is_inclusive_at_45_seconds`
  - `truncation_counts_characters_not_bytes`
  - `repo_display_separates_worktrees_of_one_repo`
  - `human_durations_read_naturally`
  - `list_runs_reports_live_and_lost_runs_with_goals_decisions_and_spend`: two leases (live and stale), goals in in_progress/review/complete/ready-parked plus an unrelated goal, an approve_review decision and a project decision, cost rows for coordinator + 2 workers + an unrelated session; spend is exactly 0.80
  - `ended_runs_appear_only_inside_the_requested_window`
  - `a_database_without_the_lease_table_reads_as_no_runs`
  - `run_status_uses_the_single_live_run_when_no_selector_is_given`
  - `run_status_offers_choices_for_zero_or_many_live_runs`
  - `run_status_resolves_every_selector_shape`
  - `attention_collects_decisions_parks_gates_and_lost_runs`
  - `every_listing_honours_its_limit`: 25 leases → 20; 60 goals → 50 with `goals_truncated`, counts uncapped; 30 journal rows → 20
  - `reads_are_scoped_to_the_pool_they_are_given`
- **harness_watch:**
  - `the_extension_ships_exactly_the_three_named_tools`
  - `the_registry_description_names_every_tool`: default-on, not hidden
  - `the_coding_recipe_does_not_carry_the_extension`
  - `harness_runs_lists_both_runs_without_leaking_message_text`
  - `harness_run_status_resolves_zero_one_and_many_live_runs`
  - `harness_attention_names_the_lost_terminal_and_what_waits`
  - `a_heartbeat_one_second_past_the_window_reads_as_lost`
  - `the_tools_read_only_the_pool_they_are_handed`: the eval-root isolation note
- **Sentinel:** `SENTINEL-7f3a-do-not-leak` is planted in worker message text, tool-call arguments, a user message, decision `detail`, journal `detail`, and snapshot `promptContext`/`promptDigest`. Every output (serialized structs, tool text and `structured_content`) is asserted not to contain it.

## Sample tool output
Hand-derived from the renderers for the seeded fixture. It has not been executed.

`harness_runs`:
```
Terminal harness runs: 1 live, 1 lost, 0 recently ended.
- [live] alpha (alpha-wt) · turn · up 15m · heartbeat 5s ago · goals 1/4 done · 1 in progress · 2 waiting on you · $0.80 · run hpl-live-000 · "Ship the lease reader"
- [LOST] beta · draining · up 15m · heartbeat 5m ago · goals 0/1 done · 1 in progress · $1.00 · run hpl-lost-000 · "Refactor beta"
Call harness_run_status with a run handle for per-goal detail, or harness_attention for what is waiting on the user.
```
`harness_run_status {}`:
```
Run hpl-live-000 · alpha (alpha-wt) · live · phase turn · pid 4242 · headless mode · heartbeat 5s ago
cwd /work/alpha-wt · session sess-coord-live ("Ship the lease reader") · spend $0.80 · 1 open decision
Coordinator last activity: developer__shell
Goals (4; complete 1, in_progress 1, ready 1, review 1):
- [in_progress] Write the lease reader · attempt 2 · worker opus · receipt running, beat 12s ago · last tool developer__shell 30s ago
    last error: éééé…(200 chars)…
- [review] Add the chat tools · attempt 1 · worker worker two · verify pass · awaiting your approval
- [complete] Schema contract · attempt 1 · landed 0123456789ab
- [ready] Decide the retention window
    ! parked for you: budget exhausted
Coordinator telemetry: 14 tool calls · verification pending · pending gate: Write
Recent activity:
- 1m ago · goal_state_changed · Write the lease reader → in_progress (system)
```
`harness_attention`:
```
5 items from terminal harness runs need the user:
- [decision approve_review] Review: Add the chat tools · goal "Add the chat tools" · run hpl-live-000 (alpha (alpha-wt), live)
- [parked goal] parked for you: budget exhausted · goal "Decide the retention window" · run hpl-live-000 (alpha (alpha-wt), live)
- [gate] coordinator is waiting on approval for Write · run hpl-live-000 (alpha (alpha-wt), live)
- [project decision project_intel_proposal] New competitor found · run hpl-live-000 (alpha (alpha-wt), live)
- [lost terminal] terminal exited unexpectedly (phase draining, last heartbeat 5m ago) with 1 goal still in progress · run hpl-lost-000 (beta, LOST)
```
Order follows the code: lost-run first within each run, then decisions, parked goals, gate and project decisions.

## Integration notes for merging with L2a
1. **Liveness window.** Replace the local `harness_runs::LEASE_LIVE_WINDOW_SECS` and `classify_lease` with `crate::harness_lease::LEASE_LIVE_WINDOW_SECS` and L2a's classifier (see the TODO at `harness_runs.rs:43`). Keep the "unparseable heartbeat → lost" behaviour, or note the difference.
2. **Schema.** Tests run `CREATE TABLE IF NOT EXISTS harness_process_leases` using the contract SQL, so once L2a's `spectral_schema.rs` creates the table in `init_spectral_db` this becomes a no-op. The fixture already uses values that satisfy the CHECK constraints the audit proposed:
   - mode `headless`
   - phases `turn`/`draining`/`idle`/`exited`
   - non-null `session_name` is not required; the fixture passes NULL. **If L2a declares `session_name TEXT NOT NULL`, change the fixture's `NULL` in `test_support::insert_lease` to `''`**; the reader already treats an empty name as absent.
   - `a_database_without_the_lease_table_reads_as_no_runs` drops the table itself, so it still holds after the merge.
3. **Heartbeat format.** The lease writer's `heartbeat_at`/`ended_at` must be RFC 3339 (or SQLite `YYYY-MM-DD HH:MM:SS`). The SQL window uses `julianday()`, which returns NULL for anything else, and such a row would be silently excluded.
4. **`last_activity`.** The lease writer's `last_activity` should stay a label (a tool name). It is surfaced, truncated to 120 chars.
5. **`dispatched_lifecycle`.** Must be the CLI process's `daemon_lifecycle_id()`, the same value L2a writes as `lease.lifecycle_id`. Otherwise the run join falls back to `created_by_session_id` and worker parentage only.
6. **Hazard A.** Once L2a fixes hazard A in `goal_status`, the new tools' "never requeues" guarantee is independent of that fix. These tools never call orchestrator code.

## Risks
- **Not compiled.** Most likely friction points:
  - sqlx 0.9 `Query<'q, Sqlite, SqliteArguments>` in `bind_run`: I checked that `SqliteArguments` has no lifetime in 0.9.
  - `concat!(run_cte!(), "...")` macro expansion.
  - type decoding of SQL expression columns (`COALESCE(SUM(..),0.0)` as f64, `COUNT` as i64).
- **Snapshot bytes were hand-computed** (1110-byte line). The first things to regenerate if insta disagrees.
- **Short run handles are 12 chars** (`hpl-` + 8 hex for uuid ids). A prefix collision just returns choices, never a wrong run.
- **Performance.** `run_goals` uses `json_extract` over all non-archived goal cards (live DB ≈110), repeated for 4–5 small queries per run and ≤20 runs. There's also one message-tail query per goal with a worker (≤50). All of it is local and bounded, but `harness_runs` with 20 runs costs about 100 small queries.
- **Spend** counts the coordinator session's whole ledger, not just since this lease started. A resumed interactive session includes its earlier spend.
- **Project-level decisions** in `harness_attention` are any open goal-less decision on the run's project. Some may be unrelated to the run, so they are labelled "project decision" to keep them distinct.
