# Lane L2a report: harness process leases + cross-process hazard fixes

Branch `lane/L2a` (worktree `permagent-worktrees/laneL2a`), based on 4dd923fc. Edit-only: **nothing has been compiled or run**. `rustfmt --edition 2021 --check` passes on every touched file.

## Commits
| sha | item |
|---|---|
| f160c3a1 | 1. `harness_process_leases` schema + liveness rule module |
| 6e624c5d | 2. CLI lease writer + wiring |
| 2a518f11 | 3. Hazard A/B fixes |
| d0cb09b6 | 4. Tests (plus a small testability change: the dispatcher lookup in the deferred recheck is now an injected `fn`) |

## Changes (file:line at d0cb09b6)

### 1. Schema and liveness rule (the contract for L2b)
- `crates/goose/src/session/spectral_schema.rs:4158` has `apply_harness_process_leases_schema`: the exact DDL and both indexes from the brief, all `IF NOT EXISTS`. It is applied in `init_spectral_db` (`:823`) and, independent of the version ladder, for existing DBs (`session_manager.rs:1835`).
- `crates/goose/src/harness_lease.rs` is new and registered in `lib.rs:58`. It contains:
  - `LEASE_LIVE_WINDOW_SECS = 45` (`:29`), plus `LEASE_HEARTBEAT_INTERVAL_SECS = 15`, `LEASE_ACTIVITY_MIN_GAP_SECS = 5` and `LEASE_ACTIVITY_MAX_CHARS = 80`.
  - The `LeasePhase` and `LeaseLiveness {Live, Lost, Ended}` enums, and `Lease` (`FromRow` + `Serialize`, all columns).
  - `classify(lease, now)` (`:147`) and `classify_parts(phase, heartbeat_at, now)`. The rule: exited → Ended; otherwise age ≤ 45 s → Live; stale or unparseable → Lost.
  - `live_lease_for_lifecycle(pool, lifecycle_id)` (`:170`), `live_lease_for_lifecycle_at` (clock injected) and `leases_for_lifecycle`.
  - `lease_timestamp()`, the canonical format: RFC3339 UTC, millisecond precision, `Z`. `heartbeat_age_secs()`.
  - `project_id_for_working_dir()`, using the same active-project root match as `cards::goal_cards_for_project_matching_cwd`.
  - `live_foreign_owner_at(pool, metadata, own_lifecycle, now) -> Option<LiveForeignOwner>` (`:290`) and `goal_owned_by_live_foreign_process(pool, card, own_lifecycle) -> bool` (`:356`). `LiveForeignOwner::describe()` returns lines like `running in terminal "name" at /cwd (pid N), heartbeat Xs ago`.

### 2. CLI writer
- `crates/goose-cli/src/session/harness_lease.rs` is new:
  - `HarnessLease::start_for_session` does nothing for non-harness sessions: no query. For a harness session it:
    - inserts a row in phase `starting` (uuid v7 id, `daemon_lifecycle_id()`, session id/name, cwd, `git rev-parse --show-toplevel` capped at 2 s, project id, mode, recipe title, pid);
    - prunes exited rows older than 7 days;
    - spawns a heartbeat task that runs one `UPDATE` every 15 s for the process lifetime.
  - `set_phase` never leaves `exited`. `note_tool_activity(name)` stores the tool name only, char-truncated, at most once per 5 s, as a detached write. `finish(status)` is idempotent: it aborts the heartbeat and writes `exited`, `ended_at` and `exit_status`.
  - `LeaseTiming` injects the interval, the activity gap and the clock.
- Wiring:
  - `session/mod.rs:298` adds a `harness_lease` field. `:451` is `start_harness_lease` and `:471` is `finish_harness_lease`.
  - Phase changes: `turn` at `:704` (entry of `process_with_harness_heartbeat`), `idle` at `:748` (after the reply), `draining` at `:1733` (in `drain_dispatched_goals`, after its scope check).
  - `exited "completed"` at `:852` (end of `interactive()`). Tool names are recorded at `:2034` (reply stream, `ToolRequest`).
  - `builder.rs:1562` starts the lease right after `CliSession::new`.
  - `cli.rs:1709` writes the interactive run's exit (`completed`/`failed`, which covers the error path). The headless run's exit is written after the drain and receipts, next to `log_session_completion`.
- The pool comes from the session manager (`Paths::spectral_db`), so `PERMAGENT_PATH_ROOT` keeps eval runs in their own DB. No daemon and no token are involved.

### 3. Hazard fixes (`orch` = `crates/goose/src/agents/platform_extensions/orchestrator.rs`)
- Foreign-owner rule (`harness_lease.rs:290`):
  - A goal is never foreign when it was dispatched by our own lifecycle.
  - If lease rows exist for the dispatching lifecycle, **the lease decides**: live → owned; lost/ended → not owned, even if the receipt is fresh. This lets a killed terminal's goals recover as soon as its lease is stale.
  - With no lease rows (the daemon, or an older CLI), a non-terminal receipt heartbeated within `DEFAULT_STALENESS_SECS` (90 s) counts as owned, whatever its lifecycle.
- **Hazard A:** `orch:10739` computes `foreign_owner` for `in_progress` goals, and `orch:10776` gates R3 on `foreign_owner.is_none()`. The status JSON reports `session_alive: true` and a new `owner` string. There's no requeue, no dispatch and no card write.
- **Hazard B:** `resume_single_goal` (`orch:26800`) returns the new `ResumeDisposition::OwnedByLiveForeignProcess { recheck }` (`orch:26079`) and doesn't touch the card.
  - `ReconcileReport.owned_by_live_foreign_process` has its own detail line and isn't counted in `reconciled()`, so no briefing is filed for it.
  - A lost lease falls through to today's recovery.
- **Receipt-only deferral** (`orch:26534`, `orch:26698` `recheck_foreign_owned_goal`): this was needed so the change doesn't break restart recovery.
  - The problem: the old daemon's receipts are still fresh (under 90 s) when launchd restarts it, and the boot pass runs only once. Skipping those goals would strand them.
  - The fix: the boot pass spawns a recheck that polls every 30 s. It stops when the goal leaves In Progress or this process redispatches it. Once no live owner remains, it runs `resume_single_goal` and, if that requeued the goal, the same bounded Ready nudge the boot pass uses.
  - Rechecks are scheduled only for receipt-only ownership. A live-lease goal gets no recheck (see Risks).

## Gate filters
```
cargo test -p permagent --lib harness_lease
cargo test -p permagent --lib goal_status_leaves_a_goal_owned_by_a_live_terminal_lease_alone
cargo test -p permagent --lib goal_status_still_recovers_a_goal_whose_terminal_lease_is_lost
cargo test -p permagent --lib resume_          # includes the 3 new resume_* tests and existing reconcile tests
cargo test -p permagent --lib deferred_recheck
cargo test -p permagent --lib the_boot_re      # new report test + existing the_boot_reconciler_reports_what_it_did
cargo test -p permagent --lib goal_status_     # existing R3 / Y4b regressions
cargo test -p permagent --lib harness_snapshot # neighbouring schema tests
cargo test -p permagent --lib identity_name_guard
cargo test -p permagent-cli --lib harness_lease
cargo test -p permagent-cli --lib session::tests   # existing CliSession source-shape tests
cargo clippy -p permagent -p permagent-cli --lib -- -D warnings
```
New tests:
- `permagent` lib:
  - `harness_lease.rs:412` schema idempotent on a reopened DB
  - `:446` table present on a fresh DB
  - `:466` 45 s boundary
  - `:502` `live_lease_for_lifecycle`
  - `:573` foreign-owner rule
  - `orch:50496` / `:50570` hazard A, live and lost
  - `orch:42416` / `:42457` hazard B, live and lost
  - `orch:42525` receipt-only deferral
  - `orch:42550` / `:42586` recheck
  - `orch:42609` report
- `permagent-cli` lib, `session/harness_lease.rs`:
  - `:533` non-harness session writes no lease
  - `:551` phase transitions and exit rows in a temp-root DB
  - `:611` interactive mode
  - `:630` heartbeat task on an injected 10 ms interval and stepping clock, stops at exit
  - `:683` activity throttle and truncation

None of the new tests sets a process-global `OnceLock` or hook: the CLI tests pass the lifecycle id explicitly, and the recheck test injects "no dispatcher".

## Manual end-to-end check
1. **Two live terminals.** In two worktrees of one repo, run `permagent run --recipe permagent-coding -t "<multi-goal task>"`. Then run `sqlite3 ~/.permagent/spectral/permagent.db "select lease_id,lifecycle_id,working_dir,pid,phase,heartbeat_at from harness_process_leases"`.
   - Expected: two rows with phases moving starting→turn→idle→draining and `heartbeat_at` advancing every ~15 s, including during the drain.
2. **Terminal 2 starting mid-run (hazard B).** Start terminal 2 after terminal 1 has goals In Progress.
   - Expected: terminal 1's goals stay In Progress.
   - The daemon/terminal-2 log shows `is running in terminal … — not reclaiming` and the boot line `N left untouched: owned by a live process elsewhere`.
3. **Chat status read (hazard A).** In app chat, call `goal_status` on one of terminal 1's goals once it's more than 180 s past dispatch.
   - Expected: `session_alive: true`, an `owner` line naming terminal 1's cwd and pid, no `retry_count`, and no second worker session.
4. **Daemon restart mid-run.** Run `permagent daemon restart`.
   - Expected: no goal from either terminal moves to Review/Ready.
5. **kill -9 terminal 1.**
   - Expected: its lease stays non-exited, and after ~45 s `classify` reads it as lost.
   - A later `goal_status` on its goal takes the old R3 path (requeue/redispatch). A later boot reclaims it.
6. **Normal exit.** Exit terminal 2 normally.
   - Expected: `phase='exited'` with `ended_at` and `exit_status` set.
7. **Eval isolation.** Run an eval with `PERMAGENT_PATH_ROOT` set.
   - Expected: its lease lands in `<root>/spectral/permagent.db`, not in `~/.permagent`.

## Risks / notes
- **Not compiled.** Likely friction points:
  - sqlx `Encode` for `&Option<String>` binds in the CLI writer
  - the `fn() -> Option<GoalDispatchHookHandle>` coercion from `goal_dispatch_hook_snapshot` / `|| None`
  - the `String == Cow<str>` assert in the CLI phase test; on macOS a tempdir `/var` vs `/private/var` path mismatch is possible if `create_session` canonicalizes
  - in-memory `test_pool` use in the new resume tests, which mirrors the existing ones
- **Receipt-only deferral is new background work.**
  - It runs in whatever process runs the boot pass, including a CLI. If the daemon crashes while a terminal started after it is still running, that terminal's recheck may requeue and dispatch the daemon's goals, as today's boot pass already did (only sooner and unconditionally).
  - The loop is bounded by the goal's lifetime.
- **A live-lease goal gets no recheck.** If terminal 1 is killed after another process has booted, nothing reclaims its goals automatically until a `goal_status` call or the next boot. That matches the audit's "flag it, don't requeue it" expectation; L2b's tools should show the goal as lost.
- **The lease is authoritative when present:** a lost lease overrides a fresh receipt. If a live process's heartbeat task stalled for more than 45 s (for example, a starved runtime), its goals would be treated as recoverable. The runtime is multi-threaded and the write is a single statement, so this is unlikely.
- **Other reclaimers aren't gated.** Other daemon paths that act on In Progress goals (wallclock/budget timeouts, maintenance ticks) weren't audited in this lane.
- **Error exits.** Any `?` early return before `finish` leaves the lease to read as lost, not exited. `process::exit` in the builder happens before the lease starts.
- **Merge conflict risk.** Lane L2b also adds a line to `crates/goose/src/lib.rs` (a trivial conflict). L2b should import `permagent::harness_lease::{Lease, classify, LeaseLiveness, LEASE_LIVE_WINDOW_SECS, lease_timestamp}` rather than re-deriving the rule.
