use crate::agents::platform_extensions::terminal_supervision::HarnessRunSnapshot;
use crate::config::paths::Paths;
use crate::config::GooseMode;
use crate::conversation::message::Message;
use crate::conversation::Conversation;
use crate::model::ModelConfig;
use crate::providers::base::{Provider, MSG_COUNT_FOR_SESSION_NAME_GENERATION};
use crate::recipe::Recipe;
use crate::session::extension_data::ExtensionData;
use anyhow::Result;
use chrono::{DateTime, Utc};
use rmcp::model::Role;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Pool, Sqlite};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use tracing::info;
use utoipa::ToSchema;

/// Default user ID for Phase 1 single-user operation (Section B.0).
pub const DEFAULT_USER_ID: &str = "default";

/// Extension-data key for the durable budget identity of the current reply.
/// Keeping this in the existing session JSON avoids a schema migration while
/// making the identity survive compaction, retries, resume, and daemon restart.
pub const BUDGET_TASK_EXTENSION_NAME: &str = "budget_task";
pub const BUDGET_TASK_EXTENSION_VERSION: &str = "v1";
const BUDGET_TASK_CONTRACT_EXTENSION_NAME: &str = "budget_task_contract";
pub const MAX_BUDGET_TASK_CONTRACT_BYTES: usize = 32 * 1024;

/// Only the origin captured with the CURRENT budget identity is authoritative.
/// A legacy task or a changed identity must never reuse another task's brief.
pub fn budget_task_contract(extension_data: &ExtensionData) -> Option<&serde_json::Value> {
    let task_id = extension_data
        .get_extension_state(BUDGET_TASK_EXTENSION_NAME, BUDGET_TASK_EXTENSION_VERSION)?
        .as_str()?;
    let contract = extension_data.get_extension_state(BUDGET_TASK_CONTRACT_EXTENSION_NAME, "v1")?;
    (contract.get("task_id")?.as_str()? == task_id).then_some(contract)
}

/// Extension-data key for the goal card whose worker owns this session.
/// Keeping this separate from the budget identity lets ledger writers recover
/// attribution from durable session state without inspecting process env.
pub const GOAL_ID_EXTENSION_NAME: &str = "goal_worker";
pub const GOAL_ID_EXTENSION_VERSION: &str = "v1";

/// Extension-data key for X-verify-1's reviewer-mutation receipts: durable,
/// append-only evidence that a `worker_persona: "reviewer"` subagent mutated
/// the working tree despite its READ-ONLY mandate (F2/H1). Kept on the
/// PARENT session — the one that issued the `delegate` call — since the
/// reviewer's own subagent session is throwaway and its record is not
/// necessarily read again.
pub const REVIEWER_MUTATION_EXTENSION_NAME: &str = "reviewer_mutation_receipts";
pub const REVIEWER_MUTATION_EXTENSION_VERSION: &str = "v1";

/// One receipt: the reviewer mutated these paths at this time.
/// [`SessionManager::record_reviewer_mutation`] APPENDS to the stored list
/// rather than replacing it, so a session with several delegate rounds keeps
/// every occurrence rather than only the latest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewerMutationReceipt {
    pub paths: Vec<String>,
    pub at: String,
    #[serde(default)]
    pub subagent_session_id: Option<String>,
}

/// Every reviewer-mutation receipt recorded on this session so far, oldest
/// first. Empty (never an error) for a session with no such receipts,
/// including one predating this field entirely.
pub fn reviewer_mutation_receipts(extension_data: &ExtensionData) -> Vec<ReviewerMutationReceipt> {
    extension_data
        .get_extension_state(
            REVIEWER_MUTATION_EXTENSION_NAME,
            REVIEWER_MUTATION_EXTENSION_VERSION,
        )
        .and_then(|v| serde_json::from_value::<Vec<ReviewerMutationReceipt>>(v.clone()).ok())
        .unwrap_or_default()
}

pub fn budget_task_id(extension_data: &ExtensionData) -> Option<String> {
    extension_data
        .get_extension_state(BUDGET_TASK_EXTENSION_NAME, BUDGET_TASK_EXTENSION_VERSION)
        .and_then(|value| value.as_str())
        .filter(|id| !id.trim().is_empty())
        .map(ToOwned::to_owned)
}

pub fn goal_id(extension_data: &ExtensionData) -> Option<String> {
    extension_data
        .get_extension_state(GOAL_ID_EXTENSION_NAME, GOAL_ID_EXTENSION_VERSION)
        .and_then(|value| value.as_str())
        .filter(|id| !id.trim().is_empty())
        .map(ToOwned::to_owned)
}

#[derive(
    Debug,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    ToSchema,
    PartialEq,
    Eq,
    Default,
    strum::Display,
    strum::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum SessionType {
    #[default]
    User,
    Scheduled,
    SubAgent,
    Hidden,
    Terminal,
    Gateway,
    Acp,
}

impl SessionType {
    /// Is a human present, watching this turn as it happens?
    ///
    /// `User` (Chat / voice / the CLI's own session) and `Terminal` are the two
    /// surfaces where a person is typing and reading the reply. Everything else
    /// — `SubAgent` (goal workers and summoned children), `Scheduled` (cron
    /// recipes), `Hidden` (`--no-session` and internal helper runs), `Gateway`
    /// and `Acp` — runs with nobody watching.
    ///
    /// This is the single definition of "a human is in the loop" in the tree.
    /// It gates the cost ledger's `is_headless` column and, since D30, which
    /// sessions hold the decision-answering capability. Fail-safe by
    /// construction: the list is an ALLOW list, so a new session type is
    /// non-interactive until someone deliberately adds it here.
    pub fn is_interactive(self) -> bool {
        matches!(self, Self::User | Self::Terminal)
    }
}

static SESSION_STORAGE: LazyLock<Arc<SessionStorage>> =
    LazyLock::new(|| Arc::new(SessionStorage::new_spectral()));

/// Billing tier of a provider call. Stored as TEXT in `cost_ledger.cost_tier`
/// (no SQLite `CHECK` — validated here in Rust to avoid the widen-in-two-places
/// footgun). `LocalFree` (Ollama et al.) and in-quota `Subscription` calls are
/// not chargeable and record `cost_usd = 0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostTier {
    /// Runs on local hardware — no marginal dollar cost (e.g. Ollama).
    LocalFree,
    /// Covered by a flat subscription / quota — no per-call charge.
    Subscription,
    /// Metered pay-per-token API — the call costs real money.
    PaidApi,
}

impl CostTier {
    pub fn as_str(self) -> &'static str {
        match self {
            CostTier::LocalFree => "local_free",
            CostTier::Subscription => "subscription",
            CostTier::PaidApi => "paid_api",
        }
    }

    /// Whether a call in this tier incurs a real marginal dollar cost.
    pub fn is_chargeable(self) -> bool {
        matches!(self, CostTier::PaidApi)
    }
}

/// The seven usage columns an authoritative ledger row carries, in the order the
/// insert binds them: model, input, output, total, and the three cache counts.
/// Named because the bare tuple tripped `clippy::type_complexity` and read as
/// noise at every call site.
type LedgerUsageColumns = (
    Option<String>,
    Option<i32>,
    Option<i32>,
    Option<i32>,
    i32,
    i32,
    i32,
);

// ── Per-call purpose and outcome (schema v60, node A4) ──────────────────────
//
// Six nullable `cost_ledger` columns that make a candidate call reviewable
// after the fact: why it was made (`call_purpose`, `trigger_ref`,
// `retry_of_call_id`), what was known before dispatch (`pre_call_evidence`),
// and what came back (`tool_call_count`, `completion_kind`).
//
// Provenance is supplied by the CALLER before dispatch and carried on
// `ProviderInvocationContext` to the ledger row; the database writer persists
// it and never invents it. A caller that does not genuinely know leaves the
// default, which is "unknown" throughout. Rows written before v60 are NULL,
// which also means unknown, and are never backfilled.

/// Why a provider call was made (`cost_ledger.call_purpose`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CallPurpose {
    /// The first provider call answering a user message.
    UserResponse,
    /// A call made because tool results came back and the model continues.
    ToolContinuation,
    /// A scheduled or background look at state (polling, monitors).
    BackgroundCheck,
    /// A re-dispatch after an earlier invocation failed; see
    /// [`CallProvenance::retry_of_call_id`].
    Retry,
    /// A call whose job is to check other work.
    Verification,
    /// Compaction, tool-pair summarisation and other context upkeep.
    ContextMaintenance,
    /// 2026-09-25 DAG K cost fix (HARNESS_COST_TRACE.md Q1): a roadmap
    /// decomposition's planner call — `run_decomposition`
    /// (`platform_extensions::orchestrator`), routed through
    /// `AccountedFastCompletion::complete_accounted_with_purpose`. Before
    /// this variant existed every such call fell through to `Unknown`
    /// (`complete_accounted`/`complete_fast_accounted` hardcoded
    /// `CallProvenance::default()`), which is why tracing a planner
    /// escalation's cost back to its cause required manual ledger
    /// archaeology instead of a `call_purpose` filter.
    Planning,
    /// The caller did not know. The honest default.
    #[default]
    Unknown,
}

impl CallPurpose {
    pub fn as_str(self) -> &'static str {
        match self {
            CallPurpose::UserResponse => "user_response",
            CallPurpose::ToolContinuation => "tool_continuation",
            CallPurpose::BackgroundCheck => "background_check",
            CallPurpose::Retry => "retry",
            CallPurpose::Verification => "verification",
            CallPurpose::ContextMaintenance => "context_maintenance",
            CallPurpose::Planning => "planning",
            CallPurpose::Unknown => "unknown",
        }
    }
}

/// What the provider response turned out to be (`cost_ledger.completion_kind`).
/// `Unknown` is deliberately distinct from `Empty`: "we did not observe the
/// response shape" is not the same fact as "the response had nothing in it".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompletionKind {
    Text,
    Tools,
    Mixed,
    Empty,
    Error,
    Cancelled,
    #[default]
    Unknown,
}

impl CompletionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CompletionKind::Text => "text",
            CompletionKind::Tools => "tools",
            CompletionKind::Mixed => "mixed",
            CompletionKind::Empty => "empty",
            CompletionKind::Error => "error",
            CompletionKind::Cancelled => "cancelled",
            CompletionKind::Unknown => "unknown",
        }
    }

    /// Classify an observed, successfully completed response by its shape:
    /// whether it carried non-blank text and how many completed tool requests.
    /// Thinking/reasoning content alone is not text.
    pub fn from_shape(has_text: bool, tool_calls: usize) -> Self {
        match (has_text, tool_calls > 0) {
            (true, true) => CompletionKind::Mixed,
            (true, false) => CompletionKind::Text,
            (false, true) => CompletionKind::Tools,
            (false, false) => CompletionKind::Empty,
        }
    }
}

/// The proposed skip rule's verdict recorded in [`PreCallEvidence`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WouldSkip {
    Yes,
    No,
    Unknown,
}

impl WouldSkip {
    pub fn as_str(self) -> &'static str {
        match self {
            WouldSkip::Yes => "yes",
            WouldSkip::No => "no",
            WouldSkip::Unknown => "unknown",
        }
    }
}

/// Upper bound on a serialized `pre_call_evidence` value, in bytes.
pub const PRE_CALL_EVIDENCE_MAX_BYTES: usize = 4096;

/// Upper bound on a `trigger_ref`, in bytes. A reference ("message:<id>",
/// "tool_result:<tool_call_id>", "schedule:<id>:<tick>") is short by design.
pub const TRIGGER_REF_MAX_BYTES: usize = 256;

/// Longest predicate name a truncation marker will carry.
const PRE_CALL_EVIDENCE_MARKER_PREDICATE_MAX_BYTES: usize = 256;

/// A versioned, bounded record of the facts available BEFORE dispatch plus the
/// proposed skip rule's result (`cost_ledger.pre_call_evidence`):
///
/// `{"v":1,"predicate":"<name>@<version>","inputs":{...},"would_skip":"yes"|"no"|"unknown"}`
///
/// `inputs` must be the predicate's actual inputs, not a state hash or a
/// model's assertion. The serialized value is capped at
/// [`PRE_CALL_EVIDENCE_MAX_BYTES`]; an oversize record is replaced by a
/// complete, well-formed truncation marker
/// (`{"v":1,"predicate":..,"truncated":true,"original_bytes":N,"would_skip":..}`),
/// never by cut-off JSON.
///
/// Plumbing only as of v60: no caller populates it yet (it is for a later
/// polling experiment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreCallEvidence {
    json: String,
    truncated: bool,
}

impl PreCallEvidence {
    pub const VERSION: u32 = 1;

    /// Build the record. `predicate` is `<name>@<version>`.
    pub fn new(predicate: &str, inputs: serde_json::Value, would_skip: WouldSkip) -> Self {
        let full = serde_json::json!({
            "v": Self::VERSION,
            "predicate": predicate,
            "inputs": inputs,
            "would_skip": would_skip.as_str(),
        })
        .to_string();
        if full.len() <= PRE_CALL_EVIDENCE_MAX_BYTES {
            return Self {
                json: full,
                truncated: false,
            };
        }
        // Keep the marker itself bounded: an absurd predicate name is dropped
        // (and said so) rather than cut.
        let predicate_value = if predicate.len() <= PRE_CALL_EVIDENCE_MARKER_PREDICATE_MAX_BYTES {
            serde_json::Value::String(predicate.to_string())
        } else {
            serde_json::Value::Null
        };
        let marker = serde_json::json!({
            "v": Self::VERSION,
            "predicate": predicate_value,
            "truncated": true,
            "original_bytes": full.len(),
            "would_skip": would_skip.as_str(),
        })
        .to_string();
        Self {
            json: marker,
            truncated: true,
        }
    }

    /// The exact JSON persisted to `cost_ledger.pre_call_evidence`.
    pub fn as_json(&self) -> &str {
        &self.json
    }

    /// Whether the record exceeded the bound and was replaced by a marker.
    pub fn is_truncated(&self) -> bool {
        self.truncated
    }
}

/// Caller-supplied, pre-dispatch provenance for one provider call. The
/// default is all-unknown, so a call site that does not know stays honest.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CallProvenance {
    pub purpose: CallPurpose,
    /// The user event, tool result, schedule tick or state change that caused
    /// this call, e.g. `message:<id>`, `tool_result:<tool_call_id>`,
    /// `schedule:<schedule_id>:<tick>`. `None` when not known.
    pub trigger_ref: Option<String>,
    /// For [`CallPurpose::Retry`], the `call_id` (invocation id) of the earlier
    /// failed invocation. That invocation may have no ledger row of its own
    /// (a failed call with no usage is recorded only in `cost_reservations`).
    pub retry_of_call_id: Option<String>,
    pub pre_call_evidence: Option<PreCallEvidence>,
}

impl CallProvenance {
    /// A call whose purpose the caller genuinely knows.
    pub fn with_purpose(purpose: CallPurpose) -> Self {
        Self {
            purpose,
            ..Self::default()
        }
    }

    /// A re-dispatch after the invocation `failed_call_id` failed.
    pub fn retry_of(failed_call_id: impl Into<String>) -> Self {
        Self {
            purpose: CallPurpose::Retry,
            retry_of_call_id: Some(failed_call_id.into()),
            ..Self::default()
        }
    }

    /// Attach a trigger reference. An empty or oversize reference is dropped
    /// (left unknown) rather than cut, since a cut reference would point at
    /// nothing.
    pub fn with_trigger(mut self, trigger_ref: impl Into<String>) -> Self {
        let trigger_ref = trigger_ref.into();
        if trigger_ref.is_empty() || trigger_ref.len() > TRIGGER_REF_MAX_BYTES {
            tracing::debug!(
                len = trigger_ref.len(),
                "call provenance trigger_ref empty or over bound; leaving it unknown"
            );
            self.trigger_ref = None;
        } else {
            self.trigger_ref = Some(trigger_ref);
        }
        self
    }

    pub fn with_evidence(mut self, evidence: PreCallEvidence) -> Self {
        self.pre_call_evidence = Some(evidence);
        self
    }
}

/// What response processing observed about one provider call. The default
/// (`tool_call_count: None`, `completion_kind: Unknown`) is "not observed",
/// which persists as NULL / `'unknown'` and never as zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CallOutcome {
    /// Completed output tool requests (not streaming fragments).
    pub tool_call_count: Option<i64>,
    pub completion_kind: CompletionKind,
}

impl CallOutcome {
    /// Outcome of a response whose shape was observed in full.
    pub fn observed(has_text: bool, tool_calls: usize) -> Self {
        Self {
            tool_call_count: Some(tool_calls as i64),
            completion_kind: CompletionKind::from_shape(has_text, tool_calls),
        }
    }

    /// Outcome of one complete (non-streamed) response message.
    pub fn from_message(message: &Message) -> Self {
        let (has_text, tool_calls) = message_shape(message);
        Self::observed(has_text, tool_calls)
    }
}

/// `(has non-blank text, completed tool request count)` for one message.
pub fn message_shape(message: &Message) -> (bool, usize) {
    use crate::conversation::message::MessageContent;
    let mut has_text = false;
    let mut tool_calls = 0usize;
    for content in &message.content {
        match content {
            MessageContent::Text(text) if !text.text.trim().is_empty() => has_text = true,
            MessageContent::ToolRequest(_) => tool_calls += 1,
            _ => {}
        }
    }
    (has_text, tool_calls)
}

/// Accumulates the response shape across the frames of one streamed provider
/// invocation. Tool requests arrive as complete `ToolRequest` contents (the
/// stream layer assembles argument fragments before emitting one), so
/// counting them per frame counts completed requests, not fragments.
#[derive(Debug, Clone, Copy, Default)]
pub struct StreamOutcomeTracker {
    has_text: bool,
    tool_calls: usize,
    saw_frame: bool,
    last_frame_was_error: bool,
}

impl StreamOutcomeTracker {
    pub fn observe_message(&mut self, message: &Message) {
        let (has_text, tool_calls) = message_shape(message);
        self.has_text |= has_text;
        self.tool_calls += tool_calls;
    }

    /// Record whether the latest stream frame was an error. A later good
    /// frame (e.g. after a first-byte-stall retry) clears it.
    pub fn observe_frame(&mut self, is_error: bool) {
        self.saw_frame = true;
        self.last_frame_was_error = is_error;
    }

    pub fn finish(&self, cancelled: bool) -> CallOutcome {
        let tool_call_count = Some(self.tool_calls as i64);
        if cancelled {
            return CallOutcome {
                tool_call_count,
                completion_kind: CompletionKind::Cancelled,
            };
        }
        if self.last_frame_was_error {
            return CallOutcome {
                tool_call_count,
                completion_kind: CompletionKind::Error,
            };
        }
        if !self.saw_frame {
            return CallOutcome::default();
        }
        CallOutcome::observed(self.has_text, self.tool_calls)
    }
}

/// One append-only per-call cost row. Every money field is the output of the
/// single canonical [`crate::providers::canonical::cost_of`] function, so the
/// ledger can never disagree with the live meter or the verification digest.
/// `turn_index` is assigned atomically at append time (the count of prior rows
/// for the session), so it is not a field here.
#[derive(Debug, Clone)]
pub struct CostLedgerRow {
    pub call_id: String,
    pub ts: String,
    pub session_id: String,
    pub parent_session_id: Option<String>,
    pub task_id: Option<String>,
    pub goal_id: Option<String>,
    /// Native execution attempt resolved from this exact provider-calling
    /// session before dispatch. `None` is honest for legacy/unverified calls.
    pub goal_attempt_id: Option<String>,
    pub subagent_id: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub cost_tier: CostTier,
    pub is_headless: bool,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub input_cost: f64,
    pub output_cost: f64,
    pub cache_read_cost: f64,
    pub cache_write_cost: f64,
    pub cost_usd: f64,
    /// Dollars saved by cache reads this call — rolled into the session's
    /// `accumulated_cache_savings_usd` (not itself a ledger column).
    pub cache_savings_usd: f64,
    /// True when tokens were estimated (no exact provider usage) rather than
    /// reported.
    pub is_estimated: bool,
    /// v60: caller-supplied pre-dispatch provenance (`call_purpose`,
    /// `trigger_ref`, `retry_of_call_id`, `pre_call_evidence`). Default =
    /// unknown.
    pub provenance: CallProvenance,
    /// v60: response-derived outcome (`tool_call_count`, `completion_kind`).
    /// Default = not observed.
    pub outcome: CallOutcome,
}

/// Immutable scope captured when a native worker call is authorized. The
/// reservation keeps these exact values through retries and late settlement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderReservationIdentity {
    pub goal_attempt_id: Option<String>,
    pub provider: String,
    pub model: String,
}

/// Result of atomically acquiring a provider-spend lease. Only `Granted`
/// permits a paid invocation to start; all other variants are deterministic
/// outcomes, not transport errors.
#[derive(Debug, Clone, PartialEq)]
pub enum CostReservationOutcome {
    Granted {
        reservation_id: String,
    },
    AlreadyReserved {
        reservation_id: String,
    },
    AlreadySettled {
        reservation_id: String,
    },
    /// Exact invocation identity may already have reached transport. Refuse
    /// replay while retaining the existing pending monetary hold unchanged.
    AlreadyDispatched {
        reservation_id: String,
    },
    NeedsGate {
        scope: crate::cost_router::budget::BudgetScope,
        spent_usd: f64,
        held_usd: f64,
        requested_usd: f64,
        ceiling_usd: f64,
    },
    Refused {
        scope: crate::cost_router::budget::BudgetScope,
        spent_usd: f64,
        held_usd: f64,
        requested_usd: f64,
        ceiling_usd: f64,
    },
    Unknown {
        reason: String,
    },
}

/// Extension uses the same granted/gate/refused/unknown vocabulary as a hold.
pub type ExtendOutcome = CostReservationOutcome;

#[derive(Serialize, Deserialize)]
struct ReservationExtension {
    attempts: u32,
    per_attempt_usd: f64,
    config: crate::cost_router::budget::BudgetConfig,
}

/// Sanitized task and session ceilings, or `None` when any threshold is
/// unavailable or invalid (authorization then fails closed as `Unknown`).
/// Shared by the single-invocation reservation path and K1b batch admission.
pub(super) fn reservation_ceilings(
    config: &crate::cost_router::budget::BudgetConfig,
) -> Option<(
    crate::cost_router::budget::BudgetCeilings,
    crate::cost_router::budget::BudgetCeilings,
)> {
    let task = config.task.sanitized();
    let session = config.session.sanitized();
    if [
        task.soft,
        task.gate,
        task.hard,
        session.soft,
        session.gate,
        session.hard,
    ]
    .iter()
    .any(|value| !value.is_finite() || *value < 0.0)
    {
        return None;
    }
    Some((task, session))
}

/// THE reservation ceiling arithmetic: settled (`cost_ledger`) plus held
/// (`pending` + `unknown` reservations) plus `amount_usd`, for the task and for
/// the whole session tree, compared hard-before-gate. `None` means the amount
/// fits under every gate. The caller owns the transaction (and commits it
/// before returning the outcome).
///
/// Factored out of `reserve_provider_invocation_inner` for K1b so a Council
/// batch is judged on the SUM of its paid slots with exactly the same
/// arithmetic a single invocation gets — not a copy of it.
pub(super) async fn reservation_ceiling_verdict(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    task_id: &str,
    session_id: &str,
    amount_usd: f64,
    task: crate::cost_router::budget::BudgetCeilings,
    session: crate::cost_router::budget::BudgetCeilings,
) -> Result<Option<CostReservationOutcome>> {
    let task_spent: f64 = sqlx::query_scalar(
        "SELECT CAST(COALESCE(SUM(cost_usd), 0.0) AS REAL)
         FROM cost_ledger WHERE task_id = ?",
    )
    .bind(task_id)
    .fetch_one(&mut **tx)
    .await?;
    let task_held: f64 = sqlx::query_scalar(
        "SELECT CAST(COALESCE(SUM(amount_usd), 0.0) AS REAL)
         FROM cost_reservations
         WHERE task_id = ? AND state IN ('pending', 'unknown')",
    )
    .bind(task_id)
    .fetch_one(&mut **tx)
    .await?;

    // Session scope includes this session and all descendants, matching
    // the durable parent-session contract rather than a process-local map.
    let session_spent: f64 = sqlx::query_scalar(
        "WITH RECURSIVE lineage(id, parent_id) AS (
             SELECT id, parent_session_id FROM sessions WHERE id = ?
             UNION ALL
             SELECT s.id, s.parent_session_id
             FROM sessions s JOIN lineage l ON s.id = l.parent_id
         ), session_tree(id) AS (
             SELECT id FROM lineage WHERE parent_id IS NULL
             UNION ALL
             SELECT s.id FROM sessions s JOIN session_tree t ON s.parent_session_id = t.id
         )
         SELECT CAST(COALESCE(SUM(l.cost_usd), 0.0) AS REAL)
         FROM cost_ledger l JOIN session_tree t ON t.id = l.session_id",
    )
    .bind(session_id)
    .fetch_one(&mut **tx)
    .await?;
    let session_held: f64 = sqlx::query_scalar(
        "WITH RECURSIVE lineage(id, parent_id) AS (
             SELECT id, parent_session_id FROM sessions WHERE id = ?
             UNION ALL
             SELECT s.id, s.parent_session_id
             FROM sessions s JOIN lineage l ON s.id = l.parent_id
         ), session_tree(id) AS (
             SELECT id FROM lineage WHERE parent_id IS NULL
             UNION ALL
             SELECT s.id FROM sessions s JOIN session_tree t ON s.parent_session_id = t.id
         )
         SELECT CAST(COALESCE(SUM(r.amount_usd), 0.0) AS REAL)
         FROM cost_reservations r JOIN session_tree t ON t.id = r.session_id
         WHERE r.state IN ('pending', 'unknown')",
    )
    .bind(session_id)
    .fetch_one(&mut **tx)
    .await?;

    for (label, value) in [
        ("task settled spend", task_spent),
        ("task held spend", task_held),
        ("session settled spend", session_spent),
        ("session held spend", session_held),
    ] {
        if !value.is_finite() || value < 0.0 {
            return Ok(Some(CostReservationOutcome::Unknown {
                reason: format!("{label} is unavailable or invalid"),
            }));
        }
    }
    let task_total = task_spent + task_held + amount_usd;
    let session_total = session_spent + session_held + amount_usd;
    if task_total >= task.hard {
        return Ok(Some(CostReservationOutcome::Refused {
            scope: crate::cost_router::budget::BudgetScope::Task,
            spent_usd: task_spent,
            held_usd: task_held,
            requested_usd: amount_usd,
            ceiling_usd: task.hard,
        }));
    }
    if session_total >= session.hard {
        return Ok(Some(CostReservationOutcome::Refused {
            scope: crate::cost_router::budget::BudgetScope::Session,
            spent_usd: session_spent,
            held_usd: session_held,
            requested_usd: amount_usd,
            ceiling_usd: session.hard,
        }));
    }
    if task_total >= task.gate {
        return Ok(Some(CostReservationOutcome::NeedsGate {
            scope: crate::cost_router::budget::BudgetScope::Task,
            spent_usd: task_spent,
            held_usd: task_held,
            requested_usd: amount_usd,
            ceiling_usd: task.gate,
        }));
    }
    if session_total >= session.gate {
        return Ok(Some(CostReservationOutcome::NeedsGate {
            scope: crate::cost_router::budget::BudgetScope::Session,
            spent_usd: session_spent,
            held_usd: session_held,
            requested_usd: amount_usd,
            ceiling_usd: session.gate,
        }));
    }
    Ok(None)
}

#[derive(Clone, Copy)]
enum ReservationRequest {
    Initial(Option<u32>),
    Extend(u32),
}

/// One child's contribution inside a [`ParentSessionCost`] rollup.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChildSessionCost {
    pub session_id: String,
    pub cost_usd: f64,
}

/// Parent-session cost rollup: this session's own spend plus every direct
/// child's. Backs `GET /api/sessions/{id}/cost` and the Build statusline's
/// "incl. N subagents $X" suffix.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ParentSessionCost {
    pub own: f64,
    pub children_total: f64,
    pub per_child: Vec<ChildSessionCost>,
}

impl ParentSessionCost {
    pub fn total(&self) -> f64 {
        self.own + self.children_total
    }

    pub fn child_count(&self) -> usize {
        self.per_child.len()
    }
}

/// What the most recent provider call on a session says about itself.
///
/// `provider`/`model` are `Option` because the ledger's columns are: a call
/// recorded without them is unusual but legal, and the meter should name what
/// it can rather than refuse to report. `estimated` is NOT optional and never
/// inferred from the other two — it is the difference between a figure and a
/// fail-closed ceiling, and it must survive a row whose model name is missing.
#[derive(Debug, Clone)]
pub struct LastCall {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub estimated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Session {
    pub id: String,
    #[schema(value_type = String)]
    pub working_dir: PathBuf,
    #[serde(alias = "description")]
    pub name: String,
    #[serde(default)]
    pub user_set_name: bool,
    #[serde(default)]
    pub session_type: SessionType,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub extension_data: ExtensionData,
    pub total_tokens: Option<i32>,
    pub input_tokens: Option<i32>,
    pub output_tokens: Option<i32>,
    pub accumulated_total_tokens: Option<i32>,
    pub accumulated_input_tokens: Option<i32>,
    pub accumulated_output_tokens: Option<i32>,
    /// Cost of the most recent provider turn (USD). Rolled up on each cost-ledger
    /// append; the live meter's "this turn" figure.
    #[serde(default)]
    pub cost_usd: Option<f64>,
    /// Running session cost (USD) = SUM(cost_ledger.cost_usd). O(1) rollup.
    #[serde(default)]
    pub accumulated_cost_usd: Option<f64>,
    #[serde(default)]
    pub accumulated_cache_read_tokens: Option<i64>,
    #[serde(default)]
    pub accumulated_cache_write_tokens: Option<i64>,
    /// Running dollars saved by cache reads vs. full input rate — the visible
    /// "cache saved $X" trust signal.
    #[serde(default)]
    pub accumulated_cache_savings_usd: Option<f64>,
    pub schedule_id: Option<String>,
    pub recipe: Option<Recipe>,
    pub user_recipe_values: Option<HashMap<String, String>>,
    pub conversation: Option<Conversation>,
    pub message_count: usize,
    pub provider_name: Option<String>,
    pub model_config: Option<ModelConfig>,
    #[serde(default)]
    pub goose_mode: GooseMode,
    #[serde(default)]
    pub thread_id: Option<String>,
    /// Session that spawned this one (SubAgent / fan-out child). `None` for
    /// top-level chats. Populated at create time via
    /// [`SessionManager::create_session_with_parent`]; copied onto each
    /// `cost_ledger` row so parent rollups do not need a join.
    #[serde(default)]
    pub parent_session_id: Option<String>,
}

pub struct SessionUpdateBuilder<'a> {
    session_manager: &'a SessionManager,
    session_id: String,
    name: Option<String>,
    user_set_name: Option<bool>,
    session_type: Option<SessionType>,
    working_dir: Option<PathBuf>,
    extension_data: Option<ExtensionData>,
    total_tokens: Option<Option<i32>>,
    input_tokens: Option<Option<i32>>,
    output_tokens: Option<Option<i32>>,
    accumulated_total_tokens: Option<Option<i32>>,
    accumulated_input_tokens: Option<Option<i32>>,
    accumulated_output_tokens: Option<Option<i32>>,
    /// Deltas folded by the DATABASE (`col = COALESCE(col,0) + ?`) rather than
    /// read-modify-written in Rust. See [`SessionUpdateBuilder::accumulate_tokens`].
    accumulated_deltas: Option<(i32, i32, i32)>,
    schedule_id: Option<Option<String>>,
    recipe: Option<Option<Recipe>>,
    user_recipe_values: Option<Option<HashMap<String, String>>>,
    provider_name: Option<Option<String>>,
    model_config: Option<Option<ModelConfig>>,
    goose_mode: Option<GooseMode>,
    thread_id: Option<Option<String>>,
}

#[derive(Serialize, ToSchema, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SessionInsights {
    pub total_sessions: usize,
    pub total_tokens: i64,
}

impl<'a> SessionUpdateBuilder<'a> {
    fn new(session_manager: &'a SessionManager, session_id: String) -> Self {
        Self {
            session_manager,
            session_id,
            name: None,
            user_set_name: None,
            session_type: None,
            working_dir: None,
            extension_data: None,
            total_tokens: None,
            input_tokens: None,
            output_tokens: None,
            accumulated_total_tokens: None,
            accumulated_input_tokens: None,
            accumulated_output_tokens: None,
            accumulated_deltas: None,
            schedule_id: None,
            recipe: None,
            user_recipe_values: None,
            provider_name: None,
            model_config: None,
            goose_mode: None,
            thread_id: None,
        }
    }

    pub async fn apply(self) -> Result<()> {
        self.session_manager.apply_update_inner(self).await
    }

    pub fn user_provided_name(mut self, name: impl Into<String>) -> Self {
        let name = name.into().trim().to_string();
        if !name.is_empty() {
            self.name = Some(name);
            self.user_set_name = Some(true);
        }
        self
    }

    pub fn system_generated_name(mut self, name: impl Into<String>) -> Self {
        let name = name.into().trim().to_string();
        if !name.is_empty() {
            self.name = Some(name);
            self.user_set_name = Some(false);
        }
        self
    }

    pub fn session_type(mut self, session_type: SessionType) -> Self {
        self.session_type = Some(session_type);
        self
    }

    pub fn working_dir(mut self, working_dir: PathBuf) -> Self {
        self.working_dir = Some(working_dir);
        self
    }

    pub fn extension_data(mut self, data: ExtensionData) -> Self {
        self.extension_data = Some(data);
        self
    }

    pub fn total_tokens(mut self, tokens: Option<i32>) -> Self {
        self.total_tokens = Some(tokens);
        self
    }

    pub fn input_tokens(mut self, tokens: Option<i32>) -> Self {
        self.input_tokens = Some(tokens);
        self
    }

    pub fn output_tokens(mut self, tokens: Option<i32>) -> Self {
        self.output_tokens = Some(tokens);
        self
    }

    pub fn accumulated_total_tokens(mut self, tokens: Option<i32>) -> Self {
        self.accumulated_total_tokens = Some(tokens);
        self
    }

    pub fn accumulated_input_tokens(mut self, tokens: Option<i32>) -> Self {
        self.accumulated_input_tokens = Some(tokens);
        self
    }

    pub fn accumulated_output_tokens(mut self, tokens: Option<i32>) -> Self {
        self.accumulated_output_tokens = Some(tokens);
        self
    }

    /// Add token deltas to the accumulated counters ATOMICALLY.
    ///
    /// The absolute setters above are a lost update when two turns share a
    /// session: each reads the current total, adds its own usage in Rust, and
    /// writes an absolute value, so whichever commits second silently discards
    /// the other's tokens. That path bills the user for less than they spent
    /// and, worse, is the input to the spend caps.
    ///
    /// This folds the addition into the UPDATE itself, so the database
    /// serializes it and no read is involved. Prefer it for anything additive;
    /// the absolute setters remain for compaction, which deliberately RESETS
    /// the counters rather than adding to them.
    pub fn accumulate_tokens(mut self, total: i32, input: i32, output: i32) -> Self {
        self.accumulated_deltas = Some((total, input, output));
        self
    }

    pub fn schedule_id(mut self, schedule_id: Option<String>) -> Self {
        self.schedule_id = Some(schedule_id);
        self
    }

    pub fn recipe(mut self, recipe: Option<Recipe>) -> Self {
        self.recipe = Some(recipe);
        self
    }

    pub fn user_recipe_values(
        mut self,
        user_recipe_values: Option<HashMap<String, String>>,
    ) -> Self {
        self.user_recipe_values = Some(user_recipe_values);
        self
    }

    pub fn provider_name(mut self, provider_name: impl Into<String>) -> Self {
        self.provider_name = Some(Some(provider_name.into()));
        self
    }

    pub fn model_config(mut self, model_config: ModelConfig) -> Self {
        self.model_config = Some(Some(model_config));
        self
    }

    pub fn clear_model_config(mut self) -> Self {
        self.model_config = Some(None);
        self
    }

    pub fn goose_mode(mut self, mode: GooseMode) -> Self {
        self.goose_mode = Some(mode);
        self
    }

    pub fn thread_id(mut self, thread_id: Option<String>) -> Self {
        self.thread_id = Some(thread_id);
        self
    }
}

// Test seam, deliberate: the real egress-audit and cost-ledger sinks reach
// storage through `SessionManager::instance()`, so a test that wants to prove
// "this code path wrote zero rows" needs those sinks to land in ITS isolated
// database, not the process-global one. It is a task-local rather than a static
// override because cargo runs tests in parallel threads inside one binary — a
// process-wide switch would leak between concurrently running tests and turn
// every such assertion order-dependent. Task-scoped, restores on unwind, and
// absent from the shipped binary.
#[cfg(test)]
tokio::task_local! {
    pub(crate) static TEST_SESSION_MANAGER: Arc<SessionManager>;
}

pub struct SessionManager {
    storage: Arc<SessionStorage>,
}

impl SessionManager {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            storage: Arc::new(SessionStorage::new(data_dir)),
        }
    }

    pub fn instance() -> Self {
        #[cfg(test)]
        if let Ok(storage) = TEST_SESSION_MANAGER.try_with(|manager| Arc::clone(&manager.storage)) {
            return Self { storage };
        }
        Self {
            storage: Arc::clone(&SESSION_STORAGE),
        }
    }

    pub fn storage(&self) -> &Arc<SessionStorage> {
        &self.storage
    }

    /// Get a clone of the DB pool for sharing with other modules (e.g., TaskLogger).
    pub async fn pool_clone(&self) -> Result<Pool<Sqlite>> {
        self.storage.pool_clone().await
    }

    /// Persist one complete coding-harness projection. The run id is the
    /// idempotency key, so retries and daemon reconnects cannot duplicate
    /// history. The in-memory terminal-supervision registry remains the live
    /// TTL projection; this store is its restart/terminal-history backing.
    pub async fn upsert_harness_run_snapshot(
        &self,
        snapshot: &HarnessRunSnapshot,
    ) -> Result<HarnessRunSnapshot> {
        self.storage.upsert_harness_run_snapshot(snapshot).await
    }

    /// Read persisted harness snapshots newest-first. `terminal_only` keeps
    /// the history surface from accidentally mixing stale active rows into a
    /// live projection; callers may request both for restart hydration.
    pub async fn list_harness_run_snapshots(
        &self,
        terminal_only: bool,
        limit: i64,
    ) -> Result<Vec<HarnessRunSnapshot>> {
        self.storage
            .list_harness_run_snapshots(terminal_only, limit)
            .await
    }

    /// Recompute the versioned budget projection from the canonical Spectral
    /// session/ledger/reservation sources. This is a read seam only: it never
    /// stores a copied spend snapshot or creates a second budget boundary.
    pub async fn budget_projection(
        &self,
        root_session_id: &str,
        config: crate::cost_router::budget::BudgetConfig,
    ) -> Result<crate::session::BudgetProjection> {
        crate::session::budget_projection::BudgetProjection::query(self, root_session_id, config)
            .await
            .map_err(|error| anyhow::anyhow!(error))
    }

    pub async fn create_session(
        &self,
        working_dir: PathBuf,
        name: String,
        session_type: SessionType,
        goose_mode: GooseMode,
    ) -> Result<Session> {
        self.create_session_with_parent(None, working_dir, name, session_type, goose_mode)
            .await
    }

    /// Create a session, optionally recording the parent that spawned it.
    /// Existing call sites keep using [`Self::create_session`]; fan-out /
    /// SubAgent spawn sites pass `Some(parent_id)` so cost ledger rows and
    /// [`Self::cost_by_parent_session`] can roll delegated spend up.
    pub async fn create_session_with_parent(
        &self,
        parent_session_id: Option<&str>,
        working_dir: PathBuf,
        name: String,
        session_type: SessionType,
        goose_mode: GooseMode,
    ) -> Result<Session> {
        self.storage
            .create_session(
                parent_session_id,
                working_dir,
                name,
                session_type,
                goose_mode,
            )
            .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn grant_reservation_after_gate_approval_with_identity(
        &self,
        invocation_id: &str,
        session_id: &str,
        task_id: &str,
        amount_usd: f64,
        lease_until: &str,
        identity: &ProviderReservationIdentity,
    ) -> Result<CostReservationOutcome> {
        self.storage
            .grant_reservation_after_gate_approval_with_identity(
                invocation_id,
                session_id,
                task_id,
                amount_usd,
                lease_until,
                identity,
            )
            .await
    }

    pub async fn get_session(&self, id: &str, include_messages: bool) -> Result<Session> {
        self.storage.get_session(id, include_messages).await
    }

    /// Start a new user task in a session. The value is persisted alongside
    /// other session extension state, so internal Agent-loop continuations do
    /// not accidentally create a new budget boundary.
    pub async fn begin_budget_task(&self, id: &str) -> Result<String> {
        self.begin_budget_task_inner(id, None).await
    }

    /// Capture user-task origin in the same write as its new budget ID. Oversized
    /// input still starts ordinary chat normally; planners can refuse the marker
    /// explicitly without storing/truncating a misleading partial contract.
    pub async fn begin_budget_task_with_contract(
        &self,
        id: &str,
        message: &Message,
    ) -> Result<String> {
        self.begin_budget_task_inner(id, Some(message)).await
    }

    async fn begin_budget_task_inner(&self, id: &str, message: Option<&Message>) -> Result<String> {
        let session = self.get_session(id, false).await?;
        let task_id = uuid::Uuid::now_v7().to_string();
        let mut extension_data = session.extension_data;
        extension_data.set_extension_state(
            BUDGET_TASK_EXTENSION_NAME,
            BUDGET_TASK_EXTENSION_VERSION,
            serde_json::Value::String(task_id.clone()),
        );
        let contract = message.map(|message| {
            let text = message.as_concat_text();
            if text.len() > MAX_BUDGET_TASK_CONTRACT_BYTES {
                serde_json::json!({"task_id": task_id, "source_message_id": message.id, "oversized_bytes": text.len()})
            } else {
                serde_json::json!({"task_id": task_id, "source_message_id": message.id, "text": text})
            }
        }).unwrap_or(serde_json::Value::Null);
        extension_data.set_extension_state(BUDGET_TASK_CONTRACT_EXTENSION_NAME, "v1", contract);
        self.update(id)
            .extension_data(extension_data)
            .apply()
            .await?;
        Ok(task_id)
    }

    /// X-verify-1: append a reviewer-mutation receipt to `id`'s extension
    /// data and persist it. Read-modify-write, same shape as
    /// [`begin_budget_task`](Self::begin_budget_task) — every occurrence is
    /// kept (append, not replace), so a session with several delegate rounds
    /// retains a full audit trail rather than only the most recent mutation.
    pub async fn record_reviewer_mutation(
        &self,
        id: &str,
        paths: Vec<String>,
        subagent_session_id: Option<String>,
    ) -> Result<()> {
        let session = self.get_session(id, false).await?;
        let mut extension_data = session.extension_data;
        let mut receipts = reviewer_mutation_receipts(&extension_data);
        receipts.push(ReviewerMutationReceipt {
            paths,
            at: Utc::now().to_rfc3339(),
            subagent_session_id,
        });
        extension_data.set_extension_state(
            REVIEWER_MUTATION_EXTENSION_NAME,
            REVIEWER_MUTATION_EXTENSION_VERSION,
            serde_json::to_value(&receipts)?,
        );
        self.update(id)
            .extension_data(extension_data)
            .apply()
            .await?;
        Ok(())
    }

    pub fn update(&self, id: &str) -> SessionUpdateBuilder<'_> {
        SessionUpdateBuilder::new(self, id.to_string())
    }

    async fn apply_update_inner(&self, builder: SessionUpdateBuilder<'_>) -> Result<()> {
        self.storage.apply_update(builder).await
    }

    /// Append one per-call cost row and advance the session's O(1) cost rollup
    /// (`cost_usd`, `accumulated_cost_usd`, cache accumulators) in a single
    /// transaction. See [`CostLedgerRow`].
    pub async fn append_cost_ledger(&self, row: &CostLedgerRow) -> Result<()> {
        self.storage.append_cost_ledger(row).await
    }

    /// Attach a turn's wall-clock timing breakdown to its already-settled
    /// `cost_ledger` row (lane X-lat, coding-harness excellence iteration).
    ///
    /// Deliberately a follow-up UPDATE rather than a field on
    /// [`CostLedgerRow`] carried through the original insert: the ledger row
    /// is written mid-turn (fail-closed, before more turn work runs, so a
    /// durable-accounting failure aborts the turn rather than losing spend),
    /// but two of the timing components — the after-turn hook chain and the
    /// tool-pair summarization await — only finish LATER in the same turn.
    /// This call is best-effort observability, not money accounting: a
    /// failure here must never fail the turn, so callers should log and
    /// continue rather than propagate. A `call_id` with no matching row
    /// (a non-turn cost_ledger writer, or a row that failed to insert) is a
    /// harmless no-op — zero rows affected, no error.
    pub async fn update_cost_ledger_timing(
        &self,
        call_id: &str,
        timing: &crate::session::turn_timing::TurnTiming,
    ) -> Result<()> {
        let timing_json = serde_json::to_string(timing)?;
        self.storage
            .update_cost_ledger_timing(call_id, &timing_json)
            .await
    }

    /// Session-level per-turn timing rollup (lane X-lat). Reads every
    /// non-null `cost_ledger.timing_json` for the session, in chronological
    /// (`ts`) order, and folds them with
    /// [`crate::session::turn_timing::compute_rollup`]. Rows with no timing
    /// recorded — pre-instrumentation history, or one of the handful of
    /// non-turn `cost_ledger` writers (recovery compaction, tool-pair
    /// summarization, council debate, orchestrator fan-out) — are skipped
    /// rather than failing the rollup. This is what backs the headless CLI's
    /// final `harness_overhead ...` line (`format_harness_overhead_line`).
    pub async fn session_timing_rollup(
        &self,
        session_id: &str,
    ) -> Result<crate::session::turn_timing::SessionTimingRollup> {
        let raw = self
            .storage
            .cost_ledger_timing_json_for_session(session_id)
            .await?;
        let records: Vec<crate::session::turn_timing::TurnTiming> = raw
            .into_iter()
            .flatten()
            .filter_map(|json| serde_json::from_str(&json).ok())
            .collect();
        Ok(crate::session::turn_timing::compute_rollup(&records))
    }

    /// Append one provider invocation and atomically update both token and
    /// money rollups. A repeated `call_id` is a successful no-op.
    // Every argument is a distinct piece of settlement evidence; collapsing them
    // into a struct would only move the same fields behind one more name.
    #[allow(clippy::too_many_arguments)]
    pub async fn append_usage_and_rollup(
        &self,
        row: &CostLedgerRow,
        schedule_id: Option<String>,
        current_total: Option<i32>,
        current_input: Option<i32>,
        current_output: Option<i32>,
        delta_total: i32,
        delta_input: i32,
        delta_output: i32,
    ) -> Result<bool> {
        self.storage
            .append_usage_and_rollup(
                row,
                schedule_id,
                current_total,
                current_input,
                current_output,
                delta_total,
                delta_input,
                delta_output,
            )
            .await
    }

    /// Atomically reserve a bounded amount for a paid provider invocation.
    /// Settled spend is read from `cost_ledger`; pending and unknown leases are
    /// added before comparing either budget scope.
    pub async fn reserve_provider_invocation(
        &self,
        invocation_id: &str,
        session_id: &str,
        task_id: Option<&str>,
        amount_usd: f64,
        lease_until: &str,
        config: &crate::cost_router::budget::BudgetConfig,
    ) -> Result<CostReservationOutcome> {
        self.storage
            .reserve_provider_invocation_inner(
                invocation_id,
                session_id,
                task_id,
                amount_usd,
                lease_until,
                config,
                ReservationRequest::Initial(None),
                None,
            )
            .await
    }

    /// Check the frozen task/session exposure before a decision that is not
    /// itself taking a monetary hold (for example a reviewer under LocalFree
    /// or subscription billing). This uses the same ceiling calculation as a
    /// paid reservation with a zero increment, and never creates/releases a
    /// hold or mutates the budget task.
    pub(crate) async fn check_provider_task_exposure(
        &self,
        session_id: &str,
        task_id: &str,
        config: &crate::cost_router::budget::BudgetConfig,
    ) -> Result<Option<CostReservationOutcome>> {
        if session_id.trim().is_empty() || task_id.trim().is_empty() {
            return Ok(Some(CostReservationOutcome::Unknown {
                reason: "task exposure requires exact session and task identities".to_string(),
            }));
        }
        let Some((task, session)) = reservation_ceilings(config) else {
            return Ok(Some(CostReservationOutcome::Unknown {
                reason: "budget ceiling is unavailable or invalid".to_string(),
            }));
        };
        let pool = self.pool_clone().await?;
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
        let extension_data: Option<String> =
            sqlx::query_scalar("SELECT extension_data FROM sessions WHERE id = ?")
                .bind(session_id)
                .fetch_optional(&mut *tx)
                .await?
                .flatten();
        let durable_task_matches = extension_data
            .as_deref()
            .and_then(|json| serde_json::from_str::<ExtensionData>(json).ok())
            .and_then(|data| budget_task_id(&data))
            .as_deref()
            == Some(task_id);
        if !durable_task_matches {
            tx.commit().await?;
            return Ok(Some(CostReservationOutcome::Unknown {
                reason: "task exposure identity is not bound to this session".to_string(),
            }));
        }
        let outcome =
            reservation_ceiling_verdict(&mut tx, task_id, session_id, 0.0, task, session).await?;
        tx.commit().await?;
        Ok(outcome)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn reserve_provider_invocation_with_identity(
        &self,
        invocation_id: &str,
        session_id: &str,
        task_id: Option<&str>,
        amount_usd: f64,
        lease_until: &str,
        config: &crate::cost_router::budget::BudgetConfig,
        identity: &ProviderReservationIdentity,
    ) -> Result<CostReservationOutcome> {
        self.storage
            .reserve_provider_invocation_inner(
                invocation_id,
                session_id,
                task_id,
                amount_usd,
                lease_until,
                config,
                ReservationRequest::Initial(None),
                Some(identity),
            )
            .await
    }

    /// Reserve a planner-produced bound and retain immutable extension evidence.
    pub async fn reserve_bounded_provider_invocation(
        &self,
        invocation_id: &str,
        session_id: &str,
        task_id: Option<&str>,
        bound: crate::cost_router::reservation::ReservationBound,
        lease_until: &str,
        config: &crate::cost_router::budget::BudgetConfig,
    ) -> Result<CostReservationOutcome> {
        anyhow::ensure!(bound.max_physical_attempts > 0, "invalid reserved attempts");
        self.storage
            .reserve_provider_invocation_inner(
                invocation_id,
                session_id,
                task_id,
                bound.amount_usd,
                lease_until,
                config,
                ReservationRequest::Initial(Some(bound.max_physical_attempts)),
                None,
            )
            .await
    }

    /// Reserve an accounted invocation with its exact native attempt and
    /// actual effective provider/model route. Existing rows must have the
    /// same identity; metadata never gets rewritten on replay or extension.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn reserve_bounded_provider_invocation_with_identity(
        &self,
        invocation_id: &str,
        session_id: &str,
        task_id: Option<&str>,
        bound: crate::cost_router::reservation::ReservationBound,
        lease_until: &str,
        config: &crate::cost_router::budget::BudgetConfig,
        identity: &ProviderReservationIdentity,
    ) -> Result<CostReservationOutcome> {
        anyhow::ensure!(bound.max_physical_attempts > 0, "invalid reserved attempts");
        self.storage
            .reserve_provider_invocation_inner(
                invocation_id,
                session_id,
                task_id,
                bound.amount_usd,
                lease_until,
                config,
                ReservationRequest::Initial(Some(bound.max_physical_attempts)),
                Some(identity),
            )
            .await
    }

    /// Extend only a live hold with durable bound/config evidence. Gate grants
    /// and legacy scalar holds lack that evidence and fail closed. No gate is
    /// auto-approved here, and settlement still releases the entire same row.
    pub async fn extend_reservation(
        &self,
        invocation_id: &str,
        additional_attempts: u32,
    ) -> Result<ExtendOutcome> {
        anyhow::ensure!(
            additional_attempts > 0,
            "extension attempts must be positive"
        );
        let pool = self.storage.pool().await?;
        let row: Option<(String, Option<String>, String, Option<String>)> = sqlx::query_as(
            "SELECT session_id, task_id, lease_until, extension_json
             FROM cost_reservations WHERE invocation_id = ?",
        )
        .bind(invocation_id)
        .fetch_optional(pool)
        .await?;
        let Some((session_id, task_id, lease, Some(json))) = row else {
            return Ok(CostReservationOutcome::Unknown {
                reason: "reservation has no durable extension evidence".to_string(),
            });
        };
        let evidence: ReservationExtension = serde_json::from_str(&json)?;
        self.storage
            .reserve_provider_invocation_inner(
                invocation_id,
                &session_id,
                task_id.as_deref(),
                evidence.per_attempt_usd,
                &lease,
                &evidence.config,
                ReservationRequest::Extend(additional_attempts),
                None,
            )
            .await
    }

    /// Directly grant a reservation for an invocation whose gate a headless
    /// policy has just auto-answered, bypassing `reserve_provider_invocation`'s
    /// gate/hard threshold math entirely.
    ///
    /// E1 (harness loop, Round E, from Lane F9 forensics 2026-09-10):
    /// `reserve_provider_invocation`'s `NeedsGate` check is pure arithmetic
    /// over `cost_ledger`/`cost_reservations` totals — answering the
    /// DECISION changes nothing about those totals, so a plain retry of the
    /// same call re-trips the identical gate immediately. This lets the
    /// EXACT invocation that was just authorized skip straight to a
    /// reservation instead of re-asking the gate for a request that already
    /// got a yes. Idempotent: an existing reservation for `invocation_id` is
    /// left untouched and its outcome reported back unchanged (mirrors
    /// `reserve_provider_invocation`'s own `AlreadyReserved`/`AlreadySettled`
    /// handling), never double-inserted.
    ///
    /// F3 (harness loop, Round F, from Lane G9's HIGH finding): `provider`
    /// and `model` name the exact call this grant authorizes and are stored
    /// on the row so a later reuse attempt (`find_pending_gate_grant`/
    /// `consume_granted_reservation`) can require an exact match before
    /// treating it as reusable.
    #[allow(clippy::too_many_arguments)]
    pub async fn grant_reservation_after_gate_approval(
        &self,
        invocation_id: &str,
        session_id: &str,
        task_id: &str,
        amount_usd: f64,
        lease_until: &str,
        provider: &str,
        model: &str,
    ) -> Result<CostReservationOutcome> {
        self.storage
            .grant_reservation_after_gate_approval(
                invocation_id,
                session_id,
                task_id,
                amount_usd,
                lease_until,
                provider,
                model,
                None,
            )
            .await
    }

    /// Default consume window for [`SessionManager::consume_granted_reservation`]
    /// — F1b's "unconsumed past this TTL, it expires" boundary. Not read
    /// internally (the TTL is an explicit parameter so tests, and any future
    /// live caller, can pin an exact value); this is the named default such
    /// a caller should pass absent a specific reason to choose otherwise.
    pub const GRANT_CONSUME_TTL_SECS_DEFAULT: i64 = 120;

    /// Consume a reservation minted by
    /// [`SessionManager::grant_reservation_after_gate_approval`], on behalf
    /// of a retry from the SAME `session_id`.
    ///
    /// F1b (harness loop, Lane F1 2026-09-10, from F10/G8 forensics): E1's
    /// grant is keyed to the ORIGINAL failed call's `invocation_id`, but
    /// EVERY provider call mints a fresh `invocation_id`
    /// (`Uuid::new_v4()` in `agents::agent`'s turn loop) — nothing today
    /// automatically reuses a grant, which is exactly why F10 traced the
    /// retried call re-hitting the same gate a second time (fx03/fx04's
    /// near-total M10 stalls). This is the primitive a caller that DOES know
    /// which grant to consume (the original invocation_id, e.g. carried on
    /// the session's own extension data) calls instead of re-running
    /// `reserve_provider_invocation`'s gate math from scratch. Wiring a real
    /// caller into the live turn loop is out of this lane's file boundary
    /// (`agents::agent`/`reply_parts.rs` — E1's own territory this round);
    /// this closes the primitive and its contract, tested directly.
    ///
    /// Returns `Ok(Some(outcome))` when a still-valid grant for this
    /// `invocation_id` belongs to `session_id` — `Granted`/`AlreadyReserved`/
    /// `AlreadySettled` per its current row state, exactly as a fresh
    /// reservation lookup would report. Returns `Ok(None)` — "nothing to
    /// consume, raise a fresh gate decision" — when no grant exists for this
    /// invocation_id, it was granted to a DIFFERENT session (never
    /// consumable across sessions — a grant only authorizes the call it was
    /// minted for), or it has aged past `consume_ttl_secs` since it was
    /// granted (its `created_at`, which `grant_reservation_after_gate_
    /// approval` never rewrites); an expired-but-still-`pending` row is
    /// released in the same call so it stops counting toward budget for a
    /// retry that never came, rather than holding it open until its full
    /// (2-hour) `lease_until`.
    ///
    /// F3 (harness loop, Round F, from Lane G9's HIGH and MEDIUM findings):
    /// `provider`/`model` must exactly match the row's own stored identity
    /// (`Ok(None)` on any mismatch, same as a wrong-session or expired
    /// grant — never touched, never reported) — a grant only authorizes the
    /// call it was minted for, not whatever provider/model the CURRENT call
    /// happens to be using (H1). A grant already consumed once
    /// (`reuse_count > 0`) is likewise refused a second time (M1): it
    /// authorizes exactly one retry of the specific call that got gated,
    /// not every later call in the session while the row is still
    /// `pending`.
    ///
    /// F3b (from Lane G10's MEDIUM finding): the TTL-staleness check runs
    /// BEFORE the `reuse_count` refusal, so an already-consumed grant that
    /// is also past `consume_ttl_secs` is released (and reported as
    /// `Ok(None)`) rather than refused-but-left-`pending` — a consumed
    /// grant keeps the same fast 120s self-release as an unconsumed one,
    /// instead of waiting on the ~8h lease-expiry sweep while it counts
    /// toward held budget.
    #[allow(clippy::too_many_arguments)]
    pub async fn consume_granted_reservation(
        &self,
        invocation_id: &str,
        session_id: &str,
        provider: &str,
        model: &str,
        consume_ttl_secs: i64,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<CostReservationOutcome>> {
        self.storage
            .consume_granted_reservation(
                invocation_id,
                session_id,
                provider,
                model,
                consume_ttl_secs,
                now,
            )
            .await
    }

    /// The `invocation_id` of this session's most recent still-`pending`
    /// reservation minted by [`SessionManager::grant_reservation_after_gate_approval`]
    /// (`granted_via_gate = 1`) FOR THIS EXACT `provider`/`model`, if any.
    ///
    /// I1 (harness loop, 2026-09-10, wiring F1b's grant-consumption
    /// primitive into the live turn loop, from F10's forensics): a caller
    /// about to mint a fresh provider-call `invocation_id` looks this up
    /// FIRST — an ordinary in-flight reservation (from
    /// `reserve_provider_invocation`'s own budget check) is never returned
    /// here, only one this exact session's gate was already headlessly
    /// auto-answered for. Deliberately does not itself validate/consume the
    /// grant (no TTL check, no state mutation) — pair with
    /// [`SessionManager::consume_granted_reservation`] before trusting it.
    ///
    /// F3 (harness loop, Round F, from Lane G9's HIGH finding): `provider`/
    /// `model` are now a hard SQL filter, not merely a caller convention —
    /// a grant minted for a different provider/model is invisible to this
    /// lookup, so a provider switch between the gated call and the retry
    /// mints a fresh invocation_id and re-asks the gate instead of silently
    /// reusing the OLD call's dollar bound.
    pub async fn find_pending_gate_grant(
        &self,
        session_id: &str,
        provider: &str,
        model: &str,
    ) -> Result<Option<String>> {
        self.storage
            .find_pending_gate_grant(session_id, provider, model)
            .await
    }

    /// K1b: admit a frozen Council plan as ONE atomic batch of ordinary
    /// `pending` reservations plus a durable manifest. See
    /// [`super::reservation_batch`] for the invariants; the ceiling arithmetic
    /// is this file's `reservation_ceiling_verdict`, applied to the SUM of
    /// the paid slots.
    pub async fn reserve_invocation_batch(
        &self,
        request: &super::reservation_batch::BatchAdmissionRequest,
        config: &crate::cost_router::budget::BudgetConfig,
    ) -> std::result::Result<
        super::reservation_batch::BatchAdmissionOutcome,
        super::reservation_batch::BatchAdmissionError,
    > {
        let pool = self.storage.pool().await?;
        super::reservation_batch::reserve_invocation_batch(pool, request, config).await
    }

    /// K1b: claim one admitted slot for `child_session_id`, transferring the
    /// slot's EXISTING reservation to it. Never creates a reservation.
    pub async fn claim_batch_invocation(
        &self,
        batch_id: &str,
        slot_id: &str,
        child_session_id: &str,
    ) -> std::result::Result<
        super::reservation_batch::BatchClaimOutcome,
        super::reservation_batch::BatchAdmissionError,
    > {
        let pool = self.storage.pool().await?;
        super::reservation_batch::claim_batch_invocation(pool, batch_id, slot_id, child_session_id)
            .await
    }

    /// K1b: idempotent close. Releases only never-claimed slots; claimed slots
    /// stay funded under U1's rules.
    pub async fn close_reservation_batch(
        &self,
        batch_id: &str,
    ) -> std::result::Result<
        super::reservation_batch::BatchCloseSummary,
        super::reservation_batch::BatchAdmissionError,
    > {
        let pool = self.storage.pool().await?;
        super::reservation_batch::close_reservation_batch(pool, batch_id).await
    }

    /// K1b: stop new claims and release unclaimed slots; never refunds a claim.
    pub async fn expire_reservation_batch(
        &self,
        batch_id: &str,
    ) -> std::result::Result<
        super::reservation_batch::BatchCloseSummary,
        super::reservation_batch::BatchAdmissionError,
    > {
        let pool = self.storage.pool().await?;
        super::reservation_batch::expire_reservation_batch(pool, batch_id).await
    }

    /// K1b: expire every open batch whose claim deadline has passed. Also runs
    /// once when the store opens and inside every batch admission.
    pub async fn reconcile_expired_batches(
        &self,
    ) -> std::result::Result<
        super::reservation_batch::BatchExpirySweep,
        super::reservation_batch::BatchAdmissionError,
    > {
        let pool = self.storage.pool().await?;
        super::reservation_batch::reconcile_expired_batches(pool).await
    }

    /// Settle a reservation and append its provider usage in one transaction.
    /// Duplicate invocation/call IDs are successful no-ops.
    // Every argument is a distinct piece of settlement evidence; collapsing them
    // into a struct would only move the same fields behind one more name.
    #[allow(clippy::too_many_arguments)]
    pub async fn settle_provider_invocation(
        &self,
        reservation_id: &str,
        row: &CostLedgerRow,
        schedule_id: Option<String>,
        current_total: Option<i32>,
        current_input: Option<i32>,
        current_output: Option<i32>,
        delta_total: i32,
        delta_input: i32,
        delta_output: i32,
    ) -> Result<bool> {
        self.storage
            .settle_provider_invocation(
                reservation_id,
                row,
                schedule_id,
                current_total,
                current_input,
                current_output,
                delta_total,
                delta_input,
                delta_output,
            )
            .await
    }

    /// Release a reservation after a provider invocation failed before it
    /// produced billable usage. Releasing an already released/settled/unknown
    /// lease is an idempotent no-op.
    pub async fn release_provider_invocation(&self, reservation_id: &str) -> Result<bool> {
        self.storage
            .release_provider_invocation(reservation_id)
            .await
    }

    /// Preserve a paid hold when dispatch may have reached the provider but no
    /// authoritative usage arrived. Unknown holds remain budget-consuming until
    /// settlement records durable usage.
    pub async fn mark_provider_invocation_unknown(&self, reservation_id: &str) -> Result<bool> {
        self.storage
            .mark_provider_invocation_unknown(reservation_id)
            .await
    }

    pub(crate) async fn mark_provider_invocation_dispatch_possible(
        &self,
        reservation_id: &str,
    ) -> Result<bool> {
        self.storage
            .mark_provider_invocation_dispatch_possible(reservation_id)
            .await
    }

    /// Roll a parent's own spend together with every direct child's spend.
    /// `own` is this session's ledger total; `children_total` sums every session
    /// whose `parent_session_id` is `parent_id`; `per_child` lists each child.
    pub async fn cost_by_parent_session(&self, parent_id: &str) -> Result<ParentSessionCost> {
        self.storage.cost_by_parent_session(parent_id).await
    }

    /// Everything spent since `since` (an RFC3339 UTC instant), USD.
    ///
    /// Summed from `cost_ledger` rather than from the `sessions` rollups: the
    /// rollups are per-session lifetime totals, so adding them up would charge
    /// today for a session that started last week. `ts` is written as
    /// `Utc::now().to_rfc3339()`, whose fixed-width date prefix makes
    /// lexicographic order chronological order — the comparison the
    /// `idx_cost_ledger_ts` index serves.
    pub async fn spend_since(&self, since: &str) -> Result<f64> {
        self.storage.spend_since(since).await
    }

    /// The most recent ledger row's provider, model, and whether its cost was
    /// estimated rather than priced.
    ///
    /// The meter names what is spending the money. `sessions` records the
    /// session's configured model, which is not necessarily the one that served
    /// the last turn — a routing decision or a fallback can differ from it, and
    /// a meter that names the wrong model is worse than one that names none.
    ///
    /// `is_estimated` travels with them because it changes what the number
    /// MEANS. A model with no published rate is billed at `worst_case_pricing`
    /// — deliberately the most expensive rate in the registry, so the spend cap
    /// fires early rather than late — and showing that as a plain dollar figure
    /// presents a safety margin as a fact. Today `zai/glm-5.3`, the coding
    /// harness's own model, has no row in the canonical table or in
    /// `published_prices`, so this is the common case on the very surface that
    /// reported the bug.
    pub async fn last_call_facts(&self, session_id: &str) -> Result<Option<LastCall>> {
        self.storage.last_call_facts(session_id).await
    }

    pub async fn add_message(&self, id: &str, message: &Message) -> Result<()> {
        self.storage.add_message(id, message).await
    }

    pub async fn replace_conversation(&self, id: &str, conversation: &Conversation) -> Result<()> {
        self.storage.replace_conversation(id, conversation).await
    }

    pub async fn list_sessions(&self) -> Result<Vec<Session>> {
        self.storage.list_sessions().await
    }

    pub async fn list_sessions_by_types(&self, types: &[SessionType]) -> Result<Vec<Session>> {
        self.storage.list_sessions_by_types(Some(types)).await
    }

    pub async fn list_all_sessions(&self) -> Result<Vec<Session>> {
        self.storage.list_sessions_by_types(None).await
    }

    /// Sessions belonging to one schedule, newest first, capped at `limit`.
    /// Filtered and limited in SQL — see the storage impl for why this exists.
    pub async fn list_sessions_by_schedule_id(
        &self,
        schedule_id: &str,
        limit: usize,
    ) -> Result<Vec<Session>> {
        self.storage
            .list_sessions_by_schedule_id(schedule_id, limit)
            .await
    }

    /// Recent sessions for every schedule that has one, in ONE query — see
    /// [`ScheduleSessionSummary`] and `SessionStorage::list_recent_sessions_by_schedule`.
    pub async fn list_recent_sessions_by_schedule(
        &self,
        limit_per_schedule: usize,
    ) -> Result<Vec<ScheduleSessionSummary>> {
        self.storage
            .list_recent_sessions_by_schedule(limit_per_schedule)
            .await
    }

    /// Lean session list (User + Scheduled) for LIST views — see
    /// [`SessionSummary`]. Use this for the HTTP `/api/sessions` list path;
    /// the heavy [`Session`] fields are served only on single-session GET.
    pub async fn list_session_summaries(&self) -> Result<Vec<SessionSummary>> {
        self.list_session_summaries_of_types(&Self::DEFAULT_LIST_TYPES)
            .await
    }

    /// The session types the list views show when a caller asks for nothing
    /// else: the ones a person started or scheduled.
    pub const DEFAULT_LIST_TYPES: [SessionType; 2] = [SessionType::User, SessionType::Scheduled];

    /// [`Self::list_session_summaries`] for an explicit set of types. History's
    /// "Agent runs" filter asks for `SubAgent` here (goal and harness workers),
    /// which the default list deliberately leaves out. An empty slice lists
    /// nothing rather than everything.
    pub async fn list_session_summaries_of_types(
        &self,
        types: &[SessionType],
    ) -> Result<Vec<SessionSummary>> {
        self.storage.list_session_summaries(Some(types)).await
    }

    pub async fn delete_session(&self, id: &str) -> Result<()> {
        self.storage.delete_session(id).await
    }

    pub async fn get_insights(&self) -> Result<SessionInsights> {
        self.storage
            .get_insights(&[SessionType::User, SessionType::Scheduled])
            .await
    }

    pub async fn export_session(&self, id: &str) -> Result<String> {
        self.storage.export_session(id).await
    }

    pub async fn import_session(
        &self,
        json: &str,
        session_type_override: Option<SessionType>,
    ) -> Result<Session> {
        self.storage
            .import_session(self, json, session_type_override)
            .await
    }

    pub async fn copy_session(&self, session_id: &str, new_name: String) -> Result<Session> {
        self.storage.copy_session(self, session_id, new_name).await
    }

    pub async fn truncate_conversation(&self, session_id: &str, timestamp: i64) -> Result<()> {
        self.storage
            .truncate_conversation(session_id, timestamp)
            .await
    }

    pub async fn maybe_update_name(&self, id: &str, provider: Arc<dyn Provider>) -> Result<()> {
        let session = self.get_session(id, true).await?;

        if session.user_set_name {
            return Ok(());
        }

        let conversation = session
            .conversation
            .ok_or_else(|| anyhow::anyhow!("No messages found"))?;

        let user_message_count = conversation
            .messages()
            .iter()
            .filter(|m| matches!(m.role, Role::User))
            .count();

        if user_message_count <= MSG_COUNT_FOR_SESSION_NAME_GENERATION {
            // A recipe whose title names the job (e.g. "Nightly Health
            // Review") titles its sessions; the generic coding-harness title
            // does not, so those sessions are named from their prompt.
            let name = match crate::providers::base::recipe_session_title(session.recipe.as_ref()) {
                Some(title) => title,
                None => provider.generate_session_name(id, &conversation).await?,
            };
            self.update(id)
                .system_generated_name(name.clone())
                .apply()
                .await?;

            // Also update the thread name so ACP clients see it via session/list.
            if let Some(ref thread_id) = session.thread_id {
                let thread_mgr = super::thread_manager::ThreadManager::new(self.storage.clone());
                let thread = thread_mgr.get_thread(thread_id).await?;
                if !thread.user_set_name {
                    thread_mgr
                        .update_thread(thread_id, Some(name), Some(false), None)
                        .await?;
                }
            }
            Ok(())
        } else {
            Ok(())
        }
    }

    pub async fn search_chat_history(
        &self,
        query: &str,
        limit: Option<usize>,
        after_date: Option<chrono::DateTime<chrono::Utc>>,
        before_date: Option<chrono::DateTime<chrono::Utc>>,
        exclude_session_id: Option<String>,
        session_types: Vec<SessionType>,
    ) -> Result<crate::session::chat_history_search::ChatRecallResults> {
        self.storage
            .search_chat_history(
                query,
                limit,
                after_date,
                before_date,
                exclude_session_id,
                session_types,
            )
            .await
    }

    pub async fn update_message_metadata<F>(id: &str, message_id: &str, f: F) -> Result<()>
    where
        F: FnOnce(
            crate::conversation::message::MessageMetadata,
        ) -> crate::conversation::message::MessageMetadata,
    {
        Self::instance()
            .storage
            .update_message_metadata(id, message_id, f)
            .await
    }
}

pub struct SessionStorage {
    pool: Pool<Sqlite>,
    initialized: tokio::sync::OnceCell<()>,
    db_path: PathBuf,
}

pub(crate) fn role_to_string(role: &Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
    }
}

/// Store the coalesced form so later turns do not re-discover the same
/// split-delta text parts. Load already coalesces (Kimi UI-crash repair);
/// persisting un-coalesced JSON is why MOIM's issue list grew every turn
/// of a live session even though reload looked fine.
fn persistable_content_json(message: &Message) -> Result<String> {
    let coalesced = message.clone().coalesce_adjacent_text_and_thinking();
    Ok(serde_json::to_string(&coalesced.content)?)
}

impl Default for Session {
    fn default() -> Self {
        Self {
            id: String::new(),
            working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            name: String::new(),
            user_set_name: false,
            session_type: SessionType::default(),
            created_at: Default::default(),
            updated_at: Default::default(),
            extension_data: ExtensionData::default(),
            total_tokens: None,
            input_tokens: None,
            output_tokens: None,
            accumulated_total_tokens: None,
            accumulated_input_tokens: None,
            accumulated_output_tokens: None,
            cost_usd: None,
            accumulated_cost_usd: None,
            accumulated_cache_read_tokens: None,
            accumulated_cache_write_tokens: None,
            accumulated_cache_savings_usd: None,
            schedule_id: None,
            recipe: None,
            user_recipe_values: None,
            conversation: None,
            message_count: 0,
            provider_name: None,
            model_config: None,
            goose_mode: GooseMode::default(),
            thread_id: None,
            parent_session_id: None,
        }
    }
}

impl Session {
    pub fn without_messages(mut self) -> Self {
        self.conversation = None;
        self
    }
}

impl sqlx::FromRow<'_, sqlx::sqlite::SqliteRow> for Session {
    fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;

        let recipe_json: Option<String> = row.try_get("recipe_json")?;
        let recipe = recipe_json.and_then(|json| serde_json::from_str(&json).ok());

        let user_recipe_values_json: Option<String> = row.try_get("user_recipe_values_json")?;
        let user_recipe_values =
            user_recipe_values_json.and_then(|json| serde_json::from_str(&json).ok());

        let model_config_json: Option<String> = row.try_get("model_config_json").ok().flatten();
        let model_config = model_config_json.and_then(|json| serde_json::from_str(&json).ok());

        let name: String = {
            let name_val: String = row.try_get("name").unwrap_or_default();
            if !name_val.is_empty() {
                name_val
            } else {
                row.try_get("description").unwrap_or_default()
            }
        };

        let user_set_name = row.try_get("user_set_name").unwrap_or(false);

        let session_type_str: String = row
            .try_get("session_type")
            .unwrap_or_else(|_| "user".to_string());
        let session_type = session_type_str.parse().unwrap_or_default();

        Ok(Session {
            id: row.try_get("id")?,
            working_dir: PathBuf::from(row.try_get::<String, _>("working_dir")?),
            name,
            user_set_name,
            session_type,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
            extension_data: serde_json::from_str(&row.try_get::<String, _>("extension_data")?)
                .unwrap_or_default(),
            total_tokens: row.try_get("total_tokens")?,
            input_tokens: row.try_get("input_tokens")?,
            output_tokens: row.try_get("output_tokens")?,
            accumulated_total_tokens: row.try_get("accumulated_total_tokens")?,
            accumulated_input_tokens: row.try_get("accumulated_input_tokens")?,
            accumulated_output_tokens: row.try_get("accumulated_output_tokens")?,
            // Robust to absence (older rows / lean projections): default to None
            // rather than erroring, mirroring provider_name / thread_id below.
            cost_usd: row.try_get("cost_usd").ok().flatten(),
            accumulated_cost_usd: row.try_get("accumulated_cost_usd").ok().flatten(),
            accumulated_cache_read_tokens: row
                .try_get("accumulated_cache_read_tokens")
                .ok()
                .flatten(),
            accumulated_cache_write_tokens: row
                .try_get("accumulated_cache_write_tokens")
                .ok()
                .flatten(),
            accumulated_cache_savings_usd: row
                .try_get("accumulated_cache_savings_usd")
                .ok()
                .flatten(),
            schedule_id: row.try_get("schedule_id")?,
            recipe,
            user_recipe_values,
            conversation: None,
            message_count: row.try_get("message_count").unwrap_or(0) as usize,
            provider_name: row.try_get("provider_name").ok().flatten(),
            model_config,
            goose_mode: row
                .try_get::<String, _>("goose_mode")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or_default(),
            thread_id: row.try_get("thread_id").ok().flatten(),
            parent_session_id: row.try_get("parent_session_id").ok().flatten(),
        })
    }
}

/// One schedule's run, as the Automate tab's run-history views render it.
/// Selects only the columns `SessionDisplayInfo`
/// (crates/goose-server/src/routes/schedule.rs) serializes — no
/// extension_data / recipe_json / user_recipe_values_json / model_config_json
/// blobs. See [`SessionStorage::list_recent_sessions_by_schedule`], added for
/// the 2026-08-25 "schedule polling storm" health-review fix: the Automate
/// tab used to fetch this same shape with one `list_sessions_by_schedule_id`
/// SQL call per job, every poll tick (N queries). This is the batched,
/// one-query replacement.
#[derive(Debug, Clone)]
pub struct ScheduleSessionSummary {
    pub id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub working_dir: PathBuf,
    pub schedule_id: Option<String>,
    pub message_count: usize,
    pub total_tokens: Option<i32>,
    pub input_tokens: Option<i32>,
    pub output_tokens: Option<i32>,
    pub accumulated_total_tokens: Option<i32>,
    pub accumulated_input_tokens: Option<i32>,
    pub accumulated_output_tokens: Option<i32>,
}

impl sqlx::FromRow<'_, sqlx::sqlite::SqliteRow> for ScheduleSessionSummary {
    fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;

        let name: String = {
            let name_val: String = row.try_get("name").unwrap_or_default();
            if !name_val.is_empty() {
                name_val
            } else {
                row.try_get("description").unwrap_or_default()
            }
        };

        Ok(ScheduleSessionSummary {
            id: row.try_get("id")?,
            name,
            created_at: row.try_get("created_at")?,
            working_dir: PathBuf::from(row.try_get::<String, _>("working_dir")?),
            schedule_id: row.try_get("schedule_id")?,
            message_count: row.try_get::<i64, _>("message_count").unwrap_or(0) as usize,
            total_tokens: row.try_get("total_tokens")?,
            input_tokens: row.try_get("input_tokens")?,
            output_tokens: row.try_get("output_tokens")?,
            accumulated_total_tokens: row.try_get("accumulated_total_tokens")?,
            accumulated_input_tokens: row.try_get("accumulated_input_tokens")?,
            accumulated_output_tokens: row.try_get("accumulated_output_tokens")?,
        })
    }
}

/// Lean projection of a session for LIST views. Excludes the heavy JSON blobs
/// (extension_data, recipe_json, user_recipe_values_json, model_config_json) and
/// the conversation, which the session-list UI discards anyway. The full
/// [`Session`] is served only on single-session GET. See #341/#371.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SessionSummary {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub user_set_name: bool,
    #[serde(default)]
    pub session_type: SessionType,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub message_count: usize,
    // What the list needs to NAME and CLASSIFY a session without the heavy
    // blobs: the UI's one label function (ui/command-center/src/lib/
    // sessionLabel.ts) re-titles sessions the old namer mis-named from the
    // first user message, names a scheduled run after its job, and splits
    // History into conversations / agent runs / system. All optional so an
    // older client (or a serialized summary from before this) still reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule_id: Option<String>,
    /// `recipe_json.title`, extracted in SQL — the recipe blob itself stays out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe_title: Option<String>,
    /// The first user-visible user message's first text part, cut to 500 chars.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_user_text: Option<String>,
    /// The goal card a goal worker ran for (`extension_data["goal_worker.v1"]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_project_id: Option<String>,
    /// The session's project: its creation-time hint, else its goal's project,
    /// else the project whose root is the session's working directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_name: Option<String>,
}

impl sqlx::FromRow<'_, sqlx::sqlite::SqliteRow> for SessionSummary {
    fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;

        let name: String = {
            let name_val: String = row.try_get("name").unwrap_or_default();
            if !name_val.is_empty() {
                name_val
            } else {
                row.try_get("description").unwrap_or_default()
            }
        };

        let session_type_str: String = row
            .try_get("session_type")
            .unwrap_or_else(|_| "user".to_string());
        let session_type = session_type_str.parse().unwrap_or_default();

        Ok(SessionSummary {
            id: row.try_get("id")?,
            name,
            user_set_name: row.try_get("user_set_name").unwrap_or(false),
            session_type,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
            message_count: row.try_get("message_count").unwrap_or(0) as usize,
            working_dir: opt_text(row, "working_dir"),
            schedule_id: opt_text(row, "schedule_id"),
            recipe_title: opt_text(row, "recipe_title"),
            first_user_text: opt_text(row, "first_user_text"),
            goal_id: opt_text(row, "goal_id"),
            goal_title: opt_text(row, "goal_title"),
            goal_project_id: opt_text(row, "goal_project_id"),
            project_id: opt_text(row, "project_id"),
            project_name: opt_text(row, "project_name"),
        })
    }
}

/// A nullable TEXT column as `Some(non-blank)`, else `None` — a missing column
/// (older query), NULL and "" all read as absent.
fn opt_text(row: &sqlx::sqlite::SqliteRow, column: &str) -> Option<String> {
    use sqlx::Row;
    row.try_get::<Option<String>, _>(column)
        .ok()
        .flatten()
        .filter(|v| !v.trim().is_empty())
}

/// SQL for [`SessionStorage::list_sessions_by_schedule_id`] — the Automate
/// tab's single-schedule run-history lookup. Pulled out to a const so the
/// idempotent-index and query-plan tests below exercise the EXACT string
/// that runs in production, not a copy that can drift. `s.schedule_id = ?`
/// is the predicate `idx_sessions_schedule_id` (migrate_v48_to_v49) targets —
/// before that index existed this was an unindexed scan of the whole
/// `sessions` table on every poll (the 2026-08-25 "schedule polling storm"
/// health review).
const LIST_SESSIONS_BY_SCHEDULE_ID_SQL: &str = r#"
    SELECT s.id, s.working_dir, s.name, s.description, s.user_set_name, s.session_type, s.created_at, s.updated_at, s.extension_data,
           s.total_tokens, s.input_tokens, s.output_tokens,
           s.accumulated_total_tokens, s.accumulated_input_tokens, s.accumulated_output_tokens,
           s.schedule_id, s.recipe_json, s.user_recipe_values_json,
           s.provider_name, s.model_config_json, s.goose_mode, s.thread_id,
           (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id) AS message_count
    FROM sessions s
    WHERE s.schedule_id = ?
    ORDER BY s.created_at DESC
    LIMIT ?
"#;

impl SessionStorage {
    fn create_pool(path: &Path) -> Pool<Sqlite> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("Failed to create Spectral database directory");
        }

        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true)
            .busy_timeout(std::time::Duration::from_secs(30))
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);

        // WAL mode allows unlimited concurrent readers; only writes serialise.
        // 20 connections keep reads from being starved if write transactions
        // (e.g. replace_conversation) momentarily hold several connections.
        // Defensive hardening only: a contention repro (#225) showed reads stay
        // <200ms even at 64 concurrent heavy writers, so this is not on the
        // critical path for any observed slowness — it just lowers the tail if
        // write load ever spikes. Overhead is ~1 fd per connection. The 5s
        // acquire timeout still bounds queueing if all connections are busy.
        SqlitePoolOptions::new()
            .max_connections(20)
            .acquire_timeout(std::time::Duration::from_secs(5))
            .connect_lazy_with(options)
    }

    /// Create a SessionStorage pointing at the Spectral database.
    pub fn new_spectral() -> Self {
        let db_path = Paths::spectral_db();
        Self {
            pool: Self::create_pool(&db_path),
            initialized: tokio::sync::OnceCell::new(),
            db_path,
        }
    }

    /// Create a SessionStorage at a custom path (used by tests).
    pub fn new(data_dir: PathBuf) -> Self {
        let db_path = data_dir.join("permagent.db");
        Self {
            pool: Self::create_pool(&db_path),
            initialized: tokio::sync::OnceCell::new(),
            db_path,
        }
    }

    /// Get a clone of the underlying pool (for sharing with TaskLogger etc.)
    pub async fn pool_clone(&self) -> Result<Pool<Sqlite>> {
        self.pool().await?;
        Ok(self.pool.clone())
    }

    pub(crate) async fn pool(&self) -> Result<&Pool<Sqlite>> {
        self.initialized
            .get_or_try_init(|| async {
                use super::spectral_schema;

                if spectral_schema::is_schema_initialized(&self.pool).await? {
                    let version = spectral_schema::verify_schema_version(&self.pool).await?;
                    // Run incremental migrations
                    if version < 3 {
                        spectral_schema::migrate_v2_to_v3(&self.pool).await?;
                    }
                    if version < 4 {
                        spectral_schema::migrate_v3_to_v4(&self.pool).await?;
                    }
                    if version < 5 {
                        spectral_schema::migrate_v4_to_v5(&self.pool).await?;
                    }
                    if version < 6 {
                        spectral_schema::migrate_v5_to_v6(&self.pool).await?;
                    }
                    if version < 7 {
                        spectral_schema::migrate_v6_to_v7(&self.pool).await?;
                    }
                    if version < 8 {
                        spectral_schema::migrate_v7_to_v8(&self.pool).await?;
                    }
                    // Decision inbox (schema v10). No v8->v9 step on this branch:
                    // v9 is reserved by session-list-perf (committed, unmerged), so
                    // the chain steps straight from v8 to v10 here. migrate_v9_to_v10
                    // is base-independent (idempotent additive DDL), so it is correct
                    // over a v8 or v9 base alike.
                    if version < 10 {
                        spectral_schema::migrate_v9_to_v10(&self.pool).await?;
                    }
                    // Recognition instrumentation (schema v11). Additive
                    // new-tables-only; base-independent.
                    if version < 11 {
                        spectral_schema::migrate_v10_to_v11(&self.pool).await?;
                    }
                    // CRM people (schema v12). Additive new-tables-only;
                    // base-independent. Runs sequentially after v11: a v10 DB runs
                    // v11 then v12, a v11 DB runs only v12.
                    if version < 12 {
                        spectral_schema::migrate_v11_to_v12(&self.pool).await?;
                    }
                    // File-intake inbox (schema v13). Additive new-tables-only;
                    // base-independent. Runs sequentially after v12.
                    if version < 13 {
                        spectral_schema::migrate_v12_to_v13(&self.pool).await?;
                    }
                    // Duplicate-column cleanup (schema v14, #453). Data fixup,
                    // base-independent and idempotent; deletes only empty manual
                    // columns in projects that also have lifecycle columns.
                    if version < 14 {
                        spectral_schema::migrate_v13_to_v14(&self.pool).await?;
                    }
                    // Consolidate legacy Doing/Done columns into the goal
                    // lifecycle (schema v15, #453): move Doing→In Progress,
                    // Done→Complete, delete the emptied columns. Backlog kept.
                    // Base-independent + idempotent; card-data-safe (moves before
                    // delete).
                    if version < 15 {
                        spectral_schema::migrate_v14_to_v15(&self.pool).await?;
                    }
                    // Backfill the Cancelled goal-lifecycle column (schema v16,
                    // #490) so cancellation has a target column on pre-existing
                    // boards. Base-independent + idempotent (insert-where-absent).
                    if version < 16 {
                        spectral_schema::migrate_v15_to_v16(&self.pool).await?;
                    }
                    // Apply the canonical goal lifecycle to ALL boards (schema
                    // v17, #502): prior fixups only reached boards that already
                    // had lifecycle columns (seeded on first goal card), so
                    // never-goal'd boards still showed legacy Backlog/Doing/Done.
                    // Seeds the lifecycle columns everywhere then consolidates
                    // Doing→In Progress / Done→Complete. Base-independent +
                    // idempotent; card-data-safe.
                    if version < 17 {
                        spectral_schema::migrate_v16_to_v17(&self.pool).await?;
                    }
                    // Reconcile the risk_policy seed (schema v18, #514): the
                    // goal_cancel row was added to the INSERT-OR-IGNORE seed AFTER
                    // the v10 table creation, so pre-#500 DBs never got it — an
                    // unknown action_class fails closed to Tier 2 and Cancel always
                    // 409s. Force-sets goal_cancel=0 + restores any absent seed row.
                    // Base-independent + idempotent; same gap class as #502/#507.
                    if version < 18 {
                        spectral_schema::migrate_v17_to_v18(&self.pool).await?;
                    }
                    // Drop the dead `memories` + `knowledge_graph` tables (schema
                    // v19): a dormant copy of the Spectral Phase-1 schema that the
                    // live Brain (separate brain/memory.db) never read or wrote.
                    // Idempotent (DROP ... IF EXISTS) and base-independent; fresh
                    // installs never create them, so this only affects existing DBs.
                    if version < 19 {
                        spectral_schema::migrate_v18_to_v19(&self.pool).await?;
                    }
                    // v20: project association join tables (project_people +
                    // project_memories). Purely additive, base-independent.
                    if version < 20 {
                        spectral_schema::migrate_v19_to_v20(&self.pool).await?;
                    }
                    // v21: people↔graph bridge column (#255/B). Adds the immutable
                    // graph_entity_id to `people`. Idempotent (PRAGMA-guarded ADD
                    // COLUMN) + base-independent; fresh installs get the column from
                    // apply_people_schema, so this runs harmlessly over both.
                    if version < 21 {
                        spectral_schema::migrate_v20_to_v21(&self.pool).await?;
                    }
                    // v22: recognition_verdict + familiarity columns (PRAGMA-
                    // guarded ADDs) + recognition_tool_events feed table
                    // (spectral-recognition prep). Cfg-gated so a feature-off
                    // build leaves the DB untouched at v21 — no behavior change
                    // when the flag is off. Idempotent + base-independent, so
                    // mixed feature-on/off build orders are all safe.
                    #[cfg(feature = "spectral-recognition")]
                    if version < 22 {
                        spectral_schema::migrate_v21_to_v22(&self.pool).await?;
                    }
                    // v23: entity_provenance side table (people-in-graph v1 #583).
                    // Purely additive, base-independent, always-on (not feature-
                    // gated). Makes runtime person-creation durable — the daemon
                    // reconciler prunes only ontology-sourced entities.
                    if version < 23 {
                        spectral_schema::migrate_v22_to_v23(&self.pool).await?;
                    }
                    // v24: project_documents hub table (#471 Layer 2). Purely
                    // additive, base-independent, always-on. Per-project file
                    // attachments backing the in-app document viewer.
                    if version < 24 {
                        spectral_schema::migrate_v23_to_v24(&self.pool).await?;
                    }
                    // v25: project_notes table. Purely additive, base-independent,
                    // always-on. Per-project freeform notes indexed into the Brain.
                    if version < 25 {
                        spectral_schema::migrate_v24_to_v25(&self.pool).await?;
                    }
                    // v26: projects.metadata_json general metadata bag (#456,
                    // GOAL_COMPLETION_AND_VERIFICATION.md ruling 3). Guarded
                    // ADD COLUMN, base-independent, always-on. First tenant:
                    // build_command — the project-level default build check the
                    // orchestrator seeds onto code-flavored goals.
                    if version < 26 {
                        spectral_schema::migrate_v25_to_v26(&self.pool).await?;
                    }
                    // v27: activity_journal table (#619). Purely additive,
                    // base-independent, always-on. Append-only journal of
                    // selected event-bus kinds behind the Home timeline.
                    if version < 27 {
                        spectral_schema::migrate_v26_to_v27(&self.pool).await?;
                    }
                    // v28: per-call cost_ledger table + O(1) cost-rollup columns
                    // on sessions (cost-transparency workstream). Purely additive
                    // (CREATE TABLE IF NOT EXISTS + PRAGMA-guarded ADD COLUMN),
                    // base-independent, always-on.
                    if version < 28 {
                        spectral_schema::migrate_v27_to_v28(&self.pool).await?;
                    }
                    // v29: egress_audit table (sovereignty). Always-on,
                    // append-only record of every cloud inference call the
                    // sovereignty guard sees. Purely additive
                    // (CREATE TABLE IF NOT EXISTS), base-independent.
                    if version < 29 {
                        spectral_schema::migrate_v28_to_v29(&self.pool).await?;
                    }
                    // v30: backfill the Failed goal-lifecycle column (#250) so
                    // parking has a target column on pre-existing boards —
                    // exhausted goals now land in a visible Failed column
                    // instead of re-pooling into Triage. Base-independent +
                    // idempotent (insert-where-absent).
                    if version < 30 {
                        spectral_schema::migrate_v29_to_v30(&self.pool).await?;
                    }
                    // v31: project_stack_entries table (#512, stack organizer).
                    // Per-project services + login-identity reference rows —
                    // reference-only, no secrets by design. Purely additive
                    // (CREATE ... IF NOT EXISTS), base-independent, always-on.
                    // (Reconciled onto v31: v30 was taken by the #250 backfill.)
                    if version < 31 {
                        spectral_schema::migrate_v30_to_v31(&self.pool).await?;
                    }
                    // v32: seed the supervised-CC-gate risk_policy classes
                    // (#430, S4 — cc_read_only/cc_workspace_edit/cc_shell). The
                    // S4 classifier maps a CC gate's tool to one of these so the
                    // gate → inbox decision is filed at the right tier (unknown
                    // tools fail closed to Tier 2). INSERT OR IGNORE, purely
                    // additive to a free-text-PK table, base-independent.
                    if version < 32 {
                        spectral_schema::migrate_v31_to_v32(&self.pool).await?;
                    }
                    // v33: per-user notification severity thresholds and the
                    // durable daily-digest queue (#66). Purely additive and
                    // idempotent; the router owns delivery, workflow emitters
                    // remain policy-free.
                    if version < 33 {
                        spectral_schema::migrate_v32_to_v33(&self.pool).await?;
                    }
                    // v34: track the last committed local digest date so a
                    // daemon restart after the configured hour catches up.
                    if version < 34 {
                        spectral_schema::migrate_v33_to_v34(&self.pool).await?;
                    }
                    // v35: cited project ecosystem and competitive intelligence
                    // (#889). New table + index, additive and idempotent.
                    if version < 35 {
                        spectral_schema::migrate_v34_to_v35(&self.pool).await?;
                    }
                    // v36: authenticated master/device principal attribution on
                    // Decision-Inbox answer audit rows. Additive and idempotent.
                    if version < 36 {
                        spectral_schema::migrate_v35_to_v36(&self.pool).await?;
                    }
                    // v37: durable outbox for Decision-Inbox effects. New table
                    // + index, additive and idempotent.
                    if version < 37 {
                        spectral_schema::migrate_v36_to_v37(&self.pool).await?;
                    }
                    // Durable chat-memory capture queue. This is intentionally
                    // applied independent of the numbered ladder so an existing
                    // database gets the queue on its first boot after upgrade.
                    spectral_schema::apply_chat_memory_outbox_schema(&self.pool).await?;
                    // Durable coding-harness snapshots. Version-independent so
                    // databases already past the numbered ladder receive the
                    // restart/terminal-history store on their first boot.
                    spectral_schema::apply_harness_run_snapshots_schema(&self.pool).await?;
                    // Harness process leases: version-independent for the same
                    // reason, so an upgraded database gets the table on its
                    // first boot.
                    spectral_schema::apply_harness_process_leases_schema(&self.pool).await?;
                    crate::goal_attempts::apply_schema(&self.pool).await?;
                    // v38: first-party analytics events (#23). New table +
                    // index, additive and idempotent.
                    if version < 38 {
                        spectral_schema::migrate_v37_to_v38(&self.pool).await?;
                    }
                    // v39: drain-ingest idempotency key on analytics_events.
                    // Additive + base-independent.
                    if version < 39 {
                        spectral_schema::migrate_v38_to_v39(&self.pool).await?;
                    }
                    // v40: analytics dimensions (properties, is_bot, session_id,
                    // utm_*, country). Additive + base-independent.
                    if version < 40 {
                        spectral_schema::migrate_v39_to_v40(&self.pool).await?;
                    }
                    // v41: seed the Steward git-health risk_policy classes
                    // (repo_worktree_reap / repo_branch_delete — Tier 2,
                    // user-only, so henry-policy can never auto-approve a
                    // deletion). INSERT OR IGNORE, purely additive to a
                    // free-text-PK table, base-independent.
                    if version < 41 {
                        spectral_schema::migrate_v40_to_v41(&self.pool).await?;
                    }
                    // v42: durable growth actions + pre-registered outcomes
                    // (docs/proposals/grow-action-outcome-loop.md). New tables
                    // + index only, additive and base-independent. Fresh
                    // installs get the same tables from init_spectral_db,
                    // which never reaches this ladder.
                    if version < 42 {
                        spectral_schema::migrate_v41_to_v42(&self.pool).await?;
                    }
                    // v43: the daemon control-plane auth audit — one row per
                    // admitted consequential request and per refused request,
                    // so same-user misuse of the daemon token is at least
                    // detectable after the fact. New table + indexes +
                    // append-only triggers, additive and base-independent.
                    if version < 43 {
                        spectral_schema::migrate_v42_to_v43(&self.pool).await?;
                    }
                    // v44: person-keyed meetings (People profile + Home calendar).
                    // New table + indexes only, additive and base-independent.
                    if version < 44 {
                        spectral_schema::migrate_v43_to_v44(&self.pool).await?;
                    }
                    // v45: follow-up date, optional project, Calendar.app uid
                    // on person_meetings. ALTER + indexes, additive.
                    if version < 45 {
                        spectral_schema::migrate_v44_to_v45(&self.pool).await?;
                    }
                    // v46: Financier ledger (watchlist / notes / positions).
                    // New tables + indexes, additive and base-independent.
                    if version < 46 {
                        spectral_schema::migrate_v45_to_v46(&self.pool).await?;
                    }
                    // v47: household spend ledger + RSI-alert dedup.
                    if version < 47 {
                        spectral_schema::migrate_v46_to_v47(&self.pool).await?;
                    }
                    // v46 tables were stamped at 11:55 today then vanished
                    // (version 46/47 present, finance_watchlist gone). The
                    // Finance tab 500'd "Unknown error" because the version
                    // gate never re-ran. Same every-boot pattern as briefings.
                    spectral_schema::apply_finance_ledger_schema(&self.pool).await?;
                    spectral_schema::apply_finance_spend_schema(&self.pool).await?;
                    // v48: The Forecaster's market-series registry, points,
                    // forecasts and briefs. New tables + indexes, additive and
                    // base-independent.
                    if version < 48 {
                        spectral_schema::migrate_v47_to_v48(&self.pool).await?;
                    }
                    // v49: sessions.schedule_id index — fixes the Automate
                    // tab's schedule polling storm (an unindexed
                    // `WHERE schedule_id = ?` full table scan). Index-only,
                    // additive and idempotent.
                    if version < 49 {
                        spectral_schema::migrate_v48_to_v49(&self.pool).await?;
                    }
                    // v50: person merge/delete bookkeeping — `person_aliases`
                    // (identifiers a survivor absorbed) and `person_merge_log`
                    // (the undo snapshot). New tables + indexes, additive and
                    // base-independent.
                    if version < 50 {
                        spectral_schema::migrate_v49_to_v50(&self.pool).await?;
                    }
                    // v51: sessions.parent_session_id — durable link from a
                    // SubAgent / fan-out child back to the spawning session so
                    // cost-ledger rows and the parent rollup can attribute
                    // delegated spend. PRAGMA-guarded ADD COLUMN + index.
                    if version < 51 {
                        spectral_schema::migrate_v50_to_v51(&self.pool).await?;
                    }
                    // v52: the RLM control-plane context store — durable,
                    // versioned evaluation context that outlives an LLM turn
                    // and a daemon restart. New table + index, additive and
                    // base-independent.
                    if version < 52 {
                        spectral_schema::migrate_v51_to_v52(&self.pool).await?;
                    }
                    // v53: the Financier's exit-notice risk_policy classes.
                    // Without them an advisory sell notice resolves fail-closed
                    // to Tier 2 and is indistinguishable from an urgent one.
                    // INSERT OR IGNORE, additive and base-independent.
                    if version < 53 {
                        spectral_schema::migrate_v52_to_v53(&self.pool).await?;
                    }
                    // v54: cost_ledger.timing_json (coding-harness excellence
                    // iteration, lane X-lat) — per-turn wall-clock breakdown
                    // (model/tool/after-turn/context-mgmt) so the harness-
                    // overhead gate can be measured instead of asserted.
                    // PRAGMA-guarded ADD COLUMN, additive and base-independent.
                    if version < 54 {
                        spectral_schema::migrate_v53_to_v54(&self.pool).await?;
                    }
                    // v55: cost_reservations.provider/.model/.reuse_count/
                    // .reused_by_invocation_id (harness loop, Round F, Lane
                    // F3, from Lane G9's HIGH/MEDIUM gate-grant-reuse
                    // findings). PRAGMA-guarded ADD COLUMNs, additive and
                    // base-independent.
                    if version < 55 {
                        spectral_schema::migrate_v54_to_v55(&self.pool).await?;
                    }
                    // v56: cross-process arbitration of a named scheduled fire.
                    if version < 56 {
                        spectral_schema::migrate_v55_to_v56(&self.pool).await?;
                    }
                    // v57: durable decision journal rows keyed by call-site.
                    if version < 57 {
                        spectral_schema::migrate_v56_to_v57(&self.pool).await?;
                    }
                    // v58: durable scheduler pause delivery and answer intents.
                    if version < 58 {
                        spectral_schema::migrate_v57_to_v58(&self.pool).await?;
                    }
                    // v59: model_review_reports and model_review_findings.
                    if version < 59 {
                        spectral_schema::migrate_v58_to_v59(&self.pool).await?;
                    }
                    // v60: cost_ledger call-purpose fields (node A4) —
                    // call_purpose/trigger_ref/retry_of_call_id/
                    // pre_call_evidence/tool_call_count/completion_kind.
                    // PRAGMA-guarded nullable ADD COLUMNs; historical rows
                    // stay NULL (= unknown). Additive and base-independent.
                    if version < 60 {
                        spectral_schema::migrate_v59_to_v60(&self.pool).await?;
                    }
                    // v61 (K1b): Council convene batch manifest. New tables
                    // only; paid slots are ordinary `cost_reservations` rows.
                    if version < 61 {
                        spectral_schema::migrate_v60_to_v61(&self.pool).await?;
                    }
                    // v62: model_review_finding_states (W1b review state).
                    if version < 62 {
                        spectral_schema::migrate_v61_to_v62(&self.pool).await?;
                    }
                    // Version-independent safety net for recognition columns.
                    // The always-on v23 above can stamp schema_version past the
                    // cfg-gated `version < 22` migration, leaving a feature-off DB
                    // permanently without the columns. This is the third instance
                    // of the cfg-gated-migration-skip hazard: schema repairs must
                    // run on every boot, regardless of the feature that writes them.
                    // Idempotent — a steady-state boot adds nothing.
                    spectral_schema::apply_recognition_v22_columns(&self.pool).await?;

                    // Version-independent: the RLM control-plane store. Applied
                    // on every boot for the same reason as the recognition
                    // columns above — a version gate is exactly how those went
                    // missing in production, and a missing rlm_context table
                    // silently costs every worker its recovered state.
                    spectral_schema::apply_rlm_context_schema(&self.pool).await?;

                    // Version-independent: ensure the skills.skill_path index
                    // column exists on any DB regardless of the recorded schema
                    // version. The on-disk SKILL.md source-of-truth migration
                    // (skills::reconcile_skills_to_disk) relies on it. Additive +
                    // idempotent, so SPECTRAL_SCHEMA_VERSION is not bumped.
                    // Version-independent: the agent-briefings table (worker
                    // agents reporting to Henry). Applied by table existence
                    // every boot for the same reason as the recognition
                    // columns above — a version gate is exactly how that one
                    // went missing in production.
                    spectral_schema::apply_briefings_schema(&self.pool).await?;

                    // Version-independent: Council tables. Applied by table
                    // existence every boot so a DB already past v50 still
                    // grows the tables without waiting on a stamp.
                    spectral_schema::apply_council_schema(&self.pool).await?;

                    spectral_schema::apply_skill_path_column(&self.pool).await?;

                    // Version-independent: ensure the projects.graph_entity_id
                    // bridge column exists (#595 — graph identity for
                    // non-ontology projects; mirrors people.graph_entity_id).
                    // Additive + idempotent, so SPECTRAL_SCHEMA_VERSION is not
                    // bumped.
                    spectral_schema::apply_project_graph_entity_column(&self.pool).await?;

                    // Version-independent: the decision-inbox schema carries the
                    // decisions kind/answer CHECK-widening rebuild (#579 —
                    // project_intel_proposal, tool_approval, session_gate, …),
                    // but it historically only ran on fresh init and the
                    // v9->v10 step. Any DB already past v10 when a widening
                    // shipped never received it, so newer decision kinds failed
                    // the old CHECK at insert (code 275) and the proposing tool
                    // errored. The function is fully idempotent (sentinel mode)
                    // and the rebuild is marker-gated to run at most once per
                    // widening, so run it on every boot.
                    spectral_schema::apply_decision_inbox_schema(&self.pool).await?;

                    // Version-independent Phase-1 failure incident capture.
                    // New-table-only and idempotent, so it runs on every boot
                    // without changing the pinned fresh-init base stamp.
                    spectral_schema::apply_incidents_schema(&self.pool).await?;

                    // The activity journal shipped at v27 without append-only guards,
                    // so DBs past v27 need them applied by table state. Steady-state
                    // boots add nothing because the function is idempotent.
                    spectral_schema::apply_activity_journal_schema(&self.pool).await?;

                    // Governed lesson pool (Phase 3). Same discipline: new tables
                    // only, idempotent, version-independent.
                    spectral_schema::apply_lessons_schema(&self.pool).await?;

                    // Session project hint + per-turn wing provenance. Version
                    // independent for the same reason as the recognition
                    // columns above: a missing column here fails every chat
                    // turn write, and a `version < N` gate is exactly how the
                    // last three schema repairs went missing in production.
                    spectral_schema::apply_session_project_hint_schema(&self.pool).await?;

                    // Parent-session link for subagent cost rollup. Same
                    // version-independent posture: a missing column here
                    // silently drops parent attribution on every child spawn.
                    spectral_schema::apply_session_parent_schema(&self.pool).await?;

                    // Durable provider-spend reservations are an additive
                    // authorization adjunct to the existing cost ledger. Run
                    // the idempotent table/index apply on every boot: databases
                    // already stamped at the current schema version would never
                    // enter a numbered migration and otherwise fail their first
                    // paid invocation with `no such table`.
                    spectral_schema::apply_cost_reservations_schema(&self.pool).await?;

                    // Version-independent: the Brain feed outbox + per-source
                    // status tables (crates/goose/src/brain_feed.rs). New
                    // tables only, idempotent, so a DB already past the
                    // current stamp still gets them without a numbered step.
                    crate::brain_feed::apply_schema(&self.pool).await?;

                    // Existing installations also need the durable meeting queue;
                    // init_spectral_db only runs for a brand-new database.
                    crate::meeting_triage::apply_schema(&self.pool).await?;

                    // Version-independent: people.archived_at (non-person
                    // bridge row archival). Additive + idempotent.
                    spectral_schema::apply_people_archived_column(&self.pool).await?;

                    // Version-independent: people.kind ('person'|'organisation').
                    // Additive + idempotent.
                    spectral_schema::apply_people_kind_column(&self.pool).await?;

                    // Boot pass over concluded `unknown` provider-spend holds
                    // (gated-turn-silence, E2E QA 2026-09-18). The session-tree
                    // reconcile in `reconcile_stale_reservations` only runs
                    // inside `reserve_provider_invocation`, i.e. on the NEXT
                    // paid turn in that chat. After the fix shipped, session
                    // 20260918_7's $1.56 hold
                    // (`invocation:e1d47602-...`, `unknown_marked_at` set, no
                    // ledger row) was still `unknown` — and so were the same
                    // kind of holds in 20260918_8/_9/_16 — because nobody had
                    // spoken in those chats since: the chat still showed the
                    // refusal, and the Budget view kept counting money nobody
                    // spent. `cost_router::budget::spawn_unknown_hold_sweeper`
                    // does this work but is not started by the daemon, so run
                    // its sweep once here, where every process that opens the
                    // store (daemon, CLI) passes before its first query. The
                    // rules are the admission path's own (expired pending
                    // turns unknown; ledger evidence settles; no evidence
                    // keeps holding), and every
                    // update is guarded by `state = 'unknown'`, so running it
                    // here as well as elsewhere is idempotent. A failure only
                    // delays settlement visibility until the next paid turn
                    // reconciles it, so it is logged and never fails opening
                    // the store.
                    match crate::cost_router::budget::reconcile_concluded_unknown_holds(
                        &self.pool,
                    )
                    .await
                    {
                        Ok(sweep)
                            if sweep.settled + sweep.released + sweep.expired_to_unknown
                                > 0 =>
                        {
                            info!(
                                target: "permagent::cost_router",
                                expired_to_unknown = sweep.expired_to_unknown,
                                settled = sweep.settled,
                                released = sweep.released,
                                still_held = sweep.still_held,
                                "boot sweep resolved concluded unknown provider-spend holds"
                            );
                        }
                        Ok(_) => {}
                        Err(e) => tracing::warn!(
                            target: "permagent::cost_router",
                            error = %e,
                            "boot sweep of unknown provider-spend holds failed; the next paid turn still reconciles them"
                        ),
                    }

                    // K1b: expire Council convene batches whose claim deadline
                    // passed while no process was running, releasing ONLY
                    // their never-claimed slots (claimed ones stay counted
                    // under U1). Admission runs the same pass, so correctness
                    // never depends on this timer; a failure is logged and
                    // never fails opening the store.
                    match super::reservation_batch::reconcile_expired_batches(&self.pool).await
                    {
                        Ok(sweep) if sweep.expired_batches > 0 => info!(
                            target: "permagent::cost_router",
                            expired_batches = sweep.expired_batches,
                            released_slots = sweep.released_slots,
                            released_usd = sweep.released_usd,
                            retained_slots = sweep.retained_slots,
                            "boot pass expired lapsed Council reservation batches"
                        ),
                        Ok(_) => {}
                        Err(e) => tracing::warn!(
                            target: "permagent::cost_router",
                            error = %e,
                            "boot pass over Council reservation batches failed; the next batch admission reconciles them"
                        ),
                    }

                    // Growth actions + analytics events. Both apply functions
                    // claimed to run on every boot; they only ran on fresh init
                    // and their version-gated migrate. A DB already past v42
                    // never got `verified_commit`, so Grow failed every read
                    // with `no column found for name: verified_commit`
                    // (health-watch 2026-08-27). Idempotent — a steady-state
                    // boot adds nothing.
                    spectral_schema::apply_growth_actions_schema(&self.pool).await?;
                    spectral_schema::apply_analytics_events_schema(&self.pool).await?;
                } else {
                    info!("Initializing Spectral schema at {:?}", self.db_path);
                    spectral_schema::init_spectral_db(&self.pool).await?;
                }
                Ok::<(), anyhow::Error>(())
            })
            .await?;
        Ok(&self.pool)
    }

    /// Create a fresh SessionStorage with Spectral schema (used by tests).
    pub async fn create(base_dir: &Path) -> Result<Self> {
        let storage = Self::new(base_dir.to_path_buf());
        super::spectral_schema::init_spectral_db(&storage.pool).await?;
        Ok(storage)
    }

    async fn create_session(
        &self,
        parent_session_id: Option<&str>,
        working_dir: PathBuf,
        name: String,
        session_type: SessionType,
        goose_mode: GooseMode,
    ) -> Result<Session> {
        let pool = self.pool().await?;
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;

        // Child workers belong to the same user task as their parent. Copy the
        // durable identity at session creation so the child can emit ledger rows
        // under the parent's task without inventing a second budget.
        let inherited_extension_data = if let Some(parent_id) = parent_session_id {
            let parent_json =
                sqlx::query_scalar::<_, String>("SELECT extension_data FROM sessions WHERE id = ?")
                    .bind(parent_id)
                    .fetch_optional(&mut *tx)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("parent session '{}' not found", parent_id))?;
            let parent_data: ExtensionData = serde_json::from_str(&parent_json)?;
            let mut child_data = ExtensionData::new();
            if let Some(task_id) = budget_task_id(&parent_data) {
                child_data.set_extension_state(
                    BUDGET_TASK_EXTENSION_NAME,
                    BUDGET_TASK_EXTENSION_VERSION,
                    serde_json::Value::String(task_id),
                );
            }
            serde_json::to_string(&child_data)?
        } else {
            "{}".to_string()
        };

        let today = chrono::Utc::now().format("%Y%m%d").to_string();
        let session = sqlx::query_as(
            r#"
                INSERT INTO sessions (id, user_id, name, user_set_name, session_type, working_dir, extension_data, goose_mode, parent_session_id)
                VALUES (
                    ? || '_' || CAST(COALESCE((
                        SELECT MAX(CAST(SUBSTR(id, 10) AS INTEGER))
                        FROM sessions
                        WHERE id LIKE ? || '_%'
                    ), 0) + 1 AS TEXT),
                    ?,
                    ?,
                    FALSE,
                    ?,
                    ?,
                    ?,
                    ?,
                    ?
                )
                RETURNING *
                "#,
        )
            .bind(&today)
            .bind(&today)
            .bind(DEFAULT_USER_ID)
            .bind(&name)
            .bind(session_type.to_string())
            .bind(&*working_dir.to_string_lossy())
            .bind(inherited_extension_data)
            .bind(goose_mode.to_string())
            .bind(parent_session_id)
            .fetch_one(&mut *tx)
            .await?;

        tx.commit().await?;
        #[cfg(feature = "telemetry")]
        crate::posthog::emit_session_started();
        Ok(session)
    }

    async fn get_session(&self, id: &str, include_messages: bool) -> Result<Session> {
        let pool = self.pool().await?;
        let mut session = sqlx::query_as::<_, Session>(
            r#"
        SELECT id, working_dir, name, description, user_set_name, session_type, created_at, updated_at, extension_data,
               total_tokens, input_tokens, output_tokens,
               accumulated_total_tokens, accumulated_input_tokens, accumulated_output_tokens,
               cost_usd, accumulated_cost_usd, accumulated_cache_read_tokens,
               accumulated_cache_write_tokens, accumulated_cache_savings_usd,
               schedule_id, recipe_json, user_recipe_values_json,
               provider_name, model_config_json, goose_mode, thread_id, parent_session_id
        FROM sessions
        WHERE id = ?
    "#,
        )
            .bind(id)
            .fetch_optional(pool)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Session not found"))?;

        if include_messages {
            let conv = self.get_conversation(&session.id).await?;
            session.message_count = conv.messages().len();
            session.conversation = Some(conv);
        } else {
            let count =
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages WHERE session_id = ?")
                    .bind(&session.id)
                    .fetch_one(pool)
                    .await? as usize;
            session.message_count = count;
        }

        Ok(session)
    }

    async fn upsert_harness_run_snapshot(
        &self,
        snapshot: &HarnessRunSnapshot,
    ) -> Result<HarnessRunSnapshot> {
        let pool = self.pool().await?;
        // Prompt context is useful to the live in-memory projection and the
        // Council preflight, but it is not durable run evidence. Keep the
        // durable row limited to the title/digest and operational/result
        // fields, so a DB inspection cannot recover the user's prompt body.
        let mut durable = snapshot.clone();
        durable.prompt_context = None;
        let snapshot_json = serde_json::to_string(&durable)?;
        let is_terminal =
            (!crate::agents::platform_extensions::terminal_supervision::HarnessRunStatus::is_active(
                durable.status,
            )) as i64;
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
        if let Some((bound_session, existing_terminal, existing_json)) =
            sqlx::query_as::<_, (String, i64, String)>(
                "SELECT session_id, is_terminal, snapshot_json
             FROM harness_run_snapshots WHERE run_id = ?",
            )
            .bind(&durable.run_id)
            .fetch_optional(&mut *tx)
            .await?
        {
            if bound_session != durable.session_id {
                return Err(anyhow::anyhow!(
                    "harness run id is already bound to another session"
                ));
            }
            // Match the process-local registry's terminal monotonicity even
            // when the daemon restarted and has not hydrated this run yet.
            if existing_terminal != 0 {
                let existing = serde_json::from_str(&existing_json).map_err(|error| {
                    anyhow::anyhow!("invalid persisted harness snapshot: {error}")
                })?;
                tx.commit().await?;
                return Ok(existing);
            }
        }
        sqlx::query(
            "INSERT INTO harness_run_snapshots
                (run_id, session_id, project, status, started_at, updated_at,
                 is_terminal, evidence, result, snapshot_json)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(run_id) DO UPDATE SET
                session_id = excluded.session_id,
                project = excluded.project,
                status = excluded.status,
                started_at = excluded.started_at,
                updated_at = excluded.updated_at,
                is_terminal = excluded.is_terminal,
                evidence = excluded.evidence,
                result = excluded.result,
                snapshot_json = excluded.snapshot_json",
        )
        .bind(&durable.run_id)
        .bind(&durable.session_id)
        .bind(&durable.project)
        .bind(serde_json::to_string(&durable.status)?.trim_matches('"'))
        .bind(durable.started_at.to_rfc3339())
        .bind(durable.updated_at.to_rfc3339())
        .bind(is_terminal)
        .bind(&durable.evidence)
        .bind(&durable.result)
        .bind(snapshot_json)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(durable)
    }

    async fn list_harness_run_snapshots(
        &self,
        terminal_only: bool,
        limit: i64,
    ) -> Result<Vec<HarnessRunSnapshot>> {
        let pool = self.pool().await?;
        let limit = limit.clamp(1, 128);
        let rows: Vec<String> = if terminal_only {
            sqlx::query_scalar(
                "SELECT snapshot_json FROM harness_run_snapshots
                 WHERE is_terminal = 1
                 ORDER BY updated_at DESC, run_id DESC LIMIT ?",
            )
            .bind(limit)
            .fetch_all(pool)
            .await?
        } else {
            sqlx::query_scalar(
                "SELECT snapshot_json FROM harness_run_snapshots
                 ORDER BY updated_at DESC, run_id DESC LIMIT ?",
            )
            .bind(limit)
            .fetch_all(pool)
            .await?
        };
        rows.into_iter()
            .map(|json| {
                serde_json::from_str(&json)
                    .map_err(|error| anyhow::anyhow!("invalid persisted harness snapshot: {error}"))
            })
            .collect()
    }

    #[allow(clippy::too_many_lines)]
    async fn apply_update(&self, builder: SessionUpdateBuilder<'_>) -> Result<()> {
        let mut updates = Vec::new();
        let mut query = String::from("UPDATE sessions SET ");

        macro_rules! add_update {
            ($field:expr, $name:expr) => {
                if $field.is_some() {
                    if !updates.is_empty() {
                        query.push_str(", ");
                    }
                    updates.push($name);
                    query.push_str($name);
                    query.push_str(" = ?");
                }
            };
        }

        add_update!(builder.name, "name");
        add_update!(builder.user_set_name, "user_set_name");
        add_update!(builder.session_type, "session_type");
        add_update!(builder.working_dir, "working_dir");
        add_update!(builder.extension_data, "extension_data");
        add_update!(builder.total_tokens, "total_tokens");
        add_update!(builder.input_tokens, "input_tokens");
        add_update!(builder.output_tokens, "output_tokens");
        // Delta accumulation takes precedence: setting both an absolute value
        // and a delta for the same column would emit it twice in one SET.
        if builder.accumulated_deltas.is_some() {
            for name in [
                "accumulated_total_tokens",
                "accumulated_input_tokens",
                "accumulated_output_tokens",
            ] {
                if !updates.is_empty() {
                    query.push_str(", ");
                }
                updates.push(name);
                query.push_str(name);
                query.push_str(" = COALESCE(");
                query.push_str(name);
                query.push_str(", 0) + ?");
            }
        } else {
            add_update!(builder.accumulated_total_tokens, "accumulated_total_tokens");
            add_update!(builder.accumulated_input_tokens, "accumulated_input_tokens");
            add_update!(
                builder.accumulated_output_tokens,
                "accumulated_output_tokens"
            );
        }
        add_update!(builder.schedule_id, "schedule_id");
        add_update!(builder.recipe, "recipe_json");
        add_update!(builder.user_recipe_values, "user_recipe_values_json");
        add_update!(builder.provider_name, "provider_name");
        add_update!(builder.model_config, "model_config_json");
        add_update!(builder.goose_mode, "goose_mode");
        add_update!(builder.thread_id, "thread_id");

        if updates.is_empty() {
            return Ok(());
        }

        query.push_str(", ");
        query.push_str("updated_at = datetime('now') WHERE id = ?");

        // `query` is assembled only from hardcoded column-name literals passed
        // to `add_update!` above and fixed SQL fragments — no external data
        // reaches the SQL text; every value is bound below.
        let mut q = sqlx::query(sqlx::AssertSqlSafe(query));

        if let Some(name) = builder.name {
            q = q.bind(name);
        }
        if let Some(user_set_name) = builder.user_set_name {
            q = q.bind(user_set_name);
        }
        if let Some(session_type) = builder.session_type {
            q = q.bind(session_type.to_string());
        }
        if let Some(wd) = builder.working_dir {
            q = q.bind(wd.to_string_lossy().to_string());
        }
        if let Some(ed) = builder.extension_data {
            q = q.bind(serde_json::to_string(&ed)?);
        }
        if let Some(tt) = builder.total_tokens {
            q = q.bind(tt);
        }
        if let Some(it) = builder.input_tokens {
            q = q.bind(it);
        }
        if let Some(ot) = builder.output_tokens {
            q = q.bind(ot);
        }
        if let Some((dt, di, dout)) = builder.accumulated_deltas {
            q = q.bind(dt).bind(di).bind(dout);
        } else {
            if let Some(att) = builder.accumulated_total_tokens {
                q = q.bind(att);
            }
            if let Some(ait) = builder.accumulated_input_tokens {
                q = q.bind(ait);
            }
            if let Some(aot) = builder.accumulated_output_tokens {
                q = q.bind(aot);
            }
        }
        if let Some(sid) = builder.schedule_id {
            q = q.bind(sid);
        }
        if let Some(recipe) = builder.recipe {
            let recipe_json = recipe.map(|r| serde_json::to_string(&r)).transpose()?;
            q = q.bind(recipe_json);
        }
        if let Some(user_recipe_values) = builder.user_recipe_values {
            let user_recipe_values_json = user_recipe_values
                .map(|urv| serde_json::to_string(&urv))
                .transpose()?;
            q = q.bind(user_recipe_values_json);
        }
        if let Some(provider_name) = builder.provider_name {
            q = q.bind(provider_name);
        }
        if let Some(model_config) = builder.model_config {
            let model_config_json = model_config
                .map(|mc| serde_json::to_string(&mc))
                .transpose()?;
            q = q.bind(model_config_json);
        }
        if let Some(goose_mode) = builder.goose_mode {
            q = q.bind(goose_mode.to_string());
        }
        if let Some(thread_id) = builder.thread_id {
            q = q.bind(thread_id);
        }

        let pool = self.pool().await?;
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
        q = q.bind(&builder.session_id);
        q.execute(&mut *tx).await?;

        tx.commit().await?;
        Ok(())
    }

    /// Insert one cost-ledger row and advance the session's O(1) cost rollup in a
    /// single `BEGIN IMMEDIATE` transaction. `turn_index` is computed atomically
    /// as the count of prior rows for the session (0-based).
    /// See [`SessionManager::spend_since`].
    async fn spend_since(&self, since: &str) -> Result<f64> {
        let pool = self.pool().await?;
        let total: Option<f64> =
            sqlx::query_scalar("SELECT SUM(cost_usd) FROM cost_ledger WHERE ts >= ?")
                .bind(since)
                .fetch_one(pool)
                .await?;
        // SUM over no rows is NULL, not 0 — a day with no calls has spent
        // nothing, which is a number, not an absence.
        Ok(total.unwrap_or(0.0))
    }

    /// See [`SessionManager::last_call_facts`].
    async fn last_call_facts(&self, session_id: &str) -> Result<Option<LastCall>> {
        let pool = self.pool().await?;
        // `provider` and `model` are NULLABLE columns (`CostLedgerRow` carries
        // them as `Option`), so they are decoded as options. Decoding straight
        // into `String` would make a single row with a null provider fail the
        // whole query — and this runs on the meter's per-turn path, so that
        // failure would present as the meter going quiet rather than as an
        // error anyone could see.
        Ok(sqlx::query_as::<_, (Option<String>, Option<String>, i64)>(
            "SELECT provider, model, is_estimated FROM cost_ledger
                  WHERE session_id = ? ORDER BY ts DESC LIMIT 1",
        )
        .bind(session_id)
        .fetch_optional(pool)
        .await?
        .map(|(provider, model, estimated)| LastCall {
            provider,
            model,
            estimated: estimated != 0,
        }))
    }

    async fn append_cost_ledger(&self, row: &CostLedgerRow) -> Result<()> {
        self.append_cost_ledger_with_usage(row, None, None)
            .await
            .map(|_| ())
    }

    /// Best-effort follow-up write: set `timing_json` on an existing
    /// `cost_ledger` row. Not transactional with anything else — this is
    /// diagnostic latency data, not money, and a missing row (a call_id the
    /// reply loop timed but that never settled a ledger row) is a silent
    /// no-op rather than an error.
    async fn update_cost_ledger_timing(&self, call_id: &str, timing_json: &str) -> Result<()> {
        let pool = self.pool().await?;
        sqlx::query("UPDATE cost_ledger SET timing_json = ? WHERE call_id = ?")
            .bind(timing_json)
            .bind(call_id)
            .execute(pool)
            .await?;
        Ok(())
    }

    /// Every `timing_json` value for a session's `cost_ledger` rows, in
    /// chronological (`ts`) order. `None` entries are rows with no timing
    /// recorded (pre-instrumentation, or a non-turn writer).
    async fn cost_ledger_timing_json_for_session(
        &self,
        session_id: &str,
    ) -> Result<Vec<Option<String>>> {
        let pool = self.pool().await?;
        let rows: Vec<(Option<String>,)> = sqlx::query_as(
            "SELECT timing_json FROM cost_ledger WHERE session_id = ? ORDER BY ts ASC",
        )
        .bind(session_id)
        .fetch_all(pool)
        .await?;
        Ok(rows.into_iter().map(|(json,)| json).collect())
    }

    // Every argument is a distinct piece of settlement evidence; collapsing them
    // into a struct would only move the same fields behind one more name.
    #[allow(clippy::too_many_arguments)]
    async fn append_usage_and_rollup(
        &self,
        row: &CostLedgerRow,
        schedule_id: Option<String>,
        current_total: Option<i32>,
        current_input: Option<i32>,
        current_output: Option<i32>,
        delta_total: i32,
        delta_input: i32,
        delta_output: i32,
    ) -> Result<bool> {
        self.append_cost_ledger_with_usage(
            row,
            Some((
                schedule_id,
                current_total,
                current_input,
                current_output,
                delta_total,
                delta_input,
                delta_output,
            )),
            None,
        )
        .await
    }

    // Every argument is a distinct piece of settlement evidence; collapsing them
    // into a struct would only move the same fields behind one more name.
    #[allow(clippy::too_many_arguments)]
    async fn settle_provider_invocation(
        &self,
        reservation_id: &str,
        row: &CostLedgerRow,
        schedule_id: Option<String>,
        current_total: Option<i32>,
        current_input: Option<i32>,
        current_output: Option<i32>,
        delta_total: i32,
        delta_input: i32,
        delta_output: i32,
    ) -> Result<bool> {
        self.append_cost_ledger_with_usage(
            row,
            Some((
                schedule_id,
                current_total,
                current_input,
                current_output,
                delta_total,
                delta_input,
                delta_output,
            )),
            Some(reservation_id),
        )
        .await
    }

    async fn release_provider_invocation(&self, reservation_id: &str) -> Result<bool> {
        let pool = self.pool().await?;
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
        let result = sqlx::query(
            "UPDATE cost_reservations
             SET state = 'released', dispatch_state = 'not_sent', updated_at = datetime('now')
             WHERE reservation_id = ? AND state = 'pending'",
        )
        .bind(reservation_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(result.rows_affected() == 1)
    }

    async fn mark_provider_invocation_dispatch_possible(
        &self,
        reservation_id: &str,
    ) -> Result<bool> {
        let pool = self.pool().await?;
        let result = sqlx::query(
            "UPDATE cost_reservations
             SET dispatch_state = 'started_possible', updated_at = datetime('now')
             WHERE reservation_id = ? AND state = 'pending'
               AND dispatch_state IN ('not_started', 'started_possible')",
        )
        .bind(reservation_id)
        .execute(pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    async fn mark_provider_invocation_unknown(&self, reservation_id: &str) -> Result<bool> {
        let pool = self.pool().await?;
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
        // `unknown_marked_at` records that THIS path concluded the attempt
        // is over (as opposed to the lease-expiry sweep, which never sets
        // it) — `reconcile_stale_reservations` uses it to reconcile an
        // explicit mark immediately while holding a merely lease-expired row
        // until it clears a grace window (YF1, G6 review, Lane Y-fix
        // 2026-09-09).
        let result = sqlx::query(
            "UPDATE cost_reservations
             SET state = 'unknown', updated_at = datetime('now'), unknown_marked_at = datetime('now')
             WHERE reservation_id = ? AND state = 'pending'",
        )
        .bind(reservation_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(result.rows_affected() == 1)
    }

    /// An expired provider call may already have been accepted remotely;
    /// preserve its hold as unknown rather than fabricating free allowance.
    /// Shared by the single-invocation path and K1b batch admission.
    pub(super) async fn expire_lapsed_pending_reservations(
        tx: &mut sqlx::Transaction<'_, Sqlite>,
        now: &str,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE cost_reservations SET state = 'unknown', updated_at = ?
             WHERE state = 'pending' AND lease_until <= ?",
        )
        .bind(now)
        .bind(now)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    /// Settle stale `unknown` reservations when durable usage is present.
    ///
    /// A missing ledger row is not proof the provider did not bill us, so this
    /// function never auto-releases unknown holds. Unknown amounts remain in
    /// held headroom until either usage is settled or an explicit non-dispatch
    /// path (`release_provider_invocation`) releases them.
    pub(super) async fn reconcile_stale_reservations(
        tx: &mut sqlx::Transaction<'_, Sqlite>,
        task_id: &str,
        session_id: &str,
        now: &str,
    ) -> Result<()> {
        let stale: Vec<(String, String)> = sqlx::query_as(
            "WITH RECURSIVE lineage(id, parent_id) AS (
                 SELECT id, parent_session_id FROM sessions WHERE id = ?
                 UNION ALL
                 SELECT s.id, s.parent_session_id
                 FROM sessions s JOIN lineage l ON s.id = l.parent_id
             ), session_tree(id) AS (
                 SELECT id FROM lineage WHERE parent_id IS NULL
                 UNION ALL
                 SELECT s.id FROM sessions s JOIN session_tree t ON s.parent_session_id = t.id
             )
             SELECT reservation_id, invocation_id
             FROM cost_reservations
             WHERE state = 'unknown'
               AND (task_id = ? OR session_id IN (SELECT id FROM session_tree))",
        )
        .bind(session_id)
        .bind(task_id)
        .fetch_all(&mut **tx)
        .await?;
        for (reservation_id, invocation_id) in stale {
            let ledger_cost: Option<f64> =
                sqlx::query_scalar("SELECT cost_usd FROM cost_ledger WHERE call_id = ?")
                    .bind(&invocation_id)
                    .fetch_optional(&mut **tx)
                    .await?;
            // THE ABSENT `else` IS THE POINT. Affirmative evidence of a charge
            // settles the hold to that amount. NO evidence settles nothing: the
            // hold is RETAINED against the ceilings until real evidence arrives.
            // This branch used to release it, on the reasoning that an attempt
            // marked unknown "already concluded locally, so the lack of evidence
            // can be trusted right away". That is true for a pre-send abort and
            // false for the ambiguous post-send failure this path exists to
            // handle — the provider may have received and billed the request
            // while we recorded nothing. Absence of our evidence is not evidence
            // of absence, and neither elapsed time nor a later reservation makes
            // it so, which is why there is no timeout branch here either.
            if let Some(cost_usd) = ledger_cost {
                sqlx::query(
                    "UPDATE cost_reservations
                     SET state = 'settled', settled_cost_usd = ?, updated_at = ?
                     WHERE reservation_id = ? AND state = 'unknown'",
                )
                .bind(cost_usd)
                .bind(now)
                .bind(&reservation_id)
                .execute(&mut **tx)
                .await?;
                tracing::info!(
                    target: "permagent::cost_router",
                    reservation_id = %reservation_id,
                    invocation_id = %invocation_id,
                    task_id,
                    settled_cost_usd = cost_usd,
                    "reconciled a stale unknown provider reservation to settled from its recorded ledger spend"
                );
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn reserve_provider_invocation_inner(
        &self,
        invocation_id: &str,
        session_id: &str,
        task_id: Option<&str>,
        mut amount_usd: f64,
        lease_until: &str,
        config: &crate::cost_router::budget::BudgetConfig,
        request: ReservationRequest,
        identity: Option<&ProviderReservationIdentity>,
    ) -> Result<CostReservationOutcome> {
        if invocation_id.trim().is_empty() {
            return Err(anyhow::anyhow!("provider invocation id must not be empty"));
        }
        if !amount_usd.is_finite() || amount_usd <= 0.0 {
            return Err(anyhow::anyhow!(
                "paid provider reservation must have a finite positive bound"
            ));
        }
        let Some(task_id) = task_id.filter(|id| !id.trim().is_empty()) else {
            return Ok(CostReservationOutcome::Unknown {
                reason: "paid provider reservation requires a durable task identity".to_string(),
            });
        };
        if lease_until.trim().is_empty() {
            return Err(anyhow::anyhow!(
                "provider reservation lease must not be empty"
            ));
        }
        let lease_until = chrono::DateTime::parse_from_rfc3339(lease_until)
            .map_err(|_| anyhow::anyhow!("provider reservation lease must be RFC3339"))?
            .with_timezone(&chrono::Utc);
        if lease_until <= chrono::Utc::now() {
            return Ok(CostReservationOutcome::Unknown {
                reason: "provider reservation lease is already expired".to_string(),
            });
        }
        let lease_until = lease_until.to_rfc3339();
        let Some((task, session)) = reservation_ceilings(config) else {
            return Ok(CostReservationOutcome::Unknown {
                reason: "budget ceiling is unavailable or invalid".to_string(),
            });
        };

        let pool = self.pool().await?;
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
        let now = chrono::Utc::now().to_rfc3339();

        let mut extension = match request {
            ReservationRequest::Initial(Some(attempts)) => Some(ReservationExtension {
                attempts,
                per_attempt_usd: amount_usd / f64::from(attempts),
                config: *config,
            }),
            _ => None,
        };
        let mut extension_from = None;
        if matches!(request, ReservationRequest::Initial(_)) {
            Self::expire_lapsed_pending_reservations(&mut tx, &now).await?;

            // Initial reservations reconcile stale unknown holds (Y1). An
            // extension must not release its own evidence or mutate on refusal.
            Self::reconcile_stale_reservations(&mut tx, task_id, session_id, &now).await?;
        }

        let reservation_id = format!("invocation:{invocation_id}");
        if let Some((existing_id, existing_session, existing_task, state, amount, json, lease,
                     existing_attempt, existing_provider, existing_model, dispatch_state)) =
            sqlx::query_as::<
                _,
                (String, String, Option<String>, String, f64, Option<String>, String,
                 Option<String>, Option<String>, Option<String>, String),
            >(
                "SELECT reservation_id, session_id, task_id, state, amount_usd, extension_json, lease_until,
                        goal_attempt_id, provider, model, dispatch_state
                 FROM cost_reservations WHERE invocation_id = ?",
            )
            .bind(invocation_id)
            .fetch_optional(&mut *tx)
            .await?
        {
            if existing_session != session_id || existing_task.as_deref() != Some(task_id) {
                return Err(anyhow::anyhow!(
                    "provider invocation id is already bound to another budget scope"
                ));
            }
            if let Some(identity) = identity {
                anyhow::ensure!(
                    existing_attempt == identity.goal_attempt_id
                        && existing_provider.as_deref() == Some(identity.provider.as_str())
                        && existing_model.as_deref() == Some(identity.model.as_str()),
                    "provider invocation id is already bound to another native attempt or route"
                );
            }
            if let ReservationRequest::Extend(additional) = request {
                anyhow::ensure!(state == "pending" && lease > now, "reservation is not live");
                let mut evidence: ReservationExtension = serde_json::from_str(
                    json.as_deref()
                        .ok_or_else(|| anyhow::anyhow!("missing extension evidence"))?,
                )?;
                let to_attempts = evidence
                    .attempts
                    .checked_add(additional)
                    .ok_or_else(|| anyhow::anyhow!("extension attempts overflow"))?;
                amount_usd = evidence.per_attempt_usd * f64::from(additional);
                anyhow::ensure!(
                    evidence.config == *config
                        && amount_usd.is_finite()
                        && amount_usd > 0.0
                        && amount.is_finite()
                        && amount > 0.0
                        && (amount + amount_usd).is_finite(),
                    "invalid extension arithmetic or changed budget evidence"
                );
                extension_from = Some((amount, evidence.attempts));
                evidence.attempts = to_attempts;
                extension = Some(evidence);
            } else {
                if identity.is_some() && state == "pending" && dispatch_state != "not_started" {
                    tx.commit().await?;
                    return Ok(CostReservationOutcome::AlreadyDispatched {
                        reservation_id: existing_id,
                    });
                }
                tx.commit().await?;
                return Ok(match state.as_str() {
                    "pending" => CostReservationOutcome::AlreadyReserved {
                        reservation_id: existing_id,
                    },
                    "settled" => CostReservationOutcome::AlreadySettled {
                        reservation_id: existing_id,
                    },
                    "unknown" => CostReservationOutcome::Unknown {
                        reason: "provider reservation lease expired before settlement".to_string(),
                    },
                    "released" => CostReservationOutcome::Unknown {
                        reason: "provider invocation reservation was already released".to_string(),
                    },
                    _ => CostReservationOutcome::Unknown {
                        reason: format!("unrecognized reservation state: {state}"),
                    },
                });
            }
        } else if matches!(request, ReservationRequest::Extend(_)) {
            return Err(anyhow::anyhow!("reservation disappeared before extension"));
        }

        let session_exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE id = ?")
            .bind(session_id)
            .fetch_one(&mut *tx)
            .await?;
        if session_exists != 1 {
            tx.commit().await?;
            return Ok(CostReservationOutcome::Unknown {
                reason: format!("session '{session_id}' does not exist"),
            });
        }
        let native_identity_valid = if let Some(attempt_id) =
            identity.and_then(|identity| identity.goal_attempt_id.as_deref())
        {
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM goal_attempts a
                         JOIN goal_execution_bindings b ON b.goal_id = a.goal_id
                         WHERE a.attempt_id = ? AND a.worker_session_id = ?
                           AND a.execution_state = 'started' AND b.authority = 'native'
                           AND b.task_id = ?",
            )
            .bind(attempt_id)
            .bind(session_id)
            .bind(task_id)
            .fetch_one(&mut *tx)
            .await?;
            count == 1
        } else {
            false
        };
        let durable_task_matches = if identity
            .and_then(|identity| identity.goal_attempt_id.as_ref())
            .is_some()
        {
            native_identity_valid
        } else {
            let extension_data_json: String =
                sqlx::query_scalar("SELECT extension_data FROM sessions WHERE id = ?")
                    .bind(session_id)
                    .fetch_one(&mut *tx)
                    .await?;
            serde_json::from_str::<ExtensionData>(&extension_data_json)
                .ok()
                .and_then(|data| budget_task_id(&data))
                .as_deref()
                == Some(task_id)
        };
        if !durable_task_matches {
            tx.commit().await?;
            return Ok(CostReservationOutcome::Unknown {
                reason: "provider reservation task identity is not bound to this session"
                    .to_string(),
            });
        }

        if let Some(outcome) =
            reservation_ceiling_verdict(&mut tx, task_id, session_id, amount_usd, task, session)
                .await?
        {
            tx.commit().await?;
            return Ok(outcome);
        }

        if let Some((from_usd, from_attempts)) = extension_from {
            let evidence = extension
                .as_ref()
                .expect("extension evidence was validated");
            let to_usd = from_usd + amount_usd;
            sqlx::query(
                "UPDATE cost_reservations SET amount_usd = ?, extension_json = ?, updated_at = ?
                 WHERE invocation_id = ? AND state = 'pending'",
            )
            .bind(to_usd)
            .bind(serde_json::to_string(evidence)?)
            .bind(&now)
            .bind(invocation_id)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            tracing::info!(
                target: "permagent::cost_router::reservation",
                invocation_id, from_usd, to_usd, from_attempts,
                to_attempts = evidence.attempts, "reservation extended"
            );
            return Ok(CostReservationOutcome::Granted { reservation_id });
        }

        sqlx::query(
            "INSERT INTO cost_reservations
                (reservation_id, invocation_id, session_id, task_id, amount_usd,
                 state, lease_until, created_at, updated_at, extension_json,
                 goal_attempt_id, provider, model, dispatch_state)
             VALUES (?, ?, ?, ?, ?, 'pending', ?, ?, ?, ?, ?, ?, ?, 'not_started')",
        )
        .bind(&reservation_id)
        .bind(invocation_id)
        .bind(session_id)
        .bind(task_id)
        .bind(amount_usd)
        .bind(lease_until)
        .bind(&now)
        .bind(&now)
        .bind(extension.as_ref().map(serde_json::to_string).transpose()?)
        .bind(identity.and_then(|i| i.goal_attempt_id.as_deref()))
        .bind(identity.map(|i| i.provider.as_str()))
        .bind(identity.map(|i| i.model.as_str()))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(CostReservationOutcome::Granted { reservation_id })
    }

    /// See [`SessionManager::grant_reservation_after_gate_approval`].
    #[allow(clippy::too_many_arguments)]
    async fn grant_reservation_after_gate_approval_with_identity(
        &self,
        invocation_id: &str,
        session_id: &str,
        task_id: &str,
        amount_usd: f64,
        lease_until: &str,
        identity: &ProviderReservationIdentity,
    ) -> Result<CostReservationOutcome> {
        self.grant_reservation_after_gate_approval(
            invocation_id,
            session_id,
            task_id,
            amount_usd,
            lease_until,
            &identity.provider,
            &identity.model,
            Some(identity),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn grant_reservation_after_gate_approval(
        &self,
        invocation_id: &str,
        session_id: &str,
        task_id: &str,
        amount_usd: f64,
        lease_until: &str,
        provider: &str,
        model: &str,
        identity: Option<&ProviderReservationIdentity>,
    ) -> Result<CostReservationOutcome> {
        if invocation_id.trim().is_empty() {
            return Err(anyhow::anyhow!("provider invocation id must not be empty"));
        }
        if !amount_usd.is_finite() || amount_usd <= 0.0 {
            return Err(anyhow::anyhow!(
                "paid provider reservation must have a finite positive bound"
            ));
        }
        if task_id.trim().is_empty() {
            return Err(anyhow::anyhow!(
                "paid provider reservation requires a durable task identity"
            ));
        }
        let lease_parsed = chrono::DateTime::parse_from_rfc3339(lease_until)
            .map_err(|_| anyhow::anyhow!("provider reservation lease must be RFC3339"))?
            .with_timezone(&chrono::Utc);
        if lease_parsed <= chrono::Utc::now() {
            return Ok(CostReservationOutcome::Unknown {
                reason: "provider reservation lease is already expired".to_string(),
            });
        }
        let lease_until = lease_parsed.to_rfc3339();

        let pool = self.pool().await?;
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;

        // Idempotent, same shape as `reserve_provider_invocation`'s own
        // early lookup: never insert twice for one invocation_id.
        if let Some((
            existing_id,
            existing_session,
            existing_task,
            state,
            existing_attempt,
            existing_provider,
            existing_model,
            dispatch_state,
        )) = sqlx::query_as::<
            _,
            (
                String,
                String,
                Option<String>,
                String,
                Option<String>,
                Option<String>,
                Option<String>,
                String,
            ),
        >(
            "SELECT reservation_id, session_id, task_id, state,
                        goal_attempt_id, provider, model, dispatch_state
                 FROM cost_reservations WHERE invocation_id = ?",
        )
        .bind(invocation_id)
        .fetch_optional(&mut *tx)
        .await?
        {
            if existing_session != session_id || existing_task.as_deref() != Some(task_id) {
                return Err(anyhow::anyhow!(
                    "provider invocation id is already bound to another budget scope"
                ));
            }
            if let Some(identity) = identity {
                anyhow::ensure!(
                    existing_attempt == identity.goal_attempt_id
                        && existing_provider.as_deref() == Some(identity.provider.as_str())
                        && existing_model.as_deref() == Some(identity.model.as_str()),
                    "provider invocation id is already bound to another native attempt or route"
                );
                anyhow::ensure!(
                    state != "pending" || dispatch_state == "not_started",
                    "native provider invocation may already have been dispatched; refusing replay"
                );
            }
            tx.commit().await?;
            return Ok(match state.as_str() {
                "pending" => CostReservationOutcome::AlreadyReserved {
                    reservation_id: existing_id,
                },
                "settled" => CostReservationOutcome::AlreadySettled {
                    reservation_id: existing_id,
                },
                "unknown" => CostReservationOutcome::Unknown {
                    reason: "provider reservation lease expired before settlement".to_string(),
                },
                "released" => CostReservationOutcome::Unknown {
                    reason: "provider invocation reservation was already released".to_string(),
                },
                _ => CostReservationOutcome::Unknown {
                    reason: format!("unrecognized reservation state: {state}"),
                },
            });
        }

        let session_exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE id = ?")
            .bind(session_id)
            .fetch_one(&mut *tx)
            .await?;
        if session_exists != 1 {
            tx.commit().await?;
            return Ok(CostReservationOutcome::Unknown {
                reason: format!("session '{session_id}' does not exist"),
            });
        }

        if let Some(identity) = identity {
            let native_attempt = if let Some(attempt_id) = identity.goal_attempt_id.as_deref() {
                let count: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM goal_attempts a
                     JOIN goal_execution_bindings b ON b.goal_id = a.goal_id
                     WHERE a.attempt_id = ? AND a.worker_session_id = ?
                       AND a.execution_state = 'started' AND b.authority = 'native'
                       AND b.task_id = ?",
                )
                .bind(attempt_id)
                .bind(session_id)
                .bind(task_id)
                .fetch_one(&mut *tx)
                .await?;
                count == 1
            } else {
                let extension_data: String =
                    sqlx::query_scalar("SELECT extension_data FROM sessions WHERE id = ?")
                        .bind(session_id)
                        .fetch_one(&mut *tx)
                        .await?;
                serde_json::from_str::<ExtensionData>(&extension_data)
                    .ok()
                    .and_then(|data| budget_task_id(&data))
                    .as_deref()
                    == Some(task_id)
            };
            if !native_attempt {
                tx.commit().await?;
                return Ok(CostReservationOutcome::Unknown {
                    reason: "gate reservation task identity is not bound to this session".into(),
                });
            }
        }

        let reservation_id = format!("invocation:{invocation_id}");
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO cost_reservations
                (reservation_id, invocation_id, session_id, task_id, amount_usd,
                 state, lease_until, created_at, updated_at, granted_via_gate,
                 provider, model, goal_attempt_id, dispatch_state)
             VALUES (?, ?, ?, ?, ?, 'pending', ?, ?, ?, 1, ?, ?, ?, 'not_started')",
        )
        .bind(&reservation_id)
        .bind(invocation_id)
        .bind(session_id)
        .bind(task_id)
        .bind(amount_usd)
        .bind(&lease_until)
        .bind(&now)
        .bind(&now)
        .bind(provider)
        .bind(model)
        .bind(identity.and_then(|i| i.goal_attempt_id.as_deref()))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(CostReservationOutcome::Granted { reservation_id })
    }

    /// See [`SessionManager::consume_granted_reservation`].
    #[allow(clippy::too_many_arguments)]
    async fn consume_granted_reservation(
        &self,
        invocation_id: &str,
        session_id: &str,
        provider: &str,
        model: &str,
        consume_ttl_secs: i64,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<CostReservationOutcome>> {
        let pool = self.pool().await?;
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;

        let Some((
            reservation_id,
            existing_session,
            existing_provider,
            existing_model,
            state,
            created_at,
            reuse_count,
        )) = sqlx::query_as::<
            _,
            (
                String,
                String,
                Option<String>,
                Option<String>,
                String,
                String,
                i64,
            ),
        >(
            "SELECT reservation_id, session_id, provider, model, state, created_at, reuse_count
             FROM cost_reservations WHERE invocation_id = ?",
        )
        .bind(invocation_id)
        .fetch_optional(&mut *tx)
        .await?
        else {
            // Nothing was ever granted under this invocation_id — the
            // caller falls through to raising a fresh gate decision.
            tx.commit().await?;
            return Ok(None);
        };

        if existing_session != session_id {
            // F1b pin: a grant belongs to the session whose failed call it
            // authorized. A different session retrying (or any other
            // session) can never consume it — never touched, never reported.
            tx.commit().await?;
            return Ok(None);
        }

        // F3 (H1, harness loop, Round F, from Lane G9's HIGH finding): a
        // grant only authorizes the exact provider/model of the call it was
        // minted for. A mismatch here is simply "not reusable" — fall
        // through exactly like a wrong-session grant, never touching the
        // row, so the caller mints a fresh invocation_id and re-asks the
        // gate instead of silently reusing the OLD call's dollar bound
        // under a NEW provider/model.
        if existing_provider.as_deref() != Some(provider)
            || existing_model.as_deref() != Some(model)
        {
            tx.commit().await?;
            return Ok(None);
        }

        let age_secs = chrono::DateTime::parse_from_rfc3339(&created_at)
            .map(|dt| (now - dt.with_timezone(&chrono::Utc)).num_seconds())
            .unwrap_or(i64::MAX);
        if state == "pending" && age_secs > consume_ttl_secs {
            // F1b pin: unconsumed past its TTL. Release rather than hold it
            // open until its full (2-hour) `lease_until` — a stale grant
            // must stop counting toward budget for a retry that never came
            // — then report "nothing to consume" so the caller raises a
            // fresh gate decision instead of trusting a stale yes.
            sqlx::query(
                "UPDATE cost_reservations SET state = 'released', updated_at = ?
                 WHERE reservation_id = ? AND state = 'pending'",
            )
            .bind(now.to_rfc3339())
            .bind(&reservation_id)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            return Ok(None);
        }

        // F3 (M1, harness loop, Round F, from Lane G9's MEDIUM finding): a
        // grant is consumable exactly once — the retry of the specific call
        // that got gated, not every later call in the session while the row
        // happens to still be `pending`. Refused the same way a wrong-
        // session or mismatched-provider grant already is: never touched,
        // never reported, caller falls through to a fresh gate decision.
        //
        // F3b (from Lane G10's MEDIUM finding): this refusal runs AFTER the
        // TTL-staleness branch above, never before it. Ordered the other
        // way, a grant consumed once could never take the fast 120s
        // self-release path again — it would short-circuit here on every
        // later call and only die via the ~8h lease-expiry + reconcile
        // sweep, inflating `task_held`/`session_held` the whole time. A
        // stale grant is released first; only a grant still WITHIN its TTL
        // is refused-but-left-`pending` for the real in-flight call to
        // settle.
        if state == "pending" && reuse_count > 0 {
            tx.commit().await?;
            return Ok(None);
        }

        if state == "pending" {
            // F3 (M1): mark this grant consumed so a later call sharing the
            // same session/provider/model (e.g. a concurrent background
            // call, or a later turn that finds the row still `pending`
            // because it has not settled yet) cannot reuse it a second
            // time — see the `reuse_count > 0` guard above.
            // `reused_by_invocation_id` is recorded explicitly rather than
            // assumed equal to this row's own `invocation_id`, even though
            // today it always is (the design reuses the SAME id for the
            // retry).
            sqlx::query(
                "UPDATE cost_reservations SET reuse_count = reuse_count + 1,
                 reused_by_invocation_id = ?, updated_at = ?
                 WHERE reservation_id = ?",
            )
            .bind(invocation_id)
            .bind(now.to_rfc3339())
            .bind(&reservation_id)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(match state.as_str() {
            "pending" => Some(CostReservationOutcome::AlreadyReserved { reservation_id }),
            "settled" => Some(CostReservationOutcome::AlreadySettled { reservation_id }),
            "unknown" => Some(CostReservationOutcome::Unknown {
                reason: "provider reservation lease expired before settlement".to_string(),
            }),
            // I1 (harness loop, 2026-09-10, gate fix): a released grant —
            // whether released by THIS call's own TTL-expiry branch above,
            // or by an EARLIER call finding it already past TTL — has
            // nothing left to consume. Idempotent with the TTL-expiry
            // branch's own `Ok(None)` a few lines up: every subsequent
            // consume attempt on an expired grant reports the SAME "raise a
            // fresh gate decision" signal, never a reported `Unknown`
            // outcome the caller might mistake for something to act on.
            "released" => None,
            other => Some(CostReservationOutcome::Unknown {
                reason: format!("unrecognized reservation state: {other}"),
            }),
        })
    }

    /// See [`SessionManager::find_pending_gate_grant`].
    async fn find_pending_gate_grant(
        &self,
        session_id: &str,
        provider: &str,
        model: &str,
    ) -> Result<Option<String>> {
        let pool = self.pool().await?;
        let invocation_id: Option<String> = sqlx::query_scalar(
            "SELECT invocation_id FROM cost_reservations
             WHERE session_id = ? AND state = 'pending' AND granted_via_gate = 1
               AND provider = ? AND model = ?
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(session_id)
        .bind(provider)
        .bind(model)
        .fetch_optional(pool)
        .await?;
        Ok(invocation_id)
    }

    async fn append_cost_ledger_with_usage(
        &self,
        row: &CostLedgerRow,
        usage: Option<LedgerUsageColumns>,
        reservation_id: Option<&str>,
    ) -> Result<bool> {
        let pool = self.pool().await?;
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;

        // YF1 (G6 review, Lane Y-fix 2026-09-09): captured outside the `if
        // let` below so the late-settlement UPDATEs further down can still
        // see the reservation's state as of this transaction, and know
        // whether this settlement is "on time" (state was `pending`) or
        // "late" (the reservation was already reconciled to `unknown` or
        // `released` before the real spend arrived).
        let mut reservation_state: Option<String> = None;

        if let Some(reservation_id) = reservation_id {
            let Some((
                reservation_invocation,
                state,
                reserved_usd,
                reservation_session,
                reservation_task,
                reservation_attempt,
                reservation_provider,
                reservation_model,
            )) = sqlx::query_as::<
                _,
                (
                    String,
                    String,
                    f64,
                    String,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                ),
            >(
                "SELECT invocation_id, state, amount_usd, session_id, task_id, goal_attempt_id,
                        provider, model
                     FROM cost_reservations
                     WHERE reservation_id = ?",
            )
            .bind(reservation_id)
            .fetch_optional(&mut *tx)
            .await?
            else {
                return Err(anyhow::anyhow!(
                    "provider reservation '{reservation_id}' not found"
                ));
            };
            if reservation_invocation != row.call_id {
                return Err(anyhow::anyhow!(
                    "provider reservation does not match ledger invocation"
                ));
            }
            if reservation_session != row.session_id
                || reservation_task.as_deref() != row.task_id.as_deref()
                || reservation_attempt.as_deref() != row.goal_attempt_id.as_deref()
                || reservation_provider
                    .as_deref()
                    .is_some_and(|provider| row.provider.as_deref() != Some(provider))
                || reservation_model
                    .as_deref()
                    .is_some_and(|model| row.model.as_deref() != Some(model))
            {
                return Err(anyhow::anyhow!(
                    "provider reservation does not match ledger scope or native attempt"
                ));
            }
            if !row.cost_usd.is_finite() || row.cost_usd < 0.0 || !reserved_usd.is_finite() {
                return Err(anyhow::anyhow!(
                    "provider settlement cost must be finite and non-negative"
                ));
            }
            if row.cost_usd > reserved_usd {
                tracing::warn!(
                    reservation_id,
                    reserved_usd,
                    actual_cost_usd = row.cost_usd,
                    "provider settlement exceeded its spend bound"
                );
            }
            match state.as_str() {
                "settled" => {
                    tx.commit().await?;
                    return Ok(false);
                }
                "pending" => {}
                // YF1: a reservation already reconciled away (settled to $0
                // / released because no ledger evidence existed at the
                // time) can still have its provider call complete for real
                // afterward. Erroring here (the pre-YF1 behavior) would
                // drop that real spend on the floor entirely — the budget
                // headroom was already given back, and now the actual bill
                // goes unrecorded too. Instead this settlement is accepted
                // as a LATE one: recorded in the ledger (flagged
                // `late_settlement`) so it still counts toward the ceiling,
                // never silently dropped (G6 finding #1).
                "unknown" | "released" => {
                    tracing::info!(
                        target: "permagent::cost_router",
                        reservation_id,
                        state = %state,
                        call_id = %row.call_id,
                        cost_usd = row.cost_usd,
                        "settling a provider call whose reservation was already reconciled away; recording as a late settlement rather than dropping the spend"
                    );
                }
                _ => {
                    return Err(anyhow::anyhow!(
                        "provider reservation '{reservation_id}' has invalid state '{state}'"
                    ));
                }
            }
            reservation_state = Some(state);
        }
        let is_late_settlement = matches!(
            reservation_state.as_deref(),
            Some("unknown") | Some("released")
        );

        let inserted = sqlx::query(
            "INSERT INTO cost_ledger (
                call_id, ts, session_id, parent_session_id, task_id, goal_id,
                subagent_id, turn_index, provider, model, cost_tier, is_chargeable,
                is_headless, input_tokens, output_tokens, cache_read_tokens,
                cache_write_tokens, input_cost, output_cost, cache_read_cost,
                cache_write_cost, cost_usd, is_estimated, late_settlement,
                call_purpose, trigger_ref, retry_of_call_id, pre_call_evidence,
                tool_call_count, completion_kind, goal_attempt_id
            ) VALUES (
                ?, ?, ?, ?, ?, ?,
                ?, (SELECT COUNT(*) FROM cost_ledger WHERE session_id = ?), ?, ?, ?, ?,
                ?, ?, ?, ?,
                ?, ?, ?, ?,
                ?, ?, ?, ?,
                ?, ?, ?, ?,
                ?, ?, ?
            ) ON CONFLICT(call_id) DO NOTHING",
        )
        .bind(&row.call_id)
        .bind(&row.ts)
        .bind(&row.session_id)
        .bind(&row.parent_session_id)
        .bind(&row.task_id)
        .bind(&row.goal_id)
        .bind(&row.subagent_id)
        .bind(&row.session_id) // turn_index subquery
        .bind(&row.provider)
        .bind(&row.model)
        .bind(row.cost_tier.as_str())
        .bind(row.cost_tier.is_chargeable())
        .bind(row.is_headless)
        .bind(row.input_tokens)
        .bind(row.output_tokens)
        .bind(row.cache_read_tokens)
        .bind(row.cache_write_tokens)
        .bind(row.input_cost)
        .bind(row.output_cost)
        .bind(row.cache_read_cost)
        .bind(row.cache_write_cost)
        .bind(row.cost_usd)
        .bind(row.is_estimated)
        .bind(is_late_settlement)
        // v60: persisted exactly as supplied. The writer never derives or
        // guesses these; an unknown purpose/kind is written as 'unknown' and
        // an unobserved tool count as NULL (distinct from 0).
        .bind(row.provenance.purpose.as_str())
        .bind(row.provenance.trigger_ref.as_deref())
        .bind(row.provenance.retry_of_call_id.as_deref())
        .bind(
            row.provenance
                .pre_call_evidence
                .as_ref()
                .map(PreCallEvidence::as_json),
        )
        .bind(row.outcome.tool_call_count)
        .bind(row.outcome.completion_kind.as_str())
        .bind(&row.goal_attempt_id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;

        if !inserted {
            if let (Some(reservation_id), Some(from_state)) = (reservation_id, &reservation_state) {
                let Some((existing_session, existing_task, existing_cost, existing_attempt)) =
                    sqlx::query_as::<_, (String, Option<String>, f64, Option<String>)>(
                        "SELECT session_id, task_id, cost_usd, goal_attempt_id FROM cost_ledger WHERE call_id = ?",
                    )
                    .bind(&row.call_id)
                    .fetch_optional(&mut *tx)
                    .await?
                else {
                    return Err(anyhow::anyhow!(
                        "ledger conflict did not leave an existing invocation row"
                    ));
                };
                if existing_session != row.session_id
                    || existing_task.as_deref() != row.task_id.as_deref()
                    || existing_attempt.as_deref() != row.goal_attempt_id.as_deref()
                    || existing_cost != row.cost_usd
                {
                    return Err(anyhow::anyhow!(
                        "existing ledger invocation does not match reservation settlement"
                    ));
                }
                sqlx::query(
                    "UPDATE cost_reservations SET state = 'settled',
                        settled_cost_usd = ?, updated_at = datetime('now')
                     WHERE reservation_id = ? AND state = ?",
                )
                .bind(row.cost_usd)
                .bind(reservation_id)
                .bind(from_state)
                .execute(&mut *tx)
                .await?;
            }
            tx.commit().await?;
            return Ok(false);
        }

        if let Some((
            schedule_id,
            current_total,
            current_input,
            current_output,
            delta_total,
            delta_input,
            delta_output,
        )) = usage
        {
            sqlx::query(
                "UPDATE sessions SET
                    total_tokens = ?,
                    input_tokens = ?,
                    output_tokens = ?,
                    accumulated_total_tokens = COALESCE(accumulated_total_tokens, 0) + ?,
                    accumulated_input_tokens = COALESCE(accumulated_input_tokens, 0) + ?,
                    accumulated_output_tokens = COALESCE(accumulated_output_tokens, 0) + ?,
                    schedule_id = ?,
                    updated_at = datetime('now')
                 WHERE id = ?",
            )
            .bind(current_total)
            .bind(current_input)
            .bind(current_output)
            .bind(delta_total)
            .bind(delta_input)
            .bind(delta_output)
            .bind(schedule_id)
            .bind(&row.session_id)
            .execute(&mut *tx)
            .await?;
        }

        // O(1) rollup: last-turn cost + running accumulators. COALESCE handles the
        // first append (columns start NULL on existing DBs).
        sqlx::query(
            "UPDATE sessions SET
                cost_usd = ?,
                accumulated_cost_usd = COALESCE(accumulated_cost_usd, 0) + ?,
                accumulated_cache_read_tokens = COALESCE(accumulated_cache_read_tokens, 0) + ?,
                accumulated_cache_write_tokens = COALESCE(accumulated_cache_write_tokens, 0) + ?,
                accumulated_cache_savings_usd = COALESCE(accumulated_cache_savings_usd, 0) + ?
             WHERE id = ?",
        )
        .bind(row.cost_usd)
        .bind(row.cost_usd)
        .bind(row.cache_read_tokens)
        .bind(row.cache_write_tokens)
        .bind(row.cache_savings_usd)
        .bind(&row.session_id)
        .execute(&mut *tx)
        .await?;

        if let (Some(reservation_id), Some(from_state)) = (reservation_id, &reservation_state) {
            let settled = sqlx::query(
                "UPDATE cost_reservations SET state = 'settled',
                    settled_cost_usd = ?, updated_at = datetime('now')
                 WHERE reservation_id = ? AND state = ?",
            )
            .bind(row.cost_usd)
            .bind(reservation_id)
            .bind(from_state)
            .execute(&mut *tx)
            .await?;
            if settled.rows_affected() != 1 {
                return Err(anyhow::anyhow!(
                    "provider reservation '{reservation_id}' changed before settlement"
                ));
            }
        }

        tx.commit().await?;
        Ok(true)
    }

    async fn cost_by_parent_session(&self, parent_id: &str) -> Result<ParentSessionCost> {
        let pool = self.pool().await?;

        // Own spend: prefer the O(1) session rollup; fall back to a ledger SUM
        // so a session that somehow lost its rollup columns still answers.
        let own: f64 = sqlx::query_scalar(
            "SELECT COALESCE(
                (SELECT accumulated_cost_usd FROM sessions WHERE id = ?),
                (SELECT SUM(cost_usd) FROM cost_ledger WHERE session_id = ?),
                0.0
             )",
        )
        .bind(parent_id)
        .bind(parent_id)
        .fetch_one(pool)
        .await?;

        let rows: Vec<(String, f64)> = sqlx::query_as(
            "SELECT id,
                    COALESCE(
                        accumulated_cost_usd,
                        (SELECT SUM(cost_usd) FROM cost_ledger WHERE session_id = sessions.id),
                        0.0
                    ) AS cost_usd
             FROM sessions
             WHERE parent_session_id = ?
             ORDER BY created_at ASC, id ASC",
        )
        .bind(parent_id)
        .fetch_all(pool)
        .await?;

        let per_child: Vec<ChildSessionCost> = rows
            .into_iter()
            .map(|(session_id, cost_usd)| ChildSessionCost {
                session_id,
                cost_usd,
            })
            .collect();
        let children_total = per_child.iter().map(|c| c.cost_usd).sum();

        Ok(ParentSessionCost {
            own,
            children_total,
            per_child,
        })
    }

    async fn get_conversation(&self, session_id: &str) -> Result<Conversation> {
        let pool = self.pool().await?;
        let rows = sqlx::query_as::<_, (String, String, i64, Option<String>, Option<String>)>(
            // Order by created_timestamp, then by id to break ties. created_timestamp is in seconds,
            // so messages created in the same second (e.g., tool request and response) need to
            // maintain their insertion order via the auto-increment id.
            "SELECT role, content_json, created_timestamp, metadata_json, message_id FROM messages WHERE session_id = ? ORDER BY created_timestamp, id",
        )
            .bind(session_id)
            .fetch_all(pool)
            .await?;

        let mut messages = Vec::new();
        for (role_str, content_json, created_timestamp, metadata_json, message_id) in
            rows.into_iter()
        {
            let role = match role_str.as_str() {
                "user" => Role::User,
                "assistant" => Role::Assistant,
                _ => continue,
            };

            let content = serde_json::from_str(&content_json)?;
            let metadata = metadata_json
                .and_then(|json| serde_json::from_str(&json).ok())
                .unwrap_or_default();

            let mut message = Message::new(role, created_timestamp, content);
            message.metadata = metadata;
            if let Some(id) = message_id {
                message = message.with_id(id);
            }
            // Repair sessions that stored per-token Thinking deltas (Kimi crash).
            messages.push(message.coalesce_adjacent_text_and_thinking());
        }

        Ok(Conversation::new_unvalidated(messages))
    }

    async fn add_message(&self, session_id: &str, message: &Message) -> Result<()> {
        let pool = self.pool().await?;
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;

        let metadata_json = serde_json::to_string(&message.metadata)?;
        let content_json = persistable_content_json(message)?;

        let message_id = message
            .id
            .clone()
            .unwrap_or_else(|| format!("msg_{}_{}", session_id, uuid::Uuid::new_v4()));

        sqlx::query(
            r#"
            INSERT INTO messages (message_id, session_id, role, content_json, created_timestamp, metadata_json)
            VALUES (?, ?, ?, ?, ?, ?)
        "#,
        )
        .bind(message_id)
        .bind(session_id)
        .bind(role_to_string(&message.role))
        .bind(content_json)
        .bind(message.created)
        .bind(metadata_json)
        .execute(&mut *tx)
        .await?;

        sqlx::query("UPDATE sessions SET updated_at = datetime('now') WHERE id = ?")
            .bind(session_id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(())
    }

    async fn replace_conversation_inner(
        pool: &Pool<Sqlite>,
        session_id: &str,
        conversation: &Conversation,
    ) -> Result<()> {
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;

        sqlx::query("DELETE FROM messages WHERE session_id = ?")
            .bind(session_id)
            .execute(&mut *tx)
            .await?;

        for message in conversation.messages() {
            let metadata_json = serde_json::to_string(&message.metadata)?;
            let content_json = persistable_content_json(message)?;

            let message_id = message
                .id
                .clone()
                .unwrap_or_else(|| format!("msg_{}_{}", session_id, uuid::Uuid::new_v4()));

            sqlx::query(
                r#"
            INSERT INTO messages (message_id, session_id, role, content_json, created_timestamp, metadata_json)
            VALUES (?, ?, ?, ?, ?, ?)
        "#,
            )
            .bind(message_id)
            .bind(session_id)
            .bind(role_to_string(&message.role))
            .bind(content_json)
            .bind(message.created)
            .bind(metadata_json)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    pub async fn replace_conversation(
        &self,
        session_id: &str,
        conversation: &Conversation,
    ) -> Result<()> {
        let pool = self.pool().await?;
        Self::replace_conversation_inner(pool, session_id, conversation).await
    }

    async fn list_sessions_by_types(&self, types: Option<&[SessionType]>) -> Result<Vec<Session>> {
        let (where_clause, binds): (String, Vec<String>) = match types {
            Some(t) if !t.is_empty() => {
                let placeholders: String = t.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
                (
                    format!("WHERE s.session_type IN ({})", placeholders),
                    t.iter().map(|t| t.to_string()).collect(),
                )
            }
            Some(_) => return Ok(Vec::new()),
            None => (String::new(), Vec::new()),
        };

        let query = format!(
            r#"
            SELECT s.id, s.working_dir, s.name, s.description, s.user_set_name, s.session_type, s.created_at, s.updated_at, s.extension_data,
                   s.total_tokens, s.input_tokens, s.output_tokens,
                   s.accumulated_total_tokens, s.accumulated_input_tokens, s.accumulated_output_tokens,
                   s.schedule_id, s.recipe_json, s.user_recipe_values_json,
                   s.provider_name, s.model_config_json, s.goose_mode, s.thread_id,
                   COALESCE(mc.c, 0) as message_count
            FROM sessions s
            LEFT JOIN (SELECT session_id, COUNT(*) AS c FROM messages GROUP BY session_id) mc
                   ON mc.session_id = s.id
            {}
            ORDER BY s.updated_at DESC
            "#,
            where_clause
        );

        // `where_clause` interpolates only "?" placeholders (count derived from
        // `types.len()`) — no external data reaches the SQL text; the actual
        // type values are bound below.
        let mut q = sqlx::query_as::<_, Session>(sqlx::AssertSqlSafe(query));
        for b in &binds {
            q = q.bind(b);
        }

        let pool = self.pool().await?;

        // #341 instrumentation: separate pool-acquire wait from SQL execution
        // time. PR #340 showed the query is single-digit ms even at 32x scale;
        // this confirms it in-process and rules acquire-wait in or out.
        let acquire_start = std::time::Instant::now();
        let mut conn = pool.acquire().await?;
        let acquire_ms = acquire_start.elapsed().as_secs_f64() * 1000.0;

        let exec_start = std::time::Instant::now();
        let rows = q.fetch_all(&mut *conn).await?;
        let exec_ms = exec_start.elapsed().as_secs_f64() * 1000.0;

        info!(
            target: "session_perf",
            acquire_ms,
            exec_ms,
            row_count = rows.len(),
            "list_sessions_by_types: pool-acquire + SQL exec timing"
        );
        Ok(rows)
    }

    async fn list_sessions(&self) -> Result<Vec<Session>> {
        self.list_sessions_by_types(Some(&[SessionType::User, SessionType::Scheduled]))
            .await
    }

    /// Sessions for a single schedule, newest first, capped at `limit`.
    ///
    /// The filter + cap are pushed into SQL so this never materialises the full
    /// session table. The Automate tab polls `/schedule/{id}/sessions` once per
    /// job every ~10s; the previous path ran the fat `list_sessions()` (an
    /// uncorrelated `GROUP BY` over the entire messages table) and filtered in
    /// memory, so N jobs meant N full scans per tick — the source of the 1.4–3.2s
    /// `exec_ms` spikes when the page cache went cold. Here `WHERE schedule_id`
    /// touches only the matching rows and the per-row `message_count` is a
    /// correlated count that rides `idx_messages_session`, computed only for the
    /// ≤`limit` rows returned.
    async fn list_sessions_by_schedule_id(
        &self,
        schedule_id: &str,
        limit: usize,
    ) -> Result<Vec<Session>> {
        let pool = self.pool().await?;
        sqlx::query_as::<_, Session>(LIST_SESSIONS_BY_SCHEDULE_ID_SQL)
            .bind(schedule_id)
            .bind(limit as i64)
            .fetch_all(pool)
            .await
            .map_err(Into::into)
    }

    /// Recent sessions for EVERY schedule that has one, batched into a
    /// SINGLE query via a window function — the replacement for the Automate
    /// tab's old pattern of calling [`Self::list_sessions_by_schedule_id`]
    /// once per job on every poll tick (N queries; the 2026-08-25 "schedule
    /// polling storm" health review). Only the lean columns
    /// [`ScheduleSessionSummary`] needs are selected — no recipe/model-config
    /// JSON blobs — and `message_count` is computed only for the rows that
    /// survive the per-schedule `LIMIT`, not the whole table. Rides
    /// `idx_sessions_schedule_id` for the initial filter.
    async fn list_recent_sessions_by_schedule(
        &self,
        limit_per_schedule: usize,
    ) -> Result<Vec<ScheduleSessionSummary>> {
        let query = r#"
            WITH ranked AS (
                SELECT s.id, s.name, s.description, s.created_at, s.working_dir, s.schedule_id,
                       s.total_tokens, s.input_tokens, s.output_tokens,
                       s.accumulated_total_tokens, s.accumulated_input_tokens, s.accumulated_output_tokens,
                       ROW_NUMBER() OVER (PARTITION BY s.schedule_id ORDER BY s.created_at DESC) AS rn
                FROM sessions s
                WHERE s.schedule_id IS NOT NULL
            )
            SELECT r.id, r.name, r.description, r.created_at, r.working_dir, r.schedule_id,
                   r.total_tokens, r.input_tokens, r.output_tokens,
                   r.accumulated_total_tokens, r.accumulated_input_tokens, r.accumulated_output_tokens,
                   (SELECT COUNT(*) FROM messages m WHERE m.session_id = r.id) AS message_count
            FROM ranked r
            WHERE r.rn <= ?
            ORDER BY r.schedule_id, r.created_at DESC
        "#;

        let pool = self.pool().await?;
        sqlx::query_as::<_, ScheduleSessionSummary>(query)
            .bind(limit_per_schedule as i64)
            .fetch_all(pool)
            .await
            .map_err(Into::into)
    }

    /// Lean LIST query: selects only the cheap scalar columns the session-list
    /// UI uses, skipping the heavy JSON blobs that the full [`Session`] query
    /// drags along (extension_data / recipe_json / user_recipe_values_json /
    /// model_config_json). See #341/#371.
    async fn list_session_summaries(
        &self,
        types: Option<&[SessionType]>,
    ) -> Result<Vec<SessionSummary>> {
        let (where_clause, binds): (String, Vec<String>) = match types {
            Some(t) if !t.is_empty() => {
                let placeholders: String = t.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
                (
                    format!("WHERE s.session_type IN ({})", placeholders),
                    t.iter().map(|t| t.to_string()).collect(),
                )
            }
            Some(_) => return Ok(Vec::new()),
            None => (String::new(), Vec::new()),
        };

        // Naming/classifying columns (see `SessionSummary`). Every json_extract
        // is behind json_valid: one malformed blob must not fail the whole
        // list. The first-user-text subquery rides idx_messages_session. The
        // project lookups are scalar subqueries with LIMIT 1, not joins, so a
        // duplicated slug or root path can never duplicate a session row. No
        // CTE: `created_at`/`updated_at` stay direct column reads, exactly as
        // before, so their decoding is untouched.
        const GOAL_ID: &str = r#"(CASE WHEN json_valid(s.extension_data)
                THEN json_extract(s.extension_data, '$."goal_worker.v1"') END)"#;
        let query = format!(
            r#"
            SELECT s.id, s.name, s.description, s.user_set_name, s.session_type,
                   s.created_at, s.updated_at, s.working_dir, s.schedule_id,
                   CASE WHEN json_valid(s.recipe_json)
                        THEN json_extract(s.recipe_json, '$.title') END AS recipe_title,
                   {goal} AS goal_id,
                   g.title AS goal_title,
                   g.project_id AS goal_project_id,
                   COALESCE(
                       (SELECT p.id FROM projects p WHERE p.slug = s.project_hint_wing LIMIT 1),
                       g.project_id,
                       (SELECT p.id FROM projects p
                         WHERE p.root_path = s.working_dir AND p.root_path <> '' LIMIT 1)
                   ) AS project_id,
                   COALESCE(
                       (SELECT p.name FROM projects p WHERE p.slug = s.project_hint_wing LIMIT 1),
                       (SELECT p.name FROM projects p WHERE p.id = g.project_id LIMIT 1),
                       (SELECT p.name FROM projects p
                         WHERE p.root_path = s.working_dir AND p.root_path <> '' LIMIT 1)
                   ) AS project_name,
                   (SELECT substr(json_extract(m.content_json, '$[0].text'), 1, 500)
                      FROM messages m
                     WHERE m.session_id = s.id
                       AND m.role = 'user'
                       AND json_valid(m.content_json)
                       AND (m.metadata_json IS NULL
                            OR NOT json_valid(m.metadata_json)
                            OR COALESCE(json_extract(m.metadata_json, '$.userVisible'), 1) <> 0)
                     ORDER BY m.id
                     LIMIT 1) AS first_user_text,
                   COALESCE(mc.c, 0) as message_count
            FROM sessions s
            LEFT JOIN cards g ON g.id = {goal}
            LEFT JOIN (SELECT session_id, COUNT(*) AS c FROM messages GROUP BY session_id) mc
                   ON mc.session_id = s.id
            {where_clause}
            ORDER BY s.updated_at DESC
            "#,
            goal = GOAL_ID,
            where_clause = where_clause,
        );

        // Same as list_sessions_by_types: `where_clause` is only "?" placeholders.
        let mut q = sqlx::query_as::<_, SessionSummary>(sqlx::AssertSqlSafe(query));
        for b in &binds {
            q = q.bind(b);
        }

        let pool = self.pool().await?;
        q.fetch_all(pool).await.map_err(Into::into)
    }

    async fn delete_session(&self, session_id: &str) -> Result<()> {
        let pool = self.pool().await?;
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;

        let exists =
            sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM sessions WHERE id = ?)")
                .bind(session_id)
                .fetch_one(&mut *tx)
                .await?;

        if !exists {
            return Err(anyhow::anyhow!("Session not found"));
        }

        sqlx::query("DELETE FROM messages WHERE session_id = ?")
            .bind(session_id)
            .execute(&mut *tx)
            .await?;

        sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(session_id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(())
    }

    async fn get_insights(&self, types: &[SessionType]) -> Result<SessionInsights> {
        if types.is_empty() {
            return Ok(SessionInsights {
                total_sessions: 0,
                total_tokens: 0,
            });
        }

        let placeholders: String = types.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
        let query = format!(
            r#"
            SELECT COUNT(*) as total_sessions,
                   COALESCE(SUM(COALESCE(accumulated_total_tokens, total_tokens, 0)), 0) as total_tokens
            FROM sessions
            WHERE session_type IN ({})
            "#,
            placeholders
        );

        let pool = self.pool().await?;
        // `placeholders` is only "?" repeated `types.len()` times.
        let mut q = sqlx::query_as::<_, (i64, Option<i64>)>(sqlx::AssertSqlSafe(query));
        for t in types {
            q = q.bind(t.to_string());
        }

        let row = q.fetch_one(pool).await?;

        Ok(SessionInsights {
            total_sessions: row.0 as usize,
            total_tokens: row.1.unwrap_or(0),
        })
    }

    async fn export_session(&self, id: &str) -> Result<String> {
        let session = self.get_session(id, true).await?;
        serde_json::to_string_pretty(&session).map_err(Into::into)
    }

    async fn import_session(
        &self,
        session_manager: &SessionManager,
        json: &str,
        session_type_override: Option<SessionType>,
    ) -> Result<Session> {
        let import: Session = serde_json::from_str(json)?;

        let session = self
            .create_session(
                None,
                import.working_dir.clone(),
                import.name.clone(),
                session_type_override.unwrap_or(import.session_type),
                import.goose_mode,
            )
            .await?;

        let mut extension_data = import.extension_data;
        // Import creates an independent session/budget. Preserve unrelated
        // extension state, but do not carry source execution attribution.
        extension_data.extension_states.remove(&format!(
            "{BUDGET_TASK_EXTENSION_NAME}.{BUDGET_TASK_EXTENSION_VERSION}"
        ));
        extension_data.extension_states.remove(&format!(
            "{GOAL_ID_EXTENSION_NAME}.{GOAL_ID_EXTENSION_VERSION}"
        ));
        let mut builder = session_manager
            .update(&session.id)
            .extension_data(extension_data)
            .total_tokens(import.total_tokens)
            .input_tokens(import.input_tokens)
            .output_tokens(import.output_tokens)
            .accumulated_total_tokens(import.accumulated_total_tokens)
            .accumulated_input_tokens(import.accumulated_input_tokens)
            .accumulated_output_tokens(import.accumulated_output_tokens)
            .schedule_id(import.schedule_id)
            .recipe(import.recipe)
            .user_recipe_values(import.user_recipe_values);

        if import.user_set_name {
            builder = builder.user_provided_name(import.name.clone());
        }

        builder.apply().await?;

        if let Some(conversation) = import.conversation {
            self.replace_conversation(&session.id, &conversation)
                .await?;
        }

        self.get_session(&session.id, true).await
    }

    async fn copy_session(
        &self,
        session_manager: &SessionManager,
        session_id: &str,
        new_name: String,
    ) -> Result<Session> {
        let original_session = self.get_session(session_id, true).await?;

        let new_session = self
            .create_session(
                None,
                original_session.working_dir.clone(),
                new_name,
                original_session.session_type,
                original_session.goose_mode,
            )
            .await?;

        let mut extension_data = original_session.extension_data;
        // A fork starts a new execution identity. Keep all unrelated
        // extension-owned state, but never attribute its future calls to the
        // source task or goal.
        extension_data.extension_states.remove(&format!(
            "{BUDGET_TASK_EXTENSION_NAME}.{BUDGET_TASK_EXTENSION_VERSION}"
        ));
        extension_data.extension_states.remove(&format!(
            "{GOAL_ID_EXTENSION_NAME}.{GOAL_ID_EXTENSION_VERSION}"
        ));
        let mut builder = session_manager
            .update(&new_session.id)
            .extension_data(extension_data)
            .schedule_id(original_session.schedule_id)
            .recipe(original_session.recipe)
            .user_recipe_values(original_session.user_recipe_values);

        // Preserve provider, model config, and goose_mode from original session
        if let Some(provider_name) = original_session.provider_name {
            builder = builder.provider_name(provider_name);
        }

        if let Some(model_config) = original_session.model_config {
            builder = builder.model_config(model_config);
        }

        builder = builder.goose_mode(original_session.goose_mode);

        builder.apply().await?;

        if let Some(conversation) = original_session.conversation {
            self.replace_conversation(&new_session.id, &conversation)
                .await?;
        }

        self.get_session(&new_session.id, true).await
    }

    async fn truncate_conversation(&self, session_id: &str, timestamp: i64) -> Result<()> {
        let pool = self.pool().await?;
        sqlx::query("DELETE FROM messages WHERE session_id = ? AND created_timestamp >= ?")
            .bind(session_id)
            .bind(timestamp)
            .execute(pool)
            .await?;

        Ok(())
    }

    async fn search_chat_history(
        &self,
        query: &str,
        limit: Option<usize>,
        after_date: Option<chrono::DateTime<chrono::Utc>>,
        before_date: Option<chrono::DateTime<chrono::Utc>>,
        exclude_session_id: Option<String>,
        session_types: Vec<SessionType>,
    ) -> Result<crate::session::chat_history_search::ChatRecallResults> {
        use crate::session::chat_history_search::ChatHistorySearch;

        let pool = self.pool().await?;
        ChatHistorySearch::new(
            pool,
            query,
            limit,
            after_date,
            before_date,
            exclude_session_id,
            session_types,
        )
        .execute()
        .await
    }

    async fn update_message_metadata<F>(
        &self,
        session_id: &str,
        message_id: &str,
        f: F,
    ) -> Result<()>
    where
        F: FnOnce(
            crate::conversation::message::MessageMetadata,
        ) -> crate::conversation::message::MessageMetadata,
    {
        let pool = self.pool().await?;
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;

        let current_metadata_json = sqlx::query_scalar::<_, String>(
            "SELECT metadata_json FROM messages WHERE message_id = ? AND session_id = ?",
        )
        .bind(message_id)
        .bind(session_id)
        .fetch_one(&mut *tx)
        .await?;

        let current_metadata: crate::conversation::message::MessageMetadata =
            serde_json::from_str(&current_metadata_json)?;

        let new_metadata = f(current_metadata);
        let metadata_json = serde_json::to_string(&new_metadata)?;

        sqlx::query(
            "UPDATE messages SET metadata_json = ? WHERE message_id = ? AND session_id = ?",
        )
        .bind(metadata_json)
        .bind(message_id)
        .bind(session_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::platform_extensions::terminal_supervision::{
        self as run_registry, HarnessRunSnapshot, HarnessRunStatus, HarnessRunUpdate,
    };
    use crate::conversation::message::{Message, MessageContent};
    use crate::cost_router::budget::{BudgetCeilings, BudgetConfig};
    use tempfile::TempDir;
    use test_case::test_case;

    const NUM_CONCURRENT_SESSIONS: i32 = 10;

    fn reservation_config(
        task_gate: f64,
        task_hard: f64,
        session_gate: f64,
        session_hard: f64,
    ) -> BudgetConfig {
        BudgetConfig {
            task: BudgetCeilings {
                soft: 0.0,
                gate: task_gate,
                hard: task_hard,
            },
            session: BudgetCeilings {
                soft: 0.0,
                gate: session_gate,
                hard: session_hard,
            },
        }
    }

    fn reservation_row(
        session_id: &str,
        task_id: &str,
        call_id: &str,
        cost_usd: f64,
    ) -> CostLedgerRow {
        CostLedgerRow {
            call_id: call_id.to_string(),
            ts: "2026-09-04T00:00:00Z".to_string(),
            session_id: session_id.to_string(),
            parent_session_id: None,
            task_id: Some(task_id.to_string()),
            goal_id: None,
            goal_attempt_id: None,
            subagent_id: None,
            provider: Some("test-provider".to_string()),
            model: Some("test-model".to_string()),
            cost_tier: CostTier::PaidApi,
            is_headless: true,
            input_tokens: 10,
            output_tokens: 5,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            input_cost: cost_usd,
            output_cost: 0.0,
            cache_read_cost: 0.0,
            cache_write_cost: 0.0,
            cost_usd,
            cache_savings_usd: 0.0,
            is_estimated: false,
            provenance: Default::default(),
            outcome: Default::default(),
        }
    }

    async fn native_started_worker(
        sm: &SessionManager,
        root: &Path,
    ) -> (Session, String, crate::goal_attempts::WorkerBinding) {
        let root = std::fs::canonicalize(root).unwrap();
        let coordinator = sm
            .create_session(
                root.to_path_buf(),
                "native test coordinator".to_string(),
                SessionType::User,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let task_id = sm
            .begin_budget_task_with_contract(
                &coordinator.id,
                &Message::user().with_text("Complete the native accounting fixture"),
            )
            .await
            .unwrap();
        let pool = sm.pool_clone().await.unwrap();
        let claim_id = format!("test-claim-{}", uuid::Uuid::now_v7());
        let card = crate::cards::create_card(
            &pool,
            crate::cards::CreateCard {
                project_id: crate::projects::PERSONAL_PROJECT_ID.to_string(),
                title: "native accounting fixture".to_string(),
                description: None,
                card_type: Some("goal".to_string()),
                column_id: None,
                created_by: None,
                metadata_json: Some(serde_json::json!({"dispatch_claim_id": claim_id})),
            },
        )
        .await
        .unwrap();
        let claim_id = card
            .metadata_json
            .get("dispatch_claim_id")
            .and_then(serde_json::Value::as_str)
            .unwrap()
            .to_string();
        let doing: String = sqlx::query_scalar(
            "SELECT id FROM board_columns WHERE project_id = ? AND state_binding = 'in_progress'",
        )
        .bind(&card.project_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("UPDATE cards SET column_id = ? WHERE id = ?")
            .bind(doing)
            .bind(&card.id)
            .execute(&pool)
            .await
            .unwrap();
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        crate::goal_attempts::bind_new_goal_tx(
            &mut tx,
            &card.id,
            &card.project_id,
            Some(&task_id),
            Some(&coordinator.id),
            None,
        )
        .await
        .unwrap();
        crate::goal_attempts::claim_tx(
            &mut tx,
            &card.id,
            &card.project_id,
            &crate::goal_attempts::ClaimInput {
                claim_id: claim_id.clone(),
                lifecycle_id: format!("lifecycle-{claim_id}"),
                worker_key: "test-worker".to_string(),
                execution_root: Some(root.to_string_lossy().into_owned()),
                input_head: None,
            },
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        let worker = sm
            .create_session_with_parent(
                Some(&coordinator.id),
                root.to_path_buf(),
                "native test worker".to_string(),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        assert_eq!(
            budget_task_id(&worker.extension_data).as_deref(),
            Some(task_id.as_str())
        );
        let mut data = worker.extension_data.clone();
        data.set_extension_state(
            crate::session::GOAL_ID_EXTENSION_NAME,
            crate::session::GOAL_ID_EXTENSION_VERSION,
            serde_json::json!(card.id),
        );
        sm.update(&worker.id)
            .extension_data(data)
            .apply()
            .await
            .unwrap();
        let worker = sm.get_session(&worker.id, false).await.unwrap();
        let permit = crate::goal_attempts::StartPermit {
            pool: pool.clone(),
            goal_id: card.id,
            claim_id: claim_id.clone(),
        };
        permit
            .prepare_worker(&worker.id, &root, Some(&task_id))
            .await
            .unwrap();
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        crate::goal_attempts::finalize_tx(
            &mut tx,
            &permit.goal_id,
            &claim_id,
            Some(&worker.id),
            None,
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        permit.start().await.unwrap();
        let binding = crate::goal_attempts::worker_binding(&pool, &worker.id)
            .await
            .unwrap()
            .unwrap();
        (worker, task_id, binding)
    }

    fn persisted_harness_snapshot(status: HarnessRunStatus) -> HarnessRunSnapshot {
        serde_json::from_value(serde_json::json!({
            "runId": "restart-run",
            "sessionId": "restart-session",
            "project": "permagent-runtime",
            "promptTitle": "Durable harness run",
            "promptDigest": "digest-123",
            "taskVersion": "dag-1/v1",
            "envelopeId": "envelope-1",
            "promptContext": "private prompt body must not be durable",
            "councilRecommendation": {
                "recommended": true,
                "reason": "architecture",
                "signals": ["architecture"]
            },
            "dagNodes": ["implement"],
            "dependencies": [],
            "activeNode": if status.is_active() { Some("implement") } else { None::<&str> },
            "worker": "worker-1",
            "provider": "local",
            "model": "test-model",
            "billingClass": "local",
            "tier": "harness",
            "routingReason": "test",
            "status": status,
            "declaredVerification": { "command": "cargo test", "verdict": null },
            "lastVerification": { "command": "cargo test", "verdict": "pass" },
            "verificationAttempts": 2,
            "verificationVerdict": "pass",
            "pendingGate": null,
            "retryCount": 1,
            "toolCalls": 3,
            "gateAttempts": 0,
            "evidence": "cargo test: pass",
            "result": "implemented",
            "parentRunId": null,
            "startedAt": "2026-09-04T12:00:00Z",
            "updatedAt": "2026-09-04T12:01:00Z",
            "elapsedMs": 60000
        }))
        .expect("valid harness snapshot fixture")
    }

    fn snapshot_as_update(snapshot: &HarnessRunSnapshot) -> HarnessRunUpdate {
        let mut value = serde_json::to_value(snapshot).expect("snapshot is serializable");
        let object = value.as_object_mut().expect("snapshot is an object");
        for field in [
            "councilRecommendation",
            "startedAt",
            "updatedAt",
            "elapsedMs",
        ] {
            object.remove(field);
        }
        serde_json::from_value(value).expect("snapshot fields match update contract")
    }

    #[tokio::test]
    async fn harness_snapshot_store_is_idempotent_private_and_restart_safe() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let terminal = persisted_harness_snapshot(HarnessRunStatus::Succeeded);

        let stored = sm.upsert_harness_run_snapshot(&terminal).await.unwrap();
        assert!(stored.prompt_context.is_none());
        // A retry is an upsert, not a duplicate history row.
        sm.upsert_harness_run_snapshot(&terminal).await.unwrap();
        let pool = sm.pool_clone().await.unwrap();
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM harness_run_snapshots WHERE run_id = 'restart-run'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 1);
        let raw: String = sqlx::query_scalar(
            "SELECT snapshot_json FROM harness_run_snapshots WHERE run_id = 'restart-run'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(!raw.contains("private prompt body"));
        assert!(raw.contains("cargo test: pass"));

        // A fresh manager is the daemon-restart boundary. The terminal row is
        // still queryable and an old active heartbeat cannot resurrect it.
        drop(sm);
        let restarted = SessionManager::new(temp_dir.path().to_path_buf());
        let history = restarted
            .list_harness_run_snapshots(true, 64)
            .await
            .unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].status, HarnessRunStatus::Succeeded);
        assert_eq!(history[0].evidence.as_deref(), Some("cargo test: pass"));
        run_registry::hydrate_harness_runs(history.clone());
        assert_eq!(
            run_registry::harness_run_snapshot("restart-run")
                .expect("restart hydration")
                .status,
            HarnessRunStatus::Succeeded
        );
        let stale = persisted_harness_snapshot(HarnessRunStatus::Running);
        let effective = restarted.upsert_harness_run_snapshot(&stale).await.unwrap();
        assert_eq!(effective.status, HarnessRunStatus::Succeeded);

        let mut rebound = terminal;
        rebound.session_id = "other-session".to_string();
        assert!(restarted
            .upsert_harness_run_snapshot(&rebound)
            .await
            .unwrap_err()
            .to_string()
            .contains("another session"));
        run_registry::remove_harness_run("restart-run");
    }

    #[tokio::test]
    async fn harness_snapshot_schema_apply_is_idempotent() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let pool = sm.pool_clone().await.unwrap();
        crate::session::spectral_schema::apply_harness_run_snapshots_schema(&pool)
            .await
            .unwrap();
        crate::session::spectral_schema::apply_harness_run_snapshots_schema(&pool)
            .await
            .unwrap();
        let table_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'harness_run_snapshots'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(table_count, 1);
    }

    #[tokio::test]
    async fn terminal_hydration_overrides_late_active_heartbeat() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let terminal = persisted_harness_snapshot(HarnessRunStatus::Succeeded);
        sm.upsert_harness_run_snapshot(&terminal).await.unwrap();

        // Simulate a daemon restart followed by a heartbeat that was already
        // in flight: the local registry sees it at `now`, later than the old
        // persisted terminal timestamp.
        run_registry::remove_harness_run("restart-run");
        let stale = persisted_harness_snapshot(HarnessRunStatus::Running);
        run_registry::update_harness_run(snapshot_as_update(&stale)).unwrap();
        assert!(run_registry::list_active_harness_runs()
            .iter()
            .any(|run| run.run_id == "restart-run"));

        let persisted = sm.list_harness_run_snapshots(false, 64).await.unwrap();
        run_registry::hydrate_harness_runs(persisted);
        assert_eq!(
            run_registry::harness_run_snapshot("restart-run")
                .expect("hydrated terminal")
                .status,
            HarnessRunStatus::Succeeded
        );
        assert!(!run_registry::list_active_harness_runs()
            .iter()
            .any(|run| run.run_id == "restart-run"));
        run_registry::remove_harness_run("restart-run");
    }

    #[test]
    fn persistable_content_json_coalesces_split_text() {
        let msg = Message::assistant().with_text("first").with_text(" answer");
        let json = persistable_content_json(&msg).unwrap();
        let content: Vec<MessageContent> = serde_json::from_str(&json).unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0].as_text().unwrap(), "first answer");
    }

    async fn run_lock_upgrade_attempt(
        pool: Pool<Sqlite>,
        session_id: String,
        begin_statement: &'static str,
        worker_id: i32,
        barrier: Option<Arc<tokio::sync::Barrier>>,
    ) -> anyhow::Result<()> {
        let mut tx = pool.begin_with(begin_statement).await?;

        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sessions WHERE id = ?")
            .bind(&session_id)
            .fetch_one(&mut *tx)
            .await?;

        if let Some(barrier) = barrier {
            barrier.wait().await;
        }

        sqlx::query("UPDATE sessions SET total_tokens = ? WHERE id = ?")
            .bind(worker_id)
            .bind(&session_id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(())
    }

    async fn run_lock_upgrade_race(
        pool: Pool<Sqlite>,
        session_id: String,
        begin_statement: &'static str,
        use_barrier: bool,
    ) -> Vec<anyhow::Result<()>> {
        let barrier = if use_barrier {
            Some(Arc::new(tokio::sync::Barrier::new(2)))
        } else {
            None
        };
        let mut handles = Vec::new();

        for worker_id in 0..2 {
            let pool = pool.clone();
            let session_id = session_id.clone();
            let barrier = barrier.clone();
            handles.push(tokio::spawn(async move {
                run_lock_upgrade_attempt(pool, session_id, begin_statement, worker_id, barrier)
                    .await
            }));
        }

        let mut results = Vec::new();
        for handle in handles {
            results.push(handle.await.expect("lock-upgrade task panicked"));
        }
        results
    }

    // ── X-verify-1: reviewer-mutation receipts ──────────────────────────────

    #[tokio::test]
    async fn reviewer_mutation_receipts_start_empty() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "author".to_string(),
                SessionType::User,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        assert!(reviewer_mutation_receipts(&session.extension_data).is_empty());
    }

    #[tokio::test]
    async fn record_reviewer_mutation_appends_and_persists() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "author".to_string(),
                SessionType::User,
                GooseMode::Auto,
            )
            .await
            .unwrap();

        sm.record_reviewer_mutation(
            &session.id,
            vec!["src/cache.rs".to_string()],
            Some("sub-1".to_string()),
        )
        .await
        .unwrap();
        sm.record_reviewer_mutation(&session.id, vec!["src/lib.rs".to_string()], None)
            .await
            .unwrap();

        let reloaded = sm.get_session(&session.id, false).await.unwrap();
        let receipts = reviewer_mutation_receipts(&reloaded.extension_data);
        assert_eq!(
            receipts.len(),
            2,
            "both receipts must be kept, not overwritten"
        );
        assert_eq!(receipts[0].paths, vec!["src/cache.rs".to_string()]);
        assert_eq!(receipts[0].subagent_session_id.as_deref(), Some("sub-1"));
        assert_eq!(receipts[1].paths, vec!["src/lib.rs".to_string()]);
        assert_eq!(receipts[1].subagent_session_id, None);
        assert!(!receipts[0].at.is_empty());
    }

    #[tokio::test]
    async fn test_begin_immediate_prevents_lock_upgrade_deadlock() {
        let temp_dir = TempDir::new().unwrap();
        let session_manager = SessionManager::new(temp_dir.path().to_path_buf());

        let session = session_manager
            .create_session(
                PathBuf::from("/tmp/lock-upgrade-test"),
                "Lock Upgrade Session".to_string(),
                SessionType::User,
                GooseMode::default(),
            )
            .await
            .unwrap();

        let pool = session_manager.storage().pool.clone();

        let results = run_lock_upgrade_race(pool.clone(), session.id.clone(), "BEGIN", true).await;
        assert!(
            results.iter().any(Result::is_err),
            "BEGIN (DEFERRED) should cause SQLITE_BUSY when two tasks try to upgrade SHARED → RESERVED"
        );

        let results = run_lock_upgrade_race(pool, session.id, "BEGIN IMMEDIATE", false).await;
        assert!(
            results.iter().all(Result::is_ok),
            "BEGIN IMMEDIATE should serialize contention without SQLITE_BUSY: {:?}",
            results
                .iter()
                .filter_map(|r| r.as_ref().err().map(ToString::to_string))
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn test_concurrent_session_creation() {
        let temp_dir = TempDir::new().unwrap();
        let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));

        let mut handles = vec![];

        for i in 0..NUM_CONCURRENT_SESSIONS {
            let sm = Arc::clone(&session_manager);
            let handle = tokio::spawn(async move {
                let working_dir = PathBuf::from(format!("/tmp/test_{}", i));
                let description = format!("Test session {}", i);

                let session = sm
                    .create_session(
                        working_dir.clone(),
                        description,
                        SessionType::User,
                        GooseMode::default(),
                    )
                    .await
                    .unwrap();

                sm.add_message(
                    &session.id,
                    &Message {
                        id: None,
                        role: Role::User,
                        created: chrono::Utc::now().timestamp_millis(),
                        content: vec![MessageContent::text("hello world")],
                        metadata: Default::default(),
                    },
                )
                .await
                .unwrap();

                sm.add_message(
                    &session.id,
                    &Message {
                        id: None,
                        role: Role::Assistant,
                        created: chrono::Utc::now().timestamp_millis(),
                        content: vec![MessageContent::text("sup world?")],
                        metadata: Default::default(),
                    },
                )
                .await
                .unwrap();

                sm.update(&session.id)
                    .user_provided_name(format!("Updated session {}", i))
                    .total_tokens(Some(100 * i))
                    .apply()
                    .await
                    .unwrap();

                let updated = sm.get_session(&session.id, true).await.unwrap();
                assert_eq!(updated.message_count, 2);
                assert_eq!(updated.total_tokens, Some(100 * i));

                session.id
            });
            handles.push(handle);
        }

        let mut results = vec![];
        for handle in handles {
            results.push(handle.await.unwrap());
        }

        assert_eq!(results.len(), NUM_CONCURRENT_SESSIONS as usize);

        let unique_ids: std::collections::HashSet<_> = results.iter().collect();
        assert_eq!(unique_ids.len(), NUM_CONCURRENT_SESSIONS as usize);

        let sessions = session_manager.list_sessions().await.unwrap();
        assert_eq!(sessions.len(), NUM_CONCURRENT_SESSIONS as usize);

        for session in &sessions {
            assert_eq!(session.message_count, 2);
            assert!(session.name.starts_with("Updated session"));
        }

        let insights = session_manager.get_insights().await.unwrap();
        assert_eq!(insights.total_sessions, NUM_CONCURRENT_SESSIONS as usize);
        let expected_tokens = 100 * NUM_CONCURRENT_SESSIONS * (NUM_CONCURRENT_SESSIONS - 1) / 2;
        assert_eq!(insights.total_tokens, expected_tokens as i64);
    }

    #[tokio::test]
    async fn test_export_import_roundtrip() {
        const DESCRIPTION: &str = "Original session";
        const TOTAL_TOKENS: i32 = 500;
        const INPUT_TOKENS: i32 = 300;
        const OUTPUT_TOKENS: i32 = 200;
        const ACCUMULATED_TOKENS: i32 = 1000;
        const USER_MESSAGE: &str = "test message";
        const ASSISTANT_MESSAGE: &str = "test response";

        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        let original = sm
            .create_session(
                PathBuf::from("/tmp/test"),
                DESCRIPTION.to_string(),
                SessionType::User,
                GooseMode::default(),
            )
            .await
            .unwrap();

        sm.update(&original.id)
            .total_tokens(Some(TOTAL_TOKENS))
            .input_tokens(Some(INPUT_TOKENS))
            .output_tokens(Some(OUTPUT_TOKENS))
            .accumulated_total_tokens(Some(ACCUMULATED_TOKENS))
            .apply()
            .await
            .unwrap();

        sm.add_message(
            &original.id,
            &Message {
                id: None,
                role: Role::User,
                created: chrono::Utc::now().timestamp_millis(),
                content: vec![MessageContent::text(USER_MESSAGE)],
                metadata: Default::default(),
            },
        )
        .await
        .unwrap();

        sm.add_message(
            &original.id,
            &Message {
                id: None,
                role: Role::Assistant,
                created: chrono::Utc::now().timestamp_millis(),
                content: vec![MessageContent::text(ASSISTANT_MESSAGE)],
                metadata: Default::default(),
            },
        )
        .await
        .unwrap();

        let exported = sm.export_session(&original.id).await.unwrap();
        let imported = sm.import_session(&exported, None).await.unwrap();

        assert_ne!(imported.id, original.id);
        assert_eq!(imported.name, DESCRIPTION);
        assert_eq!(imported.working_dir, PathBuf::from("/tmp/test"));
        assert_eq!(imported.total_tokens, Some(TOTAL_TOKENS));
        assert_eq!(imported.input_tokens, Some(INPUT_TOKENS));
        assert_eq!(imported.output_tokens, Some(OUTPUT_TOKENS));
        assert_eq!(imported.accumulated_total_tokens, Some(ACCUMULATED_TOKENS));
        assert_eq!(imported.message_count, 2);

        let conversation = imported.conversation.unwrap();
        assert_eq!(conversation.messages().len(), 2);
        assert_eq!(conversation.messages()[0].role, Role::User);
        assert_eq!(conversation.messages()[1].role, Role::Assistant);
    }

    #[tokio::test]
    async fn test_list_sessions_filters_by_type() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        let user_session = sm
            .create_session(
                PathBuf::from("/tmp/test"),
                "User session".to_string(),
                SessionType::User,
                GooseMode::default(),
            )
            .await
            .unwrap();

        sm.add_message(
            &user_session.id,
            &Message {
                id: None,
                role: Role::User,
                created: chrono::Utc::now().timestamp_millis(),
                content: vec![MessageContent::text("hello world")],
                metadata: Default::default(),
            },
        )
        .await
        .unwrap();

        let acp_session = sm
            .create_session(
                PathBuf::from("/tmp/test"),
                "ACP session".to_string(),
                SessionType::Acp,
                GooseMode::default(),
            )
            .await
            .unwrap();

        sm.add_message(
            &acp_session.id,
            &Message {
                id: None,
                role: Role::User,
                created: chrono::Utc::now().timestamp_millis(),
                content: vec![MessageContent::text("hello acp")],
                metadata: Default::default(),
            },
        )
        .await
        .unwrap();

        let default_sessions = sm.list_sessions().await.unwrap();
        assert_eq!(default_sessions.len(), 1);
        assert_eq!(default_sessions[0].name, "User session");

        let acp_sessions = sm
            .list_sessions_by_types(&[SessionType::Acp])
            .await
            .unwrap();
        assert_eq!(acp_sessions.len(), 1);
        assert_eq!(acp_sessions[0].name, "ACP session");
    }

    #[tokio::test]
    async fn test_list_sessions_by_schedule_id_filters_caps_and_orders() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        // Two sessions on schedule "sched-a" (one with a message), one on
        // "sched-b", one unscheduled — only sched-a's two should come back.
        let mut sched_a_ids = Vec::new();
        for i in 0..2 {
            let s = sm
                .create_session(
                    PathBuf::from("/tmp/test"),
                    format!("A run {i}"),
                    SessionType::Scheduled,
                    GooseMode::default(),
                )
                .await
                .unwrap();
            sm.update(&s.id)
                .schedule_id(Some("sched-a".to_string()))
                .apply()
                .await
                .unwrap();
            sched_a_ids.push(s.id);
        }

        // Give the first sched-a session one message to prove the correlated
        // message_count is computed per matched row.
        sm.add_message(
            &sched_a_ids[0],
            &Message {
                id: None,
                role: Role::User,
                created: chrono::Utc::now().timestamp_millis(),
                content: vec![MessageContent::text("hi")],
                metadata: Default::default(),
            },
        )
        .await
        .unwrap();

        let b = sm
            .create_session(
                PathBuf::from("/tmp/test"),
                "B run".to_string(),
                SessionType::Scheduled,
                GooseMode::default(),
            )
            .await
            .unwrap();
        sm.update(&b.id)
            .schedule_id(Some("sched-b".to_string()))
            .apply()
            .await
            .unwrap();

        sm.create_session(
            PathBuf::from("/tmp/test"),
            "Unscheduled".to_string(),
            SessionType::User,
            GooseMode::default(),
        )
        .await
        .unwrap();

        // Only sched-a rows, newest first.
        let rows = sm
            .list_sessions_by_schedule_id("sched-a", 10)
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows
            .iter()
            .all(|s| s.schedule_id.as_deref() == Some("sched-a")));
        assert!(rows[0].created_at >= rows[1].created_at);
        let counts: std::collections::HashMap<_, _> = rows
            .iter()
            .map(|s| (s.id.clone(), s.message_count))
            .collect();
        assert_eq!(counts[&sched_a_ids[0]], 1);
        assert_eq!(counts[&sched_a_ids[1]], 0);

        // LIMIT is pushed into SQL.
        let capped = sm.list_sessions_by_schedule_id("sched-a", 1).await.unwrap();
        assert_eq!(capped.len(), 1);

        // Unknown schedule → empty.
        let none = sm.list_sessions_by_schedule_id("nope", 10).await.unwrap();
        assert!(none.is_empty());
    }

    /// `idx_sessions_schedule_id` (migrate_v48_to_v49, 2026-08-25 "schedule
    /// polling storm" fix) must exist after a fresh init AND survive a
    /// second migration pass against an already-migrated DB — a daemon
    /// restart re-runs the whole `if version < N` ladder every boot (a
    /// fresh `SessionStorage` means a fresh `initialized` OnceCell), so
    /// every step in it must be safe to run more than once.
    #[tokio::test]
    async fn schedule_id_index_migration_is_idempotent() {
        async fn index_row_count(pool: &Pool<Sqlite>) -> i64 {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'idx_sessions_schedule_id'",
            )
            .fetch_one(pool)
            .await
            .unwrap()
        }

        let temp_dir = TempDir::new().unwrap();
        let db_dir = temp_dir.path().to_path_buf();

        // First "boot": fresh DB. `init_spectral_db` creates the index inline
        // (see spectral_schema.rs), so it must already be there.
        {
            let sm = SessionManager::new(db_dir.clone());
            let pool = sm
                .storage()
                .pool()
                .await
                .expect("fresh-DB init must succeed");
            assert_eq!(index_row_count(pool).await, 1);
        }

        // Second "boot" against the SAME on-disk DB: a brand-new
        // SessionStorage (a fresh `initialized` OnceCell) re-runs the entire
        // migration ladder — including `migrate_v48_to_v49`'s
        // `CREATE INDEX IF NOT EXISTS` — against a DB that already has
        // everything applied. Must not error, and must not duplicate the
        // index.
        let sm2 = SessionManager::new(db_dir);
        let pool2 = sm2.storage().pool().await.expect(
            "re-running the migration ladder against an already-migrated DB must be idempotent",
        );
        assert_eq!(index_row_count(pool2).await, 1);

        // And running the migration function itself twice, directly, back to
        // back — the narrowest form of the idempotency requirement.
        crate::session::spectral_schema::migrate_v48_to_v49(pool2)
            .await
            .expect("migrate_v48_to_v49 must be safe to run once more");
        crate::session::spectral_schema::migrate_v48_to_v49(pool2)
            .await
            .expect("migrate_v48_to_v49 must be safe to run twice in a row");
        assert_eq!(index_row_count(pool2).await, 1);
    }

    /// The query plan for `list_sessions_by_schedule_id`'s `WHERE
    /// s.schedule_id = ?` must use `idx_sessions_schedule_id`, not a full
    /// `sessions` table scan — the whole point of migrate_v48_to_v49 (the
    /// 2026-08-25 "schedule polling storm" fix: this exact predicate,
    /// unindexed, was the 1-3.7s slow-query source). Runs `EXPLAIN QUERY
    /// PLAN` against the SAME SQL string the production code executes
    /// (`LIST_SESSIONS_BY_SCHEDULE_ID_SQL`), so this cannot drift from what
    /// actually ships.
    #[tokio::test]
    async fn list_sessions_by_schedule_id_query_plan_uses_the_index() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let pool = sm.storage().pool().await.unwrap();

        // AUDIT (sqlx::AssertSqlSafe): both halves of this string are compile-time
        // constants — a literal prefix and `LIST_SESSIONS_BY_SCHEDULE_ID_SQL`, the
        // same `&'static str` the production path executes. No caller input reaches
        // it; the schedule id and limit are still bound as parameters below. The
        // format! exists only because `EXPLAIN QUERY PLAN` has to be prefixed onto
        // the real query for this test to prove anything about the real query.
        let plan_sql = format!("EXPLAIN QUERY PLAN {}", LIST_SESSIONS_BY_SCHEDULE_ID_SQL);
        let rows: Vec<(i64, i64, i64, String)> = sqlx::query_as(sqlx::AssertSqlSafe(plan_sql))
            .bind("some-schedule-id")
            .bind(5_i64)
            .fetch_all(pool)
            .await
            .unwrap();

        let plan_detail: String = rows
            .iter()
            .map(|(_, _, _, detail)| detail.as_str())
            .collect::<Vec<_>>()
            .join(" | ");
        assert!(
            plan_detail.contains("idx_sessions_schedule_id"),
            "expected the query plan to use idx_sessions_schedule_id, got: {plan_detail}"
        );
    }

    #[tokio::test]
    async fn test_list_session_summaries_lean_projection() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        let user_session = sm
            .create_session(
                PathBuf::from("/tmp/test"),
                "User session".to_string(),
                SessionType::User,
                GooseMode::default(),
            )
            .await
            .unwrap();

        sm.add_message(
            &user_session.id,
            &Message {
                id: None,
                role: Role::User,
                created: chrono::Utc::now().timestamp_millis(),
                content: vec![MessageContent::text("hello world")],
                metadata: Default::default(),
            },
        )
        .await
        .unwrap();

        // ACP sessions must be excluded by the User+Scheduled filter, same as list_sessions().
        sm.create_session(
            PathBuf::from("/tmp/test"),
            "ACP session".to_string(),
            SessionType::Acp,
            GooseMode::default(),
        )
        .await
        .unwrap();

        let summaries = sm.list_session_summaries().await.unwrap();
        assert_eq!(summaries.len(), 1);
        let s = &summaries[0];
        assert_eq!(s.id, user_session.id);
        assert_eq!(s.name, "User session");
        assert_eq!(s.session_type, SessionType::User);
        // message_count is computed by the lean query's subselect, not dropped.
        assert_eq!(s.message_count, 1);
    }

    /// History's "Agent runs" filter asks for SubAgent sessions explicitly; the
    /// default list must still leave them out. The summary also carries what
    /// the UI names a session by: the first user-visible user message, the
    /// working dir, the recipe title, and the goal a worker ran for.
    #[tokio::test]
    async fn test_list_session_summaries_of_types_and_naming_fields() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        let chat = sm
            .create_session(
                PathBuf::from("/tmp/chat"),
                "CLI Session".to_string(),
                SessionType::User,
                GooseMode::default(),
            )
            .await
            .unwrap();
        // A hidden (not user-visible) user message first: it must not be the
        // first_user_text.
        sm.add_message(
            &chat.id,
            &Message {
                id: None,
                role: Role::User,
                created: chrono::Utc::now().timestamp_millis(),
                content: vec![MessageContent::text("internal preamble")],
                metadata: crate::conversation::message::MessageMetadata {
                    user_visible: false,
                    agent_visible: true,
                },
            },
        )
        .await
        .unwrap();
        sm.add_message(
            &chat.id,
            &Message {
                id: None,
                role: Role::User,
                created: chrono::Utc::now().timestamp_millis(),
                content: vec![MessageContent::text("Rotate the staging keys")],
                metadata: Default::default(),
            },
        )
        .await
        .unwrap();

        let worker = sm
            .create_session(
                PathBuf::from("/tmp/repo"),
                "worker".to_string(),
                SessionType::SubAgent,
                GooseMode::default(),
            )
            .await
            .unwrap();
        let mut ext = ExtensionData::new();
        ext.set_extension_state(
            GOAL_ID_EXTENSION_NAME,
            GOAL_ID_EXTENSION_VERSION,
            serde_json::Value::String("card-that-does-not-exist".to_string()),
        );
        sm.update(&worker.id)
            .extension_data(ext)
            .apply()
            .await
            .unwrap();

        // Default: unchanged — User + Scheduled only.
        let default_ids: Vec<String> = sm
            .list_session_summaries()
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(default_ids, vec![chat.id.clone()]);

        // Opt-in: SubAgent too.
        let all = sm
            .list_session_summaries_of_types(&[
                SessionType::User,
                SessionType::Scheduled,
                SessionType::SubAgent,
            ])
            .await
            .unwrap();
        assert_eq!(all.len(), 2);
        let chat_row = all.iter().find(|s| s.id == chat.id).unwrap();
        assert_eq!(
            chat_row.first_user_text.as_deref(),
            Some("Rotate the staging keys")
        );
        assert_eq!(chat_row.working_dir.as_deref(), Some("/tmp/chat"));
        assert_eq!(chat_row.goal_id, None);
        let worker_row = all.iter().find(|s| s.id == worker.id).unwrap();
        assert_eq!(worker_row.session_type, SessionType::SubAgent);
        assert_eq!(
            worker_row.goal_id.as_deref(),
            Some("card-that-does-not-exist")
        );
        // A goal id whose card is gone: no title, no project — and no error.
        assert_eq!(worker_row.goal_title, None);
        assert_eq!(worker_row.project_id, None);

        // An empty type list is nothing, not everything.
        assert!(sm
            .list_session_summaries_of_types(&[])
            .await
            .unwrap()
            .is_empty());
    }

    /// Locks in that the GROUP BY/COALESCE rewrite of the message_count column
    /// (replacing the per-row correlated subquery) yields IDENTICAL counts —
    /// including sessions with zero messages, where the LEFT JOIN produces NULL
    /// that COALESCE must map back to 0. Covers both list query paths.
    #[tokio::test]
    async fn test_message_count_matches_across_list_paths() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        // Session with 0 messages.
        let empty = sm
            .create_session(
                PathBuf::from("/tmp/test"),
                "empty".to_string(),
                SessionType::User,
                GooseMode::default(),
            )
            .await
            .unwrap();

        // Session with 3 messages.
        let busy = sm
            .create_session(
                PathBuf::from("/tmp/test"),
                "busy".to_string(),
                SessionType::User,
                GooseMode::default(),
            )
            .await
            .unwrap();
        for _ in 0..3 {
            sm.add_message(
                &busy.id,
                &Message {
                    id: None,
                    role: Role::User,
                    created: chrono::Utc::now().timestamp_millis(),
                    content: vec![MessageContent::text("hi")],
                    metadata: Default::default(),
                },
            )
            .await
            .unwrap();
        }

        // Fat path (list_sessions_by_types via list_sessions).
        let sessions = sm.list_sessions().await.unwrap();
        let by_id: std::collections::HashMap<_, _> = sessions
            .iter()
            .map(|s| (s.id.clone(), s.message_count))
            .collect();
        assert_eq!(by_id.get(&empty.id), Some(&0));
        assert_eq!(by_id.get(&busy.id), Some(&3));

        // Lean path (list_session_summaries).
        let summaries = sm.list_session_summaries().await.unwrap();
        let lean_by_id: std::collections::HashMap<_, _> = summaries
            .iter()
            .map(|s| (s.id.clone(), s.message_count))
            .collect();
        assert_eq!(lean_by_id.get(&empty.id), Some(&0));
        assert_eq!(lean_by_id.get(&busy.id), Some(&3));
    }

    #[tokio::test]
    async fn test_import_session_with_description_field() {
        const OLD_FORMAT_JSON: &str = r#"{
            "id": "20240101_1",
            "description": "Old format session",
            "user_set_name": true,
            "working_dir": "/tmp/test",
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
            "extension_data": {},
            "message_count": 0
        }"#;

        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        let imported = sm.import_session(OLD_FORMAT_JSON, None).await.unwrap();

        assert_eq!(imported.name, "Old format session");
        assert!(imported.user_set_name);
        assert_eq!(imported.working_dir, PathBuf::from("/tmp/test"));
    }

    #[test_case(GooseMode::Approve)]
    #[test_case(GooseMode::SmartApprove)]
    #[test_case(GooseMode::Chat)]
    #[tokio::test]
    async fn test_goose_mode_persists(mode: GooseMode) {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        let session = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "test".into(),
                SessionType::User,
                mode,
            )
            .await
            .unwrap();

        let reloaded = sm.get_session(&session.id, false).await.unwrap();
        assert_eq!(reloaded.goose_mode, mode);
    }

    /// Concurrent turns on one session must not lose each other's tokens.
    ///
    /// The old path read `accumulated_*`, added in Rust, and wrote an absolute
    /// value — so two turns that both read N each wrote N+their own delta, and
    /// the second commit discarded the first's usage. This drives the spend
    /// caps, so undercounting is not cosmetic.
    ///
    /// Interleaved deliberately: BOTH updates are built from the same observed
    /// starting state before either applies, which is exactly the race. With
    /// the DB folding the addition, the result is the sum regardless of order.
    #[tokio::test]
    async fn concurrent_token_accumulation_does_not_lose_updates() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "tokens".into(),
                SessionType::User,
                GooseMode::default(),
            )
            .await
            .unwrap();

        // Both "turns" observe the same (empty) starting state first.
        let before = sm.get_session(&session.id, false).await.unwrap();
        assert_eq!(before.accumulated_total_tokens.unwrap_or(0), 0);

        sm.update(&session.id)
            .accumulate_tokens(100, 60, 40)
            .apply()
            .await
            .unwrap();
        sm.update(&session.id)
            .accumulate_tokens(30, 20, 10)
            .apply()
            .await
            .unwrap();

        let after = sm.get_session(&session.id, false).await.unwrap();
        assert_eq!(
            after.accumulated_total_tokens,
            Some(130),
            "the second write must ADD to the first, not replace it"
        );
        assert_eq!(after.accumulated_input_tokens, Some(80));
        assert_eq!(after.accumulated_output_tokens, Some(50));
    }

    /// NULL columns must start from zero, not stay NULL — a fresh session has
    /// no accumulated value and `NULL + 5` is NULL in SQL.
    #[tokio::test]
    async fn accumulating_onto_null_starts_from_zero() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "null-start".into(),
                SessionType::User,
                GooseMode::default(),
            )
            .await
            .unwrap();

        sm.update(&session.id)
            .accumulate_tokens(7, 5, 2)
            .apply()
            .await
            .unwrap();

        let after = sm.get_session(&session.id, false).await.unwrap();
        assert_eq!(after.accumulated_total_tokens, Some(7));
    }

    /// Compaction still needs to RESET the counters, so the absolute setters
    /// must keep working alongside the new delta path.
    #[tokio::test]
    async fn absolute_setters_still_replace() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "absolute".into(),
                SessionType::User,
                GooseMode::default(),
            )
            .await
            .unwrap();

        sm.update(&session.id)
            .accumulate_tokens(100, 60, 40)
            .apply()
            .await
            .unwrap();
        sm.update(&session.id)
            .accumulated_total_tokens(Some(5))
            .apply()
            .await
            .unwrap();

        let after = sm.get_session(&session.id, false).await.unwrap();
        assert_eq!(
            after.accumulated_total_tokens,
            Some(5),
            "absolute must replace"
        );
    }

    #[tokio::test]
    async fn test_goose_mode_update() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        let session = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "test".into(),
                SessionType::User,
                GooseMode::default(),
            )
            .await
            .unwrap();

        sm.update(&session.id)
            .goose_mode(GooseMode::Approve)
            .apply()
            .await
            .unwrap();

        let reloaded = sm.get_session(&session.id, false).await.unwrap();
        assert_eq!(reloaded.goose_mode, GooseMode::Approve);
    }

    #[tokio::test]
    async fn test_goose_mode_malformed_defaults_to_auto() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        let session = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "test".into(),
                SessionType::User,
                GooseMode::Approve,
            )
            .await
            .unwrap();

        let pool = &sm.storage().pool;
        sqlx::query("UPDATE sessions SET goose_mode = 'garbage' WHERE id = ?")
            .bind(&session.id)
            .execute(pool)
            .await
            .unwrap();

        let reloaded = sm.get_session(&session.id, false).await.unwrap();
        assert_eq!(reloaded.goose_mode, GooseMode::default());
    }

    #[tokio::test]
    async fn test_spectral_schema_creates_all_tables() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());

        // Trigger schema init
        let pool = sm.storage().pool().await.unwrap();

        // Verify all Spectral tables exist
        let tables: Vec<String> =
            sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
                .fetch_all(pool)
                .await
                .unwrap();

        // NB: `memories` and `knowledge_graph` are intentionally absent — they were
        // a dormant dead copy of the Spectral schema (the live Brain lives in a
        // separate brain/memory.db) and are no longer created by init_spectral_db
        // (dropped by migrate_v18_to_v19).
        let expected = vec![
            "cost_reservations",
            "cost_ledger",
            "integrations",
            "messages",
            "provider_inventory_entries",
            "provider_inventory_models",
            "schema_version",
            "sessions",
            "skill_executions",
            "skill_triggers",
            "skills",
            "tasks",
            "thread_messages",
            "threads",
            "users",
        ];
        for table in &expected {
            assert!(
                tables.contains(&table.to_string()),
                "Missing table: {}",
                table
            );
        }

        // Verify default user exists
        let user: (String, String) =
            sqlx::query_as("SELECT id, display_name FROM users WHERE id = 'default'")
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(user.0, "default");
        assert_eq!(user.1, "Default User");
    }

    #[tokio::test]
    async fn existing_current_schema_repairs_missing_cost_reservations_on_restart() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path().to_path_buf();
        let sm = SessionManager::new(data_dir.clone());
        let pool = sm.storage().pool().await.unwrap();
        sqlx::query("DROP TABLE cost_reservations")
            .execute(pool)
            .await
            .unwrap();
        drop(sm);

        // The database is already stamped at the current schema version, so a
        // numbered migration cannot repair it. The every-boot additive apply
        // must restore the authorization table before any paid call runs.
        let reopened = SessionManager::new(data_dir);
        let pool = reopened.storage().pool().await.unwrap();
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master \
             WHERE type = 'table' AND name = 'cost_reservations'",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_cost_ledger_append_and_rollup() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "cost".to_string(),
                SessionType::User,
                GooseMode::default(),
            )
            .await
            .unwrap();
        let sid = session.id.clone();

        // A chargeable (paid_api) call: cost_usd is the folded four-component total.
        let chargeable = CostLedgerRow {
            call_id: "call-1".to_string(),
            ts: "2026-07-14T00:00:00Z".to_string(),
            session_id: sid.clone(),
            parent_session_id: None,
            task_id: Some("task-1".to_string()),
            goal_id: None,
            goal_attempt_id: None,
            subagent_id: None,
            provider: Some("anthropic".to_string()),
            model: Some("claude-3.5-sonnet".to_string()),
            cost_tier: CostTier::PaidApi,
            is_headless: false,
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 200,
            cache_write_tokens: 300,
            input_cost: 0.0015,
            output_cost: 0.0075,
            cache_read_cost: 0.000_06,
            cache_write_cost: 0.001_125,
            cost_usd: 0.010_185,
            cache_savings_usd: 0.000_54,
            is_estimated: false,
            provenance: Default::default(),
            outcome: Default::default(),
        };
        sm.append_cost_ledger(&chargeable).await.unwrap();

        // A non-chargeable (local_free) call: cost_usd MUST be 0.
        let local = CostLedgerRow {
            call_id: "call-2".to_string(),
            ts: "2026-07-14T00:00:01Z".to_string(),
            session_id: sid.clone(),
            parent_session_id: None,
            task_id: None,
            goal_id: None,
            goal_attempt_id: None,
            subagent_id: None,
            provider: Some("ollama".to_string()),
            model: Some("llama3".to_string()),
            cost_tier: CostTier::LocalFree,
            is_headless: true,
            input_tokens: 400,
            output_tokens: 200,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            input_cost: 0.0,
            output_cost: 0.0,
            cache_read_cost: 0.0,
            cache_write_cost: 0.0,
            cost_usd: 0.0,
            cache_savings_usd: 0.0,
            is_estimated: false,
            provenance: Default::default(),
            outcome: Default::default(),
        };
        sm.append_cost_ledger(&local).await.unwrap();

        let pool = sm.pool_clone().await.unwrap();

        // Row 1 persisted with attribution keys + chargeable flag; turn_index=0.
        let (tier, chargeable_flag, task, turn, cost): (String, i64, Option<String>, i64, f64) =
            sqlx::query_as(
                "SELECT cost_tier, is_chargeable, task_id, turn_index, cost_usd \
                 FROM cost_ledger WHERE call_id = ?",
            )
            .bind("call-1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(tier, "paid_api");
        assert_eq!(chargeable_flag, 1);
        assert_eq!(task.as_deref(), Some("task-1"));
        assert_eq!(turn, 0, "first row for the session is turn 0");
        assert!((cost - 0.010_185).abs() < 1e-12);

        // Local row: not chargeable, cost 0, turn_index=1.
        let (tier2, chargeable2, cost2, turn2): (String, i64, f64, i64) = sqlx::query_as(
            "SELECT cost_tier, is_chargeable, cost_usd, turn_index \
             FROM cost_ledger WHERE call_id = ?",
        )
        .bind("call-2")
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(tier2, "local_free");
        assert_eq!(chargeable2, 0);
        assert_eq!(cost2, 0.0);
        assert_eq!(turn2, 1);

        // Rollup invariant: session.accumulated_cost_usd == SUM(cost_ledger.cost_usd).
        let sum: f64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(cost_usd), 0) FROM cost_ledger WHERE session_id = ?",
        )
        .bind(&sid)
        .fetch_one(&pool)
        .await
        .unwrap();
        let reread = sm.get_session(&sid, false).await.unwrap();
        let acc = reread.accumulated_cost_usd.expect("rollup populated");
        assert!((acc - sum).abs() < 1e-12, "rollup {acc} != SUM {sum}");
        assert!((acc - 0.010_185).abs() < 1e-12);
        // Last-turn cost tracks the most recent append (the local $0 call).
        assert_eq!(reread.cost_usd, Some(0.0));
        // Cache token accumulators + the visible savings accumulator.
        assert_eq!(reread.accumulated_cache_read_tokens, Some(200));
        assert_eq!(reread.accumulated_cache_write_tokens, Some(300));
        assert!((reread.accumulated_cache_savings_usd.unwrap() - 0.000_54).abs() < 1e-12);
    }

    #[tokio::test]
    async fn usage_rollup_is_idempotent_across_concurrency_and_restart() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "exact usage".to_string(),
                SessionType::User,
                GooseMode::default(),
            )
            .await
            .unwrap();
        let row = CostLedgerRow {
            call_id: "invocation-1".to_string(),
            ts: "2026-07-14T00:00:00Z".to_string(),
            session_id: session.id.clone(),
            parent_session_id: None,
            task_id: None,
            goal_id: None,
            goal_attempt_id: None,
            subagent_id: None,
            provider: Some("local".to_string()),
            model: Some("test-model".to_string()),
            cost_tier: CostTier::LocalFree,
            is_headless: false,
            input_tokens: 10,
            output_tokens: 5,
            cache_read_tokens: 2,
            cache_write_tokens: 1,
            input_cost: 0.0,
            output_cost: 0.0,
            cache_read_cost: 0.0,
            cache_write_cost: 0.0,
            cost_usd: 0.0,
            cache_savings_usd: 0.0,
            is_estimated: false,
            provenance: Default::default(),
            outcome: Default::default(),
        };

        // If a legacy caller already persisted the ledger row, the combined
        // path must not apply its token side effects on replay.
        let mut preexisting = row.clone();
        preexisting.call_id = "already-recorded".to_string();
        sm.append_cost_ledger(&preexisting).await.unwrap();
        assert!(!sm
            .append_usage_and_rollup(&preexisting, None, Some(15), Some(10), Some(5), 15, 10, 5,)
            .await
            .unwrap());
        let before_race = sm.get_session(&session.id, false).await.unwrap();
        assert_eq!(before_race.accumulated_total_tokens, None);

        // Two callbacks for one invocation race each other. SQLite's existing
        // BEGIN IMMEDIATE serializes them; the conflict path must not apply the
        // token update a second time.
        let (first, second) = tokio::join!(
            sm.append_usage_and_rollup(&row, None, Some(15), Some(10), Some(5), 15, 10, 5),
            sm.append_usage_and_rollup(&row, None, Some(15), Some(10), Some(5), 15, 10, 5),
        );
        assert_eq!(first.unwrap() as u8 + second.unwrap() as u8, 1);

        let after = sm.get_session(&session.id, false).await.unwrap();
        assert_eq!(after.accumulated_total_tokens, Some(15));
        assert_eq!(after.accumulated_input_tokens, Some(10));
        assert_eq!(after.accumulated_output_tokens, Some(5));
        let pool = sm.pool_clone().await.unwrap();
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM cost_ledger WHERE call_id = 'invocation-1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 1);

        // Re-opening the manager is the daemon restart boundary. Replaying the
        // same stable key remains a no-op rather than charging the invocation.
        drop(sm);
        let resumed = SessionManager::new(temp_dir.path().to_path_buf());
        assert!(!resumed
            .append_usage_and_rollup(&row, None, Some(15), Some(10), Some(5), 15, 10, 5)
            .await
            .unwrap());
        let after_restart = resumed.get_session(&session.id, false).await.unwrap();
        assert_eq!(after_restart.accumulated_total_tokens, Some(15));
        assert_eq!(after_restart.accumulated_cost_usd, Some(0.0));
    }

    #[tokio::test]
    async fn budget_task_contract_tracks_task_boundary_and_survives_history_replacement() {
        let temp = TempDir::new().unwrap();
        let sm = SessionManager::new(temp.path().to_path_buf());
        let session = sm
            .create_session(
                temp.path().to_path_buf(),
                "contract".to_string(),
                SessionType::User,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let original = Message::user().with_text("viewport_position: above_fold|below_fold");
        let first = sm
            .begin_budget_task_with_contract(&session.id, &original)
            .await
            .unwrap();
        sm.replace_conversation(&session.id, &Conversation::default())
            .await
            .unwrap();
        drop(sm);
        let sm = SessionManager::new(temp.path().to_path_buf());
        let loaded = sm.get_session(&session.id, false).await.unwrap();
        assert_eq!(budget_task_id(&loaded.extension_data), Some(first));
        assert_eq!(
            budget_task_contract(&loaded.extension_data).unwrap()["text"],
            original.as_concat_text()
        );
        sm.begin_budget_task_with_contract(
            &session.id,
            &Message::user().with_text("second unrelated task"),
        )
        .await
        .unwrap();
        let second = sm.get_session(&session.id, false).await.unwrap();
        assert_eq!(
            budget_task_contract(&second.extension_data).unwrap()["text"],
            "second unrelated task"
        );
        // No impact to ordinary large-message chat; planning sees explicit marker.
        sm.begin_budget_task_with_contract(
            &session.id,
            &Message::user().with_text("x".repeat(MAX_BUDGET_TASK_CONTRACT_BYTES + 1)),
        )
        .await
        .unwrap();
        let oversized = sm.get_session(&session.id, false).await.unwrap();
        let contract = budget_task_contract(&oversized.extension_data).unwrap();
        assert!(contract.get("text").is_none());
        assert_eq!(
            contract["oversized_bytes"],
            MAX_BUDGET_TASK_CONTRACT_BYTES + 1
        );
        sm.begin_budget_task(&session.id).await.unwrap();
        let legacy = sm.get_session(&session.id, false).await.unwrap();
        assert!(budget_task_contract(&legacy.extension_data).is_none());
        // A stale record cannot cross a changed task even if another writer left it.
        let mut stale = second.extension_data;
        stale.set_extension_state(
            BUDGET_TASK_EXTENSION_NAME,
            BUDGET_TASK_EXTENSION_VERSION,
            serde_json::json!("new-task"),
        );
        assert!(budget_task_contract(&stale).is_none());
    }

    #[tokio::test]
    async fn budget_task_identity_survives_resume_and_history_replacement() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "budget identity".to_string(),
                SessionType::User,
                GooseMode::default(),
            )
            .await
            .unwrap();

        let first_task = sm.begin_budget_task(&session.id).await.unwrap();
        let row = |call_id: &str, task_id: &str, cost_usd: f64| CostLedgerRow {
            call_id: call_id.to_string(),
            ts: format!("2026-07-14T00:00:{call_id}Z"),
            session_id: session.id.clone(),
            parent_session_id: None,
            task_id: Some(task_id.to_string()),
            goal_id: None,
            goal_attempt_id: None,
            subagent_id: None,
            provider: Some("test".to_string()),
            model: Some("test-model".to_string()),
            cost_tier: CostTier::PaidApi,
            is_headless: false,
            input_tokens: 1,
            output_tokens: 1,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            input_cost: cost_usd,
            output_cost: 0.0,
            cache_read_cost: 0.0,
            cache_write_cost: 0.0,
            cost_usd,
            cache_savings_usd: 0.0,
            is_estimated: false,
            provenance: Default::default(),
            outcome: Default::default(),
        };
        sm.append_cost_ledger(&row("01", &first_task, 1.0))
            .await
            .unwrap();

        // A new manager is the resume/daemon-restart boundary. The identity is
        // still present, and replacing conversation history (compaction/retry)
        // must not change it.
        drop(sm);
        let resumed = SessionManager::new(temp_dir.path().to_path_buf());
        let loaded = resumed.get_session(&session.id, false).await.unwrap();
        assert_eq!(
            budget_task_id(&loaded.extension_data),
            Some(first_task.clone())
        );
        resumed
            .replace_conversation(&session.id, &Conversation::default())
            .await
            .unwrap();
        resumed
            .append_cost_ledger(&row("02", &first_task, 0.5))
            .await
            .unwrap();

        let pool = resumed.pool_clone().await.unwrap();
        let continued_spend =
            crate::agents::platform_extensions::orchestrator::spend_snapshot(&pool, &session.id)
                .await
                .unwrap()
                .task_spent_usd;
        assert!((continued_spend - 1.5).abs() < 1e-12);

        // A genuine next reply rotates the durable identity; its spend is
        // isolated from the previous task even though the session is shared.
        let next_task = resumed.begin_budget_task(&session.id).await.unwrap();
        assert_ne!(next_task, first_task);
        resumed
            .append_cost_ledger(&row("03", &next_task, 0.25))
            .await
            .unwrap();
        let next_spend =
            crate::agents::platform_extensions::orchestrator::spend_snapshot(&pool, &session.id)
                .await
                .unwrap()
                .task_spent_usd;
        assert!((next_spend - 0.25).abs() < 1e-12);
    }

    #[tokio::test]
    async fn child_session_inherits_and_contributes_to_parent_budget_task() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let parent = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "parent".to_string(),
                SessionType::User,
                GooseMode::default(),
            )
            .await
            .unwrap();
        let task_id = sm.begin_budget_task(&parent.id).await.unwrap();
        let mut parent_extensions = sm
            .get_session(&parent.id, false)
            .await
            .unwrap()
            .extension_data;
        parent_extensions.set_extension_state("unrelated", "v1", serde_json::json!("parent-only"));
        sm.update(&parent.id)
            .extension_data(parent_extensions)
            .apply()
            .await
            .unwrap();

        let unknown_parent = sm
            .create_session_with_parent(
                Some("missing-parent"),
                temp_dir.path().to_path_buf(),
                "invalid child".to_string(),
                SessionType::SubAgent,
                GooseMode::default(),
            )
            .await
            .unwrap_err();
        assert!(unknown_parent.to_string().contains("parent session"));

        let child = sm
            .create_session_with_parent(
                Some(&parent.id),
                temp_dir.path().to_path_buf(),
                "child".to_string(),
                SessionType::SubAgent,
                GooseMode::default(),
            )
            .await
            .unwrap();
        assert_eq!(budget_task_id(&child.extension_data), Some(task_id.clone()));
        assert!(child
            .extension_data
            .get_extension_state("unrelated", "v1")
            .is_none());

        let row = |call_id: &str, session_id: &str, cost_usd: f64| CostLedgerRow {
            call_id: call_id.to_string(),
            ts: format!("2026-07-14T00:00:{call_id}Z"),
            session_id: session_id.to_string(),
            parent_session_id: Some(parent.id.clone()),
            task_id: Some(task_id.clone()),
            goal_id: None,
            goal_attempt_id: None,
            subagent_id: Some(child.id.clone()),
            provider: Some("test".to_string()),
            model: Some("test-model".to_string()),
            cost_tier: CostTier::PaidApi,
            is_headless: true,
            input_tokens: 1,
            output_tokens: 1,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            input_cost: cost_usd,
            output_cost: 0.0,
            cache_read_cost: 0.0,
            cache_write_cost: 0.0,
            cost_usd,
            cache_savings_usd: 0.0,
            is_estimated: false,
            provenance: Default::default(),
            outcome: Default::default(),
        };
        sm.append_cost_ledger(&row("11", &parent.id, 1.0))
            .await
            .unwrap();
        sm.append_cost_ledger(&row("12", &child.id, 0.5))
            .await
            .unwrap();

        let pool = sm.pool_clone().await.unwrap();
        let spend =
            crate::agents::platform_extensions::orchestrator::spend_snapshot(&pool, &parent.id)
                .await
                .unwrap()
                .task_spent_usd;
        assert!((spend - 1.5).abs() < 1e-12);
    }

    /// `sessions.parent_session_id` (v51) must exist after a fresh init AND
    /// survive re-running the migration / apply guard against an already-
    /// migrated DB — same idempotency posture as migrate_v48_to_v49.
    #[tokio::test]
    async fn parent_session_id_migration_is_idempotent() {
        async fn column_present(pool: &Pool<Sqlite>) -> bool {
            let n: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM pragma_table_info('sessions') WHERE name = 'parent_session_id'",
            )
            .fetch_one(pool)
            .await
            .unwrap();
            n == 1
        }

        let temp_dir = TempDir::new().unwrap();
        let db_dir = temp_dir.path().to_path_buf();

        {
            let sm = SessionManager::new(db_dir.clone());
            let pool = sm.storage().pool().await.expect("fresh-DB init");
            assert!(column_present(pool).await);
        }

        let sm2 = SessionManager::new(db_dir);
        let pool2 = sm2
            .storage()
            .pool()
            .await
            .expect("re-running the migration ladder must be idempotent");
        assert!(column_present(pool2).await);

        crate::session::spectral_schema::migrate_v50_to_v51(pool2)
            .await
            .expect("migrate_v50_to_v51 once more");
        crate::session::spectral_schema::migrate_v50_to_v51(pool2)
            .await
            .expect("migrate_v50_to_v51 twice in a row");
        crate::session::spectral_schema::apply_session_parent_schema(pool2)
            .await
            .expect("apply_session_parent_schema once more");
        assert!(column_present(pool2).await);
    }

    /// A SubAgent created via `create_session_with_parent` (the summon /
    /// goal_engine spawn path) persists the parent id on the session row.
    #[tokio::test]
    async fn create_session_with_parent_records_parent() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let parent = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "parent".to_string(),
                SessionType::User,
                GooseMode::default(),
            )
            .await
            .unwrap();

        let child = sm
            .create_session_with_parent(
                Some(&parent.id),
                temp_dir.path().to_path_buf(),
                "Delegated task".to_string(),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .unwrap();

        assert_eq!(child.parent_session_id.as_deref(), Some(parent.id.as_str()));
        let reread = sm.get_session(&child.id, false).await.unwrap();
        assert_eq!(
            reread.parent_session_id.as_deref(),
            Some(parent.id.as_str())
        );
        assert_eq!(reread.session_type, SessionType::SubAgent);
    }

    /// Ledger rows carry the session's parent_session_id, and
    /// `cost_by_parent_session` sums own + each child's spend.
    #[tokio::test]
    async fn cost_by_parent_session_rolls_up_children() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let parent = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "parent".to_string(),
                SessionType::User,
                GooseMode::default(),
            )
            .await
            .unwrap();
        let child_a = sm
            .create_session_with_parent(
                Some(&parent.id),
                temp_dir.path().to_path_buf(),
                "child-a".to_string(),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let child_b = sm
            .create_session_with_parent(
                Some(&parent.id),
                temp_dir.path().to_path_buf(),
                "child-b".to_string(),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .unwrap();

        let mk =
            |call_id: &str, session_id: &str, parent_id: Option<&str>, cost: f64| CostLedgerRow {
                call_id: call_id.to_string(),
                ts: "2026-08-25T00:00:00Z".to_string(),
                session_id: session_id.to_string(),
                parent_session_id: parent_id.map(|s| s.to_string()),
                task_id: None,
                goal_id: None,
                goal_attempt_id: None,
                subagent_id: None,
                provider: Some("anthropic".to_string()),
                model: Some("claude-sonnet-4".to_string()),
                cost_tier: CostTier::PaidApi,
                is_headless: false,
                input_tokens: 10,
                output_tokens: 5,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                input_cost: cost,
                output_cost: 0.0,
                cache_read_cost: 0.0,
                cache_write_cost: 0.0,
                cost_usd: cost,
                cache_savings_usd: 0.0,
                is_estimated: false,
                provenance: Default::default(),
                outcome: Default::default(),
            };

        sm.append_cost_ledger(&mk("p1", &parent.id, None, 0.40))
            .await
            .unwrap();
        sm.append_cost_ledger(&mk("a1", &child_a.id, Some(&parent.id), 0.10))
            .await
            .unwrap();
        sm.append_cost_ledger(&mk("b1", &child_b.id, Some(&parent.id), 0.07))
            .await
            .unwrap();

        let pool = sm.storage().pool().await.unwrap();
        let stored_parent: Option<String> =
            sqlx::query_scalar("SELECT parent_session_id FROM cost_ledger WHERE call_id = 'a1'")
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(stored_parent.as_deref(), Some(parent.id.as_str()));

        let rollup = sm.cost_by_parent_session(&parent.id).await.unwrap();
        assert!((rollup.own - 0.40).abs() < 1e-12);
        assert!((rollup.children_total - 0.17).abs() < 1e-12);
        assert_eq!(rollup.per_child.len(), 2);
        assert!((rollup.total() - 0.57).abs() < 1e-12);
        let by_id: std::collections::HashMap<_, _> = rollup
            .per_child
            .iter()
            .map(|c| (c.session_id.as_str(), c.cost_usd))
            .collect();
        assert!((by_id[child_a.id.as_str()] - 0.10).abs() < 1e-12);
        assert!((by_id[child_b.id.as_str()] - 0.07).abs() < 1e-12);
    }

    #[tokio::test]
    async fn extend_reservation_checks_ceiling_and_settles_the_enlarged_hold() {
        use crate::cost_router::plan_reservation_bound;
        let model = ModelConfig::new("glm-4.7")
            .unwrap()
            .with_context_limit(Some(204_800))
            .with_max_tokens(Some(131_072));
        let bound = plan_reservation_bound("zai", "glm-4.7", CostTier::PaidApi, &model, 2)
            .unwrap()
            .unwrap();
        let one = bound.amount_usd / 2.0;
        let ceiling = bound.amount_usd + one + one;
        for hard in [false, true] {
            let temp = TempDir::new().unwrap();
            let sm = SessionManager::new(temp.path().to_path_buf());
            let session = sm
                .create_session(
                    temp.path().to_path_buf(),
                    "extension".into(),
                    SessionType::SubAgent,
                    GooseMode::Auto,
                )
                .await
                .unwrap();
            let task = sm.begin_budget_task(&session.id).await.unwrap();
            let cfg = reservation_config(10.0, 20.0, ceiling, if hard { ceiling } else { 20.0 });
            let granted = sm
                .reserve_bounded_provider_invocation(
                    "extend",
                    &session.id,
                    Some(&task),
                    bound,
                    "2099-01-01T00:00:00Z",
                    &cfg,
                )
                .await
                .unwrap();
            assert!(matches!(granted, CostReservationOutcome::Granted { .. }));
            // Evidence survives restart; concurrent extensions cannot overdraw.
            let sm = SessionManager::new(temp.path().to_path_buf());
            let (a, b) = tokio::join!(
                sm.extend_reservation("extend", 1),
                sm.extend_reservation("extend", 1)
            );
            let outcomes = [a.unwrap(), b.unwrap()];
            assert_eq!(
                outcomes
                    .iter()
                    .filter(|o| matches!(o, CostReservationOutcome::Granted { .. }))
                    .count(),
                1
            );
            assert!(outcomes.iter().any(|o| if hard {
                matches!(o, CostReservationOutcome::Refused { .. })
            } else {
                matches!(o, CostReservationOutcome::NeedsGate { .. })
            }));
            let pool = sm.storage().pool().await.unwrap();
            let (amount, json): (f64, String) = sqlx::query_as(
                "SELECT amount_usd, extension_json FROM cost_reservations WHERE invocation_id = 'extend'",
            )
            .fetch_one(pool)
            .await
            .unwrap();
            assert_eq!(amount, bound.amount_usd + one);
            let evidence: ReservationExtension = serde_json::from_str(&json).unwrap();
            assert_eq!(evidence.attempts, 3);
            assert_eq!(evidence.config, cfg);
            let row = reservation_row(&session.id, &task, "extend", 0.25);
            assert!(sm
                .settle_provider_invocation(
                    "invocation:extend",
                    &row,
                    None,
                    None,
                    None,
                    None,
                    15,
                    10,
                    5,
                )
                .await
                .unwrap());
            let held: f64 = sqlx::query_scalar(
                "SELECT CAST(COALESCE(SUM(amount_usd), 0) AS REAL) FROM cost_reservations
                 WHERE state IN ('pending', 'unknown')",
            )
            .fetch_one(pool)
            .await
            .unwrap();
            assert_eq!(held, 0.0);
            let spent: f64 = sqlx::query_scalar("SELECT SUM(cost_usd) FROM cost_ledger")
                .fetch_one(pool)
                .await
                .unwrap();
            assert_eq!(spent, 0.25);
            assert!(sm.extend_reservation("extend", 1).await.is_err());
        }
    }

    #[tokio::test]
    async fn reservation_is_idempotent_across_concurrency_restart_and_settlement() {
        let temp_dir = TempDir::new().unwrap();
        let db_dir = temp_dir.path().to_path_buf();
        let sm = SessionManager::new(db_dir.clone());
        let session = sm
            .create_session(
                db_dir.clone(),
                "reservation".to_string(),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let task_id = sm.begin_budget_task(&session.id).await.unwrap();
        let cfg = reservation_config(10.0, 20.0, 10.0, 20.0);

        let (first, second) = tokio::join!(
            sm.reserve_provider_invocation(
                "provider-call-1",
                &session.id,
                Some(&task_id),
                0.50,
                "2099-01-01T00:00:00Z",
                &cfg
            ),
            sm.reserve_provider_invocation(
                "provider-call-1",
                &session.id,
                Some(&task_id),
                0.50,
                "2099-01-01T00:00:00Z",
                &cfg
            )
        );
        let outcomes = [first.unwrap(), second.unwrap()];
        let reservation_id = outcomes
            .iter()
            .map(|outcome| match outcome {
                CostReservationOutcome::Granted { reservation_id }
                | CostReservationOutcome::AlreadyReserved { reservation_id } => {
                    reservation_id.clone()
                }
                other => panic!("unexpected reservation outcome: {other:?}"),
            })
            .next()
            .unwrap();
        assert!(outcomes
            .iter()
            .any(|outcome| matches!(outcome, CostReservationOutcome::Granted { .. })));
        assert!(outcomes
            .iter()
            .any(|outcome| matches!(outcome, CostReservationOutcome::AlreadyReserved { .. })));

        let row = reservation_row(&session.id, &task_id, "provider-call-1", 0.25);
        assert!(sm
            .settle_provider_invocation(
                &reservation_id,
                &row,
                Some("schedule-1".to_string()),
                Some(15),
                Some(10),
                Some(5),
                15,
                10,
                5,
            )
            .await
            .unwrap());
        assert!(!sm
            .settle_provider_invocation(&reservation_id, &row, None, None, None, None, 15, 10, 5,)
            .await
            .unwrap());
        let released_id = match sm
            .reserve_provider_invocation(
                "provider-call-release",
                &session.id,
                Some(&task_id),
                0.50,
                "2099-01-01T00:00:00Z",
                &cfg,
            )
            .await
            .unwrap()
        {
            CostReservationOutcome::Granted { reservation_id } => reservation_id,
            outcome => panic!("unexpected release reservation outcome: {outcome:?}"),
        };
        assert!(sm.release_provider_invocation(&released_id).await.unwrap());
        assert!(!sm.release_provider_invocation(&released_id).await.unwrap());
        assert!(matches!(
            sm.reserve_provider_invocation(
                "provider-call-release",
                &session.id,
                Some(&task_id),
                0.50,
                "2099-01-01T00:00:00Z",
                &cfg,
            )
            .await
            .unwrap(),
            CostReservationOutcome::Unknown { .. }
        ));

        let unknown_id = match sm
            .reserve_provider_invocation(
                "provider-call-unknown",
                &session.id,
                Some(&task_id),
                0.50,
                "2099-01-01T00:00:00Z",
                &cfg,
            )
            .await
            .unwrap()
        {
            CostReservationOutcome::Granted { reservation_id } => reservation_id,
            outcome => panic!("unexpected unknown reservation outcome: {outcome:?}"),
        };
        assert!(sm
            .mark_provider_invocation_unknown(&unknown_id)
            .await
            .unwrap());
        assert!(!sm
            .mark_provider_invocation_unknown(&unknown_id)
            .await
            .unwrap());
        assert!(matches!(
            sm.reserve_provider_invocation(
                "provider-call-unknown",
                &session.id,
                Some(&task_id),
                0.50,
                "2099-01-01T00:00:00Z",
                &cfg,
            )
            .await
            .unwrap(),
            CostReservationOutcome::Unknown { .. }
        ));
        drop(sm);

        let restarted = SessionManager::new(db_dir);
        assert!(matches!(
            restarted
                .reserve_provider_invocation(
                    "provider-call-1",
                    &session.id,
                    Some(&task_id),
                    0.50,
                    "2099-01-01T00:00:00Z",
                    &cfg
                )
                .await
                .unwrap(),
            CostReservationOutcome::AlreadySettled { .. }
        ));
        let persisted = restarted.get_session(&session.id, false).await.unwrap();
        assert_eq!(persisted.accumulated_total_tokens, Some(15));
        assert_eq!(persisted.accumulated_cost_usd, Some(0.25));
    }

    // ── F1b: grant consumption (harness loop, Lane F1 2026-09-10, from
    // F10/G8 forensics) ──

    /// A grant minted for a session's own failed call is consumable by a
    /// retry from that SAME session, shortly after — well within the TTL.
    #[tokio::test]
    async fn consume_granted_reservation_succeeds_for_the_same_session_within_ttl() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "grant-consume".to_string(),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let task_id = sm.begin_budget_task(&session.id).await.unwrap();

        let granted = sm
            .grant_reservation_after_gate_approval(
                "invocation-1",
                &session.id,
                &task_id,
                0.30,
                "2099-01-01T00:00:00Z",
                "zai",
                "glm-4.7",
            )
            .await
            .unwrap();
        assert!(matches!(granted, CostReservationOutcome::Granted { .. }));

        let consumed = sm
            .consume_granted_reservation(
                "invocation-1",
                &session.id,
                "zai",
                "glm-4.7",
                120,
                chrono::Utc::now(),
            )
            .await
            .unwrap();
        assert!(
            matches!(
                consumed,
                Some(CostReservationOutcome::AlreadyReserved { .. })
            ),
            "the same session's retry must consume the grant: {consumed:?}"
        );
    }

    /// A DIFFERENT session can never consume another session's grant — the
    /// reservation stays exactly as granted, untouched.
    #[tokio::test]
    async fn consume_granted_reservation_refuses_a_different_session() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let granting_session = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "grant-owner".to_string(),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let other_session = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "not-the-owner".to_string(),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let task_id = sm.begin_budget_task(&granting_session.id).await.unwrap();

        sm.grant_reservation_after_gate_approval(
            "invocation-2",
            &granting_session.id,
            &task_id,
            0.30,
            "2099-01-01T00:00:00Z",
            "zai",
            "glm-4.7",
        )
        .await
        .unwrap();

        let consumed = sm
            .consume_granted_reservation(
                "invocation-2",
                &other_session.id,
                "zai",
                "glm-4.7",
                120,
                chrono::Utc::now(),
            )
            .await
            .unwrap();
        assert_eq!(
            consumed, None,
            "a different session must never be able to consume another session's grant"
        );

        // The grant itself is untouched and still consumable by its own
        // session — a refused cross-session attempt must not burn it.
        let consumed_by_owner = sm
            .consume_granted_reservation(
                "invocation-2",
                &granting_session.id,
                "zai",
                "glm-4.7",
                120,
                chrono::Utc::now(),
            )
            .await
            .unwrap();
        assert!(matches!(
            consumed_by_owner,
            Some(CostReservationOutcome::AlreadyReserved { .. })
        ));
    }

    /// A grant left unconsumed past its TTL expires: `consume_granted_
    /// reservation` reports `None` (raise a fresh gate decision) and the
    /// still-`pending` row is released rather than held open until its full
    /// 2-hour lease — it must never silently authorize a retry that shows up
    /// long after the auto-answer that minted it.
    #[tokio::test]
    async fn consume_granted_reservation_expires_past_ttl_and_releases_the_hold() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "grant-expire".to_string(),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let task_id = sm.begin_budget_task(&session.id).await.unwrap();

        let granted = sm
            .grant_reservation_after_gate_approval(
                "invocation-3",
                &session.id,
                &task_id,
                0.30,
                "2099-01-01T00:00:00Z",
                "zai",
                "glm-4.7",
            )
            .await
            .unwrap();
        let reservation_id = match granted {
            CostReservationOutcome::Granted { reservation_id } => reservation_id,
            other => panic!("unexpected grant outcome: {other:?}"),
        };

        // 200s after the grant, well past the 120s TTL.
        let past_ttl = chrono::Utc::now() + chrono::Duration::seconds(200);
        let consumed = sm
            .consume_granted_reservation(
                "invocation-3",
                &session.id,
                "zai",
                "glm-4.7",
                120,
                past_ttl,
            )
            .await
            .unwrap();
        assert_eq!(
            consumed, None,
            "a grant unconsumed past its TTL must expire, not silently authorize the retry"
        );

        let pool = sm.pool_clone().await.unwrap();
        let state: String =
            sqlx::query_scalar("SELECT state FROM cost_reservations WHERE reservation_id = ?")
                .bind(&reservation_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            state, "released",
            "an expired grant must be released, never left pending forever"
        );

        // A later consume attempt (even within what would have been the
        // TTL from now) finds nothing left to consume — the caller's next
        // gate hit is a genuinely fresh decision, not a replay of the stale
        // grant.
        let consumed_again = sm
            .consume_granted_reservation(
                "invocation-3",
                &session.id,
                "zai",
                "glm-4.7",
                120,
                past_ttl,
            )
            .await
            .unwrap();
        assert_eq!(consumed_again, None);
    }

    /// H1 pin (harness loop, Round F, Lane F3, from Lane G9's HIGH finding):
    /// a grant minted for one provider/model is invisible to
    /// `find_pending_gate_grant` for a different provider/model — the
    /// coordinator/routing machinery's own provider switch between the
    /// gated call and the retry must never silently reuse the OLD call's
    /// dollar bound under the NEW provider. The SAME provider/model still
    /// finds and consumes it normally.
    #[tokio::test]
    async fn find_pending_gate_grant_requires_an_exact_provider_model_match() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "grant-provider-mismatch".to_string(),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let task_id = sm.begin_budget_task(&session.id).await.unwrap();

        sm.grant_reservation_after_gate_approval(
            "invocation-4",
            &session.id,
            &task_id,
            0.30,
            "2099-01-01T00:00:00Z",
            "zai",
            "glm-4.7",
        )
        .await
        .unwrap();

        // A different provider (or model) on the next call finds nothing —
        // this is the "provider switched mid-session" scenario.
        let mismatched_provider = sm
            .find_pending_gate_grant(&session.id, "anthropic", "glm-4.7")
            .await
            .unwrap();
        assert_eq!(
            mismatched_provider, None,
            "a grant minted for zai/glm-4.7 must not be reusable by a different provider"
        );
        let mismatched_model = sm
            .find_pending_gate_grant(&session.id, "zai", "glm-5.3")
            .await
            .unwrap();
        assert_eq!(
            mismatched_model, None,
            "a grant minted for zai/glm-4.7 must not be reusable by a different model"
        );
        let mismatched_consume = sm
            .consume_granted_reservation(
                "invocation-4",
                &session.id,
                "anthropic",
                "glm-4.7",
                120,
                chrono::Utc::now(),
            )
            .await
            .unwrap();
        assert_eq!(
            mismatched_consume, None,
            "consume must also refuse a provider/model mismatch, defense-in-depth with lookup"
        );

        // The SAME provider/model still finds and consumes it.
        let matched = sm
            .find_pending_gate_grant(&session.id, "zai", "glm-4.7")
            .await
            .unwrap();
        assert_eq!(matched, Some("invocation-4".to_string()));
        let consumed = sm
            .consume_granted_reservation(
                "invocation-4",
                &session.id,
                "zai",
                "glm-4.7",
                120,
                chrono::Utc::now(),
            )
            .await
            .unwrap();
        assert!(
            matches!(
                consumed,
                Some(CostReservationOutcome::AlreadyReserved { .. })
            ),
            "the exact same provider/model must still be able to reuse the grant: {consumed:?}"
        );
    }

    /// M1 pin (harness loop, Round F, Lane F3, from Lane G9's MEDIUM
    /// finding): a grant authorizes exactly ONE retry of the specific call
    /// that got gated. A second attempt to consume the SAME grant — even
    /// from the same session, same provider/model, well within the TTL —
    /// must be refused, not silently handed out again.
    #[tokio::test]
    async fn consume_granted_reservation_refuses_a_second_reuse_of_the_same_grant() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "grant-reuse-once".to_string(),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let task_id = sm.begin_budget_task(&session.id).await.unwrap();

        sm.grant_reservation_after_gate_approval(
            "invocation-5",
            &session.id,
            &task_id,
            0.30,
            "2099-01-01T00:00:00Z",
            "zai",
            "glm-4.7",
        )
        .await
        .unwrap();

        let first = sm
            .consume_granted_reservation(
                "invocation-5",
                &session.id,
                "zai",
                "glm-4.7",
                120,
                chrono::Utc::now(),
            )
            .await
            .unwrap();
        assert!(
            matches!(first, Some(CostReservationOutcome::AlreadyReserved { .. })),
            "the first consume must succeed: {first:?}"
        );

        let second = sm
            .consume_granted_reservation(
                "invocation-5",
                &session.id,
                "zai",
                "glm-4.7",
                120,
                chrono::Utc::now(),
            )
            .await
            .unwrap();
        assert_eq!(
            second, None,
            "a second attempt to reuse the same grant must be refused"
        );

        let pool = sm.pool_clone().await.unwrap();
        let (reuse_count, reused_by): (i64, Option<String>) = sqlx::query_as(
            "SELECT reuse_count, reused_by_invocation_id FROM cost_reservations
             WHERE invocation_id = 'invocation-5'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            reuse_count, 1,
            "reuse_count must record exactly one consumption"
        );
        assert_eq!(reused_by.as_deref(), Some("invocation-5"));

        // The underlying reservation itself is untouched by the refused
        // second attempt — still pending, not released or corrupted.
        let state: String = sqlx::query_scalar(
            "SELECT state FROM cost_reservations WHERE invocation_id = 'invocation-5'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(state, "pending");
    }

    /// F3b pin (harness loop, from Lane G10's MEDIUM finding): the TTL
    /// self-release must still apply to a grant that was already consumed
    /// once. A grant with `reuse_count > 0` that is ALSO past its TTL is
    /// released (and reported as `None`), not refused-but-left-`pending` —
    /// otherwise the fast 120s recovery path could only ever fire on a
    /// grant's first consume attempt, and an abandoned consumed grant would
    /// keep counting toward held budget until the ~8h lease-expiry sweep.
    #[tokio::test]
    async fn consume_granted_reservation_releases_a_consumed_grant_past_ttl() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "grant-reuse-then-expire".to_string(),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let task_id = sm.begin_budget_task(&session.id).await.unwrap();

        sm.grant_reservation_after_gate_approval(
            "invocation-6",
            &session.id,
            &task_id,
            0.30,
            "2099-01-01T00:00:00Z",
            "zai",
            "glm-4.7",
        )
        .await
        .unwrap();

        let first = sm
            .consume_granted_reservation(
                "invocation-6",
                &session.id,
                "zai",
                "glm-4.7",
                120,
                chrono::Utc::now(),
            )
            .await
            .unwrap();
        assert!(
            matches!(first, Some(CostReservationOutcome::AlreadyReserved { .. })),
            "the first consume must succeed: {first:?}"
        );

        // 200s after the grant, well past the 120s TTL: the retry that
        // consumed it never settled and never released the row.
        let past_ttl = chrono::Utc::now() + chrono::Duration::seconds(200);
        let stale = sm
            .consume_granted_reservation(
                "invocation-6",
                &session.id,
                "zai",
                "glm-4.7",
                120,
                past_ttl,
            )
            .await
            .unwrap();
        assert_eq!(
            stale, None,
            "an already-consumed grant past its TTL must report nothing to consume"
        );

        let pool = sm.pool_clone().await.unwrap();
        let state: String = sqlx::query_scalar(
            "SELECT state FROM cost_reservations WHERE invocation_id = 'invocation-6'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            state, "released",
            "the TTL check must run before the reuse refusal, so a stale consumed grant \
             releases its hold instead of sitting pending until the lease-expiry sweep"
        );

        // The refusal never double-counts: the row records the single
        // consumption it actually had.
        let reuse_count: i64 = sqlx::query_scalar(
            "SELECT reuse_count FROM cost_reservations WHERE invocation_id = 'invocation-6'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(reuse_count, 1);
    }

    #[tokio::test]
    async fn reservation_scope_is_root_lineage_and_sibling_holds_block() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let parent = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "parent".to_string(),
                SessionType::User,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let task_id = sm.begin_budget_task(&parent.id).await.unwrap();
        let child_a = sm
            .create_session_with_parent(
                Some(&parent.id),
                temp_dir.path().to_path_buf(),
                "child-a".to_string(),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let child_b = sm
            .create_session_with_parent(
                Some(&parent.id),
                temp_dir.path().to_path_buf(),
                "child-b".to_string(),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let cfg = reservation_config(10.0, 20.0, 3.0, 3.0);
        assert!(matches!(
            sm.reserve_provider_invocation(
                "sibling-a",
                &child_a.id,
                Some(&task_id),
                2.0,
                "2099-01-01T00:00:00Z",
                &cfg
            )
            .await
            .unwrap(),
            CostReservationOutcome::Granted { .. }
        ));
        assert!(matches!(
            sm.reserve_provider_invocation(
                "sibling-b",
                &child_b.id,
                Some(&task_id),
                2.0,
                "2099-01-01T00:00:00Z",
                &cfg
            )
            .await
            .unwrap(),
            CostReservationOutcome::Refused {
                scope: crate::cost_router::budget::BudgetScope::Session,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn reservation_requires_task_and_expiry_retains_unknown_on_next_call() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "reservation identity".to_string(),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let task_id = sm.begin_budget_task(&session.id).await.unwrap();
        let cfg = reservation_config(10.0, 20.0, 10.0, 20.0);
        assert!(matches!(
            sm.reserve_provider_invocation(
                "spoofed-task",
                &session.id,
                Some("task-spoofed"),
                1.0,
                "2099-01-01T00:00:00Z",
                &cfg,
            )
            .await
            .unwrap(),
            CostReservationOutcome::Unknown { .. }
        ));
        assert!(matches!(
            sm.reserve_provider_invocation(
                "missing-task",
                &session.id,
                None,
                1.0,
                "2099-01-01T00:00:00Z",
                &cfg
            )
            .await
            .unwrap(),
            CostReservationOutcome::Unknown { .. }
        ));
        assert!(matches!(
            sm.reserve_provider_invocation(
                "expired-call",
                &session.id,
                Some(&task_id),
                1.0,
                "2099-01-01T00:00:00Z",
                &cfg
            )
            .await
            .unwrap(),
            CostReservationOutcome::Granted { .. }
        ));
        let pool = sm.pool_clone().await.unwrap();
        sqlx::query(
            "UPDATE cost_reservations SET lease_until = '2000-01-01T00:00:00Z'
             WHERE invocation_id = 'expired-call'",
        )
        .execute(&pool)
        .await
        .unwrap();
        // A fresh invocation for the same task is not unconditionally blocked
        // by a stale unknown hold; authorization is decided by headroom
        // arithmetic that already counts unknown holds.
        assert!(matches!(
            sm.reserve_provider_invocation(
                "after-expiry",
                &session.id,
                Some(&task_id),
                1.0,
                "2099-01-01T00:00:00Z",
                &cfg
            )
            .await
            .unwrap(),
            CostReservationOutcome::Granted { .. }
        ));
        let expired_state: String = sqlx::query_scalar(
            "SELECT state FROM cost_reservations WHERE invocation_id = 'expired-call'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            expired_state, "unknown",
            "a stale hold without durable usage evidence remains unknown"
        );
    }

    /// B6.1: transient failures may retry only within the configured physical
    /// envelope. Pre-dispatch failures release their distinct reservations;
    /// the terminal post-dispatch attempt remains unknown.
    #[tokio::test]
    #[serial_test::serial(provider_wait_status)]
    async fn b6_bounded_retry_storm_has_distinct_attempts_and_terminal_unknown() {
        use crate::providers::errors::ProviderError;
        use crate::providers::{retry_operation, RetryConfig};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let temp_dir = TempDir::new().unwrap();
        let sm = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let session = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "retry storm".to_string(),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let task_id = sm.begin_budget_task(&session.id).await.unwrap();
        let attempts = Arc::new(AtomicUsize::new(0));
        let config = RetryConfig::new(2, 0, 1.0, 0).with_rate_limit_floor_ms(0);
        let retry_result: Result<(), ProviderError> = retry_operation(&config, || {
            let sm = Arc::clone(&sm);
            let session_id = session.id.clone();
            let task_id = task_id.clone();
            let attempts = Arc::clone(&attempts);
            async move {
                let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                let invocation_id = format!("b6-retry-{attempt}");
                let reservation_id = match sm
                    .reserve_provider_invocation(
                        &invocation_id,
                        &session_id,
                        Some(&task_id),
                        0.10,
                        "2099-01-01T00:00:00Z",
                        &reservation_config(1.0, 10.0, 1.0, 10.0),
                    )
                    .await
                    .map_err(|error| ProviderError::ExecutionError(error.to_string()))?
                {
                    CostReservationOutcome::Granted { reservation_id } => reservation_id,
                    outcome => {
                        return Err(ProviderError::ExecutionError(format!(
                            "retry attempt was not authorized: {outcome:?}"
                        )))
                    }
                };

                // The first two failures are proven pre-dispatch failures and
                // may release their holds. The terminal attempt crosses the
                // boundary and must remain unknown.
                if attempt < 2 {
                    sm.release_provider_invocation(&reservation_id)
                        .await
                        .map_err(|error| ProviderError::ExecutionError(error.to_string()))?;
                } else {
                    sm.mark_provider_invocation_unknown(&reservation_id)
                        .await
                        .map_err(|error| ProviderError::ExecutionError(error.to_string()))?;
                }
                Err(ProviderError::ServerError(
                    "bounded transient fixture".into(),
                ))
            }
        })
        .await;

        assert!(retry_result.is_err());
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
        let pool = sm.pool_clone().await.unwrap();
        let states: Vec<String> = sqlx::query_scalar(
            "SELECT state FROM cost_reservations WHERE task_id = ? ORDER BY invocation_id",
        )
        .bind(&task_id)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(states, vec!["released", "released", "unknown"]);
        let distinct_invocations: i64 = sqlx::query_scalar(
            "SELECT COUNT(DISTINCT invocation_id) FROM cost_reservations WHERE task_id = ?",
        )
        .bind(&task_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(distinct_invocations, 3);
        let ledger_rows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM cost_ledger WHERE task_id = ?")
                .bind(&task_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            ledger_rows, 0,
            "transient failures must not fabricate spend"
        );
    }

    /// B6.3 (updated by B7, Lane Y1 2026-09-09 — see
    /// `reconcile_stale_reservations`): a daemon death after authorization
    /// must still refuse to replay the SAME physical invocation once its
    /// hold is stale — that identity already reached a terminal outcome
    /// (`released`, once reconciled) and re-authorizing it would be a second,
    /// unbudgeted dispatch under one invocation id. But a DIFFERENT physical
    /// invocation under the same task must NOT stay fail-closed forever just
    /// because an earlier, unrelated call was left ambiguous: on the next
    /// reservation attempt the stale hold is reconciled (settled to its
    /// ledger cost if one exists, else released — see
    /// `reconcile_stale_reservations`) and the fresh invocation is then
    /// authorized normally against the now-accurate budget.
    #[tokio::test]
    async fn b6_restart_retains_unknown_hold_and_allows_fresh_work_within_headroom() {
        let temp_dir = TempDir::new().unwrap();
        let db_dir = temp_dir.path().to_path_buf();
        let sm = SessionManager::new(db_dir.clone());
        let session = sm
            .create_session(
                db_dir.clone(),
                "restart reconciliation".to_string(),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let task_id = sm.begin_budget_task(&session.id).await.unwrap();
        let cfg = reservation_config(0.90, 1.00, 0.90, 1.00);

        assert!(matches!(
            sm.reserve_provider_invocation(
                "b6-restart-call",
                &session.id,
                Some(&task_id),
                0.75,
                "2099-01-01T00:00:00Z",
                &cfg,
            )
            .await
            .unwrap(),
            CostReservationOutcome::Granted { .. }
        ));
        drop(sm);

        // Reopening is the daemon restart boundary. The durable task identity
        // must survive before any reservation reconciliation is attempted.
        let reopened = SessionManager::new(db_dir);
        let resumed = reopened.get_session(&session.id, false).await.unwrap();
        assert_eq!(
            budget_task_id(&resumed.extension_data),
            Some(task_id.clone())
        );

        let pool = reopened.pool_clone().await.unwrap();
        sqlx::query(
            "UPDATE cost_reservations
             SET lease_until = '2000-01-01T00:00:00Z'
             WHERE invocation_id = 'b6-restart-call'",
        )
        .execute(&pool)
        .await
        .unwrap();

        // Replaying the SAME durable invocation id is still refused: that
        // invocation already has an unresolved hold and must never come back
        // as a fresh `Granted`.
        assert!(matches!(
            reopened
                .reserve_provider_invocation(
                    "b6-restart-call",
                    &session.id,
                    Some(&task_id),
                    0.75,
                    "2099-01-01T00:00:00Z",
                    &cfg,
                )
                .await
                .unwrap(),
            CostReservationOutcome::Unknown { .. }
        ));

        // A DIFFERENT physical invocation now succeeds because unknown holds
        // consume headroom but no longer trigger an unconditional block.
        assert!(matches!(
            reopened
                .reserve_provider_invocation(
                    "b6-restart-fresh",
                    &session.id,
                    Some(&task_id),
                    0.10,
                    "2099-01-01T00:00:00Z",
                    &cfg,
                )
                .await
                .unwrap(),
            CostReservationOutcome::Granted { .. }
        ));

        let (state, amount): (String, f64) = sqlx::query_as(
            "SELECT state, amount_usd FROM cost_reservations
             WHERE invocation_id = 'b6-restart-call'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            state, "unknown",
            "a stale hold with no ledger evidence remains unknown"
        );
        assert!((amount - 0.75).abs() < 1e-12);
        let fresh_state: String = sqlx::query_scalar(
            "SELECT state FROM cost_reservations WHERE invocation_id = 'b6-restart-fresh'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(fresh_state, "pending");
        let reservation_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cost_reservations")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            reservation_count, 2,
            "one unknown hold for the stale invocation and one pending row for the fresh grant — replay of the same id must not create a second row"
        );

        let projection = reopened.budget_projection(&session.id, cfg).await.unwrap();
        // `held_usd` and `unknown_usd` are SEPARATE sums (budget_projection.rs:
        // "pending" adds to held, "unknown" adds to unknown) — they are not
        // added together. The retained stale hold is $0.75 of unknown; the
        // fresh grant is $0.10 of held. Remaining is the ceiling less BOTH,
        // which is what keeps a retained hold honest without freezing work.
        assert!((projection.task.unknown_usd.unwrap() - 0.75).abs() < 1e-12);
        assert!((projection.task.held_usd.unwrap() - 0.10).abs() < 1e-12);
        assert!((projection.task.remaining_usd.unwrap() - 0.15).abs() < 1e-12);
        assert!(matches!(
            reopened
                .reserve_provider_invocation(
                    "b6-over-headroom",
                    &session.id,
                    Some(&task_id),
                    0.20,
                    "2099-01-01T00:00:00Z",
                    &cfg,
                )
                .await
                .unwrap(),
            CostReservationOutcome::Refused { .. }
        ));

        // B5.5 producer contract: the real restart projection is the shared
        // JSON consumed by downstream event/store/UI tests. Only identities
        // and wall-clock provenance are normalized; every other field must
        // match the checked-in contract exactly.
        let mut actual = serde_json::to_value(projection).unwrap();
        actual["taskId"] = serde_json::json!("<taskId>");
        actual["rootSessionId"] = serde_json::json!("<rootSessionId>");
        actual["provenance"]["asOf"] = serde_json::json!("<asOf>");
        let expected: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../scripts/testdata/budget_projection_v1.json"
        ))
        .unwrap();
        assert_eq!(actual, expected, "B5.5 budget projection contract drifted");
    }

    /// B7 (Lane Y1, 2026-09-09 harness evidence, P0 defect #1): a stale
    /// `unknown` hold with a matching `cost_ledger` row — proof the provider
    /// call really was accounted — must be SETTLED to that recorded cost on
    /// the next reservation attempt for the same task, never silently
    /// released to zero. The settled amount keeps counting toward the task
    /// ceiling exactly like any other settled call, so a fresh reservation
    /// that would push the total over budget is still correctly refused —
    /// that refusal is the load-bearing proof the recorded spend was
    /// unblocked, not dropped.
    #[tokio::test]
    async fn b7_stale_unknown_hold_with_recorded_ledger_spend_settles_not_drops() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "stale hold with recorded spend".to_string(),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let task_id = sm.begin_budget_task(&session.id).await.unwrap();

        // A loose cap for the setup grant only — the ceiling that matters is
        // enforced on the fresh call below.
        let loose = reservation_config(5.0, 5.0, 5.0, 5.0);
        let reservation_id = match sm
            .reserve_provider_invocation(
                "b7-recorded-call",
                &session.id,
                Some(&task_id),
                0.95,
                "2099-01-01T00:00:00Z",
                &loose,
            )
            .await
            .unwrap()
        {
            CostReservationOutcome::Granted { reservation_id } => reservation_id,
            other => panic!("setup reservation was not granted: {other:?}"),
        };

        // Fail-closed path: the provider call's outcome could not be
        // durably confirmed through the normal settle path, so the hold is
        // marked unknown...
        assert!(sm
            .mark_provider_invocation_unknown(&reservation_id)
            .await
            .unwrap());
        // ...but real spend WAS in fact recorded for it independently (the
        // durable evidence `reconcile_stale_reservations` looks for).
        sm.append_cost_ledger(&reservation_row(
            &session.id,
            &task_id,
            "b7-recorded-call",
            0.95,
        ))
        .await
        .unwrap();

        // A tight cap: 0.95 already-recorded spend plus a 0.10 fresh request
        // exceeds the 1.00 hard ceiling. If reconciliation dropped the
        // recorded spend instead of settling it, this fresh call would
        // wrongly succeed.
        let tight = reservation_config(0.90, 1.00, 0.90, 1.00);
        assert!(matches!(
            sm.reserve_provider_invocation(
                "b7-fresh-call",
                &session.id,
                Some(&task_id),
                0.10,
                "2099-01-01T00:00:00Z",
                &tight,
            )
            .await
            .unwrap(),
            CostReservationOutcome::Refused { .. }
        ));

        let pool = sm.pool_clone().await.unwrap();
        let (state, settled_cost): (String, Option<f64>) = sqlx::query_as(
            "SELECT state, settled_cost_usd FROM cost_reservations
             WHERE invocation_id = 'b7-recorded-call'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            state, "settled",
            "a hold with a matching ledger row must be settled, not released"
        );
        assert!((settled_cost.unwrap() - 0.95).abs() < 1e-12);

        let ledger_rows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM cost_ledger WHERE task_id = ?")
                .bind(&task_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(ledger_rows, 1, "the recorded spend must not be duplicated");
    }

    /// B7 (Lane Y1, 2026-09-09, P0 defect #1 part (c)): `dispatch_goal_fn`
    /// dispatches a goal's worker under the SAME durable task identity as
    /// the coordinator that created the card (`dispatch_budget_task_id`,
    /// `orchestrator.rs`) — not necessarily the same session lineage. In the
    /// harness evidence, a stale `unknown` hold left by the coordinator's
    /// own internal completion (the pattern behind
    /// `handle_decompose_roadmap`'s planner call,
    /// `orchestrator.rs::run_decomposition`) then refused the WORKER's very
    /// first reservation too — card 2's own `last_error` carried the exact
    /// same `"an expired provider reservation is unresolved"` string,
    /// proving the refusal reached `dispatch_goal_fn` itself. Reconciliation
    /// runs at the start of every `reserve_provider_invocation` call for the
    /// task, regardless of which session makes it, so the worker's
    /// dispatch reservation now succeeds.
    #[tokio::test]
    async fn b7_dispatch_under_shared_task_succeeds_after_coordinator_hold_is_stale() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let coordinator = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "coordinator".to_string(),
                SessionType::User,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let task_id = sm.begin_budget_task(&coordinator.id).await.unwrap();

        // A dispatched worker session, unrelated to the coordinator's own
        // session lineage, bound to the SAME task identity — exactly how
        // `dispatch_goal_fn` stamps a card's worker.
        let worker = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "dispatched worker".to_string(),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        {
            let mut extension_data = worker.extension_data.clone();
            extension_data.set_extension_state(
                crate::session::BUDGET_TASK_EXTENSION_NAME,
                crate::session::BUDGET_TASK_EXTENSION_VERSION,
                serde_json::Value::String(task_id.clone()),
            );
            sm.update(&worker.id)
                .extension_data(extension_data)
                .apply()
                .await
                .unwrap();
        }

        let cfg = reservation_config(0.90, 1.00, 0.90, 1.00);

        // The coordinator's own internal completion (mirroring
        // `handle_decompose_roadmap`'s planner call) is granted, then fails
        // in a way that cannot prove whether the provider billed it.
        let coordinator_reservation = match sm
            .reserve_provider_invocation(
                "coordinator-decompose-call",
                &coordinator.id,
                Some(&task_id),
                0.20,
                "2099-01-01T00:00:00Z",
                &cfg,
            )
            .await
            .unwrap()
        {
            CostReservationOutcome::Granted { reservation_id } => reservation_id,
            other => panic!("coordinator setup reservation was not granted: {other:?}"),
        };
        assert!(sm
            .mark_provider_invocation_unknown(&coordinator_reservation)
            .await
            .unwrap());

        // Before the headroom fix this died with an unresolved-hold refusal.
        // Now it succeeds: the stale coordinator hold remains unknown, counts
        // toward headroom, and authorization proceeds because enough ceiling
        // remains.
        assert!(matches!(
            sm.reserve_provider_invocation(
                "worker-dispatch-call",
                &worker.id,
                Some(&task_id),
                0.10,
                "2099-01-01T00:00:00Z",
                &cfg,
            )
            .await
            .unwrap(),
            CostReservationOutcome::Granted { .. }
        ));
    }

    /// YF1 (G6 review, Lane Y-fix 2026-09-09, finding #1): a hold whose
    /// LEASE merely expired must NOT be released by the very next call's
    /// reconciliation — the provider call it authorized may still be
    /// genuinely streaming past its lease. Releasing it immediately (the
    /// pre-fix behavior) hands its budget headroom to someone else while
    /// real spend is still in flight; when that spend later tries to
    /// settle, the pre-fix code hard-errored and dropped it. Until the hold
    /// has been stale for well beyond the longest legitimate call
    /// (`RECONCILE_GRACE` in `reconcile_stale_reservations`), it must stay
    /// `unknown` and keep refusing fresh calls under the same task — bounded
    /// fail-closed, not indefinite, but never a fast release on a guess.
    #[tokio::test]
    async fn yf1_lease_expired_pending_hold_is_not_released_within_grace_window() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "lease expired in flight".to_string(),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let task_id = sm.begin_budget_task(&session.id).await.unwrap();
        let cfg = reservation_config(10.0, 20.0, 10.0, 20.0);

        assert!(matches!(
            sm.reserve_provider_invocation(
                "yf1-in-flight-call",
                &session.id,
                Some(&task_id),
                1.0,
                "2099-01-01T00:00:00Z",
                &cfg,
            )
            .await
            .unwrap(),
            CostReservationOutcome::Granted { .. }
        ));

        // Simulate the lease-expiry sweep alone (no explicit
        // `mark_provider_invocation_unknown` call) by backdating the lease
        // to just a few minutes ago — well within any realistic grace
        // window, unlike the deep-past ('2000-01-01') lease used by the
        // other reconciliation tests, which is deliberately far past grace.
        let pool = sm.pool_clone().await.unwrap();
        let recent_expiry = (chrono::Utc::now() - chrono::Duration::minutes(5)).to_rfc3339();
        sqlx::query(
            "UPDATE cost_reservations SET lease_until = ?
             WHERE invocation_id = 'yf1-in-flight-call'",
        )
        .bind(&recent_expiry)
        .execute(&pool)
        .await
        .unwrap();

        // A fresh call under the same task triggers both the lease sweep
        // (pending -> unknown) and reconciliation.
        //
        // This assertion changed with the unknown-hold correction, and the
        // distinction is the whole point of that change. The stale hold is
        // RETAINED — it is not released merely because no ledger row appeared
        // (asserted below). But retention must not FREEZE the chat: an
        // unresolved hold of $1.00 against a $10.00 task ceiling leaves ample
        // headroom, so a fresh $1.00 call is granted. Blocking here was the
        // unconditional "any unknown blocks everything" behaviour that the
        // correction removed; refusing every call until a hold resolves is a
        // different failure from refunding it, and both are wrong.
        assert!(matches!(
            sm.reserve_provider_invocation(
                "yf1-fresh-call",
                &session.id,
                Some(&task_id),
                1.0,
                "2099-01-01T00:00:00Z",
                &cfg,
            )
            .await
            .unwrap(),
            CostReservationOutcome::Granted { .. }
        ));

        let (state, unknown_marked_at): (String, Option<String>) = sqlx::query_as(
            "SELECT state, unknown_marked_at FROM cost_reservations
             WHERE invocation_id = 'yf1-in-flight-call'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            state, "unknown",
            "a merely lease-expired hold must not be released while still within its grace window"
        );
        assert!(
            unknown_marked_at.is_none(),
            "the lease-expiry sweep must never set unknown_marked_at — only an explicit mark_provider_invocation_unknown call does"
        );
    }

    /// Elapsed time alone does not settle or release a lease-expired hold.
    /// A later reservation can still be authorized by remaining headroom.
    #[tokio::test]
    async fn yf1_lease_expired_pending_hold_remains_unknown_after_grace_window() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "lease expired past grace".to_string(),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let task_id = sm.begin_budget_task(&session.id).await.unwrap();
        let cfg = reservation_config(10.0, 20.0, 10.0, 20.0);

        sm.reserve_provider_invocation(
            "yf1-long-stale-call",
            &session.id,
            Some(&task_id),
            1.0,
            "2099-01-01T00:00:00Z",
            &cfg,
        )
        .await
        .unwrap();

        let pool = sm.pool_clone().await.unwrap();
        // Well past the 6h grace window (3x the standard 2h paid-call
        // lease), unlike the 5-minute backdate above.
        let long_past_expiry = (chrono::Utc::now() - chrono::Duration::hours(7)).to_rfc3339();
        sqlx::query(
            "UPDATE cost_reservations SET lease_until = ?
             WHERE invocation_id = 'yf1-long-stale-call'",
        )
        .bind(&long_past_expiry)
        .execute(&pool)
        .await
        .unwrap();

        assert!(matches!(
            sm.reserve_provider_invocation(
                "yf1-fresh-after-grace",
                &session.id,
                Some(&task_id),
                1.0,
                "2099-01-01T00:00:00Z",
                &cfg,
            )
            .await
            .unwrap(),
            CostReservationOutcome::Granted { .. }
        ));
        let state: String = sqlx::query_scalar(
            "SELECT state FROM cost_reservations WHERE invocation_id = 'yf1-long-stale-call'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(state, "unknown");
    }

    /// An interactive chat starts a NEW budget task on each user message.
    /// A prior turn's explicit unknown hold remains unknown, and the next turn
    /// still runs when session headroom allows it.
    #[tokio::test]
    async fn earlier_turn_unknown_hold_is_reconciled_by_the_next_turn_in_the_same_chat() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "chat with a dropped first turn".to_string(),
                SessionType::User,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let cfg = reservation_config(5.0, 10.0, 25.0, 50.0);

        // Turn 1: granted, then the stream drops before any reply and the
        // caller marks the hold unknown.
        let first_task = sm.begin_budget_task(&session.id).await.unwrap();
        let first_reservation = match sm
            .reserve_provider_invocation(
                "turn-1-dropped-stream",
                &session.id,
                Some(&first_task),
                1.56,
                "2099-01-01T00:00:00Z",
                &cfg,
            )
            .await
            .unwrap()
        {
            CostReservationOutcome::Granted { reservation_id } => reservation_id,
            other => panic!("turn 1 reservation was not granted: {other:?}"),
        };
        assert!(sm
            .mark_provider_invocation_unknown(&first_reservation)
            .await
            .unwrap());
        let pool = sm.pool_clone().await.unwrap();
        sqlx::query(
            "UPDATE cost_reservations SET lease_until = '2000-01-01T00:00:00Z'
             WHERE invocation_id = 'turn-1-dropped-stream'",
        )
        .execute(&pool)
        .await
        .unwrap();

        // Turn 2: the next user message starts a fresh budget task, exactly
        // as `Agent::reply` does for an interactive chat.
        let second_task = sm.begin_budget_task(&session.id).await.unwrap();
        assert_ne!(first_task, second_task);
        let outcome = sm
            .reserve_provider_invocation(
                "turn-2-calendar-sync",
                &session.id,
                Some(&second_task),
                0.98,
                "2099-01-01T00:00:00Z",
                &cfg,
            )
            .await
            .unwrap();
        assert!(
            matches!(outcome, CostReservationOutcome::Granted { .. }),
            "the next turn must not be refused by an earlier turn's concluded hold: {outcome:?}"
        );

        let state: String = sqlx::query_scalar(
            "SELECT state FROM cost_reservations WHERE invocation_id = 'turn-1-dropped-stream'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            state, "unknown",
            "no ledger row means the earlier turn remains unresolved"
        );
    }

    /// An unresolved hold inside its grace window still consumes headroom, but
    /// does not unconditionally block fresh work when enough ceiling remains.
    #[tokio::test]
    async fn in_grace_unknown_hold_still_allows_calls_within_remaining_headroom() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "chat with an in-flight hold".to_string(),
                SessionType::User,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let cfg = reservation_config(5.0, 10.0, 25.0, 50.0);

        let first_task = sm.begin_budget_task(&session.id).await.unwrap();
        sm.reserve_provider_invocation(
            "turn-1-still-streaming",
            &session.id,
            Some(&first_task),
            1.0,
            "2099-01-01T00:00:00Z",
            &cfg,
        )
        .await
        .unwrap();
        let pool = sm.pool_clone().await.unwrap();
        let recent_expiry = (chrono::Utc::now() - chrono::Duration::minutes(5)).to_rfc3339();
        sqlx::query(
            "UPDATE cost_reservations SET lease_until = ?
             WHERE invocation_id = 'turn-1-still-streaming'",
        )
        .bind(&recent_expiry)
        .execute(&pool)
        .await
        .unwrap();

        let second_task = sm.begin_budget_task(&session.id).await.unwrap();
        let outcome = sm
            .reserve_provider_invocation(
                "turn-2-allowed",
                &session.id,
                Some(&second_task),
                1.0,
                "2099-01-01T00:00:00Z",
                &cfg,
            )
            .await
            .unwrap();
        assert!(
            matches!(outcome, CostReservationOutcome::Granted { .. }),
            "authorization should proceed when headroom remains: {outcome:?}"
        );

        // The original hold is still unresolved and continues to count as held.
        let state: String = sqlx::query_scalar(
            "SELECT state FROM cost_reservations WHERE invocation_id = 'turn-1-still-streaming'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(state, "unknown");
    }

    /// On restart, boot reconciliation may settle unknown holds from ledger
    /// evidence but must not auto-release either explicit-unknown or lease-only
    /// holds that still have no durable usage row.
    #[tokio::test]
    async fn reopening_the_store_keeps_unknown_holds_without_ledger_evidence() {
        let temp_dir = TempDir::new().unwrap();
        let db_dir = temp_dir.path().to_path_buf();
        let sm = SessionManager::new(db_dir.clone());
        let session = sm
            .create_session(
                db_dir.clone(),
                "chat left silent after a dropped stream".to_string(),
                SessionType::User,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let task_id = sm.begin_budget_task(&session.id).await.unwrap();
        let cfg = reservation_config(5.0, 10.0, 25.0, 50.0);

        let mut reservations = Vec::new();
        for invocation in ["boot-concluded-drop", "boot-lease-only-in-grace"] {
            match sm
                .reserve_provider_invocation(
                    invocation,
                    &session.id,
                    Some(&task_id),
                    1.56,
                    "2099-01-01T00:00:00Z",
                    &cfg,
                )
                .await
                .unwrap()
            {
                CostReservationOutcome::Granted { reservation_id } => {
                    reservations.push(reservation_id)
                }
                other => panic!("setup reservation {invocation} was not granted: {other:?}"),
            }
        }
        // The dropped stream concluded locally and marked its own hold.
        assert!(sm
            .mark_provider_invocation_unknown(&reservations[0])
            .await
            .unwrap());
        // The other went `unknown` only because its lease lapsed 5 minutes
        // ago: it may still be streaming, so it must keep holding.
        let pool = sm.pool_clone().await.unwrap();
        let recent_expiry = (chrono::Utc::now() - chrono::Duration::minutes(5)).to_rfc3339();
        sqlx::query(
            "UPDATE cost_reservations SET state = 'unknown', lease_until = ?
             WHERE invocation_id = 'boot-lease-only-in-grace'",
        )
        .bind(&recent_expiry)
        .execute(&pool)
        .await
        .unwrap();
        drop(pool);
        drop(sm);

        // Daemon restart: open the store and read. No reservation call.
        let reopened = SessionManager::new(db_dir);
        let pool = reopened.pool_clone().await.unwrap();
        let state_of = |invocation: &'static str| {
            let pool = pool.clone();
            async move {
                sqlx::query_scalar::<_, String>(
                    "SELECT state FROM cost_reservations WHERE invocation_id = ?",
                )
                .bind(invocation)
                .fetch_one(&pool)
                .await
                .unwrap()
            }
        };
        assert_eq!(
            state_of("boot-concluded-drop").await,
            "unknown",
            "a concluded hold with no ledger row must remain retained"
        );
        assert_eq!(
            state_of("boot-lease-only-in-grace").await,
            "unknown",
            "a lease-only hold inside its grace window must keep holding across a restart"
        );
    }

    /// After restart, a chat with an earlier unknown hold still answers new
    /// messages when remaining headroom allows it; the earlier hold stays
    /// unknown and budget-consuming.
    #[tokio::test]
    async fn next_message_after_restart_is_granted_in_a_chat_whose_earlier_turn_dropped() {
        let temp_dir = TempDir::new().unwrap();
        let db_dir = temp_dir.path().to_path_buf();
        let sm = SessionManager::new(db_dir.clone());
        let session = sm
            .create_session(
                db_dir.clone(),
                "chat whose first reply dropped".to_string(),
                SessionType::User,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let cfg = reservation_config(5.0, 10.0, 25.0, 50.0);

        let first_task = sm.begin_budget_task(&session.id).await.unwrap();
        let dropped = match sm
            .reserve_provider_invocation(
                "reply-half-dropped-stream",
                &session.id,
                Some(&first_task),
                1.56,
                "2099-01-01T00:00:00Z",
                &cfg,
            )
            .await
            .unwrap()
        {
            CostReservationOutcome::Granted { reservation_id } => reservation_id,
            other => panic!("turn 1 reservation was not granted: {other:?}"),
        };
        assert!(sm.mark_provider_invocation_unknown(&dropped).await.unwrap());
        drop(sm);

        // Daemon restart, then the user speaks again in the same chat.
        let reopened = SessionManager::new(db_dir);
        let next_task = reopened.begin_budget_task(&session.id).await.unwrap();
        assert_ne!(first_task, next_task);
        let outcome = reopened
            .reserve_provider_invocation(
                "reply-half-next-message",
                &session.id,
                Some(&next_task),
                1.56,
                "2099-01-01T00:00:00Z",
                &cfg,
            )
            .await
            .unwrap();
        match outcome {
            CostReservationOutcome::Granted { .. } => {}
            CostReservationOutcome::Unknown { reason } => panic!(
                "the next message must get a reply, but the chat would show \
                 \"provider spend could not be authorized: {reason}\""
            ),
            other => panic!("the next message must be granted: {other:?}"),
        }

        let pool = reopened.pool_clone().await.unwrap();
        let (unknown, pending): (i64, i64) = sqlx::query_as(
            "SELECT COALESCE(SUM(CASE WHEN state = 'unknown' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN state = 'pending' THEN 1 ELSE 0 END), 0)
             FROM cost_reservations WHERE session_id = ?",
        )
        .bind(&session.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            unknown, 1,
            "the earlier unresolved hold must remain retained"
        );
        assert_eq!(pending, 1, "the new message creates its own live hold");
    }

    /// A late usage report against a retained unknown hold settles that hold
    /// exactly once and never double-counts.
    #[tokio::test]
    async fn yf1_late_settlement_of_retained_unknown_hold_is_exactly_once() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let (session, task_id, binding) = native_started_worker(&sm, temp_dir.path()).await;
        let loose = reservation_config(5.0, 5.0, 5.0, 5.0);

        let reservation_id = match sm
            .reserve_provider_invocation_with_identity(
                "yf1-late-call",
                &session.id,
                Some(&task_id),
                0.50,
                "2099-01-01T00:00:00Z",
                &loose,
                &ProviderReservationIdentity {
                    goal_attempt_id: Some(binding.attempt_id.clone()),
                    provider: "test-provider".to_string(),
                    model: "test-model".to_string(),
                },
            )
            .await
            .unwrap()
        {
            CostReservationOutcome::Granted { reservation_id } => reservation_id,
            other => panic!("setup reservation was not granted: {other:?}"),
        };

        // Deep in the past and still unknown: elapsed time alone does not
        // auto-release this hold.
        let pool = sm.pool_clone().await.unwrap();
        sqlx::query(
            "UPDATE cost_reservations SET lease_until = '2000-01-01T00:00:00Z'
             WHERE invocation_id = 'yf1-late-call'",
        )
        .execute(&pool)
        .await
        .unwrap();
        sm.reserve_provider_invocation_with_identity(
            "yf1-late-trigger",
            &session.id,
            Some(&task_id),
            0.01,
            "2099-01-01T00:00:00Z",
            &loose,
            &ProviderReservationIdentity {
                goal_attempt_id: Some(binding.attempt_id.clone()),
                provider: "test-provider".to_string(),
                model: "test-model".to_string(),
            },
        )
        .await
        .unwrap();
        let pre_settlement_state: String = sqlx::query_scalar(
            "SELECT state FROM cost_reservations WHERE invocation_id = 'yf1-late-call'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            pre_settlement_state, "unknown",
            "setup: the hold must stay retained before late usage arrives"
        );

        sqlx::query(
            "UPDATE goal_attempts SET execution_state = 'failed', ended_at = datetime('now')
             WHERE attempt_id = ?",
        )
        .bind(&binding.attempt_id)
        .execute(&pool)
        .await
        .unwrap();

        // The provider call completes after the authentic journal row was
        // terminalized. Settlement must not re-resolve only active workers.
        let mut row = reservation_row(&session.id, &task_id, "yf1-late-call", 0.50);
        row.goal_id = Some(binding.goal_id.clone());
        row.goal_attempt_id = Some(binding.attempt_id.clone());
        let inserted = sm
            .settle_provider_invocation(&reservation_id, &row, None, None, None, None, 15, 10, 5)
            .await
            .unwrap();
        assert!(
            inserted,
            "the late settlement must be recorded, not dropped"
        );
        let inserted_again = sm
            .settle_provider_invocation(&reservation_id, &row, None, None, None, None, 15, 10, 5)
            .await
            .unwrap();
        assert!(
            !inserted_again,
            "a second settlement attempt must be an idempotent no-op"
        );

        let cost_usd: f64 =
            sqlx::query_scalar("SELECT cost_usd FROM cost_ledger WHERE call_id = 'yf1-late-call'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let ledger_rows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM cost_ledger WHERE call_id = 'yf1-late-call'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let settled_state: String = sqlx::query_scalar(
            "SELECT state FROM cost_reservations WHERE invocation_id = 'yf1-late-call'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let stored_attempt: Option<String> = sqlx::query_scalar(
            "SELECT goal_attempt_id FROM cost_ledger WHERE call_id = 'yf1-late-call'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(settled_state, "settled");
        assert_eq!(stored_attempt.as_deref(), Some(binding.attempt_id.as_str()));
        assert_eq!(ledger_rows, 1, "late settlement must not duplicate spend");
        assert!((cost_usd - 0.50).abs() < 1e-12);

        // And it must count: task spend is now 0.50 (late) + 0.01 (trigger)
        // = 0.51 against a 0.55 hard ceiling, so a fresh 0.10 request must be
        // refused — if the late spend had vanished instead, this would
        // wrongly succeed.
        let tight = reservation_config(0.50, 0.55, 0.50, 0.55);
        assert!(matches!(
            sm.reserve_provider_invocation(
                "yf1-post-late-call",
                &session.id,
                Some(&task_id),
                0.10,
                "2099-01-01T00:00:00Z",
                &tight,
            )
            .await
            .unwrap(),
            CostReservationOutcome::Refused { .. } | CostReservationOutcome::NeedsGate { .. }
        ));
    }

    #[tokio::test]
    async fn native_local_and_paid_routes_survive_crash_before_usage_settlement() {
        let temp_dir = TempDir::new().unwrap();
        let sm = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let (session, task_id, binding) = native_started_worker(&sm, temp_dir.path()).await;
        let pool = sm.pool_clone().await.unwrap();

        // The local decision backend persists its route before dispatch. A
        // crash after transport begins but before posthoc usage must still
        // leave enough evidence to include the local model family.
        let local = crate::cost_router::metered::record_local_provider_call(
            Arc::clone(&sm),
            &session,
            "ollama",
            "qwen2.5:7b",
            32,
        )
        .await
        .unwrap();
        crate::cost_router::metered::mark_metered_call_dispatch_possible(Arc::clone(&sm), &local)
            .await
            .unwrap();

        // In the same native attempt, a paid route is journaled separately
        // from its monetary hold. Neither has a ledger row yet, simulating a
        // process stop before usage settlement.
        let config = reservation_config(5.0, 10.0, 25.0, 50.0);
        let paid_call = "native-mixed-paid";
        let reservation_id = match sm
            .reserve_provider_invocation_with_identity(
                paid_call,
                &session.id,
                Some(&task_id),
                0.25,
                "2099-01-01T00:00:00Z",
                &config,
                &ProviderReservationIdentity {
                    goal_attempt_id: Some(binding.attempt_id.clone()),
                    provider: "anthropic".to_string(),
                    model: "claude-test".to_string(),
                },
            )
            .await
            .unwrap()
        {
            CostReservationOutcome::Granted { reservation_id } => reservation_id,
            other => panic!("paid native route was not granted: {other:?}"),
        };
        crate::native_provider_calls::record(
            &pool,
            paid_call,
            &binding,
            "anthropic",
            "claude-test",
        )
        .await
        .unwrap();
        crate::native_provider_calls::mark_dispatch_possible(&pool, paid_call, &binding)
            .await
            .unwrap();
        sm.mark_provider_invocation_dispatch_possible(&reservation_id)
            .await
            .unwrap();

        let routes: Vec<(String, String, String, String)> = sqlx::query_as(
            "SELECT provider,model,coverage_version,dispatch_state
             FROM goal_attempt_provider_calls WHERE goal_attempt_id=? ORDER BY provider",
        )
        .bind(&binding.attempt_id)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(routes.len(), 2);
        assert!(routes.iter().any(|row| {
            row.0 == "ollama"
                && row.1 == "qwen2.5:7b"
                && row.2 == crate::native_provider_calls::COVERAGE_VERSION
                && row.3 == "started_possible"
        }));
        assert!(routes.iter().any(|row| {
            row.0 == "anthropic"
                && row.1 == "claude-test"
                && row.2 == crate::native_provider_calls::COVERAGE_VERSION
                && row.3 == "started_possible"
        }));
        let ledger_rows: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cost_ledger WHERE session_id=? AND goal_attempt_id=?",
        )
        .bind(&session.id)
        .bind(&binding.attempt_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            ledger_rows, 0,
            "the fixture represents a crash before settlement"
        );

        // A terminal native worker cannot fall through as an ordinary
        // session and dispatch a new local decision after cancellation.
        sqlx::query(
            "UPDATE goal_attempts SET execution_state='failed',ended_at=datetime('now')
             WHERE attempt_id=?",
        )
        .bind(&binding.attempt_id)
        .execute(&pool)
        .await
        .unwrap();
        assert!(crate::cost_router::metered::record_local_provider_call(
            Arc::clone(&sm),
            &session,
            "ollama",
            "qwen2.5:7b",
            32,
        )
        .await
        .is_err());
    }

    /// B6.2: compaction and continuation keep the same durable task identity,
    /// preserve code-bearing tool arguments byte-for-byte, and keep ledger
    /// replay idempotent across a manager restart.
    #[tokio::test]
    async fn b6_compaction_continuation_preserves_task_and_ledger_identity() {
        use crate::providers::base::{ProviderUsage, Usage};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        struct B6Compactor {
            calls: Arc<AtomicUsize>,
        }

        #[async_trait::async_trait]
        impl crate::context_mgmt::AccountedFastCompletion for B6Compactor {
            async fn complete_fast(
                &self,
                _system: &str,
                _messages: &[Message],
                _tools: &[rmcp::model::Tool],
            ) -> Result<(Message, ProviderUsage)> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                Ok((
                    Message::assistant().with_text("b6 compacted summary"),
                    ProviderUsage::new("b6-local".to_string(), Usage::default()),
                ))
            }
        }

        let temp_dir = TempDir::new().unwrap();
        let db_dir = temp_dir.path().to_path_buf();
        let sm = SessionManager::new(db_dir.clone());
        let session = sm
            .create_session(
                db_dir.clone(),
                "compaction continuation".to_string(),
                SessionType::User,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let task_id = sm.begin_budget_task(&session.id).await.unwrap();
        let before = "fn add(a: i32, b: i32) -> i32 {\n    a - b // BUG\n}";
        let after = "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}";
        let diff_args = serde_json::json!({
            "path": "/repo/src/math.rs",
            "before": before,
            "after": after,
        })
        .as_object()
        .unwrap()
        .clone();
        let conversation = Conversation::new_unvalidated(vec![
            Message::user().with_text("fix the add bug"),
            Message::assistant().with_tool_request(
                "b6-edit",
                Ok(
                    rmcp::model::CallToolRequestParams::new("developer__edit".to_string())
                        .with_arguments(diff_args.clone()),
                ),
            ),
            Message::user().with_tool_response(
                "b6-edit",
                Ok(rmcp::model::CallToolResult::success(vec![
                    rmcp::model::Content::text("edited math.rs"),
                ])),
            ),
            Message::assistant().with_tool_request(
                "b6-shell",
                Ok(rmcp::model::CallToolRequestParams::new(
                    "developer__shell".to_string(),
                )),
            ),
            Message::user().with_tool_response(
                "b6-shell",
                Ok(rmcp::model::CallToolResult::success(vec![
                    rmcp::model::Content::text("B6_SHELL_LOG_MARKER".repeat(200))
                        .with_priority(0.0),
                ])),
            ),
        ]);
        let compactor = B6Compactor {
            calls: Arc::new(AtomicUsize::new(0)),
        };
        let compaction_calls = Arc::clone(&compactor.calls);
        let (compacted, _) = crate::context_mgmt::compact_messages_accounted(
            &session.id,
            &conversation,
            false,
            &compactor,
        )
        .await
        .unwrap();
        assert_eq!(compaction_calls.load(Ordering::SeqCst), 1);

        let recovered_args = compacted
            .agent_visible_messages()
            .iter()
            .flat_map(|message| &message.content)
            .find_map(|content| match content {
                MessageContent::ToolRequest(request) => match &request.tool_call {
                    Ok(call) if call.name.ends_with("edit") => call.arguments.clone(),
                    _ => None,
                },
                _ => None,
            })
            .expect("code-bearing edit must survive compaction");
        assert_eq!(recovered_args, diff_args);

        let ledger_row = |call_id: &str, cost_usd: f64| CostLedgerRow {
            call_id: call_id.to_string(),
            ts: if call_id == "b6-compact-before" {
                "2026-09-05T00:00:00Z".to_string()
            } else {
                "2026-09-05T00:00:01Z".to_string()
            },
            session_id: session.id.clone(),
            parent_session_id: None,
            task_id: Some(task_id.clone()),
            goal_id: None,
            goal_attempt_id: None,
            subagent_id: None,
            provider: Some("b6-local".to_string()),
            model: Some("b6-local-model".to_string()),
            cost_tier: CostTier::LocalFree,
            is_headless: false,
            input_tokens: 1,
            output_tokens: 1,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            input_cost: 0.0,
            output_cost: 0.0,
            cache_read_cost: 0.0,
            cache_write_cost: 0.0,
            cost_usd,
            cache_savings_usd: 0.0,
            is_estimated: false,
            provenance: Default::default(),
            outcome: Default::default(),
        };
        sm.append_cost_ledger(&ledger_row("b6-compact-before", 0.10))
            .await
            .unwrap();
        drop(sm);

        let resumed = SessionManager::new(db_dir);
        let resumed_session = resumed.get_session(&session.id, false).await.unwrap();
        assert_eq!(
            budget_task_id(&resumed_session.extension_data),
            Some(task_id.clone())
        );
        resumed
            .replace_conversation(&session.id, &compacted)
            .await
            .unwrap();
        resumed
            .append_cost_ledger(&ledger_row("b6-compact-after", 0.20))
            .await
            .unwrap();
        // Replaying the continuation's terminal row is a no-op.
        resumed
            .append_cost_ledger(&ledger_row("b6-compact-after", 0.20))
            .await
            .unwrap();

        let pool = resumed.pool_clone().await.unwrap();
        let ledger_rows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM cost_ledger WHERE task_id = ?")
                .bind(&task_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(ledger_rows, 2);
        let task_total: f64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(cost_usd), 0.0) FROM cost_ledger WHERE task_id = ?",
        )
        .bind(&task_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!((task_total - 0.30).abs() < 1e-12);
    }

    /// B6.5: local, subscription, and paid workers can share one durable task
    /// without relabelling free work as paid or creating more than one hold.
    /// The ledger/reservation seams stand in for deterministic fake workers;
    /// no provider transport is involved.
    #[tokio::test]
    async fn b6_mixed_billing_classes_keep_one_paid_hold_and_exact_recursive_totals() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let parent = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "mixed billing".to_string(),
                SessionType::User,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let task_id = sm.begin_budget_task(&parent.id).await.unwrap();
        let child = sm
            .create_session_with_parent(
                Some(&parent.id),
                temp_dir.path().to_path_buf(),
                "subscription child".to_string(),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let grandchild = sm
            .create_session_with_parent(
                Some(&child.id),
                temp_dir.path().to_path_buf(),
                "paid grandchild".to_string(),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .unwrap();

        let row = |call_id: &str,
                   session_id: &str,
                   parent_session_id: Option<&str>,
                   tier: CostTier,
                   cost_usd: f64| CostLedgerRow {
            call_id: call_id.to_string(),
            ts: match call_id {
                "b6-local" => "2026-09-05T00:00:00Z",
                "b6-subscription" => "2026-09-05T00:00:01Z",
                "b6-paid" => "2026-09-05T00:00:02Z",
                _ => "2026-09-05T00:00:03Z",
            }
            .to_string(),
            session_id: session_id.to_string(),
            parent_session_id: parent_session_id.map(ToOwned::to_owned),
            task_id: Some(task_id.clone()),
            goal_id: None,
            goal_attempt_id: None,
            subagent_id: None,
            provider: Some(format!("{call_id}-provider")),
            model: Some("b6-fake".to_string()),
            cost_tier: tier,
            is_headless: true,
            input_tokens: 10,
            output_tokens: 5,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            input_cost: cost_usd,
            output_cost: 0.0,
            cache_read_cost: 0.0,
            cache_write_cost: 0.0,
            cost_usd,
            cache_savings_usd: 0.0,
            is_estimated: false,
            provenance: Default::default(),
            outcome: Default::default(),
        };

        sm.append_cost_ledger(&row("b6-local", &parent.id, None, CostTier::LocalFree, 0.0))
            .await
            .unwrap();
        sm.append_cost_ledger(&row(
            "b6-subscription",
            &child.id,
            Some(&parent.id),
            CostTier::Subscription,
            0.0,
        ))
        .await
        .unwrap();

        let cfg = reservation_config(4.0, 5.0, 4.0, 5.0);
        let reservation_id = match sm
            .reserve_provider_invocation(
                "b6-paid",
                &grandchild.id,
                Some(&task_id),
                0.50,
                "2099-01-01T00:00:00Z",
                &cfg,
            )
            .await
            .unwrap()
        {
            CostReservationOutcome::Granted { reservation_id } => reservation_id,
            outcome => panic!("paid fake worker was not granted: {outcome:?}"),
        };
        let paid = row(
            "b6-paid",
            &grandchild.id,
            Some(&child.id),
            CostTier::PaidApi,
            0.25,
        );
        assert!(sm
            .settle_provider_invocation(&reservation_id, &paid, None, None, None, None, 15, 10, 5,)
            .await
            .unwrap());

        let pool = sm.pool_clone().await.unwrap();
        let rows: Vec<(String, i64, f64)> = sqlx::query_as(
            "SELECT cost_tier, is_chargeable, cost_usd FROM cost_ledger
             WHERE task_id = ? ORDER BY call_id",
        )
        .bind(&task_id)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            rows,
            vec![
                ("local_free".to_string(), 0, 0.0),
                ("paid_api".to_string(), 1, 0.25),
                ("subscription".to_string(), 0, 0.0),
            ]
        );

        let reservation_counts: (i64, i64) = sqlx::query_as(
            "SELECT COUNT(*), COALESCE(SUM(state = 'settled'), 0)
             FROM cost_reservations WHERE task_id = ?",
        )
        .bind(&task_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(reservation_counts, (1, 1));

        let projection = sm.budget_projection(&parent.id, cfg).await.unwrap();
        assert!((projection.task.settled_usd.unwrap() - 0.25).abs() < 1e-12);
        assert!((projection.session.settled_usd.unwrap() - 0.25).abs() < 1e-12);
        assert_eq!(projection.task.held_usd, Some(0.0));
        assert_eq!(projection.session.unknown_usd, Some(0.0));
        assert_eq!(
            projection.task_billing.billing_class.as_deref(),
            Some("paid_api")
        );
        assert_eq!(
            projection.session_billing.billing_class.as_deref(),
            Some("paid_api")
        );
    }

    /// B6.4: duplicate child completion callbacks for one durable invocation
    /// produce one settlement, one ledger row, and one recursive roll-up; a
    /// callback replay after restart remains a no-op.
    #[tokio::test]
    async fn b6_duplicate_child_completion_settles_once_after_restart() {
        let temp_dir = TempDir::new().unwrap();
        let db_dir = temp_dir.path().to_path_buf();
        let sm = SessionManager::new(db_dir.clone());
        let parent = sm
            .create_session(
                db_dir.clone(),
                "duplicate child completion".to_string(),
                SessionType::User,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let task_id = sm.begin_budget_task(&parent.id).await.unwrap();
        let child = sm
            .create_session_with_parent(
                Some(&parent.id),
                db_dir.clone(),
                "child callback".to_string(),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        assert_eq!(budget_task_id(&child.extension_data), Some(task_id.clone()));
        let cfg = reservation_config(4.0, 5.0, 4.0, 5.0);
        let reservation_id = match sm
            .reserve_provider_invocation(
                "b6-child-completion",
                &child.id,
                Some(&task_id),
                0.50,
                "2099-01-01T00:00:00Z",
                &cfg,
            )
            .await
            .unwrap()
        {
            CostReservationOutcome::Granted { reservation_id } => reservation_id,
            outcome => panic!("child completion fixture was not authorized: {outcome:?}"),
        };
        let mut row = reservation_row(&child.id, &task_id, "b6-child-completion", 0.25);
        row.parent_session_id = Some(parent.id.clone());

        let (first, second) = tokio::join!(
            sm.settle_provider_invocation(
                &reservation_id,
                &row,
                None,
                None,
                None,
                None,
                15,
                10,
                5,
            ),
            sm.settle_provider_invocation(
                &reservation_id,
                &row,
                None,
                None,
                None,
                None,
                15,
                10,
                5,
            )
        );
        let settled_results = [first.unwrap(), second.unwrap()];
        assert_eq!(
            settled_results.iter().filter(|settled| **settled).count(),
            1
        );

        let pool = sm.pool_clone().await.unwrap();
        let ledger_rows: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cost_ledger WHERE call_id = 'b6-child-completion'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(ledger_rows, 1);
        let rollup = sm.cost_by_parent_session(&parent.id).await.unwrap();
        assert!((rollup.children_total - 0.25).abs() < 1e-12);
        let projection = sm.budget_projection(&parent.id, cfg).await.unwrap();
        assert!((projection.task.settled_usd.unwrap() - 0.25).abs() < 1e-12);
        assert!((projection.session.settled_usd.unwrap() - 0.25).abs() < 1e-12);
        drop(sm);

        let resumed = SessionManager::new(db_dir);
        assert!(!resumed
            .settle_provider_invocation(&reservation_id, &row, None, None, None, None, 15, 10, 5,)
            .await
            .unwrap());
        let resumed_pool = resumed.pool_clone().await.unwrap();
        let resumed_rows: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cost_ledger WHERE call_id = 'b6-child-completion'",
        )
        .fetch_one(&resumed_pool)
        .await
        .unwrap();
        assert_eq!(resumed_rows, 1);
    }

    /// B6.6: two distinct claims launched concurrently against the same
    /// one-call allowance must be arbitrated by the atomic reservation
    /// transaction, not by a stale preselection snapshot.
    #[tokio::test]
    async fn b6_atomic_claim_race_grants_once_and_refuses_once() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "claim race".to_string(),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let task_id = sm.begin_budget_task(&session.id).await.unwrap();
        let cfg = reservation_config(0.90, 1.00, 0.90, 1.00);

        let (first, second) = tokio::join!(
            sm.reserve_provider_invocation(
                "b6-race-a",
                &session.id,
                Some(&task_id),
                0.60,
                "2099-01-01T00:00:00Z",
                &cfg,
            ),
            sm.reserve_provider_invocation(
                "b6-race-b",
                &session.id,
                Some(&task_id),
                0.60,
                "2099-01-01T00:00:00Z",
                &cfg,
            )
        );
        let outcomes = [first.unwrap(), second.unwrap()];
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, CostReservationOutcome::Granted { .. }))
                .count(),
            1
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| {
                    matches!(
                        outcome,
                        CostReservationOutcome::Refused {
                            scope: crate::cost_router::budget::BudgetScope::Task,
                            ..
                        }
                    )
                })
                .count(),
            1
        );

        let pool = sm.pool_clone().await.unwrap();
        let reservation_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM cost_reservations WHERE task_id = ?")
                .bind(&task_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            reservation_count, 1,
            "only one fake dispatch may be authorized"
        );
        let ledger_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM cost_ledger WHERE task_id = ?")
                .bind(&task_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(ledger_count, 0, "reservation race must not fabricate spend");
    }

    #[tokio::test]
    async fn reservation_rejects_invalid_bounds_and_failed_settlement_is_atomic() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "invalid bounds".to_string(),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let task_id = sm.begin_budget_task(&session.id).await.unwrap();
        let cfg = reservation_config(10.0, 20.0, 10.0, 20.0);
        for amount in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(sm
                .reserve_provider_invocation(
                    &format!("invalid-{amount}"),
                    &session.id,
                    Some(&task_id),
                    amount,
                    "2099-01-01T00:00:00Z",
                    &cfg,
                )
                .await
                .is_err());
        }
        for ceiling in [f64::NAN, f64::INFINITY] {
            let mut invalid_cfg = cfg;
            invalid_cfg.task.soft = ceiling;
            assert!(matches!(
                sm.reserve_provider_invocation(
                    "invalid-ceiling",
                    &session.id,
                    Some(&task_id),
                    1.0,
                    "2099-01-01T00:00:00Z",
                    &invalid_cfg,
                )
                .await
                .unwrap(),
                CostReservationOutcome::Unknown { .. }
            ));
        }
        let reservation_id = match sm
            .reserve_provider_invocation(
                "atomic-call",
                &session.id,
                Some(&task_id),
                1.0,
                "2099-01-01T00:00:00Z",
                &cfg,
            )
            .await
            .unwrap()
        {
            CostReservationOutcome::Granted { reservation_id } => reservation_id,
            outcome => panic!("unexpected reservation outcome: {outcome:?}"),
        };
        let mismatched = reservation_row(&session.id, "other-task", "atomic-call", 0.5);
        assert!(sm
            .settle_provider_invocation(
                &reservation_id,
                &mismatched,
                None,
                Some(15),
                Some(10),
                Some(5),
                15,
                10,
                5,
            )
            .await
            .is_err());
        let pool = sm.pool_clone().await.unwrap();
        let state: String =
            sqlx::query_scalar("SELECT state FROM cost_reservations WHERE reservation_id = ?")
                .bind(&reservation_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(state, "pending");
        let ledger_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM cost_ledger WHERE call_id = 'atomic-call'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(ledger_count, 0);
        assert_eq!(
            sm.get_session(&session.id, false)
                .await
                .unwrap()
                .accumulated_total_tokens,
            None
        );
    }
}

#[cfg(test)]
mod meeting_triage_boot_tests {
    use super::*;

    #[tokio::test]
    async fn existing_database_installs_meeting_triage_before_note_save() {
        let dir = tempfile::tempdir().unwrap();
        let manager = SessionManager::new(dir.path().to_path_buf());
        let pool = manager.pool_clone().await.unwrap();
        // Model an installation created before the meeting queue existed.
        sqlx::query("DROP TABLE meeting_triage_items")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DROP TABLE meeting_triage")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        drop(manager);

        let reopened = SessionManager::new(dir.path().to_path_buf());
        let pool = reopened.pool_clone().await.unwrap();
        let saved = crate::meeting_triage::save_meeting_note(
            &pool,
            None,
            crate::projects::PERSONAL_PROJECT_ID,
            "upgrade-recording",
            true,
            Some("Upgrade meeting"),
            "We agreed to document the migration.",
        )
        .await
        .unwrap();
        assert_eq!(
            saved.triage.status,
            crate::meeting_triage::TriageStatus::Pending
        );
        assert_eq!(saved.triage.note_id, saved.note.id);
        let item_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM meeting_triage_items")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(item_count, 0);
    }
}

#[cfg(test)]
mod schedule_fire_claims_boot_tests {
    use super::*;

    #[tokio::test]
    async fn first_boot_and_v55_reopen_install_fire_claims() {
        let dir = tempfile::tempdir().unwrap();
        // pool_clone takes the production first-boot branch through
        // SessionStorage::pool -> init_spectral_db, not the migration helper.
        let manager = SessionManager::new(dir.path().to_path_buf());
        let pool = manager.pool_clone().await.unwrap();
        let expected = vec![
            ("job_id".to_string(), 1_i32),
            ("fire_key".to_string(), 2_i32),
        ];
        let pk: Vec<(String, i32)> = sqlx::query_as(
            "SELECT name, pk FROM pragma_table_info('schedule_fire_claims') WHERE pk > 0 ORDER BY pk",
        ).fetch_all(&pool).await.unwrap();
        assert_eq!(pk, expected);
        let column: (String, String, i32) = sqlx::query_as(
            "SELECT name, type, [notnull] FROM pragma_table_info('schedule_fire_claims') WHERE name = 'started_at'",
        ).fetch_one(&pool).await.unwrap();
        assert_eq!(column, ("started_at".to_string(), "TEXT".to_string(), 0));
        let version: i32 = sqlx::query_scalar("SELECT MAX(version) FROM schema_version")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            version,
            super::super::spectral_schema::SPECTRAL_SCHEMA_VERSION
        );
        sqlx::query("DROP TABLE schedule_fire_claims")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM schema_version")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO schema_version (version) VALUES (55)")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        drop(manager);
        let reopened = SessionManager::new(dir.path().to_path_buf());
        let pool = reopened.pool_clone().await.unwrap();
        let pk: Vec<(String, i32)> = sqlx::query_as(
            "SELECT name, pk FROM pragma_table_info('schedule_fire_claims') WHERE pk > 0 ORDER BY pk",
        ).fetch_all(&pool).await.unwrap();
        assert_eq!(pk, expected);
        let column: (String, String, i32) = sqlx::query_as(
            "SELECT name, type, [notnull] FROM pragma_table_info('schedule_fire_claims') WHERE name = 'started_at'",
        ).fetch_one(&pool).await.unwrap();
        assert_eq!(column, ("started_at".to_string(), "TEXT".to_string(), 0));
        let version: i32 = sqlx::query_scalar("SELECT MAX(version) FROM schema_version")
            .fetch_one(&pool)
            .await
            .unwrap();
        // The ladder keeps growing (v57 journal, v58 pause episodes, ...); this
        // test owns v56 and must not break every time a later step lands.
        assert!(
            version >= 56,
            "reopening a v55 database must reach at least v56, got {version}"
        );
    }
}

#[cfg(test)]
mod decision_journal_boot_tests {
    use super::*;

    #[tokio::test]
    async fn first_boot_and_v56_reopen_install_decision_journal() {
        let dir = tempfile::tempdir().unwrap();
        let manager = SessionManager::new(dir.path().to_path_buf());
        let pool = manager.pool_clone().await.unwrap();
        let cols: Vec<(String, String, i32)> = sqlx::query_as(
            "SELECT name, type, [notnull]
             FROM pragma_table_info('decision_journal')
             WHERE name IN (
                'id','ts','site','question_version','state_ref','backend',
                'distribution_json','acted','fallback_reason','latency_ms'
             )
             ORDER BY cid",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            cols,
            vec![
                ("id".to_string(), "TEXT".to_string(), 0),
                ("ts".to_string(), "TEXT".to_string(), 1),
                ("site".to_string(), "TEXT".to_string(), 1),
                ("question_version".to_string(), "TEXT".to_string(), 1),
                ("state_ref".to_string(), "TEXT".to_string(), 1),
                ("backend".to_string(), "TEXT".to_string(), 1),
                ("distribution_json".to_string(), "TEXT".to_string(), 1),
                ("acted".to_string(), "INTEGER".to_string(), 1),
                ("fallback_reason".to_string(), "TEXT".to_string(), 0),
                ("latency_ms".to_string(), "INTEGER".to_string(), 1),
            ]
        );
        let indexes: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM pragma_index_list('decision_journal') ORDER BY name",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert!(indexes
            .iter()
            .any(|name| name == "idx_decision_journal_site_ts"));

        sqlx::query("DROP TABLE decision_journal")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM schema_version")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO schema_version (version) VALUES (56)")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        drop(manager);

        let reopened = SessionManager::new(dir.path().to_path_buf());
        let pool = reopened.pool_clone().await.unwrap();
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type='table' AND name='decision_journal'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 1);
        let version: i32 = sqlx::query_scalar("SELECT MAX(version) FROM schema_version")
            .fetch_one(&pool)
            .await
            .unwrap();
        // Later ladder steps (v58 pause episodes, ...) stamp past 57; this test
        // owns the journal step and must not break each time one lands.
        assert!(
            version >= 57,
            "reopening a v56 database must reach at least v57, got {version}"
        );
    }
}

#[cfg(test)]
mod call_provenance_tests {
    use super::*;
    use crate::conversation::message::MessageContent;
    use rmcp::model::CallToolRequestParams;

    #[test]
    fn call_provenance_default_is_all_unknown() {
        let provenance = CallProvenance::default();
        assert_eq!(provenance.purpose, CallPurpose::Unknown);
        assert_eq!(provenance.purpose.as_str(), "unknown");
        assert_eq!(provenance.trigger_ref, None);
        assert_eq!(provenance.retry_of_call_id, None);
        assert_eq!(provenance.pre_call_evidence, None);

        let outcome = CallOutcome::default();
        assert_eq!(outcome.tool_call_count, None, "unobserved is not zero");
        assert_eq!(outcome.completion_kind, CompletionKind::Unknown);
        assert_ne!(outcome.completion_kind, CompletionKind::Empty);
    }

    #[test]
    fn call_provenance_trigger_ref_is_bounded_not_cut() {
        let ok = CallProvenance::with_purpose(CallPurpose::UserResponse).with_trigger("message:m1");
        assert_eq!(ok.trigger_ref.as_deref(), Some("message:m1"));

        let too_long = "x".repeat(TRIGGER_REF_MAX_BYTES + 1);
        let dropped = CallProvenance::default().with_trigger(too_long);
        assert_eq!(dropped.trigger_ref, None);

        let retry = CallProvenance::retry_of("call-0");
        assert_eq!(retry.purpose, CallPurpose::Retry);
        assert_eq!(retry.retry_of_call_id.as_deref(), Some("call-0"));
    }

    #[test]
    fn pre_call_evidence_is_versioned_and_within_bound() {
        let evidence = PreCallEvidence::new(
            "poll_unchanged@1",
            serde_json::json!({"changed_state": false, "pending_work": 2}),
            WouldSkip::No,
        );
        assert!(!evidence.is_truncated());
        let value: serde_json::Value = serde_json::from_str(evidence.as_json()).unwrap();
        assert_eq!(value["v"], 1);
        assert_eq!(value["predicate"], "poll_unchanged@1");
        assert_eq!(value["inputs"]["pending_work"], 2);
        assert_eq!(value["would_skip"], "no");
        assert!(evidence.as_json().len() <= PRE_CALL_EVIDENCE_MAX_BYTES);
    }

    #[test]
    fn pre_call_evidence_over_bound_becomes_a_well_formed_marker() {
        let huge = "y".repeat(PRE_CALL_EVIDENCE_MAX_BYTES * 2);
        let evidence = PreCallEvidence::new(
            "poll_unchanged@1",
            serde_json::json!({ "blob": huge }),
            WouldSkip::Unknown,
        );
        assert!(evidence.is_truncated());
        assert!(evidence.as_json().len() <= PRE_CALL_EVIDENCE_MAX_BYTES);
        // Never cut JSON: the marker parses, and says what happened.
        let value: serde_json::Value = serde_json::from_str(evidence.as_json()).unwrap();
        assert_eq!(value["v"], 1);
        assert_eq!(value["truncated"], true);
        assert_eq!(value["predicate"], "poll_unchanged@1");
        assert_eq!(value["would_skip"], "unknown");
        assert!(value["original_bytes"].as_u64().unwrap() > PRE_CALL_EVIDENCE_MAX_BYTES as u64);
        assert!(value.get("inputs").is_none());

        // An absurd predicate name is dropped from the marker, not cut.
        let long_name = "p".repeat(10_000);
        let evidence = PreCallEvidence::new(&long_name, serde_json::json!({}), WouldSkip::Yes);
        assert!(evidence.is_truncated());
        let value: serde_json::Value = serde_json::from_str(evidence.as_json()).unwrap();
        assert!(value["predicate"].is_null());
        assert!(evidence.as_json().len() <= PRE_CALL_EVIDENCE_MAX_BYTES);
    }

    fn tool_request(id: &str) -> MessageContent {
        MessageContent::tool_request(id.to_string(), Ok(CallToolRequestParams::new("t")))
    }

    #[test]
    fn completion_kind_derives_from_response_shape() {
        assert_eq!(CompletionKind::from_shape(true, 0), CompletionKind::Text);
        assert_eq!(CompletionKind::from_shape(false, 2), CompletionKind::Tools);
        assert_eq!(CompletionKind::from_shape(true, 1), CompletionKind::Mixed);
        assert_eq!(CompletionKind::from_shape(false, 0), CompletionKind::Empty);

        let text = Message::assistant().with_text("hello");
        assert_eq!(
            CallOutcome::from_message(&text),
            CallOutcome {
                tool_call_count: Some(0),
                completion_kind: CompletionKind::Text
            }
        );
        let blank = Message::assistant().with_text("   ");
        assert_eq!(
            CallOutcome::from_message(&blank).completion_kind,
            CompletionKind::Empty
        );
        let mut tools = Message::assistant();
        tools.content.push(tool_request("a"));
        tools.content.push(tool_request("b"));
        let tools_outcome = CallOutcome::from_message(&tools);
        assert_eq!(tools_outcome.tool_call_count, Some(2));
        assert_eq!(tools_outcome.completion_kind, CompletionKind::Tools);
        let mut mixed = Message::assistant().with_text("calling a tool");
        mixed.content.push(tool_request("c"));
        assert_eq!(
            CallOutcome::from_message(&mixed).completion_kind,
            CompletionKind::Mixed
        );
    }

    #[test]
    fn stream_outcome_tracker_counts_completed_requests_and_terminal_state() {
        // Text deltas across frames plus one complete tool request.
        let mut tracker = StreamOutcomeTracker::default();
        tracker.observe_frame(false);
        tracker.observe_message(&Message::assistant().with_text("part one"));
        tracker.observe_frame(false);
        let mut with_tool = Message::assistant();
        with_tool.content.push(tool_request("x"));
        tracker.observe_message(&with_tool);
        tracker.observe_frame(false);
        let outcome = tracker.finish(false);
        assert_eq!(outcome.tool_call_count, Some(1));
        assert_eq!(outcome.completion_kind, CompletionKind::Mixed);
        assert_eq!(
            tracker.finish(true).completion_kind,
            CompletionKind::Cancelled
        );

        // An error as the last frame is an error completion...
        let mut errored = StreamOutcomeTracker::default();
        errored.observe_frame(false);
        errored.observe_frame(true);
        assert_eq!(errored.finish(false).completion_kind, CompletionKind::Error);
        // ...but a good frame after it (a successful stall retry) clears it.
        errored.observe_frame(false);
        assert_eq!(errored.finish(false).completion_kind, CompletionKind::Empty);

        // Nothing observed at all stays unknown, never empty/zero.
        let untouched = StreamOutcomeTracker::default();
        assert_eq!(untouched.finish(false), CallOutcome::default());
    }

    fn row(call_id: &str, session_id: &str) -> CostLedgerRow {
        CostLedgerRow {
            call_id: call_id.to_string(),
            ts: "2026-09-23T00:00:00Z".to_string(),
            session_id: session_id.to_string(),
            parent_session_id: None,
            task_id: None,
            goal_id: None,
            goal_attempt_id: None,
            subagent_id: None,
            provider: Some("ollama".to_string()),
            model: Some("local-model".to_string()),
            cost_tier: CostTier::LocalFree,
            is_headless: true,
            input_tokens: 10,
            output_tokens: 5,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            input_cost: 0.0,
            output_cost: 0.0,
            cache_read_cost: 0.0,
            cache_write_cost: 0.0,
            cost_usd: 0.0,
            cache_savings_usd: 0.0,
            is_estimated: false,
            provenance: CallProvenance::default(),
            outcome: CallOutcome::default(),
        }
    }

    type V60Columns = (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i64>,
        Option<String>,
    );

    async fn read_v60(pool: &Pool<Sqlite>, call_id: &str) -> V60Columns {
        sqlx::query_as(
            "SELECT call_purpose, trigger_ref, retry_of_call_id, pre_call_evidence,
                    tool_call_count, completion_kind
             FROM cost_ledger WHERE call_id = ?",
        )
        .bind(call_id)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn cost_ledger_insert_round_trips_call_purpose_fields() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                temp_dir.path().to_path_buf(),
                "a4".to_string(),
                SessionType::User,
                GooseMode::default(),
            )
            .await
            .unwrap();

        let evidence = PreCallEvidence::new(
            "poll_unchanged@1",
            serde_json::json!({"changed_state": false}),
            WouldSkip::No,
        );
        let mut known = row("call-known", &session.id);
        known.provenance = CallProvenance::retry_of("call-failed")
            .with_trigger("tool_result:tc-1")
            .with_evidence(evidence.clone());
        known.outcome = CallOutcome::observed(true, 3);
        sm.append_cost_ledger(&known).await.unwrap();

        let unknown = row("call-unknown", &session.id);
        sm.append_cost_ledger(&unknown).await.unwrap();

        let pool = sm.pool_clone().await.unwrap();
        assert_eq!(
            read_v60(&pool, "call-known").await,
            (
                Some("retry".to_string()),
                Some("tool_result:tc-1".to_string()),
                Some("call-failed".to_string()),
                Some(evidence.as_json().to_string()),
                Some(3),
                Some("mixed".to_string()),
            )
        );
        // A caller that knew nothing: purpose/kind say 'unknown' (a post-v60
        // writer that did not know), the tool count stays NULL (not 0), and
        // nothing is invented for trigger/retry/evidence.
        assert_eq!(
            read_v60(&pool, "call-unknown").await,
            (
                Some("unknown".to_string()),
                None,
                None,
                None,
                None,
                Some("unknown".to_string()),
            )
        );
    }
}
