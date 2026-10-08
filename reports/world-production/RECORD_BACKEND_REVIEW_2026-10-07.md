# Record backend review — 2026-10-07

Independent review of the current integration worktree. Review frozen at root's request while Cursor continues fixes. Findings below are source-confirmed control-flow gaps; their runtime interleavings were not exercised by this reviewer. No Cargo commands were run because root owns the active build.

## Remaining findings at review freeze

| Priority | Evidence | Concrete consequence and required fix |
| --- | --- | --- |
| P1 | `crates/goose/src/meeting_triage.rs`, `process_triage`, cached-result branch near line 1162; `meeting_writeup.rs`, `privacy_statement` | Cached extraction constructs provider/model `cached` and privacy `LocalOnly`. A cloud result persisted before an apply failure or crash is reused on retry and exported as “Written on-device.” Persist the actual provider, model and privacy provenance with the extraction, then reuse it on retry. |
| P2 | `crates/goose-server/src/state.rs:417–433` | Meeting startup recovery and periodic draining are inside `if let Some(ref b) = brain`. Restarting with Brain unavailable leaves pre-crash `running` jobs stranded; ordinary save draining only claims pending rows, and explicit retry accepts failed/skipped. Start the meeting lifecycle independently of Brain and pass the existing optional handle. |
| P2 | `crates/goose/src/meeting_triage.rs`, `reset_abandoned_running_rows` near line 721, `claim_next` near line 749, `retry_triage` near line 609 | A crash during attempt 5 becomes `pending` with attempt_count 5 on recovery. Claims require count < 5, while explicit retry rejects pending, so the job cannot progress. Recover exhausted jobs into a retryable failure or provide a valid explicit reset path. |
| P2 | `crates/goose-server/src/routes/projects.rs:1324,1342`; `meeting_triage.rs`, enhanced export near line 1081; `project_notes.rs`, `write_note_export_inner` | The save route starts triage before writing the initial raw export. Triage can finish and write the enhanced export first, followed by the route overwriting it with its stale raw note. Serialize or version-gate export writes; merely moving the route's spawn does not exclude the periodic drain. |

Line numbers can move as Cursor edits; function names identify the relevant paths.

## Completed reviewer fix

Only source file edited by this reviewer: `crates/goose/src/session/session_manager.rs`.

- Existing-database boot now runs `crate::meeting_triage::apply_schema(&self.pool).await?` immediately after the Brain feed migration (near line 2930). Previously only fresh database initialization installed this schema, so upgraded installations could fail their first recording save with a missing-table error.
- Added `meeting_triage_boot_tests::existing_database_installs_meeting_triage_before_note_save` (near line 10119). It creates a database, removes the new meeting tables to model an old installation, closes/reopens the manager, then verifies a real meeting save produces pending triage and the item table exists.
- Diff: 51 added lines. `git diff --check -- crates/goose/src/session/session_manager.rs` passed. Regression test is authored but not run; root must run it with the build.

## Cursor corrections observed, awaiting root acceptance

- The earlier note/catalogue crash gap has been corrected in the latest source read: initial note catalogue enqueue is inside the note+triage transaction; enhanced-note and new-card outbox enqueues are inside the effects+completed transaction. Both use transaction-aware helpers and propagate errors. No runtime or rollback test was run by this reviewer.
- The earlier generic 2,000-character truncation of `FeedSource::Note` has a dedicated note policy in the latest source (`MAX_NOTE_FEED_CHARS = 131_072` and source-specific bounding). Verify final tests and intended over-limit behavior.
- Previously reported compile-level issues in `insert_note_tx`, optional Brain argument wiring, and session-view move/borrow ordering were corrected in the subsequent source snapshot. Compilation remains root's verification responsibility.

## Scope

Read-only review covered meeting persistence, extraction/writeup provenance, retry/recovery, project-scoped card materialization, note exports, Brain catalogue delivery, and session inspection. Note+triage insertion and card+completed updates use transactions; card materialization is project-scoped with stable identities. This report is a focused defect review, not a claim that every backend flow or UI flow has passed.
