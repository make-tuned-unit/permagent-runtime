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
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Row, Sqlite};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use permagent::config::{Config, ConfigError};
use permagent::cost_router::budget::{
    self, budget_verdict, BudgetBand, BudgetCeilings, BudgetConfig,
};

use crate::state::AppState;

// ── Spend ────────────────────────────────────────────────────────────────

/// A lean per-session spend row read from the `sessions` rollup columns. Kept
/// separate from the fat `Session` so the Governance view never pays for the
/// heavy message/extension blobs it does not show.
#[derive(Debug, Clone)]
pub struct SpendRow {
    pub id: String,
    pub name: String,
    pub working_dir: String,
    pub session_type: String,
    pub cost_usd: f64,
    pub tokens: i64,
    pub updated_at: String,
}

/// One session's spend, wire-friendly, with its budget band vs the session
/// ceiling so the UI can warn on sessions that crossed soft/gate/hard.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSpend {
    pub id: String,
    pub name: String,
    pub working_dir: String,
    pub session_type: String,
    pub cost_usd: f64,
    pub tokens: i64,
    pub updated_at: String,
    /// "ok" | "soft" | "gate" | "hard" — this session's spend against the
    /// session budget ceiling.
    pub band: String,
}

/// Spend rolled up to a project (grouped by working directory).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSpend {
    /// Full working directory the sessions ran in.
    pub path: String,
    /// Friendly label — the project name when a project's root path matches,
    /// otherwise the directory's final path component.
    pub label: String,
    pub cost_usd: f64,
    pub tokens: i64,
    pub session_count: usize,
}

/// A budget ceiling triplet, wire-friendly.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CeilingsView {
    pub soft: f64,
    pub gate: f64,
    pub hard: f64,
}

impl From<BudgetCeilings> for CeilingsView {
    fn from(c: BudgetCeilings) -> Self {
        Self {
            soft: c.soft,
            gate: c.gate,
            hard: c.hard,
        }
    }
}

/// The optional user budget the cost-router enforces (gates through the
/// Decision Inbox at the ceiling). Both scopes are surfaced; the session scope
/// is the "spend cap for this machine" the Governance view lets the user set.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetView {
    pub session: CeilingsView,
    pub task: CeilingsView,
}

impl From<BudgetConfig> for BudgetView {
    fn from(c: BudgetConfig) -> Self {
        Self {
            session: c.session.into(),
            task: c.task.into(),
        }
    }
}

/// The full spend snapshot for the Governance view's Spend panel.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpendSnapshot {
    /// Sum of every session's running cost — the lifetime total spend.
    pub running_total_usd: f64,
    /// Sum of every session's accumulated total tokens.
    pub total_tokens: i64,
    /// Number of sessions that actually consumed (cost or tokens > 0).
    pub session_count: usize,
    /// The current optional budget ceilings.
    pub budget: BudgetView,
    /// Per-session spend, highest first, capped at the requested limit.
    pub sessions: Vec<SessionSpend>,
    /// Per-project spend, highest first.
    pub projects: Vec<ProjectSpend>,
}

fn band_str(b: BudgetBand) -> &'static str {
    match b {
        BudgetBand::Ok => "ok",
        BudgetBand::Soft => "soft",
        BudgetBand::Gate => "gate",
        BudgetBand::Hard => "hard",
    }
}

/// Final path component of a working dir, or the whole string if it has none.
fn basename(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(path)
        .to_string()
}

/// Pure aggregation: fold lean spend rows into the wire snapshot. Sessions are
/// sorted by cost desc and capped at `limit`; projects are grouped by working
/// dir (labelled via `project_names`, a working_dir → name map) and sorted by
/// cost desc. Each session gets its band against the session budget ceiling.
/// Pure so it is unit-testable without a database.
pub fn build_spend_snapshot(
    rows: Vec<SpendRow>,
    project_names: &BTreeMap<String, String>,
    cfg: &BudgetConfig,
    limit: usize,
) -> SpendSnapshot {
    let running_total_usd: f64 = rows.iter().map(|r| r.cost_usd).sum();
    let total_tokens: i64 = rows.iter().map(|r| r.tokens).sum();
    let session_count = rows.len();

    // ── Per-project rollup (grouped by working dir) ──
    let mut by_project: BTreeMap<String, ProjectSpend> = BTreeMap::new();
    for r in &rows {
        let entry = by_project
            .entry(r.working_dir.clone())
            .or_insert_with(|| ProjectSpend {
                path: r.working_dir.clone(),
                label: project_names
                    .get(&r.working_dir)
                    .cloned()
                    .unwrap_or_else(|| basename(&r.working_dir)),
                cost_usd: 0.0,
                tokens: 0,
                session_count: 0,
            });
        entry.cost_usd += r.cost_usd;
        entry.tokens += r.tokens;
        entry.session_count += 1;
    }
    let mut projects: Vec<ProjectSpend> = by_project.into_values().collect();
    projects.sort_by(|a, b| {
        b.cost_usd
            .partial_cmp(&a.cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.path.cmp(&b.path))
    });

    // ── Per-session, banded, sorted by cost desc, capped ──
    let mut sessions: Vec<SessionSpend> = rows
        .into_iter()
        .map(|r| {
            let band = budget_verdict(0.0, r.cost_usd, cfg).band;
            SessionSpend {
                id: r.id,
                name: r.name,
                working_dir: r.working_dir,
                session_type: r.session_type,
                cost_usd: r.cost_usd,
                tokens: r.tokens,
                updated_at: r.updated_at,
                band: band_str(band).to_string(),
            }
        })
        .collect();
    sessions.sort_by(|a, b| {
        b.cost_usd
            .partial_cmp(&a.cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
    sessions.truncate(limit);

    SpendSnapshot {
        running_total_usd,
        total_tokens,
        session_count,
        budget: (*cfg).into(),
        sessions,
        projects,
    }
}

/// Read the lean per-session spend rows from the `sessions` rollup columns.
/// Only sessions that actually consumed (cost or tokens > 0) are returned, so
/// terminal/gateway sessions with no inference do not clutter the view.
pub async fn read_spend_rows(pool: &Pool<Sqlite>) -> anyhow::Result<Vec<SpendRow>> {
    let rows = sqlx::query(
        "SELECT id, \
                CASE WHEN name != '' THEN name ELSE description END AS label, \
                working_dir, session_type, \
                COALESCE(accumulated_cost_usd, 0.0) AS cost, \
                COALESCE(accumulated_total_tokens, 0) AS tokens, \
                updated_at \
         FROM sessions \
         WHERE COALESCE(accumulated_cost_usd, 0.0) > 0.0 \
            OR COALESCE(accumulated_total_tokens, 0) > 0 \
         ORDER BY cost DESC, id",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|row| SpendRow {
            id: row.try_get("id").unwrap_or_default(),
            name: row.try_get("label").unwrap_or_default(),
            working_dir: row.try_get("working_dir").unwrap_or_default(),
            session_type: row.try_get("session_type").unwrap_or_default(),
            cost_usd: row.try_get("cost").unwrap_or(0.0),
            tokens: row.try_get("tokens").unwrap_or(0),
            updated_at: {
                // TIMESTAMP columns come back as text; tolerate either.
                row.try_get::<String, _>("updated_at").unwrap_or_default()
            },
        })
        .collect())
}

/// Map each project's root path → its name, for friendly per-project labels.
/// Best-effort: a failure (or no projects table rows) yields an empty map and
/// the view falls back to the directory basename.
pub async fn read_project_names(pool: &Pool<Sqlite>) -> BTreeMap<String, String> {
    let rows = sqlx::query("SELECT root_path, name FROM projects WHERE root_path IS NOT NULL")
        .fetch_all(pool)
        .await
        .unwrap_or_default();
    rows.iter()
        .filter_map(|row| {
            let path: String = row.try_get("root_path").ok()?;
            let name: String = row.try_get("name").ok()?;
            if path.is_empty() {
                None
            } else {
                Some((path, name))
            }
        })
        .collect()
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

        assert_eq!(config.get_param::<f64>(budget::KEY_SESSION_SOFT).unwrap(), 10.0);
        assert_eq!(config.get_param::<f64>(budget::KEY_SESSION_GATE).unwrap(), 25.0);
        assert_eq!(config.get_param::<f64>(budget::KEY_SESSION_HARD).unwrap(), 50.0);
    }
}
