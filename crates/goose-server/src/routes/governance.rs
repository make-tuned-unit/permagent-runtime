//! Governance surface — the spend read the unified control view needed.
//!
//! The Governance view in the command center consolidates four things the user
//! governs about their OWN machine into one legible surface. Three of the four
//! panels compose endpoints that already exist:
//!
//!   - Models       → `GET /api/agent/workers` + `GET /config` (workers.rs / config_management.rs)
//!   - Sovereignty  → `GET/POST /api/security/sovereignty` + `GET /api/security/egress-log` (security.rs)
//!   - Approvals    → `GET /api/decisions` + `GET /config` effective goose-mode (decisions.rs / config_management.rs)
//!
//! Only **Spend** had no read: per-session and per-project token + dollar
//! consumption with a running total, and the optional user budget. The cost
//! *data* already exists (the `cost_ledger` table + the O(1) `accumulated_*`
//! rollup columns on `sessions`, schema v28); this route only aggregates it and
//! surfaces the budget ceilings the cost-router already enforces through the
//! Decision Inbox. No new tables, no migration — pure consolidation of the
//! existing ledger.
//!
//! Auth is handled by the bearer-token middleware (protected group).
//!
//! Endpoints:
//!   GET  /api/governance/spend    — per-session + per-project spend + running total + budget (?limit=)
//!   GET  /api/governance/budget   — the current optional budget ceilings
//!   POST /api/governance/budget   — set the budget ceilings { session?, task? }

use axum::{
    extract::{Json, Query, State},
    http::StatusCode,
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use sqlx::{Pool, Sqlite};
use std::collections::BTreeMap;
use std::sync::Arc;

#[allow(unused_imports)]
pub use permagent::app_views::{
    build_spend_snapshot, BudgetView, CeilingsView, ProjectSpend, SessionSpend, SpendRow,
    SpendSnapshot,
};
use permagent::config::{Config, ConfigError};
use permagent::cost_router::budget;
#[cfg(test)]
use permagent::cost_router::budget::BudgetConfig;

use crate::state::AppState;

// ── Spend ────────────────────────────────────────────────────────────────

pub use permagent::app_views::read_spend_rows;

/// Map each project's root path → its name, for friendly per-project labels.
/// Best-effort: a failure (or no projects table rows) yields an empty map and
/// the view falls back to the directory basename.
pub async fn read_project_names(pool: &Pool<Sqlite>) -> BTreeMap<String, String> {
    permagent::app_views::read_project_names(pool)
        .await
        .unwrap_or_default()
}

#[derive(Debug, Deserialize)]
struct SpendQuery {
    #[serde(default = "default_spend_limit")]
    limit: usize,
}

fn default_spend_limit() -> usize {
    100
}

async fn get_spend(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SpendQuery>,
) -> Result<Json<SpendSnapshot>, (StatusCode, String)> {
    let limit = params.limit.clamp(1, 1000);
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let rows = read_spend_rows(&pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project_names = read_project_names(&pool).await;
    let cfg = budget::load_budget_config();
    Ok(Json(build_spend_snapshot(
        rows,
        &project_names,
        &cfg,
        limit,
    )))
}

// ── Budget ───────────────────────────────────────────────────────────────

async fn get_budget() -> Json<BudgetView> {
    Json(budget::load_budget_config().into())
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CeilingsPatch {
    soft: Option<f64>,
    gate: Option<f64>,
    hard: Option<f64>,
}

/// A budget patch — either scope may be omitted (untouched), and within a scope
/// any of the three ceilings may be omitted. The main knob the Governance view
/// exposes is the SESSION ceiling ("spend cap for this machine").
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetBudgetRequest {
    session: Option<CeilingsPatch>,
    task: Option<CeilingsPatch>,
}

/// Apply a budget patch to an explicit `Config` (unit-testable against a temp
/// config, not the process-global one). Only provided keys are written, so an
/// omitted ceiling is left untouched. Negative values are rejected up front.
fn apply_budget_patch(req: &SetBudgetRequest, config: &Config) -> Result<(), ConfigError> {
    let mut updates = Vec::new();
    if let Some(s) = &req.session {
        updates.extend([
            (budget::KEY_SESSION_SOFT, s.soft),
            (budget::KEY_SESSION_GATE, s.gate),
            (budget::KEY_SESSION_HARD, s.hard),
        ]);
    }
    if let Some(t) = &req.task {
        updates.extend([
            (budget::KEY_TASK_SOFT, t.soft),
            (budget::KEY_TASK_GATE, t.gate),
            (budget::KEY_TASK_HARD, t.hard),
        ]);
    }
    config.set_params(
        updates
            .into_iter()
            .filter_map(|(key, value)| value.map(|value| (key, value))),
    )
}

/// Reject any negative ceiling before writing — a negative cap is nonsense and
/// would only be silently clamped on read.
fn validate_patch(req: &SetBudgetRequest) -> Result<(), String> {
    for scope in [&req.session, &req.task].into_iter().flatten() {
        for v in [scope.soft, scope.gate, scope.hard].into_iter().flatten() {
            if v < 0.0 || !v.is_finite() {
                return Err("budget ceilings must be finite and non-negative".to_string());
            }
        }
    }
    Ok(())
}

async fn set_budget(
    Json(req): Json<SetBudgetRequest>,
) -> Result<Json<BudgetView>, (StatusCode, String)> {
    validate_patch(&req).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    apply_budget_patch(&req, Config::global())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    // Read back through the sanitizing loader so the response reflects the
    // enforced (monotone, non-negative) ceilings, not the raw writes.
    Ok(Json(budget::load_budget_config().into()))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/governance/spend", get(get_spend))
        .route("/api/governance/budget", get(get_budget))
        .route("/api/governance/budget", post(set_budget))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn mem_pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite pool");
        permagent::session::spectral_schema::init_spectral_db(&pool)
            .await
            .expect("spectral schema init");
        pool
    }

    fn row(id: &str, dir: &str, cost: f64, tokens: i64) -> SpendRow {
        SpendRow {
            id: id.to_string(),
            name: format!("session {id}"),
            working_dir: dir.to_string(),
            session_type: "user".to_string(),
            cost_usd: cost,
            tokens,
            updated_at: "2026-07-22T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn snapshot_totals_group_and_cap() {
        let rows = vec![
            row("a", "/proj/one", 1.50, 1000),
            row("b", "/proj/one", 0.50, 400),
            row("c", "/proj/two", 3.00, 2000),
        ];
        let mut names = BTreeMap::new();
        names.insert("/proj/one".to_string(), "One".to_string());
        let cfg = BudgetConfig::default();

        let snap = build_spend_snapshot(rows, &names, &cfg, 2);

        // Running total + tokens span ALL rows, even when the session list caps.
        assert_eq!(snap.session_count, 3);
        assert!((snap.running_total_usd - 5.00).abs() < 1e-9);
        assert_eq!(snap.total_tokens, 3400);

        // Session list sorted by cost desc, capped at 2.
        assert_eq!(snap.sessions.len(), 2);
        assert_eq!(snap.sessions[0].id, "c");
        assert_eq!(snap.sessions[1].id, "a");

        // Projects grouped: two projects, /proj/two first (higher cost), and the
        // named project gets its friendly label; the unnamed one falls back to
        // the directory basename.
        assert_eq!(snap.projects.len(), 2);
        assert_eq!(snap.projects[0].path, "/proj/two");
        assert_eq!(snap.projects[0].label, "two");
        assert!((snap.projects[0].cost_usd - 3.00).abs() < 1e-9);
        let one = snap
            .projects
            .iter()
            .find(|p| p.path == "/proj/one")
            .unwrap();
        assert_eq!(one.label, "One");
        assert_eq!(one.session_count, 2);
        assert!((one.cost_usd - 2.00).abs() < 1e-9);
    }

    #[test]
    fn per_session_band_reflects_session_ceiling() {
        // Session ceilings default soft $10 / gate $25 / hard $50.
        let cfg = BudgetConfig::default();
        let rows = vec![
            row("ok", "/d", 1.0, 10),
            row("soft", "/d", 12.0, 10),
            row("gate", "/d", 30.0, 10),
            row("hard", "/d", 60.0, 10),
        ];
        let snap = build_spend_snapshot(rows, &BTreeMap::new(), &cfg, 100);
        let band = |id: &str| {
            snap.sessions
                .iter()
                .find(|s| s.id == id)
                .map(|s| s.band.as_str())
                .unwrap()
        };
        assert_eq!(band("ok"), "ok");
        assert_eq!(band("soft"), "soft");
        assert_eq!(band("gate"), "gate");
        assert_eq!(band("hard"), "hard");
    }

    #[tokio::test]
    async fn read_spend_rows_only_returns_consuming_sessions() {
        let pool = mem_pool().await;
        // Two consuming sessions + one zero-cost terminal session.
        sqlx::query(
            "INSERT INTO sessions (id, name, session_type, working_dir, accumulated_cost_usd, accumulated_total_tokens) \
             VALUES ('s1', 'build', 'user', '/proj/a', 2.5, 1500)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO sessions (id, name, session_type, working_dir, accumulated_cost_usd, accumulated_total_tokens) \
             VALUES ('s2', 'chat', 'user', '/proj/b', 0.0, 800)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO sessions (id, name, session_type, working_dir, accumulated_cost_usd, accumulated_total_tokens) \
             VALUES ('s3', 'term', 'terminal', '/proj/c', 0.0, 0)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let rows = read_spend_rows(&pool).await.unwrap();
        // s3 (no spend, no tokens) excluded; s1 + s2 returned, s1 first (cost).
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "s1");
        assert!((rows[0].cost_usd - 2.5).abs() < 1e-9);
        assert_eq!(rows[0].tokens, 1500);
        assert!(rows.iter().all(|r| r.id != "s3"));
    }

    #[tokio::test]
    async fn read_project_names_maps_root_path_to_name() {
        let pool = mem_pool().await;
        sqlx::query(
            "INSERT INTO projects (id, slug, name, root_path) VALUES ('p1', 'alpha', 'Alpha', '/proj/a')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let map = read_project_names(&pool).await;
        assert_eq!(map.get("/proj/a"), Some(&"Alpha".to_string()));
    }

    #[test]
    fn budget_patch_validation_rejects_negative() {
        let bad = SetBudgetRequest {
            session: Some(CeilingsPatch {
                soft: Some(-1.0),
                gate: None,
                hard: None,
            }),
            task: None,
        };
        assert!(validate_patch(&bad).is_err());

        let ok = SetBudgetRequest {
            session: Some(CeilingsPatch {
                soft: Some(5.0),
                gate: Some(20.0),
                hard: Some(40.0),
            }),
            task: None,
        };
        assert!(validate_patch(&ok).is_ok());
    }

    #[test]
    fn budget_patch_writes_only_provided_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let config = Config::new(tmp.path().join("config.yaml"), "permagent-test").unwrap();
        // Isolate from ambient env overrides that get_param checks first.
        let _guard = env_lock::lock_env([
            ("PERMAGENT_BUDGET_SESSION_SOFT_USD", None::<&str>),
            ("PERMAGENT_BUDGET_SESSION_HARD_USD", None::<&str>),
            ("PERMAGENT_BUDGET_TASK_SOFT_USD", None::<&str>),
        ]);

        let req = SetBudgetRequest {
            session: Some(CeilingsPatch {
                soft: Some(8.0),
                gate: None,
                hard: Some(64.0),
            }),
            task: None,
        };
        apply_budget_patch(&req, &config).unwrap();

        assert_eq!(
            config.get_param::<f64>(budget::KEY_SESSION_SOFT).unwrap(),
            8.0
        );
        assert_eq!(
            config.get_param::<f64>(budget::KEY_SESSION_HARD).unwrap(),
            64.0
        );
        // Omitted keys were never written.
        assert!(config.get_param::<f64>(budget::KEY_SESSION_GATE).is_err());
        assert!(config.get_param::<f64>(budget::KEY_TASK_SOFT).is_err());
    }

    #[test]
    fn budget_patch_failure_preserves_the_prior_triplet() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.yaml");
        let config = Config::new(&config_path, "permagent-test").unwrap();
        let _guard = env_lock::lock_env([
            ("PERMAGENT_BUDGET_SESSION_SOFT_USD", None::<&str>),
            ("PERMAGENT_BUDGET_SESSION_GATE_USD", None::<&str>),
            ("PERMAGENT_BUDGET_SESSION_HARD_USD", None::<&str>),
        ]);
        config
            .set_params([
                (budget::KEY_SESSION_SOFT, 10.0),
                (budget::KEY_SESSION_GATE, 25.0),
                (budget::KEY_SESSION_HARD, 50.0),
            ])
            .unwrap();

        // Blocking creation of the temporary file forces the single atomic
        // save to fail before the original config can be replaced.
        std::fs::create_dir(config_path.with_extension("tmp")).unwrap();
        let req = SetBudgetRequest {
            session: Some(CeilingsPatch {
                soft: Some(1.0),
                gate: Some(2.0),
                hard: Some(3.0),
            }),
            task: None,
        };
        assert!(apply_budget_patch(&req, &config).is_err());

        assert_eq!(
            config.get_param::<f64>(budget::KEY_SESSION_SOFT).unwrap(),
            10.0
        );
        assert_eq!(
            config.get_param::<f64>(budget::KEY_SESSION_GATE).unwrap(),
            25.0
        );
        assert_eq!(
            config.get_param::<f64>(budget::KEY_SESSION_HARD).unwrap(),
            50.0
        );
    }
}
