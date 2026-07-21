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
// `edit` = approve-with-edits: an acceptance that ALSO carries a revised draft
// in `answer_input` (the original lives in `payload.draft`). The delta is
// captured as Brain training by `decision_inbox::learn` (edit-as-training).
const VALID_ANSWERS: &[&str] = &["approve", "reject", "choice", "input", "edit"];

// ── Inbox-service process flag ──────────────────────────────────────────────

/// Whether THIS process serves the Decision Inbox answer path (the daemon's
/// `routes/decisions.rs` over the process-wide `AgentManager`).
///
/// Filing a `tool_approval` decision is only honest when answering it can
/// reach the parked waiter — and the answer path resolves agents through the
/// AgentManager of the process that serves the routes. A CLI session, an
/// example binary, or any other out-of-process population parks in a process
/// the answer path can never reach; a card filed from there is undeliverable
/// by construction (a zombie the user can "answer" to no effect). Those
/// populations keep their own answer surface (e.g. the CLI terminal prompt)
/// and must not file.
static PROCESS_SERVES_INBOX: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Mark this process as the one serving the Decision Inbox answer path.
/// Called exactly once by the daemon at `AppState` assembly (goose-server),
/// right where the decision routes and the shared `AgentManager` are wired.
///
/// Process-wide and irreversible. NEVER call this from `permagent` lib unit
/// tests: they share one test binary, and flipping the flag would poison the
/// flag-unset assertions (the CLI-population tests). Integration test binaries
/// (`tests/*.rs`) each get their own process and may set it freely.
pub fn mark_process_serves_inbox() {
    PROCESS_SERVES_INBOX.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// See [`mark_process_serves_inbox`]. Gates `tool_approval` decision filing.
pub fn process_serves_inbox() -> bool {
    PROCESS_SERVES_INBOX.load(std::sync::atomic::Ordering::Relaxed)
}

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

/// Payload for `kind='automation_proposal'` — the Initiative layer (#360) noticed
/// a repeated command and proposes saving it as an automation. Answered
/// approve/reject; a reject records a recognition bounce so it is never
/// re-pitched (the anti-nag guarantee, carried onto the Decision Inbox surface).
/// Provenance is deterministic: the normalized command and how often it recurred.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomationProposalPayload {
    pub normalized_command: String,
    pub occurrence_count: u64,
    #[serde(default)]
    pub exemplars: Vec<String>,
    /// The agent-drafted proposal text shown to the user, carried so an
    /// approve-with-edits (`answer='edit'`) can diff it against the user's
    /// revision (edit-as-training, `decision_inbox::learn`). Optional: the
    /// anti-nag flywheel and plain approve/reject never read it.
    #[serde(default)]
    pub draft: Option<String>,
}

/// One proposed field in an `enrichment_proposal` (#495 slice 4). The
/// `source_url` is REQUIRED — verifiability is the point of bounding the
/// Enricher to structured fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposedEnrichmentField {
    /// Must be one of [`crate::people::ENRICHABLE_FIELD_NAMES`].
    pub field_name: String,
    pub value: String,
    /// The page where the value was verified.
    pub source_url: String,
}

/// Payload for `kind='enrichment_proposal'` — the Enricher (#495 slice 4)
/// researched a person and proposes structured field values. Review-gated
/// (Approach B): the approve effect writes each field to the person's graph
/// entity via `set_entity_field` with `FieldSource::Enriched`; a reject
/// records and writes nothing. Manual-provenance fields are never overwritten
/// (enforced in Spectral's store, re-checked at apply time).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnrichmentProposalPayload {
    /// Display name at proposal time (for the human reading the inbox).
    pub person_name: String,
    /// Bare 64-hex Spectral `EntityId` of the person's graph node — the write
    /// target, resolved at proposal time so the approve path never re-resolves.
    pub graph_entity_id: String,
    /// Directory row key (`people.entity_uuid`), when known.
    #[serde(default)]
    pub entity_uuid: Option<String>,
    pub fields: Vec<ProposedEnrichmentField>,
}

/// Structural checks beyond serde for an enrichment proposal: at least one
/// field, every field name on the enrichable allowlist, non-empty values and
/// source URLs, and a well-formed 64-hex graph entity id. Failing any of
/// these stores the request as `kind='malformed'` — never coerced (S2).
fn validate_enrichment_payload(p: &EnrichmentProposalPayload) -> Result<(), String> {
    if p.fields.is_empty() {
        return Err("enrichment_proposal requires at least one field".to_string());
    }
    if p.graph_entity_id.len() != 64 || !p.graph_entity_id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!(
            "graph_entity_id must be a bare 64-hex EntityId, got '{}'",
            p.graph_entity_id
        ));
    }
    for f in &p.fields {
        if !crate::people::ENRICHABLE_FIELD_NAMES.contains(&f.field_name.as_str()) {
            return Err(format!(
                "field '{}' is not enrichable (allowed: {})",
                f.field_name,
                crate::people::ENRICHABLE_FIELD_NAMES.join(", ")
            ));
        }
        if f.value.trim().is_empty() {
            return Err(format!("field '{}' has an empty value", f.field_name));
        }
        if f.source_url.trim().is_empty() {
            return Err(format!(
                "field '{}' is missing its source_url",
                f.field_name
            ));
        }
    }
    Ok(())
}

/// Payload for `kind='file_to_project'` — the `file_to_project` platform tool
/// proposes filing content the user is looking at (an email open in the
/// embedded browser, pasted text) onto a project. Review-gated: the approve
/// effect creates a project note through the ONE composed note path
/// ([`crate::project_notes::create_note_indexed`] — durable row, Brain-indexed,
/// Librarian-enriched) and adds the named people to the project ADDRESS-LESS
/// (display name only — email/phone can never ride this payload); a reject
/// records and persists nothing. This decision IS the explicit per-item
/// override of the "browser reads are never persisted" guarantee: content
/// only ever persists through this user-approved seam.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileToProjectPayload {
    /// Resolved project row id — the write target, resolved at proposal time
    /// so the approve path never re-resolves a fuzzy name.
    pub project_id: String,
    /// Project display name at proposal time (for the human reading the inbox).
    pub project_name: String,
    /// Optional note title.
    #[serde(default)]
    pub title: Option<String>,
    /// The full text to file as the note body.
    pub body: String,
    /// Where the content came from, in plain words (e.g. "email open in the
    /// embedded browser"). Provenance for the human reviewing the proposal.
    pub content_origin: String,
    /// Display names of people to add to the project, ADDRESS-LESS. There is
    /// deliberately no field for email/phone — the enrichment hard-forbid on
    /// proposing contact addresses has no exception here.
    #[serde(default)]
    pub people: Vec<String>,
}

/// Structural checks beyond serde for a file_to_project proposal: a non-empty
/// resolved project id/name and body, and non-empty person names. Failing any
/// of these stores the request as `kind='malformed'` — never coerced (S2).
fn validate_file_to_project_payload(p: &FileToProjectPayload) -> Result<(), String> {
    if p.project_id.trim().is_empty() {
        return Err("file_to_project requires a resolved project_id".to_string());
    }
    if p.project_name.trim().is_empty() {
        return Err("file_to_project requires the project's display name".to_string());
    }
    if p.body.trim().is_empty() {
        return Err("file_to_project requires a non-empty body".to_string());
    }
    if p.content_origin.trim().is_empty() {
        return Err(
            "file_to_project requires content_origin (where the content came from)".to_string(),
        );
    }
    for name in &p.people {
        if name.trim().is_empty() {
            return Err(
                "file_to_project people entries must be non-empty display names".to_string(),
            );
        }
    }
    Ok(())
}

/// Payload for `kind='tool_approval'` — an agent turn parked on a needs-approval
/// tool call (GOOSE_MODE `approve`/`smart_approve`). The park lives either on a
/// `ToolConfirmationRouter` oneshot (core tool loop) or inside an
/// ActionRequired-routing provider's own pending map (claude-code / ACP
/// subprocess `can_use_tool` parks). Answering this decision approve/reject
/// delivers the confirmation back to that exact parked await through
/// `Agent::handle_confirmation` — provider first, router fallback — (see
/// `crates/goose-server/src/routes/decisions.rs::deliver_tool_confirmation`),
/// so approve runs the tool and reject skips it. `session_id` + `request_id`
/// are the routing keys: `session_id` selects the per-session Agent, `request_id`
/// is the key the parked waiter registered.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolApprovalPayload {
    /// The chat session whose turn is parked awaiting this confirmation.
    pub session_id: String,
    /// The tool-call request id the `ToolConfirmationRouter` is keyed on.
    pub request_id: String,
    /// Tool the assistant wants to run (e.g. `developer__shell`).
    pub tool_name: String,
    /// Arguments the tool was called with, shown to the approver.
    #[serde(default)]
    pub arguments: serde_json::Value,
    /// Optional note from tool inspection (e.g. a prompt-injection finding).
    #[serde(default)]
    pub security_message: Option<String>,
}

/// Payload for `kind='session_gate'` (S3, #429) — a supervised terminal
/// Claude Code session (epic #399) is BLOCKED on a `can_use_tool` permission
/// gate. Filed by the S2 gate parser's bridge
/// (`agents::platform_extensions::terminal_supervision::bridge_report_to_inbox`)
/// when a `control_request`/`can_use_tool` line is observed in the session's
/// PTY output.
///
/// Routing keys: `target_session_id` (the supervised loop session, `sup-<uuid>`)
/// plus `request_id` (the gate's id INSIDE that session — claude numbers them
/// per-session, so it is NOT globally unique; always match on the pair).
/// `pty_session_id` is the S5 relay address (`write_to_pty`); S5's effect arm
/// consumes it. Until S5 lands, answering this decision records the ruling and
/// returns the exact `control_response` NDJSON line to type into the session's
/// visible terminal tab (the S1/S2 escape hatch) — see
/// [`session_gate_relay_line`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionGatePayload {
    /// Plain-language restatement of what the session is asking.
    pub question: String,
    /// The supervised session id (`sup-<uuid>`) the gate belongs to.
    pub target_session_id: String,
    /// The Tauri PTY id (`pty-<uuid>`) — the S5 relay address. Absent when the
    /// gate was parsed before the tee attached the PTY (first-chunk race).
    #[serde(default)]
    pub pty_session_id: Option<String>,
    /// The gate's `control_request` id — the key a `control_response` must echo.
    pub request_id: String,
    /// Tool the session wants to run (e.g. `Write`, `Bash`).
    pub tool_name: String,
    /// The tool's input object, verbatim from the gate line. S4 classifies it;
    /// an `allow` answer must echo it back as `updatedInput`.
    #[serde(default)]
    pub input: serde_json::Value,
    /// The gate's tool-use id, echoed back in an `allow` (`toolUseID`).
    #[serde(default)]
    pub tool_use_id: Option<String>,
    /// The answers the gate accepts. Fixed vocabulary today: `allow` / `deny`
    /// (the `can_use_tool` protocol's two behaviors).
    pub options: Vec<String>,
}

/// Structural checks beyond serde for a session gate: non-empty routing keys
/// and 2..=8 non-empty options (mirrors `choice`). Failing any of these stores
/// the request as `kind='malformed'` — never coerced (S2).
fn validate_session_gate_payload(p: &SessionGatePayload) -> Result<(), String> {
    for (name, value) in [
        ("question", &p.question),
        ("target_session_id", &p.target_session_id),
        ("request_id", &p.request_id),
        ("tool_name", &p.tool_name),
    ] {
        if value.trim().is_empty() {
            return Err(format!("session_gate requires a non-empty '{name}'"));
        }
    }
    if !(2..=8).contains(&p.options.len()) {
        return Err(format!(
            "session_gate requires 2..=8 options, got {}",
            p.options.len()
        ));
    }
    if p.options.iter().any(|o| o.trim().is_empty()) {
        return Err("session_gate options must be non-empty".to_string());
    }
    Ok(())
}

/// The exact `control_response` NDJSON line that answers this gate on the
/// session's stdin — the wire shape the in-repo protocol implementation
/// (`providers/claude_code.rs`: `ControlResponse` + `PermissionResponse`)
/// already sends in production:
///
/// - allow: `{"type":"control_response","response":{"subtype":"success",
///   "request_id":…,"response":{"behavior":"allow","updatedInput":{…},
///   "toolUseID":…}}}`
/// - deny:  same envelope with `{"behavior":"deny","message":…}`.
///
/// Until S5's relay lands, this line is surfaced to the operator to type into
/// the session's visible terminal tab (whose PTY forwards stdin to the CLI —
/// the S1 `cat '<file>' -` pipeline). S5 will write the same line through
/// `write_to_pty` instead.
pub fn session_gate_relay_line(payload: &SessionGatePayload, allow: bool) -> String {
    let response = if allow {
        serde_json::json!({
            "behavior": "allow",
            // The protocol echoes the (possibly edited) input back; we echo it
            // verbatim — nothing here edits tool input.
            "updatedInput": if payload.input.is_object() {
                payload.input.clone()
            } else {
                serde_json::Value::Object(serde_json::Map::new())
            },
            "toolUseID": payload.tool_use_id.clone().unwrap_or_default(),
        })
    } else {
        serde_json::json!({
            "behavior": "deny",
            "message": "Denied from the Decision Inbox",
        })
    };
    serde_json::json!({
        "type": "control_response",
        "response": {
            "subtype": "success",
            "request_id": payload.request_id,
            "response": response,
        },
    })
    .to_string()
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
    if ![
        "approve_review",
        "unblock",
        "choice",
        "risk_gate",
        "automation_proposal",
        "enrichment_proposal",
        "file_to_project",
        "tool_approval",
        "session_gate",
    ]
    .contains(&req.kind.as_str())
    {
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
        "automation_proposal" => {
            serde_json::from_value::<AutomationProposalPayload>(req.payload.clone())
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
        "enrichment_proposal" => {
            match serde_json::from_value::<EnrichmentProposalPayload>(req.payload.clone()) {
                Ok(p) => validate_enrichment_payload(&p),
                Err(e) => Err(e.to_string()),
            }
        }
        "file_to_project" => {
            match serde_json::from_value::<FileToProjectPayload>(req.payload.clone()) {
                Ok(p) => validate_file_to_project_payload(&p),
                Err(e) => Err(e.to_string()),
            }
        }
        "tool_approval" => serde_json::from_value::<ToolApprovalPayload>(req.payload.clone())
            .map(|_| ())
            .map_err(|e| e.to_string()),
        "session_gate" => match serde_json::from_value::<SessionGatePayload>(req.payload.clone()) {
            Ok(p) => validate_session_gate_payload(&p),
            Err(e) => Err(e.to_string()),
        },
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

/// Close a single OPEN decision as `superseded`: it was resolved through
/// another channel, so the inbox card is moot. Used when the legacy
/// `/action-required/tool-confirmation` prompt answers a request that also has
/// a mirrored `tool_approval` inbox card — leaving that card open would be a
/// zombie whose later answer could do nothing.
///
/// `superseded` (not `answered`) because nobody answered it HERE: the tier
/// gate in [`answer_decision`] rightly refuses a system-actor answer on a
/// Tier-2 row, and `expired` would claim a timeout that never happened. The
/// honest `note` lands in `answer_note` and in a hash-chained audit row
/// (`acted_by='system'`, outcome `superseded: <note>`), so history shows what
/// really resolved it.
///
/// Returns `Ok(false)` when the decision was not open (already answered,
/// expired, superseded, or unknown) — a benign no-op for racing resolvers.
pub async fn supersede_decision(
    pool: &Pool<Sqlite>,
    decision_id: &str,
    note: &str,
) -> Result<bool, String> {
    let decision = match get_decision(pool, decision_id).await? {
        Some(d) => d,
        None => return Ok(false),
    };

    // BEGIN IMMEDIATE for the same reason as `record_effect_outcome`:
    // append_audit_tx reads the audit-chain head before its INSERT, and that
    // read→write upgrade hits an un-retryable BUSY if a concurrent writer
    // commits in between.
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|e| e.to_string())?;

    // Atomic open → superseded: zero rows means someone resolved it first.
    let res = sqlx::query(
        "UPDATE decisions SET status = 'superseded', answer_note = ?, resolved_at = ? \
         WHERE id = ? AND status = 'open'",
    )
    .bind(note)
    .bind(now_timestamp())
    .bind(decision_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    if res.rows_affected() == 0 {
        // Dropping the tx rolls back (nothing was written).
        return Ok(false);
    }

    append_audit_tx(
        &mut tx,
        decision_id,
        decision.goal_id.as_deref(),
        ACTOR_SYSTEM,
        decision.tier,
        &format!("superseded: {}", note),
        None,
    )
    .await?;

    tx.commit().await.map_err(|e| e.to_string())?;

    // Same live signal an answer emits — the card leaves the inbox now, not on
    // the next poll. Consumers key on the event type (refresh), not the answer.
    crate::events::emit(crate::events::decision_resolved(
        decision_id,
        &decision.kind,
        "superseded",
        ACTOR_SYSTEM,
        decision.tier,
    ));

    Ok(true)
}

/// Find the open `tool_approval` decision whose payload carries `request_id`
/// (the `ToolConfirmationRouter` key), if any. Lets the legacy per-tool
/// prompt locate the mirrored inbox card it is about to make moot.
pub async fn find_open_tool_approval_by_request_id(
    pool: &Pool<Sqlite>,
    request_id: &str,
) -> Result<Option<Decision>, String> {
    let row = sqlx::query(&format!(
        "SELECT {} FROM decisions \
         WHERE kind = 'tool_approval' AND status = 'open' \
           AND json_extract(payload_json, '$.request_id') = ? \
         ORDER BY created_at DESC LIMIT 1",
        DECISION_COLUMNS
    ))
    .bind(request_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(row.as_ref().map(row_to_decision))
}

/// Find the open `session_gate` decision for a gate, addressed by the
/// (`target_session_id`, `request_id`) PAIR — claude numbers gate request ids
/// per-session (`perm_1`, `perm_2`, …), so `request_id` alone can collide
/// across concurrent supervised sessions. Used by the S3 bridge both to
/// dedupe filing (a re-observed gate line must not double-escalate) and to
/// locate the card to supersede when the gate resolves outside the inbox
/// (hand-typed answer, session end).
pub async fn find_open_session_gate(
    pool: &Pool<Sqlite>,
    target_session_id: &str,
    request_id: &str,
) -> Result<Option<Decision>, String> {
    let row = sqlx::query(&format!(
        "SELECT {} FROM decisions \
         WHERE kind = 'session_gate' AND status = 'open' \
           AND json_extract(payload_json, '$.target_session_id') = ? \
           AND json_extract(payload_json, '$.request_id') = ? \
         ORDER BY created_at DESC LIMIT 1",
        DECISION_COLUMNS
    ))
    .bind(target_session_id)
    .bind(request_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(row.as_ref().map(row_to_decision))
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
    // #464/#515: ONE definition of "in flight" everywhere. This must match
    // `cards::list_active_goals` (the `/api/goals/active` source the dashboard
    // uses): ACTIVE_BINDINGS states, parked (needs_human_attention) and
    // archived excluded. It previously counted only `in_progress` without the
    // parked filter — a different number than the list right next to it.
    let placeholders = vec!["?"; crate::goal_state::GoalState::ACTIVE_BINDINGS.len()].join(", ");
    let goals_sql = format!(
        "SELECT COUNT(*) FROM cards c JOIN board_columns bc ON c.column_id = bc.id \
         WHERE c.card_type = 'goal' AND c.archived_at IS NULL \
           AND bc.state_binding IN ({}) \
           AND COALESCE(json_extract(c.metadata_json, '$.needs_human_attention'), 0) = 0",
        placeholders
    );
    let mut goals_q = sqlx::query_scalar(&goals_sql);
    for b in crate::goal_state::GoalState::ACTIVE_BINDINGS {
        goals_q = goals_q.bind(*b);
    }
    let goals_in_flight: i64 = goals_q.fetch_one(pool).await.map_err(|e| e.to_string())?;
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
            "answer must be one of approve|reject|choice|input|edit, got '{}'",
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
            // approve-with-edits carries the revised draft in input_text.
            if answer.answer == "edit" && answer.input_text.as_deref().unwrap_or("").is_empty() {
                return Err(AnswerError::Invalid(
                    "answer 'edit' requires input_text (the revised draft)".to_string(),
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
    hex::encode(hasher.finalize())
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
    // BEGIN IMMEDIATE: append_audit_tx reads the audit-chain head before its
    // INSERT, so this write-back would hit an un-retryable BUSY lock-upgrade if a
    // concurrent writer commits in between; take the write lock up front.
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|e| e.to_string())?;
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

    fn valid_file_to_project() -> NewDecision {
        NewDecision {
            kind: "file_to_project".to_string(),
            project_id: None,
            headline: Some("File an email to \"Acme\"".to_string()),
            detail: Some("Source: email open in the embedded browser".to_string()),
            payload: serde_json::json!({
                "project_id": "proj-1",
                "project_name": "Acme",
                "title": "Email from Dana",
                "body": "Hi — can we move the call to Thursday?",
                "content_origin": "email open in the embedded browser",
                "people": ["Dana Example"]
            }),
            ..Default::default()
        }
    }

    /// The file_to_project kind is accepted with a typed payload and, with no
    /// seeded risk_policy class, resolves fail-closed to Tier 2.
    #[tokio::test]
    async fn file_to_project_created_at_tier2_fail_closed() {
        let pool = test_pool().await;
        let d = create_decision(&pool, valid_file_to_project())
            .await
            .unwrap();
        assert_eq!(d.kind, "file_to_project");
        assert_eq!(
            d.tier, 2,
            "unseeded action class must fail closed to Tier 2"
        );
        assert_eq!(d.status, "open");
        assert_eq!(
            d.payload
                .get("people")
                .and_then(|v| v.as_array())
                .map(|a| a.len()),
            Some(1)
        );
    }

    /// Structural failures (empty body, empty person name) and payload fields
    /// beyond the schema (e.g. a smuggled email address field) are stored as
    /// malformed — never coerced. The payload cannot carry addresses at all:
    /// deny_unknown_fields rejects any such field outright.
    #[tokio::test]
    async fn file_to_project_bad_payloads_are_malformed() {
        let pool = test_pool().await;

        let mut req = valid_file_to_project();
        req.payload["body"] = serde_json::json!("   ");
        let d = create_decision(&pool, req).await.unwrap();
        assert_eq!(d.kind, "malformed");

        let mut req = valid_file_to_project();
        req.payload["people"] = serde_json::json!(["  "]);
        let d = create_decision(&pool, req).await.unwrap();
        assert_eq!(d.kind, "malformed");

        let mut req = valid_file_to_project();
        req.payload["email"] = serde_json::json!("dana@example.com");
        let d = create_decision(&pool, req).await.unwrap();
        assert_eq!(
            d.kind, "malformed",
            "address-shaped fields must be rejected"
        );
        assert_eq!(
            d.payload.get("original_kind").and_then(|v| v.as_str()),
            Some("file_to_project")
        );
    }

    fn valid_tool_approval() -> NewDecision {
        NewDecision {
            kind: "tool_approval".to_string(),
            headline: Some("Approve tool call: developer__shell".to_string()),
            detail: Some("The assistant wants to run 'developer__shell'".to_string()),
            payload: serde_json::json!({
                "session_id": "sess-1",
                "request_id": "req-1",
                "tool_name": "developer__shell",
                "arguments": {"command": "ls -la"},
            }),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn tool_approval_created_at_tier2_fail_closed() {
        let pool = test_pool().await;
        let d = create_decision(&pool, valid_tool_approval()).await.unwrap();
        assert_eq!(d.kind, "tool_approval");
        // No seeded risk_policy class for tool_approval → fail-closed to Tier 2,
        // so only 'jesse' (the human) can answer it — never henry-policy/system.
        assert_eq!(d.tier, 2);
        assert_eq!(d.status, "open");
        // Routing keys round-trip through the stored payload.
        assert_eq!(
            d.payload.get("request_id").and_then(|v| v.as_str()),
            Some("req-1")
        );
        assert_eq!(
            d.payload.get("session_id").and_then(|v| v.as_str()),
            Some("sess-1")
        );
    }

    #[tokio::test]
    async fn tool_approval_missing_routing_keys_is_malformed() {
        let pool = test_pool().await;
        let mut req = valid_tool_approval();
        // Missing request_id + tool_name violates the typed payload schema.
        req.payload = serde_json::json!({"session_id": "sess-1"});
        let d = create_decision(&pool, req).await.unwrap();
        assert_eq!(d.kind, "malformed");
        assert_eq!(d.tier, 2);
        assert_eq!(
            d.payload.get("original_kind").and_then(|v| v.as_str()),
            Some("tool_approval")
        );
    }

    // ── Supersede (single decision, legacy-prompt desync fix) ──

    fn tool_approval_with_request_id(request_id: &str) -> NewDecision {
        let mut req = valid_tool_approval();
        req.payload = serde_json::json!({
            "session_id": "sess-1",
            "request_id": request_id,
            "tool_name": "developer__shell",
            "arguments": {"command": "ls -la"},
        });
        req
    }

    /// The legacy-prompt path: an open tool_approval closes as `superseded`
    /// (never `answered` — nobody answered it here) with the honest note on the
    /// row AND on a hash-chained audit row, so it leaves the inbox and still
    /// shows up in history with what really resolved it.
    #[tokio::test]
    async fn supersede_decision_closes_open_row_with_audited_note() {
        let pool = test_pool().await;
        let d = create_decision(&pool, valid_tool_approval()).await.unwrap();

        let note = "answered via the legacy per-tool prompt";
        assert!(supersede_decision(&pool, &d.id, note).await.unwrap());

        let after = get_decision(&pool, &d.id).await.unwrap().unwrap();
        assert_eq!(after.status, "superseded");
        assert_eq!(after.answer, None, "superseded is not an answer");
        assert_eq!(after.acted_by, None, "no human/policy actor answered it");
        assert_eq!(after.answer_note.as_deref(), Some(note));
        assert!(after.resolved_at.is_some());

        // Audit row: system actor, outcome carries the note.
        let (acted_by, outcome): (String, String) = sqlx::query_as(
            "SELECT acted_by, outcome FROM decision_audit WHERE decision_id = ? \
             ORDER BY seq DESC LIMIT 1",
        )
        .bind(&d.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(acted_by, ACTOR_SYSTEM);
        assert_eq!(outcome, format!("superseded: {}", note));

        // It left the open inbox.
        assert!(list_open_decisions(&pool)
            .await
            .unwrap()
            .iter()
            .all(|i| i.decision.id != d.id));

        // And it can no longer be answered — the zombie card is dead for real.
        let err = answer_decision(
            &pool,
            &d.id,
            &DecisionAnswer {
                answer: "approve".to_string(),
                ..Default::default()
            },
            ACTOR_JESSE,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, AnswerError::AlreadyResolved(ref s) if s == "superseded"),
            "expected AlreadyResolved(superseded), got {:?}",
            err
        );
    }

    /// Racing resolvers: superseding a non-open or unknown decision is a benign
    /// no-op that writes no audit row.
    #[tokio::test]
    async fn supersede_decision_is_noop_when_not_open() {
        let pool = test_pool().await;

        // Unknown id.
        assert!(!supersede_decision(&pool, "no-such-id", "note")
            .await
            .unwrap());

        // Already answered via the inbox (inbox-first, legacy-second).
        let d = create_decision(&pool, valid_tool_approval()).await.unwrap();
        answer_decision(
            &pool,
            &d.id,
            &DecisionAnswer {
                answer: "approve".to_string(),
                ..Default::default()
            },
            ACTOR_JESSE,
        )
        .await
        .unwrap();
        let audit_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM decision_audit")
            .fetch_one(&pool)
            .await
            .unwrap();

        assert!(!supersede_decision(&pool, &d.id, "note").await.unwrap());

        let after = get_decision(&pool, &d.id).await.unwrap().unwrap();
        assert_eq!(after.status, "answered", "answered row must be untouched");
        assert_eq!(after.answer.as_deref(), Some("approve"));
        let audit_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM decision_audit")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(audit_before, audit_after, "no-op must not write audit rows");
    }

    /// Superseded decisions stay visible in history via their audit row (the
    /// history query joins decision_audit; a status flip alone would vanish).
    #[tokio::test]
    async fn superseded_decision_appears_in_history() {
        let pool = test_pool().await;
        let d = create_decision(&pool, valid_tool_approval()).await.unwrap();
        supersede_decision(&pool, &d.id, "answered via the legacy per-tool prompt")
            .await
            .unwrap();

        let items = decision_history(&pool, 50, None).await.unwrap();
        let item = items
            .iter()
            .find(|i| i.decision.id == d.id)
            .expect("superseded decision must appear in history");
        assert_eq!(item.audit_acted_by, ACTOR_SYSTEM);
        assert!(item.outcome.starts_with("superseded:"), "{}", item.outcome);
    }

    #[tokio::test]
    async fn find_open_tool_approval_by_request_id_matches_open_only() {
        let pool = test_pool().await;
        let d1 = create_decision(&pool, tool_approval_with_request_id("req-A"))
            .await
            .unwrap();
        let d2 = create_decision(&pool, tool_approval_with_request_id("req-B"))
            .await
            .unwrap();

        let found = find_open_tool_approval_by_request_id(&pool, "req-A")
            .await
            .unwrap()
            .expect("open req-A must be found");
        assert_eq!(found.id, d1.id);

        // Unknown request_id → none.
        assert!(find_open_tool_approval_by_request_id(&pool, "req-Z")
            .await
            .unwrap()
            .is_none());

        // Once superseded it is no longer a candidate; req-B is untouched.
        supersede_decision(&pool, &d1.id, "answered via the legacy per-tool prompt")
            .await
            .unwrap();
        assert!(find_open_tool_approval_by_request_id(&pool, "req-A")
            .await
            .unwrap()
            .is_none());
        assert_eq!(
            find_open_tool_approval_by_request_id(&pool, "req-B")
                .await
                .unwrap()
                .unwrap()
                .id,
            d2.id
        );
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
    async fn automation_proposal_is_valid_and_user_only() {
        let pool = test_pool().await;
        let req = NewDecision {
            kind: "automation_proposal".to_string(),
            project_id: Some(PERSONAL_PROJECT_ID.to_string()),
            headline: Some("Automate your morning git sync?".to_string()),
            detail: Some("You've run `git status && git pull` 3 times.".to_string()),
            payload: serde_json::json!({
                "normalized_command": "git status && git pull",
                "occurrence_count": 3,
                "exemplars": ["git status && git pull"]
            }),
            ..Default::default()
        };
        let d = create_decision(&pool, req).await.unwrap();
        assert_eq!(
            d.kind, "automation_proposal",
            "a real proposal, not coerced"
        );
        assert_eq!(d.tier, 2, "no seeded class → user-only (fail-closed)");
        assert_eq!(d.status, "open");
        assert_eq!(
            d.payload["normalized_command"],
            serde_json::json!("git status && git pull")
        );
    }

    #[tokio::test]
    async fn automation_proposal_rejects_unknown_payload_fields() {
        let pool = test_pool().await;
        let req = NewDecision {
            kind: "automation_proposal".to_string(),
            headline: Some("Automate something?".to_string()),
            detail: Some("detail".to_string()),
            payload: serde_json::json!({
                "normalized_command": "ls",
                "occurrence_count": 3,
                "bogus": true
            }),
            ..Default::default()
        };
        let d = create_decision(&pool, req).await.unwrap();
        assert_eq!(d.kind, "malformed", "deny_unknown_fields holds");
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

    // ── Enrichment proposals (#495 slice 4) ──

    fn enrichment_fields(fields: serde_json::Value) -> NewDecision {
        NewDecision {
            kind: "enrichment_proposal".to_string(),
            headline: Some("Approve enriched details for Jane Doe".to_string()),
            detail: Some("linkedin: … (source: …)".to_string()),
            payload: serde_json::json!({
                "person_name": "Jane Doe",
                "graph_entity_id": "ab".repeat(32),
                "entity_uuid": "0197-fake",
                "fields": fields,
            }),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn enrichment_proposal_is_valid_and_user_only() {
        let pool = test_pool().await;
        let req = enrichment_fields(serde_json::json!([
            {
                "field_name": "linkedin",
                "value": "https://www.linkedin.com/in/janedoe",
                "source_url": "https://www.linkedin.com/in/janedoe"
            },
            {
                "field_name": "company",
                "value": "Acme Corp",
                "source_url": "https://acme.example/about"
            }
        ]));
        let d = create_decision(&pool, req).await.unwrap();
        assert_eq!(
            d.kind, "enrichment_proposal",
            "a real proposal, not coerced"
        );
        assert_eq!(d.tier, 2, "no seeded class → user-only (fail-closed)");
        assert_eq!(d.status, "open");
        assert_eq!(
            d.payload["fields"][1]["value"],
            serde_json::json!("Acme Corp")
        );
    }

    #[tokio::test]
    async fn enrichment_proposal_rejects_manual_only_field() {
        let pool = test_pool().await;
        // email is manual-only — OFF LIMITS to enrichment by ruling.
        let req = enrichment_fields(serde_json::json!([
            {
                "field_name": "email",
                "value": "jane@example.com",
                "source_url": "https://acme.example/team"
            }
        ]));
        let d = create_decision(&pool, req).await.unwrap();
        assert_eq!(d.kind, "malformed", "manual-only field must be rejected");
    }

    #[tokio::test]
    async fn enrichment_proposal_rejects_empty_fields_and_missing_source() {
        let pool = test_pool().await;
        let empty = enrichment_fields(serde_json::json!([]));
        let d = create_decision(&pool, empty).await.unwrap();
        assert_eq!(d.kind, "malformed", "zero fields is not a proposal");

        let no_source = enrichment_fields(serde_json::json!([
            { "field_name": "job_title", "value": "CTO", "source_url": "  " }
        ]));
        let d = create_decision(&pool, no_source).await.unwrap();
        assert_eq!(d.kind, "malformed", "source_url is required per field");
    }

    #[tokio::test]
    async fn enrichment_proposal_rejects_bad_entity_id_and_unknown_payload_fields() {
        let pool = test_pool().await;
        let mut bad_id = enrichment_fields(serde_json::json!([
            { "field_name": "company", "value": "Acme", "source_url": "https://a.example" }
        ]));
        bad_id.payload["graph_entity_id"] = serde_json::json!("not-hex");
        let d = create_decision(&pool, bad_id).await.unwrap();
        assert_eq!(d.kind, "malformed", "graph_entity_id must be 64-hex");

        let mut extra = enrichment_fields(serde_json::json!([
            { "field_name": "company", "value": "Acme", "source_url": "https://a.example" }
        ]));
        extra.payload["bogus"] = serde_json::json!(true);
        let d = create_decision(&pool, extra).await.unwrap();
        assert_eq!(d.kind, "malformed", "deny_unknown_fields holds");
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

    // ── approve-with-edits (edit-as-training) ──

    fn draft_proposal() -> NewDecision {
        NewDecision {
            kind: "automation_proposal".to_string(),
            project_id: Some(PERSONAL_PROJECT_ID.to_string()),
            headline: Some("Automate your morning git sync?".to_string()),
            detail: Some("You've run `git status && git pull` 3 times.".to_string()),
            payload: serde_json::json!({
                "normalized_command": "git status && git pull",
                "occurrence_count": 3,
                "exemplars": ["git status && git pull"],
                "draft": "git status && git pull",
            }),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn draft_field_is_accepted_not_coerced_to_malformed() {
        let pool = test_pool().await;
        let d = create_decision(&pool, draft_proposal()).await.unwrap();
        assert_eq!(
            d.kind, "automation_proposal",
            "payload.draft must not trip deny_unknown_fields"
        );
        assert_eq!(
            d.payload["draft"],
            serde_json::json!("git status && git pull")
        );
    }

    #[tokio::test]
    async fn edit_answer_stores_revised_input_and_is_accepted() {
        let pool = test_pool().await;
        let d = create_decision(&pool, draft_proposal()).await.unwrap();

        let ans = DecisionAnswer {
            answer: "edit".to_string(),
            input_text: Some("git status && git pull --rebase".to_string()),
            ..Default::default()
        };
        let (answered, proof) = answer_decision(&pool, &d.id, &ans, ACTOR_JESSE)
            .await
            .unwrap();
        assert_eq!(answered.status, "answered");
        assert_eq!(answered.answer.as_deref(), Some("edit"));
        assert_eq!(
            answered.answer_input.as_deref(),
            Some("git status && git pull --rebase"),
            "the revised draft is stored in answer_input"
        );
        // The original draft is untouched in the payload — the delta is diffable.
        assert_eq!(
            answered.payload["draft"],
            serde_json::json!("git status && git pull")
        );
        assert_eq!(proof.answer(), "edit");
    }

    #[tokio::test]
    async fn edit_answer_requires_input_text() {
        let pool = test_pool().await;
        let d = create_decision(&pool, draft_proposal()).await.unwrap();
        // An edit with no revision has nothing to accept or learn.
        let ans = DecisionAnswer {
            answer: "edit".to_string(),
            ..Default::default()
        };
        let err = answer_decision(&pool, &d.id, &ans, ACTOR_JESSE)
            .await
            .unwrap_err();
        assert!(
            matches!(err, AnswerError::Invalid(_)),
            "edit without input_text must be Invalid: {:?}",
            err
        );
    }

    #[tokio::test]
    async fn edit_answer_rejected_on_choice_kind() {
        let pool = test_pool().await;
        let req = NewDecision {
            kind: "choice".to_string(),
            headline: Some("Pick a colour for the new room".to_string()),
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
        // You pick a choice; you don't edit it.
        let ans = DecisionAnswer {
            answer: "edit".to_string(),
            input_text: Some("purple".to_string()),
            ..Default::default()
        };
        let err = answer_decision(&pool, &d.id, &ans, ACTOR_JESSE)
            .await
            .unwrap_err();
        assert!(matches!(err, AnswerError::Invalid(_)));
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

    // ── session_gate (S3, #429) ──

    fn session_gate_payload_json(session: &str, request: &str) -> serde_json::Value {
        serde_json::json!({
            "question": "Allow the session to run Write?",
            "target_session_id": session,
            "pty_session_id": "pty-1",
            "request_id": request,
            "tool_name": "Write",
            "input": {"path": "foo.txt", "content": "hello"},
            "tool_use_id": "tu_1",
            "options": ["allow", "deny"],
        })
    }

    fn valid_session_gate(session: &str, request: &str) -> NewDecision {
        NewDecision {
            kind: "session_gate".to_string(),
            headline: Some("A terminal session is waiting for permission".to_string()),
            detail: Some("Write {\"path\":\"foo.txt\"}".to_string()),
            payload: session_gate_payload_json(session, request),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn session_gate_created_at_tier2_fail_closed() {
        let pool = test_pool().await;
        let d = create_decision(&pool, valid_session_gate("sup-a", "perm_1"))
            .await
            .unwrap();
        assert_eq!(d.kind, "session_gate");
        // No seeded risk_policy class for 'session_gate' (S4 adds the
        // tool→action_class mapping) → fail-closed to Tier 2, human-only.
        assert_eq!(d.tier, 2);
        assert_eq!(d.payload["request_id"], "perm_1");
        assert_eq!(d.payload["target_session_id"], "sup-a");
    }

    #[tokio::test]
    async fn session_gate_missing_routing_keys_is_malformed() {
        let pool = test_pool().await;
        let mut req = valid_session_gate("sup-a", "perm_1");
        req.payload = serde_json::json!({
            "question": "Allow?",
            "tool_name": "Write",
            "options": ["allow", "deny"],
        });
        let d = create_decision(&pool, req).await.unwrap();
        assert_eq!(d.kind, "malformed");
        assert_eq!(
            d.payload.get("original_kind").and_then(|v| v.as_str()),
            Some("session_gate")
        );
    }

    #[tokio::test]
    async fn session_gate_with_too_few_options_is_malformed() {
        let pool = test_pool().await;
        let mut req = valid_session_gate("sup-a", "perm_1");
        req.payload["options"] = serde_json::json!(["allow"]);
        let d = create_decision(&pool, req).await.unwrap();
        assert_eq!(d.kind, "malformed");
    }

    /// The finder must match on the (session, request) PAIR: claude's gate ids
    /// are per-session (`perm_1`, `perm_2`, …), so two concurrent sessions can
    /// both be blocked on a `perm_1`.
    #[tokio::test]
    async fn find_open_session_gate_matches_on_session_and_request_pair() {
        let pool = test_pool().await;
        let a = create_decision(&pool, valid_session_gate("sup-a", "perm_1"))
            .await
            .unwrap();
        let b = create_decision(&pool, valid_session_gate("sup-b", "perm_1"))
            .await
            .unwrap();

        let found = find_open_session_gate(&pool, "sup-a", "perm_1")
            .await
            .unwrap()
            .expect("open gate for sup-a must be found");
        assert_eq!(found.id, a.id);
        let found_b = find_open_session_gate(&pool, "sup-b", "perm_1")
            .await
            .unwrap()
            .expect("open gate for sup-b must be found");
        assert_eq!(found_b.id, b.id);
        assert!(find_open_session_gate(&pool, "sup-c", "perm_1")
            .await
            .unwrap()
            .is_none());

        // Superseding closes it for the finder (open-only contract).
        assert!(supersede_decision(&pool, &a.id, "answered in the terminal")
            .await
            .unwrap());
        assert!(find_open_session_gate(&pool, "sup-a", "perm_1")
            .await
            .unwrap()
            .is_none());
    }

    /// The relay line must match the wire shape the in-repo protocol
    /// implementation (`providers/claude_code.rs`) sends in production.
    #[test]
    fn session_gate_relay_line_matches_protocol_wire_shape() {
        let payload: SessionGatePayload =
            serde_json::from_value(session_gate_payload_json("sup-a", "perm_1")).unwrap();

        let allow: serde_json::Value =
            serde_json::from_str(&session_gate_relay_line(&payload, true)).unwrap();
        assert_eq!(allow["type"], "control_response");
        assert_eq!(allow["response"]["subtype"], "success");
        assert_eq!(allow["response"]["request_id"], "perm_1");
        assert_eq!(allow["response"]["response"]["behavior"], "allow");
        assert_eq!(
            allow["response"]["response"]["updatedInput"],
            serde_json::json!({"path": "foo.txt", "content": "hello"})
        );
        assert_eq!(allow["response"]["response"]["toolUseID"], "tu_1");

        let deny: serde_json::Value =
            serde_json::from_str(&session_gate_relay_line(&payload, false)).unwrap();
        assert_eq!(deny["response"]["request_id"], "perm_1");
        assert_eq!(deny["response"]["response"]["behavior"], "deny");
        assert!(deny["response"]["response"]["message"]
            .as_str()
            .unwrap()
            .contains("Denied"));

        // A non-object input must degrade to an empty updatedInput object,
        // never a type error on the session's stdin.
        let mut odd = payload.clone();
        odd.input = serde_json::json!("not-an-object");
        let allow_odd: serde_json::Value =
            serde_json::from_str(&session_gate_relay_line(&odd, true)).unwrap();
        assert!(allow_odd["response"]["response"]["updatedInput"].is_object());
    }
}
