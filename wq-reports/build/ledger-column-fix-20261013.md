# Ledger column fix, 2026-10-13

## Cause
Installed smoke (2026-10-13 09:00, live profile) ended a chat reply with "durable accounting failed: ... table cost_ledger has no column named goal_attempt_id".
`apply_cost_ledger_schema` (spectral_schema.rs) adds `goal_attempt_id` (and every other cost_ledger column) with PRAGMA guards, but it was only called from `init_spectral_db` (fresh install) and from numbered migrations (v27->28, v53->54, and one more). The live DB was stamped at schema_version 62 by an earlier daemon from another branch, so none of those steps ran for it, and no open-path call existed. Version numbers collide across lanes, so numbered steps cannot be trusted to add new ledger columns.

## Change
- `crates/goose/src/session/session_manager.rs`: in the open path, right beside the existing unconditional `apply_cost_reservations_schema` (after the numbered chain, every open), added an unconditional `spectral_schema::apply_cost_ledger_schema(&self.pool)`.
- `crates/goose/src/session/spectral_schema.rs`: `apply_cost_ledger_schema` now logs one INFO line per column actually added: `cost_ledger: added missing column <col>` (and `sessions: added missing column <col>` for the rollup columns).

## Audit of init_spectral_db vs the open path
Ensures run by init_spectral_db and NOT on the open path (only reached by numbered steps): cost_ledger, cost_reservation_batches, daemon_auth_audit, decision_journal, effect_outbox, egress_audit, entity_provenance, forecaster, inbox, model_review, model_review_states, notification_routing, person_meetings, person_merge, project_association, project_documents, project_intel, project_notes, project_stack, recognition (+feed), schedule_fire_claims, schedule_pause, sessions_schedule_id_index, activity-journal-adjacent tables.
Of these only `apply_cost_ledger_schema` is purely additive AND adds columns by PRAGMA guard to a table that gets new columns over time, so it is the only one included. `apply_cost_reservations_schema` was already unconditional on open (present before this change). The others are CREATE TABLE IF NOT EXISTS for tables whose numbered step already exists, or contain data writes/seed INSERTs (activity_journal, egress_audit, schedule_pause, council, etc. use triggers/inserts); excluded per "do not rewrite data". `apply_decision_audit_principal_schema` (column-guarded) already runs on open via `apply_decision_inbox_schema`.
Follow-up candidates if the same drift hits them: cost_reservation_batches, model_review(_states), recognition (columns already covered by `apply_recognition_v22_columns`).

## Tests (crates/goose, temp SQLite)
- `session::session_manager::tests::open_adds_missing_cost_ledger_column_without_migration`: opens twice to reach the latest stamp, `ALTER TABLE cost_ledger DROP COLUMN goal_attempt_id`, confirms an INSERT with it fails, reopens, asserts schema_version max unchanged, column exists and the INSERT succeeds.
- `session::session_manager::tests::fresh_install_has_cost_ledger_goal_attempt_id_and_reopen_is_idempotent`: fresh-install path has the column, INSERT works, reopen keeps the row.
(First attempt of the first test failed because a fresh install stamps 14 and the first reopen migrates to 62; test corrected to open twice.)

## Gates (all through run-build, RUST_MIN_STACK=8388608, INSTA_UPDATE=no)
1. `cargo test -p permagent --lib --no-run`: exit 0.
2. `cargo test -p permagent --lib -- spectral_schema session_manager::tests --test-threads=2`: exit 0, 109 passed, 0 failed (6366 filtered). An earlier run was 108 passed / 1 failed (the test bug above).
3. Daemon lib gate: NOT RUN TO COMPLETION. `cargo test -p permagent-daemon --lib --no-run` was aborted by the build guard at 10:42: "continuous floor reached (GiB free vs floor): /System/Volumes/Data (8.0, 8)". Free space fell from 28 GiB at 09:57 to 3 GiB (external target volume is separate, so something else on the data volume consumed ~25 GiB). Per the >=15 GiB rule, not retried; the 11:00-11:45 window is also near. The change is confined to goose's open path and does not touch daemon code, but 0 failures is unverified. Re-run `./scripts/test-daemon.sh --lib` once disk is back above 15 GiB.

## Highest migration vs live
Highest numbered migration in code: `migrate_v61_to_v62` (v62, model_review_finding_states). Live ledger DB schema_version tops out at 62. They match, so the live DB is not behind in version; the missing column is a lane-collision drift that no numbered step would repair. The on-open ensure fixes it on the next daemon start without any migration.
