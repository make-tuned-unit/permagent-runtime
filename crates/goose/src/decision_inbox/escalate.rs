//! The `escalate` tool: parameter schema, validation, and mapping to a
//! typed [`DecisionDraft`].
//!
//! Persistence is behind the [`DecisionSink`] trait — Part A ships an
//! in-memory implementation; Part B installs a sink backed by Lane L1's
//! decisions table.
//!
//! Security note (S2): every string in the payload is DATA. It is stored
//! verbatim, never rendered as markdown, and never treated as instructions.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ── Caps (Phase 0 schema) ───────────────────────────────────────────────

pub const MAX_SPECIFIC_ASK_CHARS: usize = 140;
pub const MAX_WHY_BLOCKED_CHARS: usize = 2000;
pub const MAX_EVIDENCE_REFS: usize = 10;
pub const MAX_EVIDENCE_REF_CHARS: usize = 512;
pub const MIN_OPTIONS: usize = 2;
pub const MAX_OPTIONS: usize = 5;
pub const MAX_OPTION_ID_CHARS: usize = 32;
pub const MAX_OPTION_LABEL_CHARS: usize = 80;
pub const MAX_OPTION_CONSEQUENCE_CHARS: usize = 280;

/// Amendment A1: the user-facing headline is a plain-language outcome
/// statement, hard-capped at 80 characters.
pub const MAX_HEADLINE_CHARS: usize = 80;

/// Amendment A2: user-facing fallback when no roster name is known.
pub const ANONYMOUS_WORKER_DISPLAY: &str = "one of Henry's workers";

// ── Tool parameters (Phase 0 schema, emitted via schemars) ──────────────

/// What is being asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum EscalationKind {
    Credential,
    Decision,
    Capability,
    Information,
    Approval,
}

/// Resume behavior after the decision is acted on. Phase 1 supports only "auto".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ResumeMode {
    Auto,
}

/// One selectable option for a `kind=decision` escalation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EscalationOption {
    /// Stable slug for this option: lowercase letters, digits, hyphens. Max 32 chars.
    pub id: String,
    /// Short human-readable label. Max 80 chars.
    pub label: String,
    /// What happens if this option is chosen. Max 280 chars.
    pub consequence: String,
}

/// Parameters for the `escalate` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EscalateParams {
    /// What is being asked for: "credential", "decision", "capability",
    /// "information", or "approval".
    pub kind: EscalationKind,
    /// The one-line "add X so I can proceed" ask. This becomes the
    /// plain-language headline Jesse sees: max 80 characters, no PR numbers,
    /// branch names, file counts, or internal IDs — technical identifiers
    /// belong in why_blocked and evidence_refs.
    pub specific_ask: String,
    /// Why work cannot continue without it (technical detail). Max 2000 chars.
    pub why_blocked: String,
    /// Opaque references: session ids, card ids, file paths, URLs. NOT prose.
    /// Max 10 items, each max 512 chars.
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    /// Required iff kind == "decision": 2 to 5 options to choose between.
    #[serde(default)]
    pub options: Option<Vec<EscalationOption>>,
    /// Resume behavior after the decision is acted on. Only "auto" is supported.
    pub resume: ResumeMode,
}

// ── DecisionDraft (typed validation output) ─────────────────────────────

/// Decision kind in Lane L1's decisions-table vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DecisionKind {
    /// `escalate.kind = approval`
    ApproveReview,
    /// `escalate.kind = decision` (carries options)
    Choice,
    /// `escalate.kind = credential | information`
    Unblock,
    /// `escalate.kind = capability` — always Tier 2.
    RiskGate,
    /// Payload failed validation — raw payload + error list, always Tier 2.
    Malformed,
}

impl DecisionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            DecisionKind::ApproveReview => "approve_review",
            DecisionKind::Choice => "choice",
            DecisionKind::Unblock => "unblock",
            DecisionKind::RiskGate => "risk_gate",
            DecisionKind::Malformed => "malformed",
        }
    }

    /// Minimum tier for this kind. The final tier is computed by L1 at row
    /// creation; this floor is the part L3 owns: risk gates and malformed
    /// escalations are never below Tier 2 (human eyes required).
    pub fn minimum_tier(&self) -> u8 {
        match self {
            DecisionKind::RiskGate | DecisionKind::Malformed => 2,
            _ => 1,
        }
    }
}

/// Where the escalation came from. Internal IDs stay in evidence fields;
/// only `worker_roster_name` may surface to the user (Amendment A2).
#[derive(Debug, Clone, Default)]
pub struct DraftSource {
    /// Session that issued the tool call (internal ID — evidence only).
    pub session_id: Option<String>,
    /// Worker name from the roster (agent.yaml), if known. User-facing.
    pub worker_roster_name: Option<String>,
}

/// Typed, validated decision draft — the L3 → L1 contract object.
///
/// Amendment A1: `headline` is the plain-language outcome statement
/// (max 80 chars, no technical identifiers); `detail` carries the
/// technical content.
#[derive(Debug, Clone)]
pub struct DecisionDraft {
    pub kind: DecisionKind,
    /// Plain-language outcome statement, max 80 chars (Amendment A1).
    pub headline: String,
    /// Technical detail: why blocked, identifiers, validation errors.
    pub detail: String,
    pub evidence_refs: Vec<String>,
    /// Non-empty only for `DecisionKind::Choice`.
    pub options: Vec<EscalationOption>,
    pub resume: ResumeMode,
    /// Tier floor (final tier computed by L1).
    pub minimum_tier: u8,
    /// Internal source session id (evidence only — never user-facing).
    pub source_session_id: Option<String>,
    /// User-facing attribution: roster name or the anonymous fallback (A2).
    pub requested_by_display: String,
    /// The raw payload, kept verbatim for `Malformed` drafts.
    pub raw_payload: Option<serde_json::Value>,
    /// Validation errors — non-empty iff kind is `Malformed`.
    pub validation_errors: Vec<String>,
}

// ── Validation ──────────────────────────────────────────────────────────

/// Outcome of validating a raw escalate payload.
#[derive(Debug, Clone)]
pub enum EscalationOutcome {
    Valid(Box<EscalateParams>),
    Malformed { errors: Vec<String> },
}

/// Amendment A2 helper: map an optional roster name to the user-facing
/// display string. Never returns an internal worker ID.
pub fn worker_display_name(roster_name: Option<&str>) -> String {
    match roster_name {
        Some(n) if !n.trim().is_empty() => n.trim().to_string(),
        _ => ANONYMOUS_WORKER_DISPLAY.to_string(),
    }
}

/// Check a candidate headline against the Amendment A1 plain-language
/// rules. Returns an empty vec when the text is acceptable.
///
/// Rules (deterministic heuristics, pinned by tests):
/// - non-empty, at most 80 characters
/// - no PR/issue numbers (`#123`, `PR 123`)
/// - no path- or branch-like tokens (anything containing `/` or `\`)
/// - no file counts (`12 files`)
/// - no internal IDs (UUIDs, long hex identifiers)
pub fn headline_violations(text: &str) -> Vec<String> {
    let mut violations = Vec::new();
    let trimmed = text.trim();

    if trimmed.is_empty() {
        violations.push("headline is empty".to_string());
        return violations;
    }
    if trimmed.chars().count() > MAX_HEADLINE_CHARS {
        violations.push(format!(
            "headline exceeds {} characters",
            MAX_HEADLINE_CHARS
        ));
    }

    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    for (i, raw_token) in tokens.iter().enumerate() {
        let token = raw_token.trim_matches(|c: char| ",.;:()[]{}'\"!?".contains(c));
        if token.is_empty() {
            continue;
        }
        if token.contains('/') || token.contains('\\') {
            violations.push(format!(
                "headline contains a path, branch name, or URL ('{}')",
                token
            ));
            continue;
        }
        if contains_issue_ref(token) {
            violations.push(format!("headline contains a PR/issue number ('{}')", token));
            continue;
        }
        if looks_like_internal_id(token) {
            violations.push(format!("headline contains an internal ID ('{}')", token));
            continue;
        }
        // "PR 123" / "12 files" patterns need the next token.
        if let Some(next) = tokens.get(i + 1) {
            let next = next.trim_matches(|c: char| ",.;:()[]{}'\"!?".contains(c));
            let token_is_number = !token.is_empty() && token.chars().all(|c| c.is_ascii_digit());
            if token.eq_ignore_ascii_case("pr")
                && !next.is_empty()
                && next.chars().all(|c| c.is_ascii_digit())
            {
                violations.push(format!("headline contains a PR number ('{} {}')", token, next));
            } else if token_is_number && next.to_ascii_lowercase().starts_with("file") {
                violations.push(format!("headline contains a file count ('{} {}')", token, next));
            }
        }
    }

    violations
}

/// `#` immediately followed by a digit anywhere in the token.
fn contains_issue_ref(token: &str) -> bool {
    let bytes = token.as_bytes();
    bytes
        .windows(2)
        .any(|w| w[0] == b'#' && w[1].is_ascii_digit())
}

/// UUIDs and hex-ish identifiers (commit SHAs, session ids).
fn looks_like_internal_id(token: &str) -> bool {
    // UUID shape: 8-4-4-4-12 hex groups.
    let parts: Vec<&str> = token.split('-').collect();
    if parts.len() == 5 {
        let lens = [8usize, 4, 4, 4, 12];
        if parts
            .iter()
            .zip(lens.iter())
            .all(|(p, &l)| p.len() == l && p.chars().all(|c| c.is_ascii_hexdigit()))
        {
            return true;
        }
    }
    // Long hex run (>= 12 hex chars) anywhere in the token.
    let mut run = 0usize;
    for c in token.chars() {
        if c.is_ascii_hexdigit() {
            run += 1;
            if run >= 12 {
                return true;
            }
        } else {
            run = 0;
        }
    }
    // Hex-only token of SHA-ish length that contains at least one digit
    // (avoids flagging hex-only English words like "deadbeef"-free prose).
    let len = token.chars().count();
    (7..=40).contains(&len)
        && token.chars().all(|c| c.is_ascii_hexdigit())
        && token.chars().any(|c| c.is_ascii_digit())
}

/// Validate a raw escalate payload. A payload that deserializes as JSON but
/// fails schema or invariant checks is reported as `Malformed` — it is
/// recorded for human review, never dropped.
pub fn validate_escalation(raw: &serde_json::Value) -> EscalationOutcome {
    let params: EscalateParams = match serde_json::from_value(raw.clone()) {
        Ok(p) => p,
        Err(e) => {
            return EscalationOutcome::Malformed {
                errors: vec![format!("payload does not match the escalate schema: {}", e)],
            };
        }
    };

    let mut errors = Vec::new();

    // specific_ask: Phase 0 cap plus Amendment A1 headline rules.
    if params.specific_ask.chars().count() > MAX_SPECIFIC_ASK_CHARS {
        errors.push(format!(
            "specific_ask exceeds {} characters",
            MAX_SPECIFIC_ASK_CHARS
        ));
    }
    for v in headline_violations(&params.specific_ask) {
        errors.push(format!("specific_ask (headline): {}", v));
    }

    if params.why_blocked.trim().is_empty() {
        errors.push("why_blocked is empty".to_string());
    }
    if params.why_blocked.chars().count() > MAX_WHY_BLOCKED_CHARS {
        errors.push(format!(
            "why_blocked exceeds {} characters",
            MAX_WHY_BLOCKED_CHARS
        ));
    }

    if params.evidence_refs.len() > MAX_EVIDENCE_REFS {
        errors.push(format!(
            "evidence_refs has more than {} items",
            MAX_EVIDENCE_REFS
        ));
    }
    for (i, r) in params.evidence_refs.iter().enumerate() {
        if r.chars().count() > MAX_EVIDENCE_REF_CHARS {
            errors.push(format!(
                "evidence_refs[{}] exceeds {} characters",
                i, MAX_EVIDENCE_REF_CHARS
            ));
        }
    }

    // options: required iff kind == decision.
    match (&params.kind, &params.options) {
        (EscalationKind::Decision, None) => {
            errors.push("kind=decision requires options (2 to 5)".to_string());
        }
        (EscalationKind::Decision, Some(opts)) => {
            if opts.len() < MIN_OPTIONS || opts.len() > MAX_OPTIONS {
                errors.push(format!(
                    "options must have between {} and {} entries, got {}",
                    MIN_OPTIONS,
                    MAX_OPTIONS,
                    opts.len()
                ));
            }
            let mut seen_ids = std::collections::HashSet::new();
            for (i, o) in opts.iter().enumerate() {
                if o.id.is_empty()
                    || o.id.chars().count() > MAX_OPTION_ID_CHARS
                    || !o
                        .id
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                {
                    errors.push(format!(
                        "options[{}].id must be 1-{} chars of [a-z0-9-]",
                        i, MAX_OPTION_ID_CHARS
                    ));
                }
                if !seen_ids.insert(o.id.clone()) {
                    errors.push(format!("options[{}].id '{}' is duplicated", i, o.id));
                }
                if o.label.trim().is_empty()
                    || o.label.chars().count() > MAX_OPTION_LABEL_CHARS
                {
                    errors.push(format!(
                        "options[{}].label must be 1-{} characters",
                        i, MAX_OPTION_LABEL_CHARS
                    ));
                }
                if o.consequence.trim().is_empty()
                    || o.consequence.chars().count() > MAX_OPTION_CONSEQUENCE_CHARS
                {
                    errors.push(format!(
                        "options[{}].consequence must be 1-{} characters",
                        i, MAX_OPTION_CONSEQUENCE_CHARS
                    ));
                }
            }
        }
        (_, Some(opts)) if !opts.is_empty() => {
            errors.push("options are only allowed when kind=decision".to_string());
        }
        _ => {}
    }

    if errors.is_empty() {
        EscalationOutcome::Valid(Box::new(params))
    } else {
        EscalationOutcome::Malformed { errors }
    }
}

/// Map an escalation kind to its decisions-table kind (Phase 0 table).
pub fn map_kind(kind: EscalationKind) -> DecisionKind {
    match kind {
        EscalationKind::Approval => DecisionKind::ApproveReview,
        EscalationKind::Decision => DecisionKind::Choice,
        EscalationKind::Credential | EscalationKind::Information => DecisionKind::Unblock,
        EscalationKind::Capability => DecisionKind::RiskGate,
    }
}

/// Validate a raw payload and produce the typed [`DecisionDraft`].
///
/// Malformed payloads produce a `Malformed` draft carrying the raw payload
/// and the error list — recorded for human review, never dropped.
pub fn draft_from_payload(raw: serde_json::Value, source: &DraftSource) -> DecisionDraft {
    let requested_by_display = worker_display_name(source.worker_roster_name.as_deref());
    match validate_escalation(&raw) {
        EscalationOutcome::Valid(params) => {
            let kind = map_kind(params.kind);
            DecisionDraft {
                kind,
                headline: params.specific_ask.trim().to_string(),
                detail: params.why_blocked.trim().to_string(),
                evidence_refs: params.evidence_refs.clone(),
                options: match kind {
                    DecisionKind::Choice => params.options.clone().unwrap_or_default(),
                    _ => Vec::new(),
                },
                resume: params.resume,
                minimum_tier: kind.minimum_tier(),
                source_session_id: source.session_id.clone(),
                requested_by_display,
                raw_payload: None,
                validation_errors: Vec::new(),
            }
        }
        EscalationOutcome::Malformed { errors } => {
            let candidate = format!(
                "{} sent an escalation that needs review",
                requested_by_display
            );
            let headline = if headline_violations(&candidate).is_empty() {
                candidate
            } else {
                "An escalation could not be read and needs review".to_string()
            };
            let detail = format!(
                "Escalation payload failed validation:\n- {}",
                errors.join("\n- ")
            );
            DecisionDraft {
                kind: DecisionKind::Malformed,
                headline,
                detail,
                evidence_refs: Vec::new(),
                options: Vec::new(),
                resume: ResumeMode::Auto,
                minimum_tier: DecisionKind::Malformed.minimum_tier(),
                source_session_id: source.session_id.clone(),
                requested_by_display,
                raw_payload: Some(raw),
                validation_errors: errors,
            }
        }
    }
}

/// Tool result text returned to the calling agent. The malformed path is
/// success-with-notice: the caller is told to wait, not retry-loop.
pub fn tool_result_text(draft: &DecisionDraft, decision_id: &str) -> String {
    match draft.kind {
        DecisionKind::Malformed => format!(
            "Escalation recorded as malformed (decision {}). Validation issues: {}. \
             A human will review it — do not retry; wait for resolution.",
            decision_id,
            draft.validation_errors.join("; ")
        ),
        _ => format!(
            "Escalation recorded (decision {}, kind {}): \"{}\". \
             It is now in the decision inbox; work resumes automatically once it is answered.",
            decision_id,
            draft.kind.as_str(),
            draft.headline
        ),
    }
}

// ── Persistence seam ────────────────────────────────────────────────────

/// Result of recording a decision draft.
#[derive(Debug, Clone)]
pub struct RecordedDecision {
    pub decision_id: String,
}

/// Persistence seam for decision drafts. Part A ships
/// [`InMemoryDecisionSink`]; Part B installs a sink that writes to Lane
/// L1's decisions table.
#[async_trait::async_trait]
pub trait DecisionSink: Send + Sync {
    async fn record(&self, draft: DecisionDraft) -> anyhow::Result<RecordedDecision>;
}

/// In-memory sink: used by tests and as the Part A default until the
/// L1-backed sink is installed (Part B).
#[derive(Default)]
pub struct InMemoryDecisionSink {
    counter: AtomicU64,
    drafts: Mutex<Vec<(String, DecisionDraft)>>,
}

impl InMemoryDecisionSink {
    /// Snapshot of everything recorded so far (id, draft).
    pub fn recorded(&self) -> Vec<(String, DecisionDraft)> {
        self.drafts.lock().expect("sink poisoned").clone()
    }
}

#[async_trait::async_trait]
impl DecisionSink for InMemoryDecisionSink {
    async fn record(&self, draft: DecisionDraft) -> anyhow::Result<RecordedDecision> {
        let n = self.counter.fetch_add(1, Ordering::SeqCst) + 1;
        let decision_id = format!("pending-{}", n);
        tracing::info!(
            target: "permagentd::brain",
            decision_id = %decision_id,
            kind = draft.kind.as_str(),
            "Escalation recorded (in-memory sink — persistence wiring lands in Part B)"
        );
        self.drafts
            .lock()
            .expect("sink poisoned")
            .push((decision_id.clone(), draft));
        Ok(RecordedDecision { decision_id })
    }
}

static GLOBAL_SINK: OnceLock<Arc<dyn DecisionSink>> = OnceLock::new();

/// Install the process-wide decision sink (Part B wires the L1-backed
/// implementation here). Returns Err if a sink was already installed.
pub fn install_decision_sink(sink: Arc<dyn DecisionSink>) -> Result<(), Arc<dyn DecisionSink>> {
    GLOBAL_SINK.set(sink)
}

/// The process-wide decision sink, defaulting to an in-memory sink until
/// one is installed.
pub fn global_decision_sink() -> Arc<dyn DecisionSink> {
    GLOBAL_SINK
        .get_or_init(|| Arc::new(InMemoryDecisionSink::default()))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn valid_approval() -> serde_json::Value {
        json!({
            "kind": "approval",
            "specific_ask": "Approve the login page redesign so it can ship",
            "why_blocked": "Goal reached Review; verifier passed; PR #284 awaits sign-off.",
            "evidence_refs": ["card:1234", "session:abcd"],
            "resume": "auto"
        })
    }

    fn valid_decision() -> serde_json::Value {
        json!({
            "kind": "decision",
            "specific_ask": "Choose where new user data should be stored",
            "why_blocked": "Two viable storage backends; schema work diverges from here.",
            "evidence_refs": [],
            "options": [
                {"id": "sqlite", "label": "Local SQLite", "consequence": "Simple, single-machine only"},
                {"id": "postgres", "label": "Hosted Postgres", "consequence": "Scales, adds ops burden"}
            ],
            "resume": "auto"
        })
    }

    fn source() -> DraftSource {
        DraftSource {
            session_id: Some("sess-123".to_string()),
            worker_roster_name: None,
        }
    }

    // ── kind mapping ──

    #[test]
    fn kind_mapping_matches_phase0_table() {
        let cases = [
            (EscalationKind::Approval, DecisionKind::ApproveReview),
            (EscalationKind::Decision, DecisionKind::Choice),
            (EscalationKind::Credential, DecisionKind::Unblock),
            (EscalationKind::Information, DecisionKind::Unblock),
            (EscalationKind::Capability, DecisionKind::RiskGate),
        ];
        for (input, expected) in cases {
            assert_eq!(map_kind(input), expected, "kind {:?}", input);
        }
    }

    #[test]
    fn capability_and_malformed_are_always_tier_2() {
        assert_eq!(DecisionKind::RiskGate.minimum_tier(), 2);
        assert_eq!(DecisionKind::Malformed.minimum_tier(), 2);
        assert_eq!(DecisionKind::ApproveReview.minimum_tier(), 1);
        assert_eq!(DecisionKind::Choice.minimum_tier(), 1);
        assert_eq!(DecisionKind::Unblock.minimum_tier(), 1);
    }

    // ── valid paths ──

    #[test]
    fn valid_approval_maps_to_approve_review_draft() {
        let draft = draft_from_payload(valid_approval(), &source());
        assert_eq!(draft.kind, DecisionKind::ApproveReview);
        assert_eq!(
            draft.headline,
            "Approve the login page redesign so it can ship"
        );
        assert!(draft.detail.contains("PR #284")); // identifiers belong in detail
        assert_eq!(draft.evidence_refs.len(), 2);
        assert!(draft.options.is_empty());
        assert_eq!(draft.minimum_tier, 1);
        assert!(draft.validation_errors.is_empty());
        assert!(draft.raw_payload.is_none());
        assert_eq!(draft.source_session_id.as_deref(), Some("sess-123"));
    }

    #[test]
    fn valid_decision_keeps_options() {
        let draft = draft_from_payload(valid_decision(), &source());
        assert_eq!(draft.kind, DecisionKind::Choice);
        assert_eq!(draft.options.len(), 2);
        assert_eq!(draft.options[0].id, "sqlite");
        assert!(draft.validation_errors.is_empty());
    }

    #[test]
    fn capability_kind_maps_to_risk_gate_tier_2() {
        let mut payload = valid_approval();
        payload["kind"] = json!("capability");
        payload["specific_ask"] = json!("Allow workers to send outbound email");
        let draft = draft_from_payload(payload, &source());
        assert_eq!(draft.kind, DecisionKind::RiskGate);
        assert_eq!(draft.minimum_tier, 2);
    }

    #[test]
    fn credential_and_information_map_to_unblock() {
        for kind in ["credential", "information"] {
            let mut payload = valid_approval();
            payload["kind"] = json!(kind);
            payload["specific_ask"] = json!("Add the staging database password");
            let draft = draft_from_payload(payload, &source());
            assert_eq!(draft.kind, DecisionKind::Unblock, "kind {}", kind);
        }
    }

    // ── malformed paths ──

    fn assert_malformed(payload: serde_json::Value, expected_fragment: &str) {
        let draft = draft_from_payload(payload.clone(), &source());
        assert_eq!(draft.kind, DecisionKind::Malformed, "payload: {}", payload);
        assert_eq!(draft.minimum_tier, 2);
        assert_eq!(draft.raw_payload, Some(payload));
        assert!(
            draft
                .validation_errors
                .iter()
                .any(|e| e.contains(expected_fragment)),
            "expected error containing '{}', got {:?}",
            expected_fragment,
            draft.validation_errors
        );
        // Malformed drafts must still present a rule-clean headline.
        assert!(headline_violations(&draft.headline).is_empty());
    }

    #[test]
    fn unknown_kind_is_malformed_with_raw_payload() {
        let mut payload = valid_approval();
        payload["kind"] = json!("vibes");
        assert_malformed(payload, "does not match the escalate schema");
    }

    #[test]
    fn non_auto_resume_is_malformed() {
        let mut payload = valid_approval();
        payload["resume"] = json!("manual");
        assert_malformed(payload, "does not match the escalate schema");
    }

    #[test]
    fn decision_without_options_is_malformed() {
        let mut payload = valid_decision();
        payload.as_object_mut().unwrap().remove("options");
        assert_malformed(payload, "kind=decision requires options");
    }

    #[test]
    fn options_on_non_decision_kind_are_malformed() {
        let mut payload = valid_approval();
        payload["options"] = valid_decision()["options"].clone();
        assert_malformed(payload, "only allowed when kind=decision");
    }

    #[test]
    fn option_count_out_of_range_is_malformed() {
        let mut payload = valid_decision();
        let one = json!([{"id": "a", "label": "A", "consequence": "c"}]);
        payload["options"] = one;
        assert_malformed(payload, "between 2 and 5");

        let mut payload = valid_decision();
        let opt = json!({"id": "a", "label": "A", "consequence": "c"});
        payload["options"] = json!(vec![opt; 6]);
        let draft = draft_from_payload(payload, &source());
        assert_eq!(draft.kind, DecisionKind::Malformed);
    }

    #[test]
    fn bad_option_id_and_duplicate_ids_are_malformed() {
        let mut payload = valid_decision();
        payload["options"][0]["id"] = json!("Not_A_Slug!");
        assert_malformed(payload, "[a-z0-9-]");

        let mut payload = valid_decision();
        payload["options"][1]["id"] = json!("sqlite");
        assert_malformed(payload, "duplicated");
    }

    #[test]
    fn oversized_fields_are_malformed() {
        let mut payload = valid_approval();
        payload["specific_ask"] = json!("x".repeat(141));
        assert_malformed(payload, "specific_ask exceeds 140");

        let mut payload = valid_approval();
        payload["why_blocked"] = json!("x".repeat(2001));
        assert_malformed(payload, "why_blocked exceeds 2000");

        let mut payload = valid_approval();
        payload["evidence_refs"] = json!(vec!["r"; 11]);
        assert_malformed(payload, "more than 10");

        let mut payload = valid_approval();
        payload["evidence_refs"] = json!([String::from("y").repeat(513)]);
        assert_malformed(payload, "evidence_refs[0] exceeds 512");
    }

    #[test]
    fn empty_why_blocked_is_malformed() {
        let mut payload = valid_approval();
        payload["why_blocked"] = json!("   ");
        assert_malformed(payload, "why_blocked is empty");
    }

    // ── headline rules (Amendment A1) ──

    #[test]
    fn headline_rules_table() {
        let ok: &[&str] = &[
            "Approve the login page redesign so it can ship",
            "Choose where new user data should be stored",
            "Decide whether to keep supporting the old importer",
        ];
        for h in ok {
            assert!(
                headline_violations(h).is_empty(),
                "expected clean: {} → {:?}",
                h,
                headline_violations(h)
            );
        }

        let bad: &[(&str, &str)] = &[
            ("", "empty"),
            (&"x".repeat(81), "exceeds 80"),
            ("Approve PR #284 for the login flow", "PR/issue number"),
            ("Approve PR 284 for the login flow", "PR number"),
            ("Merge inbox/henry-loop into main", "path, branch name"),
            ("Update src\\main.rs error handling", "path, branch name"),
            ("Review changes across 12 files in the parser", "file count"),
            (
                "Approve goal 0aa2b716-9d8f-4a2e-8c1d-1b2c3d4e5f60",
                "internal ID",
            ),
            ("Approve commit 0522cd2ab44f for release", "internal ID"),
        ];
        for (h, expected) in bad {
            let v = headline_violations(h);
            assert!(
                v.iter().any(|e| e.contains(expected)),
                "headline '{}' expected violation containing '{}', got {:?}",
                h,
                expected,
                v
            );
        }
    }

    #[test]
    fn specific_ask_violating_headline_rules_is_malformed() {
        let mut payload = valid_approval();
        payload["specific_ask"] = json!("Approve PR #284 on inbox/henry-loop");
        let draft = draft_from_payload(payload, &source());
        assert_eq!(draft.kind, DecisionKind::Malformed);
        assert!(draft
            .validation_errors
            .iter()
            .any(|e| e.starts_with("specific_ask (headline):")));
    }

    // ── Amendment A2: worker display names ──

    #[test]
    fn worker_display_name_falls_back_to_anonymous() {
        assert_eq!(worker_display_name(None), ANONYMOUS_WORKER_DISPLAY);
        assert_eq!(worker_display_name(Some("  ")), ANONYMOUS_WORKER_DISPLAY);
        assert_eq!(worker_display_name(Some("codex")), "codex");
    }

    #[test]
    fn malformed_draft_uses_display_name_not_internal_id() {
        let src = DraftSource {
            session_id: Some("sess-deadbeef-123".to_string()),
            worker_roster_name: None,
        };
        let draft = draft_from_payload(json!({"kind": "vibes"}), &src);
        assert!(draft.headline.contains(ANONYMOUS_WORKER_DISPLAY));
        assert!(!draft.headline.contains("sess-"));
    }

    // ── tool result text ──

    #[test]
    fn tool_result_text_for_valid_and_malformed() {
        let draft = draft_from_payload(valid_approval(), &source());
        let text = tool_result_text(&draft, "d-1");
        assert!(text.contains("decision d-1"));
        assert!(text.contains("approve_review"));
        assert!(text.contains("resumes automatically"));

        let draft = draft_from_payload(json!({"kind": "vibes"}), &source());
        let text = tool_result_text(&draft, "d-2");
        assert!(text.contains("malformed"));
        assert!(text.contains("do not retry"));
    }

    // ── DecisionSink seam ──

    #[tokio::test]
    async fn in_memory_sink_records_drafts_with_unique_ids() {
        let sink = InMemoryDecisionSink::default();
        let d1 = draft_from_payload(valid_approval(), &source());
        let d2 = draft_from_payload(valid_decision(), &source());
        let r1 = sink.record(d1).await.unwrap();
        let r2 = sink.record(d2).await.unwrap();
        assert_ne!(r1.decision_id, r2.decision_id);
        let recorded = sink.recorded();
        assert_eq!(recorded.len(), 2);
        assert_eq!(recorded[0].0, r1.decision_id);
        assert_eq!(recorded[1].1.kind, DecisionKind::Choice);
    }
}
