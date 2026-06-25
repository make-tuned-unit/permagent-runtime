//! Decision Inbox — the daemon's single channel for human/policy decisions.
//!
//! Every consequential goal-state change (and every Tier-1/Tier-2 action in
//! general) is gated on a row in the `decisions` table being answered by an
//! authorized actor. Answering mints a [`DecisionProof`] — a non-Copy,
//! non-Clone token with a private constructor (the SafeBrain compile-time
//! pattern, PR #277) that the goal-transition guard demands before executing
//! gated effects. No code outside this module can fabricate one.
//!
//! Every decision lifecycle event is appended to `decision_audit`, an
//! append-only (DB-trigger-enforced) hash-chained log.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Pool, Row, Sqlite};
use uuid::Uuid;

/// Maximum length of a decision headline (Jesse amendment A1).
pub const MAX_HEADLINE_CHARS: usize = 80;

/// Actors allowed to answer decisions (S5 attribution).
pub const ACTOR_JESSE: &str = "jesse";
pub const ACTOR_HENRY: &str = "henry-policy";
pub const ACTOR_SYSTEM: &str = "system";

const VALID_ACTORS: &[&str] = &[ACTOR_JESSE, ACTOR_HENRY, ACTOR_SYSTEM];
const VALID_ANSWERS: &[&str] = &["approve", "reject", "choice", "input"];

// ── Typed payloads (S2) ─────────────────────────────────────────────────────

/// Payload for `kind='approve_review'` — a goal finished work and awaits
/// approval to move Review → Complete.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApproveReviewPayload {
    /// sha256 of the evidence bundle shown to the approver.
    #[serde(default)]
    pub evidence_digest: Option<String>,
    /// Paths the worker's diff touched.
    #[serde(default)]
    pub diff_paths: Vec<String>,
    /// Result of the completion check, if one ran.
    #[serde(default)]
    pub completion_check: Option<String>,
}

/// Why a goal is blocked (`kind='unblock'`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnblockReason {
    TokenBudget,
    AttemptCap,
    WallclockCap,
    Stuck,
}

/// Payload for `kind='unblock'` — a goal exhausted its budget and is parked.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnblockPayload {
    pub reason: UnblockReason,
    #[serde(default)]
    pub spent: Option<u64>,
    #[serde(default)]
    pub cap: Option<u64>,
}

/// One selectable option in a `choice` decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChoiceOption {
    pub id: String,
    pub label: String,
}

/// Payload for `kind='choice'` — pick one of 2..=8 options.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChoicePayload {
    pub question: String,
    pub options: Vec<ChoiceOption>,
    #[serde(default)]
    pub default: Option<String>,
}

/// Payload for `kind='risk_gate'` — permission to perform a risky action class.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RiskGatePayload {
    pub action_class: String,
    pub description: String,
    pub requested_by: String,
}

// ── Decision rows ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct Decision {
    pub id: String,
    pub kind: String,
    pub goal_id: Option<String>,
    pub project_id: Option<String>,
    pub tier: i64,
    pub headline: String,
    pub detail: String,
    pub payload: serde_json::Value,
    pub rank: Option<f64>,
    pub status: String,
    pub answer: Option<String>,
    pub answer_note: Option<String>,
    pub answer_choice_id: Option<String>,
    pub answer_input: Option<String>,
    pub acted_by: Option<String>,
    pub created_at: String,
    pub resolved_at: Option<String>,
}

fn row_to_decision(r: &sqlx::sqlite::SqliteRow) -> Decision {
    let payload_str: String = r.get("payload_json");
    Decision {
        id: r.get("id"),
        kind: r.get("kind"),
        goal_id: r.get("goal_id"),
        project_id: r.get("project_id"),
        tier: r.get("tier"),
        headline: r.get("headline"),
        detail: r.get("detail"),
        payload: serde_json::from_str(&payload_str).unwrap_or(serde_json::Value::Null),
        rank: r.get("rank"),
        status: r.get("status"),
        answer: r.get("answer"),
        answer_note: r.get("answer_note"),
        answer_choice_id: r.get("answer_choice_id"),
        answer_input: r.get("answer_input"),
        acted_by: r.get("acted_by"),
        created_at: r.get("created_at"),
        resolved_at: r.get("resolved_at"),
    }
}

const DECISION_COLUMNS: &str = "id, kind, goal_id, project_id, tier, headline, detail, \
     payload_json, rank, status, answer, answer_note, answer_choice_id, answer_input, \
     acted_by, created_at, resolved_at";

/// Request to create a decision. `headline` and `detail` are both REQUIRED
/// (Jesse amendment A1): `headline` is a plain-language outcome statement
/// (<= 80 chars, no technical identifiers); `detail` carries the technical
/// content. Requests missing either, or whose payload fails its kind's typed
/// schema, are stored as `kind='malformed'` — never coerced (S2).
#[derive(Debug, Clone, Default)]
pub struct NewDecision {
    pub kind: String,
    pub goal_id: Option<String>,
    pub project_id: Option<String>,
    pub headline: Option<String>,
    pub detail: Option<String>,
    pub payload: serde_json::Value,
    pub rank: Option<f64>,
    /// Explicit risk_policy action_class override. If absent, derived from kind.
    pub action_class: Option<String>,
}

/// Resolve the risk_policy action class for a decision request.
fn resolve_action_class(req: &NewDecision) -> String {
    if let Some(ref class) = req.action_class {
        return class.clone();
    }
    match req.kind.as_str() {
        "approve_review" => "goal_approve_standard".to_string(),
        "unblock" => "goal_retry_within_budget".to_string(),
        "risk_gate" => req
            .payload
            .get("action_class")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string(),
        // No seeded class for bare choices → resolves fail-closed to Tier 2.
        other => other.to_string(),
    }
}

/// Look up the tier for an action class. Unknown classes are Tier 2 (fail-closed).
pub async fn tier_for_action_class(pool: &Pool<Sqlite>, action_class: &str) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT tier FROM risk_policy WHERE action_class = ?")
        .bind(action_class)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .unwrap_or(2)
}

/// Validate a NewDecision. Returns Err(reason) when the request must be
/// stored as malformed.
fn validate_new_decision(req: &NewDecision) -> Result<(), String> {
    if !["approve_review", "unblock", "choice", "risk_gate"].contains(&req.kind.as_str()) {
        return Err(format!("unknown decision kind '{}'", req.kind));
    }
    let headline = req.headline.as_deref().map(str::trim).unwrap_or("");
    if headline.is_empty() {
        return Err("missing required field 'headline'".to_string());
    }
    if headline.chars().count() > MAX_HEADLINE_CHARS {
        return Err(format!(
            "headline exceeds {} characters",
            MAX_HEADLINE_CHARS
        ));
    }
    let detail = req.detail.as_deref().map(str::trim).unwrap_or("");
    if detail.is_empty() {
        return Err("missing required field 'detail'".to_string());
    }
    // Typed payload validation (deny_unknown_fields) — never coerced.
    let payload_result: Result<(), String> = match req.kind.as_str() {
        "approve_review" => serde_json::from_value::<ApproveReviewPayload>(req.payload.clone())
            .map(|_| ())
            .map_err(|e| e.to_string()),
        "unblock" => serde_json::from_value::<UnblockPayload>(req.payload.clone())
            .map(|_| ())
            .map_err(|e| e.to_string()),
        "choice" => match serde_json::from_value::<ChoicePayload>(req.payload.clone()) {
            Ok(p) if (2..=8).contains(&p.options.len()) => Ok(()),
            Ok(p) => Err(format!(
                "choice requires 2..=8 options, got {}",
                p.options.len()
            )),
            Err(e) => Err(e.to_string()),
        },
        "risk_gate" => serde_json::from_value::<RiskGatePayload>(req.payload.clone())
            .map(|_| ())
            .map_err(|e| e.to_string()),
        _ => unreachable!("kind validated above"),
    };
    payload_result.map_err(|e| format!("payload failed schema for kind '{}': {}", req.kind, e))
}

fn truncate_headline(s: &str) -> String {
    if s.chars().count() <= MAX_HEADLINE_CHARS {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(MAX_HEADLINE_CHARS - 1).collect();
        format!("{}…", truncated)
    }
}

/// Create a decision. Validation failures produce a `kind='malformed'` row
/// (Tier 2, fail-closed) carrying the original request — never coerced.
pub async fn create_decision(pool: &Pool<Sqlite>, req: NewDecision) -> Result<Decision, String> {
    let (kind, tier, headline, detail, payload_json) = match validate_new_decision(&req) {
        Ok(()) => {
            let action_class = resolve_action_class(&req);
            let tier = tier_for_action_class(pool, &action_class).await;
            (
                req.kind.clone(),
                tier,
                req.headline.clone().unwrap_or_default().trim().to_string(),
                req.detail.clone().unwrap_or_default().trim().to_string(),
                serde_json::to_string(&req.payload).map_err(|e| e.to_string())?,
            )
        }
        Err(error) => {
            let malformed_payload = serde_json::json!({
                "original_kind": req.kind,
                "raw": req.payload,
                "error": error,
            });
            let headline = req
                .headline
                .as_deref()
                .map(str::trim)
                .filter(|h| !h.is_empty())
                .map(truncate_headline)
                .unwrap_or_else(|| "A malformed decision request needs review".to_string());
            let detail = format!(
                "Rejected decision request (kind '{}'): {}. Original detail: {}",
                req.kind,
                error,
                req.detail.as_deref().unwrap_or("(missing)")
            );
            (
                "malformed".to_string(),
                2, // fail closed: only Jesse can resolve malformed requests
                headline,
                detail,
                serde_json::to_string(&malformed_payload).map_err(|e| e.to_string())?,
            )
        }
    };

    // Only reference goals that exist (FK is best-effort; avoid insert failure).
    let goal_id = match req.goal_id {
        Some(gid) => {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM cards WHERE id = ?)")
                    .bind(&gid)
                    .fetch_one(pool)
                    .await
                    .map_err(|e| e.to_string())?;
            exists.then_some(gid)
        }
        None => None,
    };

    let id = Uuid::now_v7().to_string();
    let created_at = now_timestamp();

    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query(
        "INSERT INTO decisions (id, kind, goal_id, project_id, tier, headline, detail, \
         payload_json, rank, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&kind)
    .bind(&goal_id)
    .bind(&req.project_id)
    .bind(tier)
    .bind(&headline)
    .bind(&detail)
    .bind(&payload_json)
    .bind(req.rank)
    .bind(&created_at)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    append_audit_tx(
        &mut tx,
        &id,
        goal_id.as_deref(),
        ACTOR_SYSTEM,
        tier,
        "created",
        None,
    )
    .await?;

    tx.commit().await.map_err(|e| e.to_string())?;

    // Live event for the Decision Inbox consumer (event-driven, not 15s poll).
    crate::events::emit(crate::events::decision_created(&id, &kind, tier));

    get_decision(pool, &id)
        .await?
        .ok_or_else(|| "Failed to read created decision".to_string())
}

pub async fn get_decision(pool: &Pool<Sqlite>, id: &str) -> Result<Option<Decision>, String> {
    let row = sqlx::query(&format!(
        "SELECT {} FROM decisions WHERE id = ?",
        DECISION_COLUMNS
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(row.as_ref().map(row_to_decision))
}

/// Find an open decision of `kind` for a goal, to avoid duplicates.
pub async fn find_open_decision_for_goal(
    pool: &Pool<Sqlite>,
    goal_id: &str,
    kind: &str,
) -> Result<Option<Decision>, String> {
    let row = sqlx::query(&format!(
        "SELECT {} FROM decisions WHERE goal_id = ? AND kind = ? AND status = 'open' \
         ORDER BY created_at DESC LIMIT 1",
        DECISION_COLUMNS
    ))
    .bind(goal_id)
    .bind(kind)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(row.as_ref().map(row_to_decision))
}

/// Mark every open decision for a goal as `superseded` (#490). Used when a goal
/// is cancelled: any pending approve_review / unblock item is moot, so it leaves
/// the inbox rather than lingering against a terminal goal. Returns the number
/// of decisions superseded.
pub async fn supersede_open_decisions_for_goal(
    pool: &Pool<Sqlite>,
    goal_id: &str,
) -> Result<u64, String> {
    let res = sqlx::query(
        "UPDATE decisions SET status = 'superseded' WHERE goal_id = ? AND status = 'open'",
    )
    .bind(goal_id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(res.rows_affected())
}

// ── Inbox queries (Lane L4 contract) ────────────────────────────────────────

/// Summary envelope returned beside the open-decision list.
#[derive(Debug, Clone, Serialize)]
pub struct InboxSummary {
    pub total_pending: i64,
    pub handled_count: i64,
    pub goals_in_flight: i64,
    pub oldest_pending_at: Option<String>,
}

/// An open inbox item, ranked, with the goal title joined in.
#[derive(Debug, Clone, Serialize)]
pub struct OpenDecisionItem {
    #[serde(flatten)]
    pub decision: Decision,
    pub goal_title: Option<String>,
}

pub async fn list_open_decisions(pool: &Pool<Sqlite>) -> Result<Vec<OpenDecisionItem>, String> {
    let rows = sqlx::query(
        "SELECT d.id, d.kind, d.goal_id, d.project_id, d.tier, d.headline, d.detail, \
                d.payload_json, d.rank, d.status, d.answer, d.answer_note, \
                d.answer_choice_id, d.answer_input, d.acted_by, d.created_at, d.resolved_at, \
                c.title AS goal_title \
         FROM decisions d LEFT JOIN cards c ON d.goal_id = c.id \
         WHERE d.status = 'open' \
         ORDER BY d.rank DESC NULLS LAST, d.created_at ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .map(|r| OpenDecisionItem {
            decision: row_to_decision(r),
            goal_title: r.get("goal_title"),
        })
        .collect())
}

pub async fn inbox_summary(pool: &Pool<Sqlite>) -> Result<InboxSummary, String> {
    let total_pending: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM decisions WHERE status = 'open'")
            .fetch_one(pool)
            .await
            .map_err(|e| e.to_string())?;
    let handled_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM decisions WHERE status != 'open'")
            .fetch_one(pool)
            .await
            .map_err(|e| e.to_string())?;
    let goals_in_flight: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM cards c JOIN board_columns bc ON c.column_id = bc.id \
         WHERE c.card_type = 'goal' AND bc.state_binding = 'in_progress' \
           AND c.archived_at IS NULL",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;
    let oldest_pending_at: Option<String> =
        sqlx::query_scalar("SELECT MIN(created_at) FROM decisions WHERE status = 'open'")
            .fetch_one(pool)
            .await
            .map_err(|e| e.to_string())?;

    Ok(InboxSummary {
        total_pending,
        handled_count,
        goals_in_flight,
        oldest_pending_at,
    })
}

/// A resolved decision joined with its audit rows (history endpoint).
#[derive(Debug, Clone, Serialize)]
pub struct HistoryItem {
    pub seq: i64,
    pub outcome: String,
    pub audit_acted_by: String,
    pub audit_created_at: String,
    #[serde(flatten)]
    pub decision: Decision,
}

/// Resolved decisions joined with audit rows, cursor-paginated on audit.seq
/// (descending; pass `before` = the smallest seq from the previous page).
pub async fn decision_history(
    pool: &Pool<Sqlite>,
    limit: i64,
    before: Option<i64>,
) -> Result<Vec<HistoryItem>, String> {
    let limit = limit.clamp(1, 200);
    let before = before.unwrap_or(i64::MAX);
    let rows = sqlx::query(
        "SELECT a.seq, a.outcome, a.acted_by AS audit_acted_by, \
                a.created_at AS audit_created_at, \
                d.id, d.kind, d.goal_id, d.project_id, d.tier, d.headline, d.detail, \
                d.payload_json, d.rank, d.status, d.answer, d.answer_note, \
                d.answer_choice_id, d.answer_input, d.acted_by, d.created_at, d.resolved_at \
         FROM decision_audit a JOIN decisions d ON a.decision_id = d.id \
         WHERE d.status != 'open' AND a.seq < ? \
         ORDER BY a.seq DESC LIMIT ?",
    )
    .bind(before)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .map(|r| HistoryItem {
            seq: r.get("seq"),
            outcome: r.get("outcome"),
            audit_acted_by: r.get("audit_acted_by"),
            audit_created_at: r.get("audit_created_at"),
            decision: row_to_decision(r),
        })
        .collect())
}

// ── Answering ───────────────────────────────────────────────────────────────

/// Proof that a specific decision was answered by an authorized actor.
///
/// Non-Copy, non-Clone, private fields, private constructor: the ONLY way to
/// obtain one is [`answer_decision`] succeeding against a real `decisions`
/// row. Consumed by value by the goal-transition guard, so each answer
/// authorizes at most one gated effect.
#[derive(Debug)]
pub struct DecisionProof {
    decision_id: String,
    kind: String,
    goal_id: Option<String>,
    tier: i64,
    answer: String,
    acted_by: String,
}

impl DecisionProof {
    pub fn decision_id(&self) -> &str {
        &self.decision_id
    }
    pub fn kind(&self) -> &str {
        &self.kind
    }
    pub fn goal_id(&self) -> Option<&str> {
        self.goal_id.as_deref()
    }
    pub fn tier(&self) -> i64 {
        self.tier
    }
    pub fn answer(&self) -> &str {
        &self.answer
    }
    pub fn acted_by(&self) -> &str {
        &self.acted_by
    }
}

/// The answer body for a decision.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DecisionAnswer {
    pub answer: String,
    pub note: Option<String>,
    pub choice_id: Option<String>,
    pub input_text: Option<String>,
}

/// Errors from answering a decision, discriminated for HTTP status mapping.
#[derive(Debug)]
pub enum AnswerError {
    /// Decision id does not exist (404).
    NotFound,
    /// Decision is not open (409); carries the current status.
    AlreadyResolved(String),
    /// Actor is not authorized for this tier (403).
    Forbidden(String),
    /// Request is invalid (400).
    Invalid(String),
    /// Database failure (500).
    Db(String),
}

impl std::fmt::Display for AnswerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "Decision not found"),
            Self::AlreadyResolved(s) => write!(f, "Decision already resolved (status: {})", s),
            Self::Forbidden(s) => write!(f, "Forbidden: {}", s),
            Self::Invalid(s) => write!(f, "Invalid: {}", s),
            Self::Db(s) => write!(f, "Database error: {}", s),
        }
    }
}

/// Answer an open decision: tier gate → atomic open→answered flip (409 on
/// race) → audit append → mint [`DecisionProof`].
///
/// Actor attribution (S5): HTTP = 'jesse', Henry in-process = 'henry-policy',
/// timers = 'system'. Tier 2 requires 'jesse'; Tier 1 requires 'jesse' or
/// 'henry-policy'.
pub async fn answer_decision(
    pool: &Pool<Sqlite>,
    decision_id: &str,
    answer: &DecisionAnswer,
    acted_by: &str,
) -> Result<(Decision, DecisionProof), AnswerError> {
    if !VALID_ACTORS.contains(&acted_by) {
        return Err(AnswerError::Invalid(format!(
            "unknown actor '{}'",
            acted_by
        )));
    }
    if !VALID_ANSWERS.contains(&answer.answer.as_str()) {
        return Err(AnswerError::Invalid(format!(
            "answer must be one of approve|reject|choice|input, got '{}'",
            answer.answer
        )));
    }

    let decision = get_decision(pool, decision_id)
        .await
        .map_err(AnswerError::Db)?
        .ok_or(AnswerError::NotFound)?;

    if decision.status != "open" {
        return Err(AnswerError::AlreadyResolved(decision.status));
    }

    // Tier gate (S5).
    match decision.tier {
        2 if acted_by != ACTOR_JESSE => {
            return Err(AnswerError::Forbidden(format!(
                "Tier-2 decisions require acted_by='jesse' (got '{}')",
                acted_by
            )));
        }
        1 if acted_by != ACTOR_JESSE && acted_by != ACTOR_HENRY => {
            return Err(AnswerError::Forbidden(format!(
                "Tier-1 decisions require acted_by 'jesse' or 'henry-policy' (got '{}')",
                acted_by
            )));
        }
        _ => {}
    }

    // Kind/answer compatibility.
    match decision.kind.as_str() {
        "choice" => {
            if answer.answer != "choice" {
                return Err(AnswerError::Invalid(
                    "choice decisions must be answered with answer='choice'".to_string(),
                ));
            }
            let choice_id = answer.choice_id.as_deref().ok_or_else(|| {
                AnswerError::Invalid("choice answer requires choice_id".to_string())
            })?;
            let payload: ChoicePayload = serde_json::from_value(decision.payload.clone())
                .map_err(|e| AnswerError::Db(format!("stored choice payload unreadable: {}", e)))?;
            if !payload.options.iter().any(|o| o.id == choice_id) {
                return Err(AnswerError::Invalid(format!(
                    "choice_id '{}' is not one of the offered options",
                    choice_id
                )));
            }
        }
        _ => {
            if answer.answer == "choice" {
                return Err(AnswerError::Invalid(format!(
                    "answer 'choice' is only valid for choice decisions (kind is '{}')",
                    decision.kind
                )));
            }
            if answer.answer == "input" && answer.input_text.as_deref().unwrap_or("").is_empty() {
                return Err(AnswerError::Invalid(
                    "answer 'input' requires input_text".to_string(),
                ));
            }
        }
    }

    let resolved_at = now_timestamp();
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| AnswerError::Db(e.to_string()))?;

    // Atomic open → answered: zero rows affected means we lost a race.
    let result = sqlx::query(
        "UPDATE decisions SET status = 'answered', answer = ?, answer_note = ?, \
         answer_choice_id = ?, answer_input = ?, acted_by = ?, resolved_at = ? \
         WHERE id = ? AND status = 'open'",
    )
    .bind(&answer.answer)
    .bind(&answer.note)
    .bind(&answer.choice_id)
    .bind(&answer.input_text)
    .bind(acted_by)
    .bind(&resolved_at)
    .bind(decision_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| AnswerError::Db(e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AnswerError::AlreadyResolved("answered".to_string()));
    }

    append_audit_tx(
        &mut tx,
        decision_id,
        decision.goal_id.as_deref(),
        acted_by,
        decision.tier,
        &answer.answer,
        decision
            .payload
            .get("evidence_digest")
            .and_then(|v| v.as_str()),
    )
    .await
    .map_err(AnswerError::Db)?;

    tx.commit()
        .await
        .map_err(|e| AnswerError::Db(e.to_string()))?;

    // Live event for the Decision Inbox consumer (the card leaves the board).
    crate::events::emit(crate::events::decision_resolved(
        decision_id,
        &decision.kind,
        &answer.answer,
        acted_by,
        decision.tier,
    ));

    let updated = get_decision(pool, decision_id)
        .await
        .map_err(AnswerError::Db)?
        .ok_or(AnswerError::NotFound)?;

    let proof = DecisionProof {
        decision_id: decision_id.to_string(),
        kind: decision.kind.clone(),
        goal_id: decision.goal_id.clone(),
        tier: decision.tier,
        answer: answer.answer.clone(),
        acted_by: acted_by.to_string(),
    };

    Ok((updated, proof))
}

// ── Audit hash chain (S3) ───────────────────────────────────────────────────

/// Compute the hash of one audit row. Pure; shared with `permagent doctor`'s
/// chain-integrity check. NULLs hash as empty strings; the genesis row's
/// prev_hash is the empty string.
///
/// One positional argument per hashed audit column, in chain order — a struct
/// would obscure the field order the hash depends on.
#[allow(clippy::too_many_arguments)]
pub fn compute_audit_row_hash(
    prev_hash: &str,
    decision_id: &str,
    goal_id: &str,
    acted_by: &str,
    tier: i64,
    outcome: &str,
    evidence_digest: &str,
    created_at: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(prev_hash.as_bytes());
    hasher.update(b"|");
    hasher.update(decision_id.as_bytes());
    hasher.update(b"|");
    hasher.update(goal_id.as_bytes());
    hasher.update(b"|");
    hasher.update(acted_by.as_bytes());
    hasher.update(b"|");
    hasher.update(tier.to_string().as_bytes());
    hasher.update(b"|");
    hasher.update(outcome.as_bytes());
    hasher.update(b"|");
    hasher.update(evidence_digest.as_bytes());
    hasher.update(b"|");
    hasher.update(created_at.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn now_timestamp() -> String {
    chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

/// Append a hash-chained audit row inside an existing transaction.
///
/// Crate-internal: only the decisions module and the goal-transition guard
/// may write audit rows.
pub(crate) async fn append_audit_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    decision_id: &str,
    goal_id: Option<&str>,
    acted_by: &str,
    tier: i64,
    outcome: &str,
    evidence_digest: Option<&str>,
) -> Result<(), String> {
    let prev_hash: Option<String> =
        sqlx::query_scalar("SELECT row_hash FROM decision_audit ORDER BY seq DESC LIMIT 1")
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;

    let created_at = now_timestamp();
    let row_hash = compute_audit_row_hash(
        prev_hash.as_deref().unwrap_or(""),
        decision_id,
        goal_id.unwrap_or(""),
        acted_by,
        tier,
        outcome,
        evidence_digest.unwrap_or(""),
        &created_at,
    );

    sqlx::query(
        "INSERT INTO decision_audit (decision_id, goal_id, acted_by, tier, outcome, \
         evidence_digest, prev_hash, row_hash, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(decision_id)
    .bind(goal_id)
    .bind(acted_by)
    .bind(tier)
    .bind(outcome)
    .bind(evidence_digest)
    .bind(&prev_hash)
    .bind(&row_hash)
    .bind(&created_at)
    .execute(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}

/// Record a post-answer effect outcome (success detail or failure) as an
/// audit row chained to the decision.
pub async fn record_effect_outcome(
    pool: &Pool<Sqlite>,
    decision: &Decision,
    outcome: &str,
) -> Result<(), String> {
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    append_audit_tx(
        &mut tx,
        &decision.id,
        decision.goal_id.as_deref(),
        decision.acted_by.as_deref().unwrap_or(ACTOR_SYSTEM),
        decision.tier,
        outcome,
        None,
    )
    .await?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(())
}

/// Result of walking the audit hash chain.
#[derive(Debug, Clone, Serialize)]
pub struct AuditChainReport {
    pub total_rows: u64,
    pub intact: bool,
    /// seq of the first broken row, if any.
    pub break_seq: Option<i64>,
    pub detail: String,
}

/// Walk `decision_audit` in seq order verifying prev_hash linkage and
/// recomputing each row_hash. Reports the first break point.
pub async fn verify_audit_chain(pool: &Pool<Sqlite>) -> Result<AuditChainReport, String> {
    let rows = sqlx::query(
        "SELECT seq, decision_id, goal_id, acted_by, tier, outcome, evidence_digest, \
         prev_hash, row_hash, created_at FROM decision_audit ORDER BY seq ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut expected_prev = String::new();
    for r in &rows {
        let seq: i64 = r.get("seq");
        let prev_hash: Option<String> = r.get("prev_hash");
        let row_hash: String = r.get("row_hash");
        let goal_id: Option<String> = r.get("goal_id");
        let evidence: Option<String> = r.get("evidence_digest");

        let stored_prev = prev_hash.unwrap_or_default();
        if stored_prev != expected_prev {
            return Ok(AuditChainReport {
                total_rows: rows.len() as u64,
                intact: false,
                break_seq: Some(seq),
                detail: format!(
                    "prev_hash linkage broken at seq {} (expected '{}', stored '{}')",
                    seq, expected_prev, stored_prev
                ),
            });
        }

        let recomputed = compute_audit_row_hash(
            &stored_prev,
            r.get::<String, _>("decision_id").as_str(),
            goal_id.as_deref().unwrap_or(""),
            r.get::<String, _>("acted_by").as_str(),
            r.get::<i64, _>("tier"),
            r.get::<String, _>("outcome").as_str(),
            evidence.as_deref().unwrap_or(""),
            r.get::<String, _>("created_at").as_str(),
        );
        if recomputed != row_hash {
            return Ok(AuditChainReport {
                total_rows: rows.len() as u64,
                intact: false,
                break_seq: Some(seq),
                detail: format!(
                    "row_hash mismatch at seq {} (row contents do not match stored hash)",
                    seq
                ),
            });
        }
        expected_prev = row_hash;
    }

    Ok(AuditChainReport {
        total_rows: rows.len() as u64,
        intact: true,
        break_seq: None,
        detail: format!("{} audit row(s), chain intact", rows.len()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn valid_unblock() -> NewDecision {
        NewDecision {
            kind: "unblock".to_string(),
            project_id: Some(PERSONAL_PROJECT_ID.to_string()),
            headline: Some("A goal is stuck and needs your direction".to_string()),
            detail: Some("attempt_cap reached: 3/3 attempts spent".to_string()),
            payload: serde_json::json!({"reason": "attempt_cap", "spent": 3, "cap": 3}),
            ..Default::default()
        }
    }

    // ── S2: typed payloads, malformed fallback ──

    #[tokio::test]
    async fn valid_decision_created_with_tier_from_policy() {
        let pool = test_pool().await;
        let d = create_decision(&pool, valid_unblock()).await.unwrap();
        assert_eq!(d.kind, "unblock");
        assert_eq!(d.tier, 1, "goal_retry_within_budget seeds at tier 1");
        assert_eq!(d.status, "open");
    }

    #[tokio::test]
    async fn malformed_payload_becomes_malformed_row_never_coerced() {
        let pool = test_pool().await;
        let mut req = valid_unblock();
        // unknown field violates deny_unknown_fields
        req.payload = serde_json::json!({"reason": "attempt_cap", "bogus_field": true});
        let d = create_decision(&pool, req).await.unwrap();
        assert_eq!(d.kind, "malformed");
        assert_eq!(d.tier, 2, "malformed rows are tier-2 fail-closed");
        assert_eq!(
            d.payload.get("original_kind").and_then(|v| v.as_str()),
            Some("unblock")
        );
        assert!(d.payload.get("error").is_some());
        assert!(d.payload.get("raw").is_some());
    }

    #[tokio::test]
    async fn missing_headline_or_detail_is_malformed() {
        let pool = test_pool().await;

        let mut no_headline = valid_unblock();
        no_headline.headline = None;
        let d = create_decision(&pool, no_headline).await.unwrap();
        assert_eq!(d.kind, "malformed");

        let mut no_detail = valid_unblock();
        no_detail.detail = Some("   ".to_string());
        let d = create_decision(&pool, no_detail).await.unwrap();
        assert_eq!(d.kind, "malformed");
    }

    #[tokio::test]
    async fn headline_over_80_chars_is_malformed() {
        let pool = test_pool().await;
        let mut req = valid_unblock();
        req.headline = Some("x".repeat(81));
        let d = create_decision(&pool, req).await.unwrap();
        assert_eq!(d.kind, "malformed");
    }

    #[tokio::test]
    async fn choice_option_count_enforced() {
        let pool = test_pool().await;
        let req = NewDecision {
            kind: "choice".to_string(),
            headline: Some("Pick a direction for the project".to_string()),
            detail: Some("technical context".to_string()),
            payload: serde_json::json!({
                "question": "Which?",
                "options": [{"id": "a", "label": "Only one"}]
            }),
            ..Default::default()
        };
        let d = create_decision(&pool, req).await.unwrap();
        assert_eq!(d.kind, "malformed");
    }

    #[tokio::test]
    async fn unknown_action_class_fails_closed_to_tier_2() {
        let pool = test_pool().await;
        let req = NewDecision {
            kind: "risk_gate".to_string(),
            headline: Some("Something unusual wants permission".to_string()),
            detail: Some("technical".to_string()),
            payload: serde_json::json!({
                "action_class": "never_seeded_class",
                "description": "?",
                "requested_by": "test"
            }),
            ..Default::default()
        };
        let d = create_decision(&pool, req).await.unwrap();
        assert_eq!(d.kind, "risk_gate");
        assert_eq!(d.tier, 2, "unknown action_class must fail closed");
    }

    // ── S5: tier gates on answering ──

    #[tokio::test]
    async fn tier2_requires_jesse() {
        let pool = test_pool().await;
        let req = NewDecision {
            kind: "risk_gate".to_string(),
            headline: Some("Permission to delete saved work".to_string()),
            detail: Some("user_data_deletion of goal X".to_string()),
            payload: serde_json::json!({
                "action_class": "user_data_deletion",
                "description": "delete goal",
                "requested_by": "test"
            }),
            ..Default::default()
        };
        let d = create_decision(&pool, req).await.unwrap();
        assert_eq!(d.tier, 2);

        let ans = DecisionAnswer {
            answer: "approve".to_string(),
            ..Default::default()
        };
        let err = answer_decision(&pool, &d.id, &ans, ACTOR_HENRY)
            .await
            .unwrap_err();
        assert!(
            matches!(err, AnswerError::Forbidden(_)),
            "henry-policy must not answer tier-2: {:?}",
            err
        );
        let err = answer_decision(&pool, &d.id, &ans, ACTOR_SYSTEM)
            .await
            .unwrap_err();
        assert!(matches!(err, AnswerError::Forbidden(_)));

        // Jesse can.
        let (answered, proof) = answer_decision(&pool, &d.id, &ans, ACTOR_JESSE)
            .await
            .unwrap();
        assert_eq!(answered.status, "answered");
        assert_eq!(proof.acted_by(), ACTOR_JESSE);
        assert_eq!(proof.tier(), 2);
    }

    #[tokio::test]
    async fn tier1_allows_henry_but_not_system() {
        let pool = test_pool().await;
        let d = create_decision(&pool, valid_unblock()).await.unwrap();
        assert_eq!(d.tier, 1);

        let ans = DecisionAnswer {
            answer: "reject".to_string(),
            ..Default::default()
        };
        let err = answer_decision(&pool, &d.id, &ans, ACTOR_SYSTEM)
            .await
            .unwrap_err();
        assert!(matches!(err, AnswerError::Forbidden(_)));

        let (answered, proof) = answer_decision(&pool, &d.id, &ans, ACTOR_HENRY)
            .await
            .unwrap();
        assert_eq!(answered.acted_by.as_deref(), Some(ACTOR_HENRY));
        assert_eq!(proof.answer(), "reject");
    }

    #[tokio::test]
    async fn answering_twice_is_already_resolved() {
        let pool = test_pool().await;
        let d = create_decision(&pool, valid_unblock()).await.unwrap();
        let ans = DecisionAnswer {
            answer: "approve".to_string(),
            ..Default::default()
        };
        answer_decision(&pool, &d.id, &ans, ACTOR_JESSE)
            .await
            .unwrap();
        let err = answer_decision(&pool, &d.id, &ans, ACTOR_JESSE)
            .await
            .unwrap_err();
        assert!(matches!(err, AnswerError::AlreadyResolved(_)));
    }

    #[tokio::test]
    async fn choice_answer_validates_option_id() {
        let pool = test_pool().await;
        let req = NewDecision {
            kind: "choice".to_string(),
            headline: Some("Pick the colour for the new room".to_string()),
            detail: Some("technical context".to_string()),
            payload: serde_json::json!({
                "question": "Which colour?",
                "options": [
                    {"id": "red", "label": "Red"},
                    {"id": "blue", "label": "Blue"}
                ]
            }),
            ..Default::default()
        };
        let d = create_decision(&pool, req).await.unwrap();

        let bad = DecisionAnswer {
            answer: "choice".to_string(),
            choice_id: Some("green".to_string()),
            ..Default::default()
        };
        let err = answer_decision(&pool, &d.id, &bad, ACTOR_JESSE)
            .await
            .unwrap_err();
        assert!(matches!(err, AnswerError::Invalid(_)));

        let good = DecisionAnswer {
            answer: "choice".to_string(),
            choice_id: Some("blue".to_string()),
            ..Default::default()
        };
        let (answered, _proof) = answer_decision(&pool, &d.id, &good, ACTOR_JESSE)
            .await
            .unwrap();
        assert_eq!(answered.answer_choice_id.as_deref(), Some("blue"));
    }

    // ── S3: audit hash chain ──

    #[tokio::test]
    async fn audit_chain_verifies_and_detects_tampering() {
        let pool = test_pool().await;

        // Generate several audit rows: 2 creates + 2 answers.
        let d1 = create_decision(&pool, valid_unblock()).await.unwrap();
        let d2 = create_decision(&pool, valid_unblock()).await.unwrap();
        let ans = DecisionAnswer {
            answer: "approve".to_string(),
            ..Default::default()
        };
        answer_decision(&pool, &d1.id, &ans, ACTOR_JESSE)
            .await
            .unwrap();
        answer_decision(&pool, &d2.id, &ans, ACTOR_HENRY)
            .await
            .unwrap();

        let report = verify_audit_chain(&pool).await.unwrap();
        assert!(report.intact, "fresh chain must verify: {}", report.detail);
        assert_eq!(report.total_rows, 4);

        // Forge a row with a fabricated hash (INSERT is allowed; UPDATE/DELETE
        // are blocked by triggers). The chain walk must detect it.
        sqlx::query(
            "INSERT INTO decision_audit (decision_id, goal_id, acted_by, tier, outcome, \
             prev_hash, row_hash, created_at) VALUES (?, NULL, 'jesse', 2, 'approve', \
             'forged-prev', 'forged-hash', '2026-06-11T00:00:00.000Z')",
        )
        .bind(&d1.id)
        .execute(&pool)
        .await
        .unwrap();

        let report = verify_audit_chain(&pool).await.unwrap();
        assert!(!report.intact, "forged row must break the chain");
        assert!(report.break_seq.is_some());
    }

    #[tokio::test]
    async fn audit_rows_are_append_only() {
        let pool = test_pool().await;
        create_decision(&pool, valid_unblock()).await.unwrap();

        let upd = sqlx::query("UPDATE decision_audit SET outcome = 'tampered' WHERE seq = 1")
            .execute(&pool)
            .await;
        assert!(upd.is_err(), "UPDATE on decision_audit must be blocked");

        let del = sqlx::query("DELETE FROM decision_audit WHERE seq = 1")
            .execute(&pool)
            .await;
        assert!(del.is_err(), "DELETE on decision_audit must be blocked");
    }

    // ── Inbox queries ──

    #[tokio::test]
    async fn inbox_lists_open_ranked_and_summarizes() {
        let pool = test_pool().await;
        let mut low = valid_unblock();
        low.rank = Some(0.1);
        let mut high = valid_unblock();
        high.rank = Some(0.9);
        let low = create_decision(&pool, low).await.unwrap();
        let high = create_decision(&pool, high).await.unwrap();

        let ans = DecisionAnswer {
            answer: "approve".to_string(),
            ..Default::default()
        };
        answer_decision(&pool, &low.id, &ans, ACTOR_JESSE)
            .await
            .unwrap();

        let open = list_open_decisions(&pool).await.unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].decision.id, high.id);

        let summary = inbox_summary(&pool).await.unwrap();
        assert_eq!(summary.total_pending, 1);
        assert_eq!(summary.handled_count, 1);
        assert!(summary.oldest_pending_at.is_some());

        let history = decision_history(&pool, 50, None).await.unwrap();
        assert!(
            history.iter().any(|h| h.decision.id == low.id),
            "resolved decision must appear in history"
        );
    }
}
