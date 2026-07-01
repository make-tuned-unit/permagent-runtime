# Daemon Durability Audit — the "weeks-untouched" bar

**Status:** Diagnosis only (ZERO code this round). Ranked findings + instrumentation
design + fix-priority list + decision points.
**Date:** 2026-06-30
**Scope:** The `permagentd` daemon (`permagent-daemon` crate, dir `crates/goose-server`;
core lib `permagent`, dir `crates/goose`). Disjoint from S1/#558 (webview), #554 (CRM),
#552 (steward).
**Method:** Static code read across 6 durability surfaces + **live on-disk/process
evidence from the running system** (the truth-teller). Where an agent's claim was wrong,
it was corrected against the code and the live filesystem — see *Corrections* below.

---

## The bar

> `permagentd` runs UNTOUCHED on a Mac for **weeks** (ideally indefinitely) without
> crashing, wedging, leaking, or drifting. The user then opens a **remote connection**
> (future: Tailscale + Bearer token), talks to the daemon, and it **just works** —
> because nothing died, wedged, leaked, or drifted in the interim.

The UI is **not** in this path. The daemon is the thing answering the phone. So daemon
durability is load-bearing; UI durability (S1–S4) is secondary to this scenario.

Everything below is ranked by **"what breaks the weeks-untouched bar soonest"** — a fault
that silently stops the daemon doing its job, or fills a chronically-near-full disk, ranks
above a slow drift.

---

## Live evidence snapshot (ground truth, captured 2026-06-30 ~20:50)

This is the strongest signal in the whole audit — real accumulation on the real machine.

| Artifact | Live size / state | Growth type | Rotation? |
|---|---|---|---|
| `~/.permagent/logs/daemon.err` | **20 MB** (launchd StandardErrorPath) | automatic, every log line | **NONE** |
| `~/.permagent/brain/memory.db` | **143 MB** | automatic (Brain writes) | n/a (data) |
| `~/.permagent/brain/memory.db-wal` | **30 MB** | automatic; large WAL ⇒ checkpoint lagging | autocheckpoint at default only |
| `~/.permagent/brain/memory.db.pre-*` backups | **~700 MB** across 8 files (May–Jun) | **manual** one-offs | never pruned |
| `~/.permagent/brain/graph.kz.bak.*` | 1 file (Jun 9) | **on ontology-change** (rare) | never pruned |
| `~/.permagent/spectral/permagent.db(-wal)` | 9 MB + 354 KB WAL | automatic | — |
| `~/.permagent/logs/llm_request.<uuid>.jsonl` | ~26 orphan files (Jun 6) | on crash mid-request | ring 0–9 only; UUIDs never cleaned |
| `~/.permagent/crashes/` | 1 crash log (2026-06-23) | on panic | capped at 20 |
| `~/.permagent/schedule.json` | 3 jobs; `storage-insights` `currently_running:true` | — | — |

Two facts jump out: **daemon.err has no rotation at all**, and the **Brain WAL is 30 MB**
(≈7× SQLite's default 4 MB autocheckpoint threshold) — checkpointing is not keeping up.

---

## The launchd contract (recovery substrate)

`~/Library/LaunchAgents/ai.permagent.daemon.plist`:

```
KeepAlive = { SuccessfulExit = false }   # restart on any NON-zero exit, NOT on clean exit 0
RunAtLoad = true
ProgramArguments = [".../permagentd", "agent"]
StandardErrorPath = ~/.permagent/logs/daemon.err   # append-only, never rotated
StandardOutPath  = ~/.permagent/logs/daemon.log
# No ThrottleInterval / RestartDelay override → launchd's default ~10s throttle applies.
```

Two consequences that shape every finding below:

1. **launchd only restarts on process EXIT.** A daemon that is *alive but broken* (a
   background task died, but the process still runs) is invisible to launchd — it will
   **never** be restarted. This is the central durability weakness (Finding 1).
2. **`SuccessfulExit = false` means a clean `exit(0)` is terminal** — launchd will not
   relaunch. Verified: no unintended `exit(0)` path exists today (only `exit(1)` on
   error), so this is latent, not active. But any future "graceful shutdown on condition X"
   would silently take the daemon down until manual relaunch.

Confirmed: the workspace `Cargo.toml` has **no `panic = "abort"`** → default `unwind`. A
panic in a spawned Tokio task unwinds *that task only*; the process survives. The global
panic hook (`crash_capture.rs:183`, installed `main.rs:68`) logs to `~/.permagent/crashes/`
on **any** panic — so **a crash log does NOT prove the process restarted.** It usually
proves the opposite: a task died and the process limped on.

---

## TIER 1 — breaks the bar first (weeks or a single crash; no self-heal)

### F1. Half-dead daemon: a panicking background task is silent and permanent
**Severity: CRITICAL.** `panic = unwind` (Cargo.toml, no override) + launchd restarts only
on process exit ⇒ when any long-lived spawned task panics, the task dies, the hook logs a
crash file, and **the process keeps running and answering HTTP** — so launchd never
restarts it and the user sees a healthy-looking daemon that has silently stopped doing part
of its job. The tasks at risk are exactly the load-bearing ones:
- goal recovery / orphan sweep — `orchestrator.rs:370-391` (spawned, not awaited)
- activity ingestion — `state.rs:427-463` → `spawn_blocking` at `state.rs:446`
- startup migration/backfill tasks — `state.rs:235-308`

These have `match`-based error handling today, so a *panic* (vs `Err`) needs an
`.unwrap()`/`.expect()`/slice-index/arithmetic overflow on an unexpected input — which is
precisely what accumulates over weeks of varied real data. **No detection, no restart.**
*Needs runtime confirmation:* which task died in the 2026-06-23 crash log (read it), and
whether the process restarted then.

### F2. Scheduler wedge: one crash freezes a job forever (`currently_running` never reset)
**Severity: CRITICAL.** `currently_running` is set `true` before a job runs
(`scheduler.rs:289-295`) and cleared only on completion (`scheduler.rs:330-336`). On
startup, `load_jobs_from_storage` (`scheduler.rs:497-565`) re-registers jobs **without
resetting a stale `currently_running`**, and the execution guard checks only `!paused`
(`scheduler.rs:273-279`) — there is **no startup reconciliation**. So if the daemon dies
(or F1 half-dies) while Librarian/Steward/storage-insights is mid-run, that job's flag is
stuck and — combined with no staleness check on `process_start_time` (`scheduler.rs:150`,
set but never inspected) — the daemon's **autonomous work silently stops** until the user
hand-edits `schedule.json`. One crash is enough. (Live `schedule.json` currently shows
`storage-insights: currently_running:true` — benign now because it genuinely just started,
but it is the exact state that becomes permanent after a crash.)
*Correction:* `docs/inventory/SCHEDULER_AUDIT.md:319` claims "next cron trigger will reset
it" — **false**; no such reset exists in the code.

### F3. Job panic may kill the whole scheduler loop (no isolation boundary)
**Severity: HIGH.** `execute_job()` is awaited inline in the cron closure
(`scheduler.rs:314-323`) with no `catch_unwind`/`AssertUnwindSafe`. Whether a job panic
takes down *only that job* or *the entire `tokio-cron-scheduler` loop* (killing all future
firings of all jobs) depends on undocumented `tokio-cron-scheduler 0.14` internals.
*Needs runtime confirmation:* schedule a deliberately-panicking job and observe whether
other jobs keep firing.

### F4. SQLite WAL growth / checkpoint wedge on a near-full disk
**Severity: HIGH (with fast tail-risk).** Live Brain WAL is **30 MB** and there is **no
explicit `wal_autocheckpoint`, `synchronous`, or manual `wal_checkpoint(TRUNCATE)`** for
`memory.db` anywhere (`spectral_schema.rs:58` sets `journal_mode=WAL` and stops). Worse,
`read_only_brain_conn()` (`brain_ops.rs:229-235`) opens read-only connections with **no
`busy_timeout`**, used from many routes. Two failure modes over weeks:
- A **long-lived / leaked read snapshot pins the WAL** so checkpoints can't truncate it →
  WAL grows unbounded → on a chronically-near-full disk (16 GB typical) this can fill the
  disk in **days** and wedge both the daemon and the machine.
- Under write contention a read-only conn **immediately `SQLITE_BUSY`-fails** (no retry),
  degrading request-handling — the remote-path responsiveness the bar depends on.
The Spectral pool (`session_manager.rs:614-638`) is correctly configured (max 20,
`busy_timeout 30s`, WAL) — the gap is entirely on the **Brain** side.
*Needs runtime confirmation:* watch `memory.db-wal` size over 24–48 h; if it never shrinks
during idle, a reader is pinning it.

---

## TIER 2 — degrades over weeks (slower, or lower probability)

### F5. `daemon.err` unbounded + no rotation
**Severity: MEDIUM (chronic) / HIGH (tail).** Console layer writes to `std::io::stderr`
(`logging.rs:50`) at **INFO** level for `goose`/`goose_server`/`permagentd`
(`logging.rs:39-41`); launchd appends to `daemon.err` and **never rotates**. At baseline
(~MB/day) this is slow, but it is **unbounded with no cap**, so any error loop (a job
failing every tick, `SQLITE_BUSY` spam from F4, or a restart loop) can spike the rate 100×.
On a near-full disk that is a latent killer. *Note:* the app's own `server/` date-dir logs
**are** cleaned (>2 weeks, `logging.rs`), so the exposure is specifically the launchd
stderr file.

### F6. Startup crash-loop on corrupt state or port-in-use
**Severity: MEDIUM.** Deterministic startup panic vectors: a corrupt `permagent.db`
failing the 20-step migration chain (`session_manager.rs:666-776`), or `TcpListener::bind`
(`agent.rs:172`) failing `EADDRINUSE` if a prior instance's socket lingers (no
`SO_REUSEADDR`, no fallback). Each yields `exit(1)` → launchd restarts → same failure →
tight loop capped only by launchd's ~10s default throttle (no explicit backoff). Permanent
outage until manual repair. No pre-migration integrity check on the hot path.

### F7. Stuck goals in the dispatch window
**Severity: MEDIUM.** If the daemon dies between engine-spawn and the atomic state
transition (`orchestrator.rs:722-861`), the goal is left in `ready` (not `in_progress`), so
`resume_in_progress_goals` (`orchestrator.rs:3004-3048`, which queries `state_binding =
'in_progress'`) never recovers it, and the in-flight worker is orphaned. The boot
worktree sweep (`orchestrator.rs:3050-3111`) reclaims the *disk*, but the goal is stranded
until manually re-dispatched. Recovery for the normal `in_progress` case is otherwise
solid (dead-session detection, budget-aware requeue/park, `daemon_lifecycle_id` UUID guard
against double-recovery — `orchestrator.rs:2984-2989`).

---

## TIER 3 — slow drift (real, but weeks-to-months; low kill-probability)

- **F8. `recipe_session_tracker` HashSet, insert-only** (`state.rs:26,602-610`). Keyed by
  `session_id`, never removed — not even on session delete (`routes/session.rs:334` removes
  the event bus but not this). ~40–70 bytes/session ⇒ ~10–15 MB/year at 50–100 sess/day.
  Slow RSS creep.
- **F9. `session_buses` HashMap only pruned on explicit DELETE** (`state.rs:32,627-630`;
  removal at `routes/session.rs:334`). Orphaned/never-deleted sessions leak one
  `SessionEventBus` each (each holds a 512-entry replay VecDeque + broadcast state). Medium
  RSS over weeks of session churn. *Note:* the per-bus buffers themselves are correctly
  bounded (256 broadcast / 512 replay, `session_event_bus.rs:7-8`); the leak is the *count
  of buses*, not their size.
- **F10. `llm_request.<uuid>.jsonl` orphans on crash** (`providers/utils.rs:401,452`). The
  numbered ring (0–9) is bounded; the UUID temp files are only renamed away on clean
  `finish()`, so each crash mid-request leaves one behind forever. Slow disk (live: ~26
  files).
- **F11. Manual DB backup proliferation** (~700 MB live). `memory.db.pre-rebuild-*` /
  `pre-fk-cleanup-*` are **manual one-offs** (dated across distinct rebuild days), plus the
  automatic-but-rare `graph.kz.bak` (ontology-change gated, `brain_sync.rs:71-73`). Nothing
  prunes any of them. Disk creep that only advances when the user (or a rare event) acts.

---

## Corrections (verification overrode agent claims)

The static agents were useful but two of their **top-ranked** findings were wrong; ranking
them as reported would have mis-prioritized the whole doc. Both were caught by reading the
call sites + the live filesystem:

1. **AGENT_RUNTIME "LARGE unbounded leak"** (`events/mod.rs:132-162`) → **DOWNGRADED to
   negligible.** The only non-test caller passes the literal `"henry"`
   (`agents/agent.rs:1273-1285`); every dynamic-`id` caller is `#[cfg(test)]`. The map is
   keyed by a fixed handful of static worker names, so it is bounded, not cumulative.
2. **`graph.kz.bak` "one-per-boot, 600 MB–1.8 GB/month"** (`brain_sync.rs`) → **DOWNGRADED
   to rare/manual-frequency.** The backup is behind an ontology-changed early-return
   (`brain_sync.rs:40`), not per-boot — confirmed by live evidence: exactly **one**
   `graph.kz.bak` exists. Real fast disk-fillers are `daemon.err` (F5) and the Brain WAL
   (F4), not this.

Also confirmed *safe* (correctly bounded): global `EVENT_BUS` buffer (1000,
`events/mod.rs:36`), `ACTIVITY_BUFFER` (500, `activity.rs:166`), `GOAL_WORKERS` (removed in
completion tracker, `orchestrator.rs:729`), `AgentManager` LRU (capacity-bounded,
`manager.rs:22`), SSE task teardown on disconnect (`session_events.rs:232-310`), crash-log
cap (20), server-log 2-week cleanup, and DB snapshot rotation (7 daily + 4 weekly,
`backup.rs:265`).

---

## What CANNOT be determined statically (needs the probe + time)

- **RSS trajectory over weeks.** The confirmed leaks (F8/F9) are slow; whether they matter
  in practice depends on real session churn — only a multi-week RSS series settles it.
- **WAL checkpoint cadence (F4).** Is a reader pinning the WAL? Only watching `memory.db-wal`
  size vs. write activity over 24–48 h answers this.
- **Scheduler-panic blast radius (F3).** `tokio-cron-scheduler 0.14` panic semantics — needs
  a deliberate panicking-job experiment.
- **`daemon.err` real fill-rate (F5).** Baseline vs. fault-mode rate needs a size-over-time
  series.
- **Which task died on 2026-06-23 (F1)** and whether the process restarted — read that crash
  log + correlate with a PID/uptime series.
- **fd / TCP-connection trajectory on `/events`** after weeks — static read shows teardown
  paths exist; only a live count over time proves no slow leak.

---

## Instrumentation — the durability probe (recommendation, not built this round)

"The running daemon is the truth-teller." Durability claims need captured data over time,
not a code read. Design a **lightweight two-part probe**:

### Part A — external sampler (zero daemon code; ship first)
A small shell/Python script run by a **separate launchd agent** every 5 min, appending one
JSON line to `~/.permagent/logs/durability-probe.jsonl` (**self-rotated** — cap at N MB, or
one file per day with a 14-day cull, so the probe itself never becomes F5):

| Metric | Source | Catches |
|---|---|---|
| RSS, %CPU, PID, start-time | `ps -o rss,pcpu,pid,lstart -p <pid>` | leaks (F1/F8/F9); **PID change = restart** |
| open fd count | `lsof -p <pid> \| wc -l` | fd leaks |
| TCP conns on daemon port | `lsof -nP -iTCP:<port> \| wc -l` | `/events` socket leaks |
| `memory.db-wal`, `permagent.db-wal` sizes | `stat` | **F4** WAL wedge |
| disk free on Data volume | `df -k /System/Volumes/Data` | **F5/F11** disk fill |
| `daemon.err` size | `stat` | **F5** growth-rate |
| `llm_request.*` file count | `ls \| wc -l` | **F10** orphan accrual |
| worktree count | `git worktree list \| wc -l` | worktree accrual |
| crash-file count + newest mtime | `ls ~/.permagent/crashes/` | **F1** silent panics |
| per-job `last_run` age + `currently_running` | parse `schedule.json` | **F2** wedged jobs |

Detection heuristics: RSS monotonically rising over 7 d; fd or conn count trending up;
either WAL > 100 MB; disk free < 5 GB; `daemon.err` slope spike; any job with
`currently_running:true` **and** `last_run` age > 2× its interval; a **new** crash file
while PID is unchanged (⇒ F1 half-dead).

### Part B — internal `/api/health/durability` endpoint (small daemon add, follow-up round)
Exposes counters the external sampler can't see, behind the existing Bearer choke point:
`uptime_secs`, `last_panic_ts`, recovery-task liveness, `in_progress` goal count,
`session_buses.len()`, `recipe_session_tracker.len()`, current WAL sizes, scheduler
job-status vector. This is what turns "PID unchanged" (looks healthy) into "background task
X is dead" (F1) — the single most important thing the external probe cannot determine.

---

## Fix-priority list (for follow-up BUILD rounds — not this round)

| # | Fix | Addresses | Effort | Priority |
|---|---|---|---|---|
| 1 | **Startup reconciliation**: on load, reset `currently_running` when `process_start_time` is stale / from a prior PID | F2 | S | **P0** |
| 2 | **Panic circuit-breaker**: count panics via the hook; on N within M min, `exit(1)` to force a clean launchd restart instead of limping half-dead | F1 | S | **P0** |
| 3 | **`catch_unwind` around `execute_job`** so one job's panic can't kill the scheduler loop | F3 | S | **P0** |
| 4 | **Brain WAL hygiene**: set explicit `wal_autocheckpoint` + a periodic `wal_checkpoint(TRUNCATE)` task; add `busy_timeout` to `read_only_brain_conn` | F4 | M | **P1** |
| 5 | **Rotate `daemon.err`**: app-side rolling file appender (or a `newsyslog.d` conf, or periodic truncate) instead of raw launchd stderr | F5 | S | **P1** |
| 6 | **Ship the external durability probe (Part A)** so the rest is measured, not guessed | all | S | **P1** |
| 7 | **Atomic dispatch**: transition goal state before/with tracker spawn to close the dispatch-window gap | F7 | M | P2 |
| 8 | **Startup cleanup passes**: reset-then-sweep orphan `llm_request.<uuid>` files; prune old manual DB backups by count/age | F10, F11 | S | P2 |
| 9 | **Remove `recipe_session_tracker`/`session_buses` entries on session delete**; add idle-expiry for buses | F8, F9 | S | P2 |
| 10 | **launchd hardening**: explicit `ThrottleInterval` (backoff) + `SO_REUSEADDR`/bind-retry to avoid startup crash-loops | F6 | S | P2 |
| 11 | **`/api/health/durability` endpoint (probe Part B)** | F1 detection | M | P2 |

---

## Decision points for Jesse (Tier-2 rulings — not auto-buildable)

1. **Crash philosophy (F1).** Add the panic circuit-breaker so a dead background task forces
   a *whole-process* restart (clean state, launchd relaunches) — **or** keep the current
   "tolerate half-dead" behavior and rely on the health endpoint + external supervision?
   This is the single biggest lever on the bar and it's a design choice, not a mechanical
   fix.
2. **Supervision model.** launchd only restarts on process exit. Do we want a watchdog that
   restarts on *health-check failure* (a separate agent hitting `/api/health/durability`),
   or is "circuit-breaker → exit → launchd" sufficient?
3. **Log verbosity vs. disk (F5).** Drop daemon stderr to WARN by default (quieter, slower
   fill) or keep INFO for debuggability and rely on rotation? Affects both disk and remote
   debuggability.
4. **WAL checkpoint aggressiveness (F4).** TRUNCATE-on-a-timer (bounded WAL, periodic brief
   write-lock) vs. PASSIVE autocheckpoint (never blocks, but a pinned reader can still let it
   grow). Trade-off between disk-safety and write-latency.
5. **Probe cadence & retention.** 5-min sampling / 14-day retention is a starting point —
   confirm before it ships (it writes to the same near-full disk it's watching).

---

## Honest bottom line

- **What would break the bar first:** F1 (silent half-dead daemon) and F2 (one crash freezes
  the scheduler) — both need only a *single* fault over weeks, both self-heal *never*, and
  both are currently invisible. F4 (WAL/disk wedge) is the fastest *disk* killer on a
  near-full machine but needs runtime confirmation of the checkpoint cause.
- **What is genuinely fine:** most in-memory buffers are correctly bounded; crash logging,
  server logs, DB snapshots, and goal worktrees all have working rotation/cleanup; goal
  recovery for the common case is solid. The memory "leaks" are real but slow (Tier 3).
- **What we can't yet know:** the actual RSS/WAL/fd trajectories — which is exactly why the
  probe (Fix #6) should ship before we claim any weeks-untouched guarantee. Right now the
  bar is **plausible but unproven**, and F1/F2 are concrete reasons it would *not* hold today
  through the first background-task panic or mid-job crash.
