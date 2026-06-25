//! Goal transition guard — the sole legal mutator of goal state.
//!
//! Every goal-state change in the daemon funnels through this module:
//! [`advance_goal_checked`] for the five legal state-machine actions, plus
//! the audited system paths [`park_goal`] and [`requeue_goal`], and the
//! Tier-2-gated [`delete_goal_checked`].
//!
//! Each checked advance runs as a single SQLite transaction:
//! re-read → `validate_transition` → `risk_policy` tier lookup → tier gate →
//! write → audit append. Tier-1/Tier-2 transitions demand a
//! [`DecisionProof`](crate::decisions::DecisionProof) — unforgeable outside
//! the decisions module (SafeBrain pattern) — so no handler, including the
//! orchestrator's own tools, can perform a gated transition without an
//! answered decision.
//!
//! Budgets (S4) also live here: attempt/token/wallclock caps stored under the
//! protected `budget` metadata key; exhaustion emits a `kind='unblock'`
//! decision and parks the goal — never a silent retry.

use sqlx::{Pool, Row, Sqlite};

use crate::decisions::{self, DecisionProof};
use crate::goal_state::{validate_transition, GoalAction, GoalState};

/// Default attempt cap when a goal carries no explicit budget
/// (replaces the former hardcoded MAX_GOAL_ATTEMPTS comparisons).
pub const DEFAULT_ATTEMPT_CAP: u64 = 3;

/// Metadata keys that only this guard may write on goal cards.
pub const PROTECTED_GOAL_METADATA_KEYS: &[&str] = &[
    "goal_state",
    "needs_human_attention",
    "attempt_count",
    "last_error",
    "budget",
    "completed_at",
];

// ── Errors ──────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum GuardError {
    /// Card or column not found.
    NotFound(String),
    /// Request shape / state-machine violation.
    Invalid(String),
    /// Tier gate refused the transition (maps to HTTP 403).
    Denied(String),
    /// Database failure.
    Db(String),
}

impl std::fmt::Display for GuardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(s) => write!(f, "Not found: {}", s),
            Self::Invalid(s) => write!(f, "Invalid: {}", s),
            Self::Denied(s) => write!(f, "Denied: {}", s),
            Self::Db(s) => write!(f, "Database error: {}", s),
        }
    }
}

impl From<GuardError> for String {
    fn from(e: GuardError) -> Self {
        e.to_string()
    }
}

fn db_err(e: sqlx::Error) -> GuardError {
    GuardError::Db(e.to_string())
}

// ── Checked advance ─────────────────────────────────────────────────────────

/// The risk_policy action class governing each goal action.
pub fn action_class_for(action: GoalAction) -> &'static str {
    match action {
        GoalAction::Ready => "goal_ready",
        GoalAction::Dispatch => "goal_dispatch",
        GoalAction::Review => "goal_review",
        GoalAction::Approve => "goal_approve_standard",
        GoalAction::Reject => "goal_retry_within_budget",
        GoalAction::Cancel => "goal_cancel",
    }
}

/// Side payload applied atomically with a checked transition.
///
/// `metadata_patch` may touch protected keys — this guard IS the legal writer.
/// A JSON `null` value removes the key. Internal trusted-code API only; it is
/// never reachable from HTTP or MCP tool input.
#[derive(Debug, Default)]
pub struct TransitionEffects {
    pub review_notes: Option<String>,
    pub metadata_patch: serde_json::Map<String, serde_json::Value>,
    pub assigned_to: Option<String>,
}

struct GoalRow {
    project_id: String,
    card_type: String,
    column_id: String,
    metadata: serde_json::Map<String, serde_json::Value>,
}

async fn read_goal_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    card_id: &str,
) -> Result<GoalRow, GuardError> {
    let row = sqlx::query(
        "SELECT project_id, card_type, column_id, metadata_json FROM cards WHERE id = ?",
    )
    .bind(card_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(db_err)?
    .ok_or_else(|| GuardError::NotFound(format!("Card '{}' not found", card_id)))?;

    let meta_str: String = row.get("metadata_json");
    let metadata = serde_json::from_str::<serde_json::Value>(&meta_str)
        .ok()
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();

    Ok(GoalRow {
        project_id: row.get("project_id"),
        card_type: row.get("card_type"),
        column_id: row.get("column_id"),
        metadata,
    })
}

async fn state_binding_of_column_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    column_id: &str,
) -> Result<Option<String>, GuardError> {
    sqlx::query_scalar::<_, Option<String>>("SELECT state_binding FROM board_columns WHERE id = ?")
        .bind(column_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(db_err)?
        .ok_or_else(|| GuardError::NotFound(format!("Column '{}' not found", column_id)))
}

async fn goal_column_id_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    project_id: &str,
    state_binding: &str,
) -> Result<String, GuardError> {
    sqlx::query_scalar::<_, String>(
        "SELECT id FROM board_columns WHERE project_id = ? AND state_binding = ?",
    )
    .bind(project_id)
    .bind(state_binding)
    .fetch_optional(&mut **tx)
    .await
    .map_err(db_err)?
    .ok_or_else(|| {
        GuardError::NotFound(format!(
            "Column for state '{}' not found in project",
            state_binding
        ))
    })
}

async fn next_position_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    column_id: &str,
) -> Result<i32, GuardError> {
    let max: Option<i32> = sqlx::query_scalar(
        "SELECT MAX(position) FROM cards WHERE column_id = ? AND archived_at IS NULL",
    )
    .bind(column_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(db_err)?;
    Ok(max.unwrap_or(-1) + 1)
}

fn apply_patch(
    meta: &mut serde_json::Map<String, serde_json::Value>,
    patch: &serde_json::Map<String, serde_json::Value>,
) {
    for (k, v) in patch {
        if v.is_null() {
            meta.remove(k);
        } else {
            meta.insert(k.clone(), v.clone());
        }
    }
}

/// Validate a proof against a tier gate and the target card/action.
fn check_proof(
    tier: i64,
    proof: Option<&DecisionProof>,
    card_id: &str,
    action: GoalAction,
) -> Result<(), GuardError> {
    if let Some(p) = proof {
        if let Some(gid) = p.goal_id() {
            if gid != card_id {
                return Err(GuardError::Denied(format!(
                    "Decision {} is for goal '{}', not '{}'",
                    p.decision_id(),
                    gid,
                    card_id
                )));
            }
        }
        match action {
            GoalAction::Approve if p.answer() != "approve" => {
                return Err(GuardError::Denied(format!(
                    "Action 'approve' requires a decision answered 'approve' (got '{}')",
                    p.answer()
                )));
            }
            GoalAction::Reject if p.answer() != "reject" => {
                return Err(GuardError::Denied(format!(
                    "Action 'reject' requires a decision answered 'reject' (got '{}')",
                    p.answer()
                )));
            }
            _ => {}
        }
    }

    match tier {
        0 => Ok(()),
        1 => match proof {
            Some(p)
                if p.acted_by() == decisions::ACTOR_JESSE
                    || p.acted_by() == decisions::ACTOR_HENRY =>
            {
                Ok(())
            }
            Some(p) => Err(GuardError::Denied(format!(
                "Tier-1 transition requires a decision answered by 'jesse' or 'henry-policy' \
                 (got '{}')",
                p.acted_by()
            ))),
            None => Err(GuardError::Denied(
                "Tier-1 transition requires an answered decision; none was provided. \
                 Answer the corresponding decision in the inbox."
                    .to_string(),
            )),
        },
        _ => match proof {
            Some(p) if p.acted_by() == decisions::ACTOR_JESSE => Ok(()),
            Some(p) => Err(GuardError::Denied(format!(
                "Tier-2 transition requires a decision answered by 'jesse' (got '{}')",
                p.acted_by()
            ))),
            None => Err(GuardError::Denied(
                "Tier-2 transition requires a decision answered by 'jesse'; none was provided."
                    .to_string(),
            )),
        },
    }
}

/// The sole legal mutator of goal lifecycle state.
///
/// Single transaction: re-read → validate_transition → risk_policy tier
/// lookup → tier gate (proof) → write (metadata + column) → audit append.
/// The proof is consumed by value: one answered decision authorizes at most
/// one transition.
pub async fn advance_goal_checked(
    pool: &Pool<Sqlite>,
    card_id: &str,
    action: GoalAction,
    actor: &str,
    proof: Option<DecisionProof>,
    effects: TransitionEffects,
) -> Result<GoalState, GuardError> {
    let mut tx = pool.begin().await.map_err(db_err)?;

    let goal = read_goal_tx(&mut tx, card_id).await?;
    if goal.card_type != "goal" {
        return Err(GuardError::Invalid(format!(
            "Card '{}' is type '{}', not 'goal'",
            card_id, goal.card_type
        )));
    }

    let binding = state_binding_of_column_tx(&mut tx, &goal.column_id).await?;
    let current_state = binding
        .as_deref()
        .and_then(GoalState::from_binding)
        .ok_or_else(|| {
            GuardError::Invalid(format!(
                "Card '{}' is in a column with no state_binding; goal cards must live in \
                 state-bound columns",
                card_id
            ))
        })?;

    let new_state = validate_transition(current_state, action)
        .map_err(|e| GuardError::Invalid(e.to_string()))?;

    // Risk tier (fail-closed for unknown classes).
    let action_class = action_class_for(action);
    let tier: i64 = sqlx::query_scalar("SELECT tier FROM risk_policy WHERE action_class = ?")
        .bind(action_class)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_err)?
        .unwrap_or(2);

    check_proof(tier, proof.as_ref(), card_id, action)?;

    // Build new metadata.
    let mut meta = goal.metadata;
    apply_patch(&mut meta, &effects.metadata_patch);
    if let Some(ref notes) = effects.review_notes {
        meta.insert(
            "review_notes".to_string(),
            serde_json::Value::String(notes.clone()),
        );
    }
    meta.insert(
        "goal_state".to_string(),
        serde_json::Value::String(new_state.binding().to_string()),
    );
    let meta_str = serde_json::to_string(&serde_json::Value::Object(meta))
        .map_err(|e| GuardError::Db(e.to_string()))?;

    let target_col = goal_column_id_tx(&mut tx, &goal.project_id, new_state.binding()).await?;
    let position = next_position_tx(&mut tx, &target_col).await?;

    if let Some(ref assignee) = effects.assigned_to {
        sqlx::query("UPDATE cards SET assigned_to = ? WHERE id = ?")
            .bind(assignee)
            .bind(card_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }

    sqlx::query("UPDATE cards SET metadata_json = ?, column_id = ?, position = ? WHERE id = ?")
        .bind(&meta_str)
        .bind(&target_col)
        .bind(position)
        .bind(card_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

    let (decision_id, audit_actor) = match proof.as_ref() {
        Some(p) => (p.decision_id().to_string(), p.acted_by().to_string()),
        None => ("none".to_string(), actor.to_string()),
    };
    decisions::append_audit_tx(
        &mut tx,
        &decision_id,
        Some(card_id),
        &audit_actor,
        tier,
        &format!("transition:{}", action),
        None,
    )
    .await
    .map_err(GuardError::Db)?;

    tx.commit().await.map_err(db_err)?;

    // Live goal-state push (#454-follow): every transition emits so the
    // dashboard surfaces and Kanban board react within a frame instead of
    // waiting on a poll or a full reload.
    crate::events::emit(crate::events::goal_state_changed(
        card_id,
        Some(&goal.project_id),
        Some(current_state.binding()),
        new_state.binding(),
    ));

    Ok(new_state)
}

// ── Audited system paths (park / requeue / failure record) ─────────────────

/// Park a goal: move it to Triage with `needs_human_attention=true` and the
/// blocking reason in `last_error`. System action (audited, tier 0 semantics:
/// parking restricts, never advances).
pub async fn park_goal(
    pool: &Pool<Sqlite>,
    card_id: &str,
    actor: &str,
    reason: &str,
) -> Result<(), GuardError> {
    let mut tx = pool.begin().await.map_err(db_err)?;

    let goal = read_goal_tx(&mut tx, card_id).await?;
    if goal.card_type != "goal" {
        return Err(GuardError::Invalid(format!(
            "Card '{}' is not a goal",
            card_id
        )));
    }

    let mut meta = goal.metadata;
    meta.insert(
        "goal_state".to_string(),
        serde_json::Value::String("triage".to_string()),
    );
    meta.insert(
        "needs_human_attention".to_string(),
        serde_json::Value::Bool(true),
    );
    meta.insert(
        "last_error".to_string(),
        serde_json::Value::String(reason.to_string()),
    );
    let meta_str = serde_json::to_string(&serde_json::Value::Object(meta))
        .map_err(|e| GuardError::Db(e.to_string()))?;

    let triage_col = goal_column_id_tx(&mut tx, &goal.project_id, "triage").await?;
    let position = next_position_tx(&mut tx, &triage_col).await?;

    sqlx::query("UPDATE cards SET metadata_json = ?, column_id = ?, position = ? WHERE id = ?")
        .bind(&meta_str)
        .bind(&triage_col)
        .bind(position)
        .bind(card_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

    decisions::append_audit_tx(&mut tx, "none", Some(card_id), actor, 0, "park", None)
        .await
        .map_err(GuardError::Db)?;

    tx.commit().await.map_err(db_err)?;

    crate::events::emit(crate::events::goal_state_changed(
        card_id,
        Some(&goal.project_id),
        None,
        "triage",
    ));

    Ok(())
}

/// Requeue a goal whose worker died (e.g. daemon restart): InProgress → Ready
/// with an incremented attempt count and the abandonment reason recorded.
pub async fn requeue_goal(
    pool: &Pool<Sqlite>,
    card_id: &str,
    actor: &str,
    new_attempt_count: u64,
    reason: &str,
) -> Result<(), GuardError> {
    let mut tx = pool.begin().await.map_err(db_err)?;

    let goal = read_goal_tx(&mut tx, card_id).await?;
    if goal.card_type != "goal" {
        return Err(GuardError::Invalid(format!(
            "Card '{}' is not a goal",
            card_id
        )));
    }

    let binding = state_binding_of_column_tx(&mut tx, &goal.column_id).await?;
    if binding.as_deref() != Some("in_progress") {
        return Err(GuardError::Invalid(format!(
            "Only in_progress goals can be requeued (card '{}' is in '{}')",
            card_id,
            binding.as_deref().unwrap_or("?")
        )));
    }

    let mut meta = goal.metadata;
    meta.insert(
        "goal_state".to_string(),
        serde_json::Value::String("ready".to_string()),
    );
    meta.insert(
        "attempt_count".to_string(),
        serde_json::json!(new_attempt_count),
    );
    meta.insert(
        "last_error".to_string(),
        serde_json::Value::String(reason.to_string()),
    );
    let meta_str = serde_json::to_string(&serde_json::Value::Object(meta))
        .map_err(|e| GuardError::Db(e.to_string()))?;

    let ready_col = goal_column_id_tx(&mut tx, &goal.project_id, "ready").await?;
    let position = next_position_tx(&mut tx, &ready_col).await?;

    sqlx::query("UPDATE cards SET metadata_json = ?, column_id = ?, position = ? WHERE id = ?")
        .bind(&meta_str)
        .bind(&ready_col)
        .bind(position)
        .bind(card_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

    decisions::append_audit_tx(&mut tx, "none", Some(card_id), actor, 0, "requeue", None)
        .await
        .map_err(GuardError::Db)?;

    tx.commit().await.map_err(db_err)?;

    crate::events::emit(crate::events::goal_state_changed(
        card_id,
        Some(&goal.project_id),
        Some("in_progress"),
        "ready",
    ));

    Ok(())
}

/// Record a retriable worker failure on an in-progress goal without changing
/// its state (`last_error` is a protected key, so this guarded path exists).
pub async fn record_goal_failure(
    pool: &Pool<Sqlite>,
    card_id: &str,
    error: &str,
) -> Result<(), GuardError> {
    let mut tx = pool.begin().await.map_err(db_err)?;
    let goal = read_goal_tx(&mut tx, card_id).await?;
    let mut meta = goal.metadata;
    meta.insert(
        "last_error".to_string(),
        serde_json::Value::String(error.to_string()),
    );
    meta.insert(
        "goal_state".to_string(),
        serde_json::Value::String("in_progress".to_string()),
    );
    let meta_str = serde_json::to_string(&serde_json::Value::Object(meta))
        .map_err(|e| GuardError::Db(e.to_string()))?;
    sqlx::query("UPDATE cards SET metadata_json = ? WHERE id = ?")
        .bind(&meta_str)
        .bind(card_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;

    // State is unchanged (in_progress, retriable), but the latched error is
    // new — emit so any error badge on the live surfaces updates.
    crate::events::emit(crate::events::goal_state_changed(
        card_id,
        Some(&goal.project_id),
        Some("in_progress"),
        "in_progress",
    ));

    Ok(())
}

/// Delete a goal card. Tier-2 gated: requires a `risk_gate` decision for this
/// goal answered 'approve' by 'jesse' (action class `user_data_deletion`).
pub async fn delete_goal_checked(
    pool: &Pool<Sqlite>,
    card_id: &str,
    proof: DecisionProof,
) -> Result<bool, GuardError> {
    if proof.kind() != "risk_gate" {
        return Err(GuardError::Denied(format!(
            "Goal deletion requires a risk_gate decision (got kind '{}')",
            proof.kind()
        )));
    }
    if proof.answer() != "approve" {
        return Err(GuardError::Denied(
            "Goal deletion requires an 'approve' answer".to_string(),
        ));
    }
    if proof.acted_by() != decisions::ACTOR_JESSE {
        return Err(GuardError::Denied(format!(
            "Goal deletion is Tier 2 and requires acted_by='jesse' (got '{}')",
            proof.acted_by()
        )));
    }
    if proof.goal_id() != Some(card_id) {
        return Err(GuardError::Denied(format!(
            "Decision {} does not reference goal '{}'",
            proof.decision_id(),
            card_id
        )));
    }

    let mut tx = pool.begin().await.map_err(db_err)?;
    decisions::append_audit_tx(
        &mut tx,
        proof.decision_id(),
        Some(card_id),
        proof.acted_by(),
        2,
        "delete",
        None,
    )
    .await
    .map_err(GuardError::Db)?;
    let result = sqlx::query("DELETE FROM cards WHERE id = ?")
        .bind(card_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;

    let deleted = result.rows_affected() > 0;
    if deleted {
        crate::events::emit(crate::events::goal_state_changed(
            card_id, None, None, "deleted",
        ));
    }
    Ok(deleted)
}

// ── Budgets (S4) ────────────────────────────────────────────────────────────

/// Per-goal budget, stored under the protected `budget` metadata key.
#[derive(Debug, Clone)]
pub struct GoalBudget {
    pub token_budget: Option<u64>,
    pub attempt_cap: u64,
    pub wallclock_cap_secs: Option<u64>,
}

impl Default for GoalBudget {
    fn default() -> Self {
        Self {
            token_budget: None,
            attempt_cap: DEFAULT_ATTEMPT_CAP,
            wallclock_cap_secs: None,
        }
    }
}

/// Parse a goal card's budget from its metadata (defaults when absent).
pub fn goal_budget(metadata: &serde_json::Value) -> GoalBudget {
    let b = metadata.get("budget");
    GoalBudget {
        token_budget: b
            .and_then(|b| b.get("token_budget"))
            .and_then(|v| v.as_u64()),
        attempt_cap: b
            .and_then(|b| b.get("attempt_cap"))
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_ATTEMPT_CAP),
        wallclock_cap_secs: b
            .and_then(|b| b.get("wallclock_cap_secs"))
            .and_then(|v| v.as_u64()),
    }
}

/// Which budget dimension is exhausted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetExhaustion {
    AttemptCap { spent: u64, cap: u64 },
    TokenBudget { spent: u64, cap: u64 },
    Wallclock { spent_secs: u64, cap_secs: u64 },
}

impl BudgetExhaustion {
    pub fn reason(&self) -> decisions::UnblockReason {
        match self {
            Self::AttemptCap { .. } => decisions::UnblockReason::AttemptCap,
            Self::TokenBudget { .. } => decisions::UnblockReason::TokenBudget,
            Self::Wallclock { .. } => decisions::UnblockReason::WallclockCap,
        }
    }

    pub fn spent_and_cap(&self) -> (u64, u64) {
        match *self {
            Self::AttemptCap { spent, cap } => (spent, cap),
            Self::TokenBudget { spent, cap } => (spent, cap),
            Self::Wallclock {
                spent_secs,
                cap_secs,
            } => (spent_secs, cap_secs),
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Self::AttemptCap { spent, cap } => {
                format!("attempt cap exhausted ({}/{} attempts)", spent, cap)
            }
            Self::TokenBudget { spent, cap } => {
                format!("token budget exhausted ({}/{} tokens)", spent, cap)
            }
            Self::Wallclock {
                spent_secs,
                cap_secs,
            } => format!(
                "wallclock cap exhausted ({}s elapsed of {}s)",
                spent_secs, cap_secs
            ),
        }
    }
}

/// Sum of accumulated total tokens across all worker sessions a goal has used
/// (per-attempt history in `worker_session_ids` plus the current
/// `worker_session_id`).
pub async fn goal_spent_tokens(
    pool: &Pool<Sqlite>,
    metadata: &serde_json::Value,
) -> Result<u64, String> {
    let mut ids: Vec<String> = metadata
        .get("worker_session_ids")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    if let Some(sid) = metadata.get("worker_session_id").and_then(|v| v.as_str()) {
        if !ids.iter().any(|i| i == sid) {
            ids.push(sid.to_string());
        }
    }
    if ids.is_empty() {
        return Ok(0);
    }

    let placeholders = vec!["?"; ids.len()].join(", ");
    let sql = format!(
        "SELECT COALESCE(SUM(COALESCE(accumulated_total_tokens, 0)), 0) FROM sessions \
         WHERE id IN ({})",
        placeholders
    );
    let mut query = sqlx::query_scalar::<_, i64>(&sql);
    for id in &ids {
        query = query.bind(id);
    }
    let total = query.fetch_one(pool).await.map_err(|e| e.to_string())?;
    Ok(total.max(0) as u64)
}

/// Check a goal's budget. Returns the first exhausted dimension, or None when
/// the goal may consume another attempt.
pub async fn check_budget(
    pool: &Pool<Sqlite>,
    metadata: &serde_json::Value,
) -> Result<Option<BudgetExhaustion>, String> {
    let budget = goal_budget(metadata);

    let attempt_count = metadata
        .get("attempt_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    if attempt_count >= budget.attempt_cap {
        return Ok(Some(BudgetExhaustion::AttemptCap {
            spent: attempt_count,
            cap: budget.attempt_cap,
        }));
    }

    if let Some(cap) = budget.token_budget {
        let spent = goal_spent_tokens(pool, metadata).await?;
        if spent >= cap {
            return Ok(Some(BudgetExhaustion::TokenBudget { spent, cap }));
        }
    }

    if let Some(cap_secs) = budget.wallclock_cap_secs {
        if let Some(first) = metadata
            .get("first_dispatched_at")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        {
            let elapsed = (chrono::Utc::now() - first.with_timezone(&chrono::Utc)).num_seconds();
            if elapsed >= 0 && elapsed as u64 >= cap_secs {
                return Ok(Some(BudgetExhaustion::Wallclock {
                    spent_secs: elapsed as u64,
                    cap_secs,
                }));
            }
        }
    }

    Ok(None)
}

/// Budget exhaustion handler: emit a `kind='unblock'` decision (deduplicated
/// per goal) and park the goal. NEVER a silent retry (S4).
///
/// Returns the id of the open unblock decision for the goal.
pub async fn exhaust_and_park(
    pool: &Pool<Sqlite>,
    card_id: &str,
    goal_title: &str,
    project_id: &str,
    exhaustion: BudgetExhaustion,
    last_error: Option<&str>,
) -> Result<String, String> {
    // Dedupe: one open unblock decision per goal.
    let decision_id = match decisions::find_open_decision_for_goal(pool, card_id, "unblock").await?
    {
        Some(existing) => existing.id,
        None => {
            let (spent, cap) = exhaustion.spent_and_cap();
            let headline = truncate_chars(
                &format!(
                    "\"{}\" is out of budget and needs your direction",
                    goal_title
                ),
                decisions::MAX_HEADLINE_CHARS,
            );
            let detail = format!(
                "Goal {} parked: {}. Last error: {}",
                card_id,
                exhaustion.describe(),
                last_error.unwrap_or("(none recorded)")
            );
            let payload = serde_json::to_value(decisions::UnblockPayload {
                reason: exhaustion.reason(),
                spent: Some(spent),
                cap: Some(cap),
            })
            .map_err(|e| e.to_string())?;

            decisions::create_decision(
                pool,
                decisions::NewDecision {
                    kind: "unblock".to_string(),
                    goal_id: Some(card_id.to_string()),
                    project_id: Some(project_id.to_string()),
                    headline: Some(headline),
                    detail: Some(detail),
                    payload,
                    ..Default::default()
                },
            )
            .await?
            .id
        }
    };

    park_goal(
        pool,
        card_id,
        decisions::ACTOR_SYSTEM,
        &format!(
            "{}{}",
            exhaustion.describe(),
            last_error
                .map(|e| format!("; last error: {}", e))
                .unwrap_or_default()
        ),
    )
    .await
    .map_err(String::from)?;

    Ok(decision_id)
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max - 1).collect();
        format!("{}…", cut)
    }
}

// ── Dependency promotion (pool-only, no orchestrator needed) ────────────────

/// Promote Triage goals whose dependencies are all Complete to Ready
/// (tier-0 'ready' transitions through the guard). Dispatching the promoted
/// goals remains the orchestrator's job.
pub async fn promote_eligible_dependents(
    pool: &Pool<Sqlite>,
    project_id: &str,
) -> Result<u32, String> {
    let complete_col: Option<String> = sqlx::query_scalar(
        "SELECT id FROM board_columns WHERE project_id = ? AND state_binding = 'complete'",
    )
    .bind(project_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    let complete_col = match complete_col {
        Some(c) => c,
        None => return Ok(0),
    };

    let triage_goals = sqlx::query_as::<_, (String, String)>(
        "SELECT c.id, c.metadata_json FROM cards c \
         JOIN board_columns bc ON c.column_id = bc.id \
         WHERE c.project_id = ? AND c.card_type = 'goal' \
           AND bc.state_binding = 'triage' AND c.archived_at IS NULL",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut promoted = 0u32;
    for (goal_id, meta_str) in &triage_goals {
        let meta: serde_json::Value = serde_json::from_str(meta_str).unwrap_or_default();

        // Parked goals stay parked until their unblock decision is answered.
        if meta
            .get("needs_human_attention")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            continue;
        }

        let deps: Vec<String> = meta
            .get("depends_on")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        if deps.is_empty() {
            continue;
        }

        let mut all_complete = true;
        for dep_id in &deps {
            let dep_col: Option<String> =
                sqlx::query_scalar("SELECT column_id FROM cards WHERE id = ?")
                    .bind(dep_id)
                    .fetch_optional(pool)
                    .await
                    .map_err(|e| e.to_string())?;
            if dep_col.as_deref() != Some(complete_col.as_str()) {
                all_complete = false;
                break;
            }
        }

        if all_complete {
            advance_goal_checked(
                pool,
                goal_id,
                GoalAction::Ready,
                decisions::ACTOR_SYSTEM,
                None,
                TransitionEffects::default(),
            )
            .await
            .map_err(String::from)?;
            promoted += 1;
        }
    }

    Ok(promoted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decisions::{answer_decision, create_decision, DecisionAnswer, NewDecision};
    use crate::projects::PERSONAL_PROJECT_ID;

    async fn test_pool() -> Pool<Sqlite> {
        use crate::session::spectral_schema::init_spectral_db;
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        pool
    }

    async fn goal_in_state(pool: &Pool<Sqlite>, state: &str, attempts: u64) -> crate::cards::Card {
        crate::cards::seed_goal_columns(pool, PERSONAL_PROJECT_ID)
            .await
            .unwrap();
        let col = crate::cards::get_goal_column(pool, PERSONAL_PROJECT_ID, state)
            .await
            .unwrap()
            .unwrap();
        let mut meta = serde_json::Map::new();
        meta.insert("attempt_count".to_string(), serde_json::json!(attempts));
        meta.insert(
            "goal_state".to_string(),
            serde_json::Value::String(state.to_string()),
        );
        crate::cards::create_card(
            pool,
            crate::cards::CreateCard {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                title: format!("Guard test goal in {}", state),
                description: Some("test".to_string()),
                card_type: Some("goal".to_string()),
                column_id: Some(col.id.clone()),
                created_by: None,
                metadata_json: Some(serde_json::Value::Object(meta)),
            },
        )
        .await
        .unwrap()
    }

    async fn state_of(pool: &Pool<Sqlite>, card_id: &str) -> String {
        let card = crate::cards::get_card(pool, card_id)
            .await
            .unwrap()
            .unwrap();
        let col = crate::cards::get_column(pool, &card.column_id)
            .await
            .unwrap()
            .unwrap();
        col.state_binding.unwrap_or_default()
    }

    async fn approve_decision_proof(
        pool: &Pool<Sqlite>,
        goal_id: &str,
        actor: &str,
    ) -> crate::decisions::DecisionProof {
        let d = create_decision(
            pool,
            NewDecision {
                kind: "approve_review".to_string(),
                goal_id: Some(goal_id.to_string()),
                project_id: Some(PERSONAL_PROJECT_ID.to_string()),
                headline: Some("Approve the finished work on the test goal".to_string()),
                detail: Some("evidence: tests pass".to_string()),
                payload: serde_json::json!({}),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let (_, proof) = answer_decision(
            pool,
            &d.id,
            &DecisionAnswer {
                answer: "approve".to_string(),
                ..Default::default()
            },
            actor,
        )
        .await
        .unwrap();
        proof
    }

    // ── Tier gating ──

    #[tokio::test]
    async fn tier0_transitions_need_no_proof() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "triage", 0).await;
        let new = advance_goal_checked(
            &pool,
            &goal.id,
            GoalAction::Ready,
            "system",
            None,
            TransitionEffects::default(),
        )
        .await
        .unwrap();
        assert_eq!(new, GoalState::Ready);
        assert_eq!(state_of(&pool, &goal.id).await, "ready");
    }

    #[tokio::test]
    async fn tier1_approve_without_decision_is_denied() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "review", 1).await;
        let err = advance_goal_checked(
            &pool,
            &goal.id,
            GoalAction::Approve,
            "system",
            None,
            TransitionEffects::default(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, GuardError::Denied(_)), "{:?}", err);
        assert_eq!(state_of(&pool, &goal.id).await, "review", "no state change");
    }

    #[tokio::test]
    async fn tier1_approve_with_henry_decision_succeeds() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "review", 1).await;
        let proof = approve_decision_proof(&pool, &goal.id, crate::decisions::ACTOR_HENRY).await;
        let new = advance_goal_checked(
            &pool,
            &goal.id,
            GoalAction::Approve,
            "henry-policy",
            Some(proof),
            TransitionEffects::default(),
        )
        .await
        .unwrap();
        assert_eq!(new, GoalState::Complete);
        assert_eq!(state_of(&pool, &goal.id).await, "complete");
    }

    #[tokio::test]
    async fn tier2_advance_without_jesse_decision_is_denied() {
        let pool = test_pool().await;
        // Raise approve to Tier 2 via the trust dial.
        sqlx::query("UPDATE risk_policy SET tier = 2 WHERE action_class = 'goal_approve_standard'")
            .execute(&pool)
            .await
            .unwrap();

        let goal = goal_in_state(&pool, "review", 1).await;

        // No decision at all → denied.
        let err = advance_goal_checked(
            &pool,
            &goal.id,
            GoalAction::Approve,
            "system",
            None,
            TransitionEffects::default(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, GuardError::Denied(_)));

        // Henry-answered decision is NOT enough for Tier 2... but the decisions
        // module already refuses to mint a henry proof for a tier-2 decision,
        // so simulate the only remaining hole: a henry proof minted while the
        // policy said tier 1, replayed after the dial moved to tier 2.
        sqlx::query("UPDATE risk_policy SET tier = 1 WHERE action_class = 'goal_approve_standard'")
            .execute(&pool)
            .await
            .unwrap();
        let henry_proof =
            approve_decision_proof(&pool, &goal.id, crate::decisions::ACTOR_HENRY).await;
        sqlx::query("UPDATE risk_policy SET tier = 2 WHERE action_class = 'goal_approve_standard'")
            .execute(&pool)
            .await
            .unwrap();
        let err = advance_goal_checked(
            &pool,
            &goal.id,
            GoalAction::Approve,
            "henry-policy",
            Some(henry_proof),
            TransitionEffects::default(),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, GuardError::Denied(_)),
            "tier gate must re-check at transition time: {:?}",
            err
        );
        assert_eq!(state_of(&pool, &goal.id).await, "review");

        // Jesse-answered decision passes.
        let jesse_proof =
            approve_decision_proof(&pool, &goal.id, crate::decisions::ACTOR_JESSE).await;
        let new = advance_goal_checked(
            &pool,
            &goal.id,
            GoalAction::Approve,
            "jesse",
            Some(jesse_proof),
            TransitionEffects::default(),
        )
        .await
        .unwrap();
        assert_eq!(new, GoalState::Complete);
    }

    #[tokio::test]
    async fn proof_for_other_goal_is_rejected() {
        let pool = test_pool().await;
        let goal_a = goal_in_state(&pool, "review", 1).await;
        let goal_b = goal_in_state(&pool, "review", 1).await;
        let proof_b =
            approve_decision_proof(&pool, &goal_b.id, crate::decisions::ACTOR_JESSE).await;
        let err = advance_goal_checked(
            &pool,
            &goal_a.id,
            GoalAction::Approve,
            "jesse",
            Some(proof_b),
            TransitionEffects::default(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, GuardError::Denied(_)));
    }

    #[tokio::test]
    async fn reject_answer_cannot_authorize_approve() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "review", 0).await;
        let d = create_decision(
            &pool,
            NewDecision {
                kind: "approve_review".to_string(),
                goal_id: Some(goal.id.clone()),
                headline: Some("Approve the finished work on the test goal".to_string()),
                detail: Some("evidence".to_string()),
                payload: serde_json::json!({}),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let (_, proof) = answer_decision(
            &pool,
            &d.id,
            &DecisionAnswer {
                answer: "reject".to_string(),
                ..Default::default()
            },
            crate::decisions::ACTOR_JESSE,
        )
        .await
        .unwrap();
        let err = advance_goal_checked(
            &pool,
            &goal.id,
            GoalAction::Approve,
            "jesse",
            Some(proof),
            TransitionEffects::default(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, GuardError::Denied(_)));
    }

    #[tokio::test]
    async fn invalid_transition_rejected_before_gates() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "triage", 0).await;
        let err = advance_goal_checked(
            &pool,
            &goal.id,
            GoalAction::Approve,
            "system",
            None,
            TransitionEffects::default(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, GuardError::Invalid(_)));
    }

    // ── Defense-in-depth trigger ──

    #[tokio::test]
    async fn raw_sql_move_to_complete_without_decision_is_blocked_by_trigger() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "review", 1).await;
        let complete_col = crate::cards::get_goal_column(&pool, PERSONAL_PROJECT_ID, "complete")
            .await
            .unwrap()
            .unwrap();

        let result = sqlx::query("UPDATE cards SET column_id = ? WHERE id = ?")
            .bind(&complete_col.id)
            .bind(&goal.id)
            .execute(&pool)
            .await;
        assert!(
            result.is_err(),
            "raw column update into complete must be blocked by trg_goal_complete_guard"
        );
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("answered approve decision"), "{}", msg);
    }

    // ── Audit ──

    #[tokio::test]
    async fn every_checked_transition_is_audited_and_chain_verifies() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "triage", 0).await;
        advance_goal_checked(
            &pool,
            &goal.id,
            GoalAction::Ready,
            "system",
            None,
            TransitionEffects::default(),
        )
        .await
        .unwrap();
        advance_goal_checked(
            &pool,
            &goal.id,
            GoalAction::Dispatch,
            "system",
            None,
            TransitionEffects::default(),
        )
        .await
        .unwrap();

        let outcomes: Vec<String> = sqlx::query_scalar(
            "SELECT outcome FROM decision_audit WHERE goal_id = ? ORDER BY seq ASC",
        )
        .bind(&goal.id)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(outcomes, vec!["transition:ready", "transition:dispatch"]);

        let report = crate::decisions::verify_audit_chain(&pool).await.unwrap();
        assert!(report.intact, "{}", report.detail);
    }

    // ── Budgets ──

    #[tokio::test]
    async fn budget_defaults_and_overrides() {
        let meta = serde_json::json!({});
        let b = goal_budget(&meta);
        assert_eq!(b.attempt_cap, DEFAULT_ATTEMPT_CAP);
        assert!(b.token_budget.is_none());

        let meta = serde_json::json!({
            "budget": {"attempt_cap": 5, "token_budget": 100000, "wallclock_cap_secs": 3600}
        });
        let b = goal_budget(&meta);
        assert_eq!(b.attempt_cap, 5);
        assert_eq!(b.token_budget, Some(100000));
        assert_eq!(b.wallclock_cap_secs, Some(3600));
    }

    #[tokio::test]
    async fn exhaustion_emits_unblock_decision_and_parks_never_silent_retry() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "in_progress", 3).await;

        let exhaustion = check_budget(&pool, &goal.metadata_json).await.unwrap();
        let exhaustion = exhaustion.expect("3 attempts at default cap 3 is exhausted");
        assert!(matches!(exhaustion, BudgetExhaustion::AttemptCap { .. }));

        let decision_id = exhaust_and_park(
            &pool,
            &goal.id,
            "Guard test goal in in_progress",
            PERSONAL_PROJECT_ID,
            exhaustion,
            Some("worker crashed"),
        )
        .await
        .unwrap();

        // Goal is parked, not retried.
        assert_eq!(state_of(&pool, &goal.id).await, "triage");
        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            card.metadata_json.get("needs_human_attention"),
            Some(&serde_json::Value::Bool(true))
        );

        // An open unblock decision exists.
        let d = crate::decisions::get_decision(&pool, &decision_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(d.kind, "unblock");
        assert_eq!(d.status, "open");
        assert_eq!(d.goal_id.as_deref(), Some(goal.id.as_str()));

        // Re-exhausting dedupes onto the same open decision.
        let again = exhaust_and_park(
            &pool,
            &goal.id,
            "Guard test goal in in_progress",
            PERSONAL_PROJECT_ID,
            BudgetExhaustion::AttemptCap { spent: 3, cap: 3 },
            None,
        )
        .await;
        // park_goal of a triage goal is a triage->triage write; it must not error.
        let again = again.unwrap();
        assert_eq!(again, decision_id, "must not create a duplicate decision");
    }

    #[tokio::test]
    async fn within_budget_is_not_exhausted() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "in_progress", 1).await;
        let exhaustion = check_budget(&pool, &goal.metadata_json).await.unwrap();
        assert!(exhaustion.is_none());
    }

    // ── Deletion gate ──

    #[tokio::test]
    async fn goal_deletion_requires_jesse_risk_gate() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "triage", 0).await;

        // A non-risk_gate proof is refused.
        let wrong_kind =
            approve_decision_proof(&pool, &goal.id, crate::decisions::ACTOR_JESSE).await;
        let err = delete_goal_checked(&pool, &goal.id, wrong_kind)
            .await
            .unwrap_err();
        assert!(matches!(err, GuardError::Denied(_)));

        // A proper risk_gate (user_data_deletion) approved by jesse deletes.
        let d = create_decision(
            &pool,
            NewDecision {
                kind: "risk_gate".to_string(),
                goal_id: Some(goal.id.clone()),
                headline: Some("Permission to delete the test goal".to_string()),
                detail: Some("user_data_deletion of a goal card".to_string()),
                payload: serde_json::json!({
                    "action_class": "user_data_deletion",
                    "description": "delete goal",
                    "requested_by": "test"
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let (_, proof) = answer_decision(
            &pool,
            &d.id,
            &DecisionAnswer {
                answer: "approve".to_string(),
                ..Default::default()
            },
            crate::decisions::ACTOR_JESSE,
        )
        .await
        .unwrap();
        let deleted = delete_goal_checked(&pool, &goal.id, proof).await.unwrap();
        assert!(deleted);
        assert!(crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .is_none());
    }

    // ── Requeue / promote ──

    #[tokio::test]
    async fn requeue_moves_in_progress_to_ready_with_attempts() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "in_progress", 1).await;
        requeue_goal(
            &pool,
            &goal.id,
            "system",
            2,
            "Abandoned during daemon restart",
        )
        .await
        .unwrap();
        assert_eq!(state_of(&pool, &goal.id).await, "ready");
        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            card.metadata_json
                .get("attempt_count")
                .and_then(|v| v.as_u64()),
            Some(2)
        );
    }

    #[tokio::test]
    async fn promote_skips_parked_goals() {
        let pool = test_pool().await;
        let dep = goal_in_state(&pool, "complete", 0).await;
        let goal = goal_in_state(&pool, "triage", 3).await;

        // Give the triage goal a satisfied dependency but mark it parked.
        let mut meta = goal.metadata_json.as_object().cloned().unwrap();
        meta.insert("depends_on".to_string(), serde_json::json!([dep.id]));
        meta.insert("needs_human_attention".to_string(), serde_json::json!(true));
        let meta_str = serde_json::to_string(&serde_json::Value::Object(meta)).unwrap();
        sqlx::query("UPDATE cards SET metadata_json = ? WHERE id = ?")
            .bind(&meta_str)
            .bind(&goal.id)
            .execute(&pool)
            .await
            .unwrap();

        let promoted = promote_eligible_dependents(&pool, PERSONAL_PROJECT_ID)
            .await
            .unwrap();
        assert_eq!(promoted, 0, "parked goals must not auto-promote");
        assert_eq!(state_of(&pool, &goal.id).await, "triage");
    }
}
