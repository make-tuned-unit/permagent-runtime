//! Canonical, read-time budget projection over Spectral.
//!
//! This module deliberately contains no copied spend snapshot and no new
//! persistence.  `BudgetProjection` is recomputed from the existing session,
//! ledger, reservation, and current configuration sources.  A numeric zero
//! means an authoritative query found no dollars; `None` means the source was
//! unavailable or invalid and must remain fail-closed to authorization.

use std::collections::{HashMap, HashSet, VecDeque};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::cost_router::budget::BudgetConfig;
use crate::session::session_manager::SessionManager;

pub const BUDGET_PROJECTION_VERSION: &str = "budget-projection.v1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionCompleteness {
    Complete,
    Partial,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionBand {
    Ok,
    Soft,
    Gate,
    Hard,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CapTriplet {
    pub soft_usd: Option<f64>,
    pub gate_usd: Option<f64>,
    pub hard_usd: Option<f64>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BudgetScopeProjection {
    pub cap: CapTriplet,
    pub settled_usd: Option<f64>,
    pub held_usd: Option<f64>,
    pub unknown_usd: Option<f64>,
    pub effective_used_usd: Option<f64>,
    pub remaining_usd: Option<f64>,
    pub band: Option<ProjectionBand>,
    pub completeness: ProjectionCompleteness,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BillingEvidence {
    pub billing_class: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub call_id: Option<String>,
    pub is_estimated: Option<bool>,
    pub observed_at: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectionProvenance {
    pub version: String,
    pub as_of: String,
    pub completeness: ProjectionCompleteness,
    pub sources: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BudgetProjection {
    pub task_id: Option<String>,
    pub root_session_id: String,
    pub task: BudgetScopeProjection,
    pub session: BudgetScopeProjection,
    pub task_billing: BillingEvidence,
    pub session_billing: BillingEvidence,
    pub provenance: ProjectionProvenance,
}

#[derive(Debug, thiserror::Error)]
pub enum ProjectionError {
    #[error("budget projection query failed: {0}")]
    Query(#[from] sqlx::Error),
    #[error("budget projection session store failed: {0}")]
    Store(String),
    #[error("budget projection data is invalid: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone)]
struct SessionNode {
    id: String,
    parent_id: Option<String>,
}

#[derive(Debug, Clone)]
struct LedgerEvidenceRow {
    session_id: String,
    task_id: Option<String>,
    cost_usd: f64,
    billing_class: String,
    provider: Option<String>,
    model: Option<String>,
    call_id: String,
    is_estimated: bool,
    observed_at: String,
}

#[derive(Debug, Clone)]
struct ReservationEvidenceRow {
    session_id: String,
    task_id: Option<String>,
    amount_usd: f64,
    state: String,
}

#[derive(Debug, Clone, Default)]
struct ScopeAggregate {
    settled_usd: f64,
    held_usd: f64,
    unknown_usd: f64,
    unpriced_calls: u32,
    active_reservations: bool,
    invalid_error: Option<String>,
}

struct ProjectionInputs {
    root_session_id: String,
    task_id: Option<String>,
    sessions: Vec<SessionNode>,
    task_ledger: Vec<LedgerEvidenceRow>,
    session_ledger: Vec<LedgerEvidenceRow>,
    task_reservations: Vec<ReservationEvidenceRow>,
    session_reservations: Vec<ReservationEvidenceRow>,
    task_aggregate: Option<ScopeAggregate>,
    session_aggregate: Option<ScopeAggregate>,
    task_latest_ledger: Option<LedgerEvidenceRow>,
    session_latest_ledger: Option<LedgerEvidenceRow>,
    config: BudgetConfig,
}

impl CapTriplet {
    fn from_scope(scope: crate::cost_router::budget::BudgetCeilings) -> Result<Self, String> {
        // Authorization uses BudgetCeilings::sanitized(); use the same
        // canonical normalization so an inverted finite config cannot make
        // the projection disagree with the gate thresholds.
        let scope = scope.sanitized();
        let values = [scope.soft, scope.gate, scope.hard];
        if values
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
            || scope.soft > scope.gate
            || scope.gate > scope.hard
        {
            return Err("budget cap is non-finite, negative, or unordered".to_string());
        }
        Ok(Self {
            soft_usd: Some(scope.soft),
            gate_usd: Some(scope.gate),
            hard_usd: Some(scope.hard),
            source: "current_budget_config".to_string(),
        })
    }

    fn unavailable(error: &str) -> Self {
        Self {
            soft_usd: None,
            gate_usd: None,
            hard_usd: None,
            source: format!("unavailable:{error}"),
        }
    }
}

impl BudgetScopeProjection {
    fn unknown(cap: CapTriplet, error: impl Into<String>) -> Self {
        Self {
            cap,
            settled_usd: None,
            held_usd: None,
            unknown_usd: None,
            effective_used_usd: None,
            remaining_usd: None,
            band: Some(ProjectionBand::Unknown),
            completeness: ProjectionCompleteness::Unknown,
            error: Some(error.into()),
        }
    }
}

fn read_ledger_row(row: sqlx::sqlite::SqliteRow) -> Result<LedgerEvidenceRow, sqlx::Error> {
    use sqlx::Row;
    Ok(LedgerEvidenceRow {
        session_id: row.try_get("session_id")?,
        task_id: row.try_get("task_id")?,
        cost_usd: row.try_get("cost_usd")?,
        billing_class: row.try_get("cost_tier")?,
        provider: row.try_get("provider")?,
        model: row.try_get("model")?,
        call_id: row.try_get("call_id")?,
        is_estimated: row.try_get::<i64, _>("is_estimated")? != 0,
        observed_at: row.try_get("ts")?,
    })
}

fn summary_from_sql(
    row: &sqlx::sqlite::SqliteRow,
    prefix: &str,
) -> Result<ScopeAggregate, sqlx::Error> {
    use sqlx::Row;
    let invalid_amount_name = format!("{prefix}_invalid_amount");
    let invalid_amount: i64 = row.try_get(invalid_amount_name.as_str())?;
    let invalid_timestamp = if prefix == "task" || prefix == "session" {
        let name = format!("{prefix}_invalid_timestamp");
        row.try_get::<i64, _>(name.as_str())?
    } else {
        0
    };
    let invalid_reservation_name = format!("{prefix}_invalid_reservation");
    let invalid_reservation: i64 = row.try_get(invalid_reservation_name.as_str())?;
    let invalid_error = if invalid_amount > 0 || invalid_timestamp > 0 || invalid_reservation > 0 {
        let mut reasons = Vec::new();
        if invalid_amount > 0 {
            reasons.push("ledger or reservation amount is invalid");
        }
        if invalid_timestamp > 0 {
            reasons.push("ledger timestamp is invalid");
        }
        if invalid_reservation > 0 {
            reasons.push("reservation amount is invalid");
        }
        Some(reasons.join("; "))
    } else {
        None
    };
    let unpriced_name = format!("{prefix}_unpriced_calls");
    let unpriced_calls: i64 = row.try_get(unpriced_name.as_str())?;
    let active_name = format!("{prefix}_active_reservations");
    let active_reservations: i64 = row.try_get(active_name.as_str())?;
    let settled_name = format!("{prefix}_settled");
    let held_name = format!("{prefix}_held");
    let unknown_name = format!("{prefix}_unknown");
    Ok(ScopeAggregate {
        settled_usd: row.try_get(settled_name.as_str())?,
        held_usd: row.try_get(held_name.as_str())?,
        unknown_usd: row.try_get(unknown_name.as_str())?,
        unpriced_calls: u32::try_from(unpriced_calls).unwrap_or(u32::MAX),
        active_reservations: active_reservations > 0,
        invalid_error,
    })
}

impl BudgetProjection {
    /// Recompute the projection from one canonical SessionManager/Spectral
    /// query seam. This is the only production entry point; callers must not
    /// add session rollups or copied snapshots to these values.
    pub async fn query(
        manager: &SessionManager,
        root_session_id: &str,
        config: BudgetConfig,
    ) -> Result<Self, ProjectionError> {
        let pool = manager
            .pool_clone()
            .await
            .map_err(|error| ProjectionError::Store(error.to_string()))?;
        let mut tx = pool.begin().await?;
        // A caller can hand us any descendant session. Resolve the canonical
        // top-level root first so parent, sibling, and grandchild callers all
        // observe the same projection. UNION (rather than UNION ALL) makes a
        // malformed parent cycle terminate safely; no root then fails closed.
        let canonical_root_id = sqlx::query_scalar::<_, String>(
            "WITH RECURSIVE ancestors(id, parent_id) AS (
                 SELECT id, parent_session_id FROM sessions WHERE id = ?
                 UNION
                 SELECT s.id, s.parent_session_id
                 FROM sessions s JOIN ancestors a ON s.id = a.parent_id
             )
             SELECT id FROM ancestors WHERE parent_id IS NULL LIMIT 1",
        )
        .bind(root_session_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| {
            ProjectionError::Invalid(
                "session does not resolve to a canonical top-level root".to_string(),
            )
        })?;
        let root = sqlx::query_as::<_, (String, Option<String>, String)>(
            "SELECT id, parent_session_id, extension_data FROM sessions WHERE id = ?",
        )
        .bind(&canonical_root_id)
        .fetch_one(&mut *tx)
        .await?;
        let task_id = serde_json::from_str::<crate::session::ExtensionData>(&root.2)
            .ok()
            .and_then(|data| crate::session::budget_task_id(&data));

        // All scope totals and evidence are read in this one transaction. The
        // first SELECT establishes SQLite's read snapshot, preventing a
        // concurrent ledger append from producing torn task/session totals.
        let aggregate_row = sqlx::query(
            "WITH RECURSIVE session_tree(id) AS (
                 SELECT id FROM sessions WHERE id = ?
                 UNION
                 SELECT s.id FROM sessions s JOIN session_tree t
                   ON s.parent_session_id = t.id
             ),
             task_ledger AS (
                 SELECT * FROM cost_ledger WHERE task_id = ?
             ),
             session_ledger AS (
                 SELECT l.* FROM cost_ledger l JOIN session_tree t ON t.id = l.session_id
             ),
             task_reservations AS (
                 SELECT * FROM cost_reservations
                 WHERE task_id = ? AND state IN ('pending', 'unknown')
             ),
             session_reservations AS (
                 SELECT r.* FROM cost_reservations r JOIN session_tree t ON t.id = r.session_id
                 WHERE r.state IN ('pending', 'unknown')
             )
             SELECT
               (SELECT total(CASE WHEN cost_usd IS NOT NULL AND cost_usd >= 0
                                  AND cost_usd = cost_usd THEN cost_usd ELSE 0.0 END)
                  FROM task_ledger) AS task_settled,
               (SELECT COALESCE(SUM(CASE WHEN cost_usd IS NULL OR cost_usd < 0
                                  OR cost_usd != cost_usd THEN 1 ELSE 0 END), 0)
                  FROM task_ledger) AS task_invalid_amount,
               (SELECT COALESCE(SUM(CASE WHEN ts IS NULL OR julianday(ts) IS NULL
                                  THEN 1 ELSE 0 END), 0) FROM task_ledger)
                  AS task_invalid_timestamp,
               (SELECT COALESCE(SUM(CASE WHEN is_estimated = 1 THEN 1 ELSE 0 END), 0)
                  FROM task_ledger) AS task_unpriced_calls,
               (SELECT total(CASE WHEN state = 'pending' AND amount_usd IS NOT NULL
                                  AND amount_usd >= 0 AND amount_usd = amount_usd
                                  THEN amount_usd ELSE 0.0 END)
                  FROM task_reservations) AS task_held,
               (SELECT total(CASE WHEN state = 'unknown' AND amount_usd IS NOT NULL
                                  AND amount_usd >= 0 AND amount_usd = amount_usd
                                  THEN amount_usd ELSE 0.0 END)
                  FROM task_reservations) AS task_unknown,
               (SELECT COALESCE(SUM(CASE WHEN amount_usd IS NULL OR amount_usd < 0
                                  OR amount_usd != amount_usd THEN 1 ELSE 0 END), 0)
                  FROM task_reservations) AS task_invalid_reservation,
               (SELECT COUNT(*) FROM task_reservations) AS task_active_reservations,
               (SELECT total(CASE WHEN cost_usd IS NOT NULL AND cost_usd >= 0
                                  AND cost_usd = cost_usd THEN cost_usd ELSE 0.0 END)
                  FROM session_ledger) AS session_settled,
               (SELECT COALESCE(SUM(CASE WHEN cost_usd IS NULL OR cost_usd < 0
                                  OR cost_usd != cost_usd THEN 1 ELSE 0 END), 0)
                  FROM session_ledger) AS session_invalid_amount,
               (SELECT COALESCE(SUM(CASE WHEN ts IS NULL OR julianday(ts) IS NULL
                                  THEN 1 ELSE 0 END), 0) FROM session_ledger)
                  AS session_invalid_timestamp,
               (SELECT COALESCE(SUM(CASE WHEN is_estimated = 1 THEN 1 ELSE 0 END), 0)
                  FROM session_ledger) AS session_unpriced_calls,
               (SELECT total(CASE WHEN state = 'pending' AND amount_usd IS NOT NULL
                                  AND amount_usd >= 0 AND amount_usd = amount_usd
                                  THEN amount_usd ELSE 0.0 END)
                  FROM session_reservations) AS session_held,
               (SELECT total(CASE WHEN state = 'unknown' AND amount_usd IS NOT NULL
                                  AND amount_usd >= 0 AND amount_usd = amount_usd
                                  THEN amount_usd ELSE 0.0 END)
                  FROM session_reservations) AS session_unknown,
               (SELECT COALESCE(SUM(CASE WHEN amount_usd IS NULL OR amount_usd < 0
                                  OR amount_usd != amount_usd THEN 1 ELSE 0 END), 0)
                  FROM session_reservations) AS session_invalid_reservation,
               (SELECT COUNT(*) FROM session_reservations) AS session_active_reservations",
        )
        .bind(&canonical_root_id)
        .bind(task_id.as_deref())
        .bind(task_id.as_deref())
        .fetch_one(&mut *tx)
        .await?;
        let task_aggregate = summary_from_sql(&aggregate_row, "task")?;
        let session_aggregate = summary_from_sql(&aggregate_row, "session")?;

        // Only one candidate per scope is fetched for billing provenance. SQL
        // orders by parsed Julian time and call_id; Rust parses RFC3339 again
        // before exposing it, so malformed timestamps cannot silently win.
        let task_latest_ledger = sqlx::query(
            "SELECT session_id, task_id, cost_usd, cost_tier, provider, model, call_id,
                    is_estimated, ts
             FROM cost_ledger
             WHERE task_id = ? AND ts IS NOT NULL AND julianday(ts) IS NOT NULL
             ORDER BY julianday(ts) DESC, call_id DESC LIMIT 1",
        )
        .bind(task_id.as_deref())
        .fetch_optional(&mut *tx)
        .await?
        .map(read_ledger_row)
        .transpose()?;
        let session_latest_ledger = sqlx::query(
            "WITH RECURSIVE session_tree(id) AS (
                 SELECT id FROM sessions WHERE id = ?
                 UNION
                 SELECT s.id FROM sessions s JOIN session_tree t
                   ON s.parent_session_id = t.id
             )
             SELECT l.session_id, l.task_id, l.cost_usd, l.cost_tier,
                    l.provider, l.model, l.call_id, l.is_estimated, l.ts
             FROM cost_ledger l JOIN session_tree t ON t.id = l.session_id
             WHERE l.ts IS NOT NULL AND julianday(l.ts) IS NOT NULL
             ORDER BY julianday(l.ts) DESC, l.call_id DESC LIMIT 1",
        )
        .bind(&canonical_root_id)
        .fetch_optional(&mut *tx)
        .await?
        .map(read_ledger_row)
        .transpose()?;

        tx.commit().await?;

        Ok(Self::from_inputs(ProjectionInputs {
            root_session_id: root.0,
            task_id,
            sessions: vec![SessionNode {
                id: canonical_root_id.clone(),
                parent_id: None,
            }],
            task_ledger: Vec::new(),
            session_ledger: Vec::new(),
            task_reservations: Vec::new(),
            session_reservations: Vec::new(),
            task_aggregate: Some(task_aggregate),
            session_aggregate: Some(session_aggregate),
            task_latest_ledger,
            session_latest_ledger,
            config,
        }))
    }

    fn from_inputs(inputs: ProjectionInputs) -> Self {
        let as_of = Utc::now().to_rfc3339();
        let descendants = descendant_ids(&inputs.root_session_id, &inputs.sessions);
        let task_cap = CapTriplet::from_scope(inputs.config.task);
        let session_cap = CapTriplet::from_scope(inputs.config.session);
        let task = match (&inputs.task_id, task_cap, inputs.task_aggregate.as_ref()) {
            (None, _, _) => BudgetScopeProjection::unknown(
                CapTriplet::unavailable("unbound_task"),
                "session is not bound to a durable task",
            ),
            (Some(_), Err(error), _) => {
                BudgetScopeProjection::unknown(CapTriplet::unavailable(&error), error)
            }
            (Some(_), Ok(cap), Some(summary)) => scope_from_aggregate(cap, summary),
            (Some(task_id), Ok(cap), None) => scope_from_rows(
                cap,
                inputs
                    .task_ledger
                    .iter()
                    .filter(|row| row.task_id.as_deref() == Some(task_id.as_str())),
                inputs
                    .task_reservations
                    .iter()
                    .filter(|row| row.task_id.as_deref() == Some(task_id.as_str())),
            ),
        };
        let session = match (session_cap, inputs.session_aggregate.as_ref()) {
            (Err(error), _) => {
                BudgetScopeProjection::unknown(CapTriplet::unavailable(&error), error)
            }
            (Ok(cap), Some(summary)) => scope_from_aggregate(cap, summary),
            (Ok(cap), None) => scope_from_rows(
                cap,
                inputs
                    .session_ledger
                    .iter()
                    .filter(|row| descendants.contains(&row.session_id)),
                inputs
                    .session_reservations
                    .iter()
                    .filter(|row| descendants.contains(&row.session_id)),
            ),
        };

        let task_billing = if let Some(summary) = inputs.task_aggregate.as_ref() {
            billing_from_query(inputs.task_latest_ledger.as_ref(), summary)
        } else {
            billing_evidence(
                inputs.task_ledger.iter().filter(|row| {
                    inputs.task_id.is_some() && inputs.task_id.as_deref() == row.task_id.as_deref()
                }),
                inputs.task_reservations.iter().filter(|row| {
                    inputs.task_id.is_some() && inputs.task_id.as_deref() == row.task_id.as_deref()
                }),
            )
        };
        let session_billing = if let Some(summary) = inputs.session_aggregate.as_ref() {
            billing_from_query(inputs.session_latest_ledger.as_ref(), summary)
        } else {
            billing_evidence(
                inputs
                    .session_ledger
                    .iter()
                    .filter(|row| descendants.contains(&row.session_id)),
                inputs
                    .session_reservations
                    .iter()
                    .filter(|row| descendants.contains(&row.session_id)),
            )
        };
        let mut errors = Vec::new();
        if let Some(error) = &task.error {
            errors.push(format!("task: {error}"));
        }
        if let Some(error) = &session.error {
            errors.push(format!("session: {error}"));
        }
        let completeness = if errors.is_empty()
            && task.completeness == ProjectionCompleteness::Complete
            && session.completeness == ProjectionCompleteness::Complete
        {
            ProjectionCompleteness::Complete
        } else if task.completeness == ProjectionCompleteness::Partial
            || session.completeness == ProjectionCompleteness::Partial
            || inputs.task_id.is_none()
        {
            ProjectionCompleteness::Partial
        } else {
            ProjectionCompleteness::Unknown
        };
        BudgetProjection {
            task_id: inputs.task_id,
            root_session_id: inputs.root_session_id,
            task,
            session,
            task_billing,
            session_billing,
            provenance: ProjectionProvenance {
                version: BUDGET_PROJECTION_VERSION.to_string(),
                as_of,
                completeness,
                sources: vec![
                    "sessions".to_string(),
                    "cost_ledger".to_string(),
                    "cost_reservations".to_string(),
                    "current_budget_config".to_string(),
                ],
                error: (!errors.is_empty()).then(|| errors.join("; ")),
            },
        }
    }
}

fn descendant_ids(root: &str, sessions: &[SessionNode]) -> HashSet<String> {
    let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
    for session in sessions {
        children
            .entry(session.parent_id.as_deref().unwrap_or(""))
            .or_default()
            .push(&session.id);
    }
    let mut result = HashSet::from([root.to_string()]);
    let mut queue = VecDeque::from([root.to_string()]);
    while let Some(parent) = queue.pop_front() {
        for child in children.get(parent.as_str()).into_iter().flatten() {
            if result.insert((*child).to_string()) {
                queue.push_back((*child).to_string());
            }
        }
    }
    result
}

fn scope_from_rows<'a>(
    cap: CapTriplet,
    ledger: impl Iterator<Item = &'a LedgerEvidenceRow>,
    reservations: impl Iterator<Item = &'a ReservationEvidenceRow>,
) -> BudgetScopeProjection {
    let mut summary = ScopeAggregate::default();
    for row in ledger {
        if !row.cost_usd.is_finite() || row.cost_usd < 0.0 {
            summary.invalid_error = Some("ledger amount is invalid".to_string());
        } else {
            summary.settled_usd += row.cost_usd;
        }
        summary.unpriced_calls = summary
            .unpriced_calls
            .saturating_add(row.is_estimated as u32);
        if DateTime::parse_from_rfc3339(&row.observed_at).is_err() {
            summary.invalid_error = Some("ledger timestamp is invalid".to_string());
        }
    }
    for row in reservations {
        if !row.amount_usd.is_finite() || row.amount_usd < 0.0 {
            summary.invalid_error = Some("reservation amount is invalid".to_string());
        } else {
            match row.state.as_str() {
                "pending" => summary.held_usd += row.amount_usd,
                "unknown" => summary.unknown_usd += row.amount_usd,
                _ => summary.invalid_error = Some("reservation state is invalid".to_string()),
            }
        }
        summary.active_reservations = true;
    }
    scope_from_aggregate(cap, &summary)
}

fn scope_from_aggregate(cap: CapTriplet, summary: &ScopeAggregate) -> BudgetScopeProjection {
    if let Some(error) = &summary.invalid_error {
        return BudgetScopeProjection::unknown(cap, error.clone());
    }
    let Some(hard) = cap.hard_usd else {
        return BudgetScopeProjection::unknown(cap, "cap is unavailable");
    };
    if !summary.settled_usd.is_finite()
        || !summary.held_usd.is_finite()
        || !summary.unknown_usd.is_finite()
        || summary.settled_usd < 0.0
        || summary.held_usd < 0.0
        || summary.unknown_usd < 0.0
    {
        return BudgetScopeProjection::unknown(cap, "effective spend is invalid");
    }
    let used = summary.settled_usd + summary.held_usd + summary.unknown_usd;
    if !used.is_finite() {
        return BudgetScopeProjection::unknown(cap, "effective spend is non-finite");
    }
    let mut band = if summary.unknown_usd > 0.0 {
        ProjectionBand::Unknown
    } else if used >= hard {
        ProjectionBand::Hard
    } else if used >= cap.gate_usd.unwrap_or(hard) {
        ProjectionBand::Gate
    } else if used >= cap.soft_usd.unwrap_or(hard) {
        ProjectionBand::Soft
    } else {
        ProjectionBand::Ok
    };
    if summary.unpriced_calls > 0 && band == ProjectionBand::Ok {
        band = ProjectionBand::Soft;
    }
    BudgetScopeProjection {
        cap,
        settled_usd: Some(summary.settled_usd),
        held_usd: Some(summary.held_usd),
        unknown_usd: Some(summary.unknown_usd),
        effective_used_usd: Some(used),
        remaining_usd: Some((hard - used).max(0.0)),
        band: Some(band),
        completeness: if summary.unknown_usd > 0.0 || summary.unpriced_calls > 0 {
            ProjectionCompleteness::Partial
        } else {
            ProjectionCompleteness::Complete
        },
        error: None,
    }
}

fn billing_from_query(
    latest_ledger: Option<&LedgerEvidenceRow>,
    summary: &ScopeAggregate,
) -> BillingEvidence {
    // An active hold is the strongest evidence: it represents a paid dispatch
    // which may not yet have a settled ledger row.
    if summary.active_reservations {
        return BillingEvidence {
            billing_class: Some("paid_api".to_string()),
            provider: None,
            model: None,
            call_id: None,
            is_estimated: None,
            observed_at: None,
            source: "cost_reservations".to_string(),
        };
    }
    latest_ledger
        .map(billing_from_ledger)
        .unwrap_or_else(|| BillingEvidence {
            billing_class: None,
            provider: None,
            model: None,
            call_id: None,
            is_estimated: None,
            observed_at: None,
            source: "no_billing_rows".to_string(),
        })
}

fn billing_from_ledger(row: &LedgerEvidenceRow) -> BillingEvidence {
    BillingEvidence {
        billing_class: Some(row.billing_class.clone()),
        provider: row.provider.clone(),
        model: row.model.clone(),
        call_id: Some(row.call_id.clone()),
        is_estimated: Some(row.is_estimated),
        observed_at: Some(row.observed_at.clone()),
        source: "cost_ledger".to_string(),
    }
}

fn billing_evidence<'a>(
    ledger: impl Iterator<Item = &'a LedgerEvidenceRow>,
    mut reservations: impl Iterator<Item = &'a ReservationEvidenceRow>,
) -> BillingEvidence {
    // Holds outrank ledger evidence even when the ledger has a newer-looking
    // string timestamp: an active paid dispatch is the current billing fact.
    if reservations.next().is_some() {
        return BillingEvidence {
            billing_class: Some("paid_api".to_string()),
            provider: None,
            model: None,
            call_id: None,
            is_estimated: None,
            observed_at: None,
            source: "cost_reservations".to_string(),
        };
    }
    let mut malformed = false;
    let latest = ledger
        .filter_map(|row| {
            let parsed = DateTime::parse_from_rfc3339(&row.observed_at);
            if parsed.is_err() {
                malformed = true;
                return None;
            }
            Some((parsed.unwrap(), row))
        })
        .max_by(|(left_time, left), (right_time, right)| {
            left_time
                .cmp(right_time)
                .then_with(|| left.call_id.cmp(&right.call_id))
        });
    if malformed {
        return BillingEvidence {
            billing_class: None,
            provider: None,
            model: None,
            call_id: None,
            is_estimated: None,
            observed_at: None,
            source: "invalid_ledger_timestamp".to_string(),
        };
    }
    if let Some((_, row)) = latest {
        return billing_from_ledger(row);
    }
    BillingEvidence {
        billing_class: None,
        provider: None,
        model: None,
        call_id: None,
        is_estimated: None,
        observed_at: None,
        source: "no_billing_rows".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> BudgetConfig {
        crate::cost_router::budget::budget_config_from(
            Some(1.0),
            Some(3.0),
            Some(5.0),
            Some(2.0),
            Some(6.0),
            Some(10.0),
        )
    }

    fn inputs() -> ProjectionInputs {
        ProjectionInputs {
            root_session_id: "root".to_string(),
            task_id: Some("task".to_string()),
            sessions: vec![
                SessionNode {
                    id: "root".to_string(),
                    parent_id: None,
                },
                SessionNode {
                    id: "child".to_string(),
                    parent_id: Some("root".to_string()),
                },
                SessionNode {
                    id: "grandchild".to_string(),
                    parent_id: Some("child".to_string()),
                },
                SessionNode {
                    id: "sibling".to_string(),
                    parent_id: None,
                },
            ],
            task_ledger: Vec::new(),
            session_ledger: Vec::new(),
            task_reservations: Vec::new(),
            session_reservations: Vec::new(),
            task_aggregate: None,
            session_aggregate: None,
            task_latest_ledger: None,
            session_latest_ledger: None,
            config: config(),
        }
    }

    #[test]
    fn zero_is_authoritative_and_exact_or_over_cap_is_clamped() {
        let zero = BudgetProjection::from_inputs(inputs());
        assert_eq!(zero.task.settled_usd, Some(0.0));
        assert_eq!(zero.session.remaining_usd, Some(10.0));
        let mut exact = inputs();
        exact.session_ledger.push(ledger("root", 10.0));
        let projection = BudgetProjection::from_inputs(exact);
        assert_eq!(projection.session.band, Some(ProjectionBand::Hard));
        assert_eq!(projection.session.remaining_usd, Some(0.0));
        let mut over = inputs();
        over.session_ledger.push(ledger("root", 11.0));
        let projection = BudgetProjection::from_inputs(over);
        assert_eq!(projection.session.band, Some(ProjectionBand::Hard));
        assert_eq!(projection.session.remaining_usd, Some(0.0));
    }

    #[test]
    fn pending_and_unknown_are_separate_and_unknown_is_not_zero() {
        let mut data = inputs();
        data.session_reservations
            .push(reservation("child", 2.0, "pending"));
        data.session_reservations
            .push(reservation("grandchild", 4.0, "unknown"));
        let projection = BudgetProjection::from_inputs(data);
        assert_eq!(projection.session.held_usd, Some(2.0));
        assert_eq!(projection.session.unknown_usd, Some(4.0));
        assert_eq!(projection.session.effective_used_usd, Some(6.0));
        assert_eq!(projection.session.band, Some(ProjectionBand::Unknown));
    }

    #[test]
    fn recursive_descendants_are_included_once_and_siblings_are_excluded() {
        let mut data = inputs();
        data.session_ledger.push(ledger("grandchild", 2.0));
        data.session_ledger.push(ledger("sibling", 9.0));
        let projection = BudgetProjection::from_inputs(data);
        assert_eq!(projection.session.settled_usd, Some(2.0));
    }

    #[test]
    fn invalid_values_and_unbound_tasks_fail_closed_without_fabricating_zero() {
        let mut invalid = inputs();
        invalid.session_ledger.push(ledger("root", f64::NAN));
        let projection = BudgetProjection::from_inputs(invalid);
        assert_eq!(projection.session.settled_usd, None);
        assert_eq!(projection.session.remaining_usd, None);
        let mut negative = inputs();
        negative
            .session_reservations
            .push(reservation("root", -1.0, "pending"));
        assert_eq!(
            BudgetProjection::from_inputs(negative)
                .session
                .effective_used_usd,
            None
        );
        let mut unbound = inputs();
        unbound.task_id = None;
        let projection = BudgetProjection::from_inputs(unbound);
        assert_eq!(projection.task.settled_usd, None);
        assert_eq!(
            projection.provenance.completeness,
            ProjectionCompleteness::Partial
        );
    }

    #[test]
    fn invalid_caps_and_query_failures_have_explicit_unknown_states() {
        let mut invalid = inputs();
        invalid.config.task.hard = f64::INFINITY;
        let projection = BudgetProjection::from_inputs(invalid);
        assert_eq!(projection.task.band, Some(ProjectionBand::Unknown));
        assert!(projection.task.error.is_some());
        let unknown = BudgetScopeProjection::unknown(
            CapTriplet::unavailable("query_failure"),
            "spectral query failed",
        );
        assert_eq!(unknown.settled_usd, None);
        assert_eq!(unknown.remaining_usd, None);
    }

    #[test]
    fn estimated_ledger_rows_match_authorization_unpriced_band() {
        let mut data = inputs();
        data.task_ledger
            .push(ledger_with_estimate("root", 0.0, true));
        data.session_ledger
            .push(ledger_with_estimate("root", 0.0, true));
        let projection = BudgetProjection::from_inputs(data);
        let verdict =
            crate::cost_router::budget::budget_verdict_with_unpriced(0.0, 0.0, 1, &config());
        assert_eq!(verdict.band, crate::cost_router::budget::BudgetBand::Soft);
        assert_eq!(projection.task.band, Some(ProjectionBand::Soft));
        assert_eq!(projection.session.band, Some(ProjectionBand::Soft));
        assert_eq!(
            projection.session.completeness,
            ProjectionCompleteness::Partial
        );
    }

    #[test]
    fn billing_timestamp_order_is_chronological_and_tie_breaks_by_call_id() {
        let mut data = inputs();
        let mut earlier_lexically = ledger("root", 1.0);
        earlier_lexically.call_id = "call-a".to_string();
        earlier_lexically.observed_at = "2025-12-31T19:00:00-05:00".to_string();
        let mut later_lexically = ledger("child", 1.0);
        later_lexically.call_id = "call-z".to_string();
        later_lexically.observed_at = "2026-01-01T00:00:00+00:00".to_string();
        data.session_ledger.push(earlier_lexically);
        data.session_ledger.push(later_lexically);
        let projection = BudgetProjection::from_inputs(data);
        assert_eq!(
            projection.session_billing.call_id.as_deref(),
            Some("call-z")
        );
        let mut tie = inputs();
        let mut first = ledger("root", 1.0);
        first.call_id = "call-a".to_string();
        first.observed_at = "2026-01-01T00:00:00Z".to_string();
        let mut second = ledger("child", 1.0);
        second.call_id = "call-z".to_string();
        second.observed_at = "2025-12-31T19:00:00-05:00".to_string();
        tie.session_ledger.extend([first, second]);
        assert_eq!(
            BudgetProjection::from_inputs(tie)
                .session_billing
                .call_id
                .as_deref(),
            Some("call-z")
        );
    }

    #[test]
    fn malformed_ledger_timestamp_fails_closed_instead_of_winning_evidence() {
        let mut data = inputs();
        data.session_ledger.push(ledger("root", 1.0));
        let mut malformed = ledger("child", 1.0);
        malformed.observed_at = "9999-not-a-timestamp".to_string();
        data.session_ledger.push(malformed);
        let projection = BudgetProjection::from_inputs(data);
        assert_eq!(projection.session.settled_usd, None);
        assert_eq!(
            projection.session_billing.source,
            "invalid_ledger_timestamp"
        );
    }

    #[test]
    fn cap_normalization_matches_authorization_for_inverted_finite_values() {
        let normalized = crate::cost_router::budget::budget_config_from(
            Some(8.0),
            Some(2.0),
            Some(1.0),
            Some(9.0),
            Some(3.0),
            Some(2.0),
        );
        let mut normalized_inputs = inputs();
        normalized_inputs.config = normalized;
        normalized_inputs.task_ledger.push(ledger("root", 8.0));
        normalized_inputs.session_ledger.push(ledger("root", 9.0));
        let projection = BudgetProjection::from_inputs(normalized_inputs);
        let verdict = crate::cost_router::budget::budget_verdict(8.0, 9.0, &normalized);
        assert_eq!(verdict.band, crate::cost_router::budget::BudgetBand::Hard);
        assert_eq!(projection.task.cap.soft_usd, Some(8.0));
        assert_eq!(projection.task.cap.gate_usd, Some(8.0));
        assert_eq!(projection.task.cap.hard_usd, Some(8.0));
        assert_eq!(projection.task.band, Some(ProjectionBand::Hard));
        assert_eq!(projection.session.band, Some(ProjectionBand::Hard));
    }

    #[test]
    fn projection_serialization_has_stable_golden_shape() {
        let mut projection = BudgetProjection::from_inputs(inputs());
        projection.provenance.as_of = "2026-09-05T00:00:00Z".to_string();
        let json = serde_json::to_value(&projection).unwrap();
        assert_eq!(json["taskId"], "task");
        assert_eq!(json["rootSessionId"], "root");
        assert_eq!(json["task"]["cap"]["softUsd"], 1.0);
        assert_eq!(json["task"]["cap"]["gateUsd"], 3.0);
        assert_eq!(json["task"]["cap"]["hardUsd"], 5.0);
        assert_eq!(json["task"]["settledUsd"], 0.0);
        assert_eq!(json["session"]["remainingUsd"], 10.0);
        assert_eq!(json["provenance"]["version"], BUDGET_PROJECTION_VERSION);
        assert_eq!(json["provenance"]["asOf"], "2026-09-05T00:00:00Z");
        let round_trip: BudgetProjection = serde_json::from_value(json).unwrap();
        assert_eq!(round_trip, projection);
    }

    #[tokio::test]
    async fn query_resolves_grandchild_to_root_and_scopes_tree_rows() {
        let temp = tempfile::tempdir().unwrap();
        let manager = SessionManager::new(temp.path().to_path_buf());
        let root = manager
            .create_session(
                temp.path().to_path_buf(),
                "root".to_string(),
                crate::session::SessionType::User,
                crate::config::GooseMode::Auto,
            )
            .await
            .unwrap();
        let task_id = manager.begin_budget_task(&root.id).await.unwrap();
        let child = manager
            .create_session_with_parent(
                Some(&root.id),
                temp.path().to_path_buf(),
                "child".to_string(),
                crate::session::SessionType::SubAgent,
                crate::config::GooseMode::Auto,
            )
            .await
            .unwrap();
        let grandchild = manager
            .create_session_with_parent(
                Some(&child.id),
                temp.path().to_path_buf(),
                "grandchild".to_string(),
                crate::session::SessionType::SubAgent,
                crate::config::GooseMode::Auto,
            )
            .await
            .unwrap();
        let sibling = manager
            .create_session_with_parent(
                Some(&root.id),
                temp.path().to_path_buf(),
                "sibling".to_string(),
                crate::session::SessionType::SubAgent,
                crate::config::GooseMode::Auto,
            )
            .await
            .unwrap();
        let unrelated_root = manager
            .create_session(
                temp.path().to_path_buf(),
                "unrelated".to_string(),
                crate::session::SessionType::User,
                crate::config::GooseMode::Auto,
            )
            .await
            .unwrap();
        let pool = manager.pool_clone().await.unwrap();
        for (call_id, session_id, task, amount) in [
            ("root-call", root.id.as_str(), Some(task_id.as_str()), 1.0),
            (
                "grandchild-call",
                grandchild.id.as_str(),
                Some(task_id.as_str()),
                2.0,
            ),
            (
                "sibling-call",
                sibling.id.as_str(),
                Some(task_id.as_str()),
                4.0,
            ),
            ("unrelated-call", unrelated_root.id.as_str(), None, 9.0),
        ] {
            sqlx::query(
                "INSERT INTO cost_ledger
                    (call_id, ts, session_id, task_id, cost_tier, cost_usd)
                 VALUES (?, ?, ?, ?, 'paid_api', ?)",
            )
            .bind(call_id)
            .bind("2026-01-01T00:00:00Z")
            .bind(session_id)
            .bind(task)
            .bind(amount)
            .execute(&pool)
            .await
            .unwrap();
        }
        for (id, session_id, state, amount) in [
            ("pending", child.id.as_str(), "pending", 2.0),
            ("unknown", grandchild.id.as_str(), "unknown", 3.0),
        ] {
            sqlx::query(
                "INSERT INTO cost_reservations
                    (reservation_id, invocation_id, session_id, task_id, amount_usd,
                     state, lease_until, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(format!("reservation-{id}"))
            .bind(format!("invocation-{id}"))
            .bind(session_id)
            .bind(&task_id)
            .bind(amount)
            .bind(state)
            .bind("2099-01-01T00:00:00Z")
            .bind("2026-01-01T00:00:00Z")
            .bind("2026-01-01T00:00:00Z")
            .execute(&pool)
            .await
            .unwrap();
        }
        let parent_projection = BudgetProjection::query(&manager, &root.id, config())
            .await
            .unwrap();
        let grandchild_projection = BudgetProjection::query(&manager, &grandchild.id, config())
            .await
            .unwrap();
        assert_eq!(parent_projection.task_id, Some(task_id));
        assert_eq!(parent_projection.root_session_id, root.id);
        assert_eq!(
            grandchild_projection.root_session_id,
            parent_projection.root_session_id
        );
        assert_eq!(grandchild_projection.task, parent_projection.task);
        assert_eq!(grandchild_projection.session, parent_projection.session);
        assert_eq!(parent_projection.task.settled_usd, Some(7.0));
        assert_eq!(parent_projection.session.settled_usd, Some(7.0));
        assert_eq!(parent_projection.session.held_usd, Some(2.0));
        assert_eq!(parent_projection.session.unknown_usd, Some(3.0));
        assert_eq!(
            parent_projection.session.band,
            Some(ProjectionBand::Unknown)
        );
        assert_eq!(parent_projection.task_billing.source, "cost_reservations");
        assert_eq!(
            parent_projection.session_billing.source,
            "cost_reservations"
        );
    }

    #[tokio::test]
    async fn parent_cycle_fails_closed_without_an_unbounded_recursive_walk() {
        let temp = tempfile::tempdir().unwrap();
        let manager = SessionManager::new(temp.path().to_path_buf());
        let root = manager
            .create_session(
                temp.path().to_path_buf(),
                "cycle-root".to_string(),
                crate::session::SessionType::User,
                crate::config::GooseMode::Auto,
            )
            .await
            .unwrap();
        let child = manager
            .create_session_with_parent(
                Some(&root.id),
                temp.path().to_path_buf(),
                "cycle-child".to_string(),
                crate::session::SessionType::SubAgent,
                crate::config::GooseMode::Auto,
            )
            .await
            .unwrap();
        let pool = manager.pool_clone().await.unwrap();
        sqlx::query("UPDATE sessions SET parent_session_id = ? WHERE id = ?")
            .bind(&child.id)
            .bind(&root.id)
            .execute(&pool)
            .await
            .unwrap();
        let error = BudgetProjection::query(&manager, &child.id, config())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("canonical top-level root"));
    }

    #[test]
    fn query_source_contract_is_bounded_by_task_and_recursive_tree() {
        let source = include_str!("budget_projection.rs");
        assert!(!source.contains("SELECT id, parent_session_id FROM sessions\""));
        let unbounded_fetch_method = ["fetch", "_all"].concat();
        assert!(!source.contains(unbounded_fetch_method.as_str()));
        assert!(source.contains("let mut tx = pool.begin().await?"));
        assert!(source.contains("tx.commit().await?"));
        assert!(source.contains("total(CASE WHEN cost_usd"));
        assert!(source.contains("LIMIT 1"));
        assert!(source.contains("WHERE task_id = ?"));
        assert!(source.contains("JOIN session_tree t ON t.id = l.session_id"));
        assert!(source.contains("JOIN session_tree t ON t.id = r.session_id"));
        let schema = include_str!("spectral_schema.rs");
        assert!(schema.contains("idx_sessions_parent ON sessions(parent_session_id)"));
        assert!(schema.contains("idx_cost_ledger_task ON cost_ledger(task_id, ts)"));
        assert!(schema.contains("idx_cost_ledger_session ON cost_ledger(session_id, ts)"));
        assert!(schema.contains("idx_cost_reservations_task_state"));
        assert!(schema.contains("idx_cost_reservations_session_state"));
    }

    fn ledger(session_id: &str, cost_usd: f64) -> LedgerEvidenceRow {
        ledger_with_estimate(session_id, cost_usd, false)
    }

    fn ledger_with_estimate(
        session_id: &str,
        cost_usd: f64,
        is_estimated: bool,
    ) -> LedgerEvidenceRow {
        LedgerEvidenceRow {
            session_id: session_id.to_string(),
            task_id: Some("task".to_string()),
            cost_usd,
            billing_class: "paid_api".to_string(),
            provider: Some("fixture".to_string()),
            model: Some("fixture-model".to_string()),
            call_id: format!("call-{session_id}"),
            is_estimated,
            observed_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    fn reservation(session_id: &str, amount_usd: f64, state: &str) -> ReservationEvidenceRow {
        ReservationEvidenceRow {
            session_id: session_id.to_string(),
            task_id: Some("task".to_string()),
            amount_usd,
            state: state.to_string(),
        }
    }
}
