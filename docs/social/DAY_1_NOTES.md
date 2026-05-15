# permagent-social Day 1 — Build Notes

Branch: `feat/permagent-social-day1` (committed, pushed to origin)
Date: 2026-05-15

## Dependency deviations from original spec

| Spec declared | Actual used | Reason |
|---|---|---|
| `atrium-oauth-client = "0.5"` | `atrium-oauth = "0.1"` (resolved 0.1.7) | `atrium-oauth-client` does not exist on crates.io. The correct crate is `atrium-oauth`. |
| `atrium-api = "0.24"` | `atrium-api = "0.25"` (resolved 0.25.8) | 0.24 is not the latest semver family; 0.25 is the current release line. |
| `axum = "0.7"` | `axum = { workspace = true }` (resolved 0.8.8) | Workspace already pins axum 0.8. axum 0.7 -> 0.8 has breaking API changes (Router, handler signatures). Day 2+ HTTP code must target 0.8. |

## Workspace dependency unification

11 deps switched from pinned versions to `workspace = true`: tokio, axum, serde, serde_json, thiserror, anyhow, chrono, tracing, async-trait, rand, tempfile. This avoids duplicate versions in the lockfile.

The tokio `[dev-dependencies]` entry from the spec was dropped entirely — the main dependency already has `features = ["full"]` which is a superset.

## Phase 2 overlap (reason for pause)

The following Day 1 tables duplicate primitives planned for Phase 2 Epics:

- `projects` — Epic #69 (Projects) will own the canonical project table
- `cards` / `board_columns` — Epic #69 Kanban primitives

When Phase 2 lands, Day 1's schema should be refactored to:
1. Drop `projects`, `cards`, `board_columns` from `permagent-social`
2. Import those types from the Phase 2 crate (likely `permagent-projects` or equivalent)
3. Keep only `social_posts`, `social_post_targets`, `social_accounts` — the social-specific tables

The `seed_default_projects()` function will also move to the Phase 2 crate.

## SQLite runtime queries

All queries use `sqlx::query` / `sqlx::query_scalar` / `sqlx::query_as` (runtime, not compile-time macros). This is intentional — avoids requiring `DATABASE_URL` at build time. The `sqlx::migrate!("./migrations")` macro is the only compile-time sqlx call (it embeds .sql files, no DB connection needed).

## Restart checklist

When resuming social work after Phase 2:

1. Rebase `feat/permagent-social-day1` onto the post-Phase-2 main
2. Remove duplicated tables/models, import from Phase 2 crate
3. Verify `atrium-api` and `atrium-oauth` haven't had breaking releases (both are pre-1.0)
4. Confirm axum 0.8 handler patterns match the daemon's existing HTTP layer
5. Re-run `cargo build -p permagent-social && cargo test -p permagent-social`
