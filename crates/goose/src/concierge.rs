//! The Concierge — the first daily-life roster character (EPIC #640).
//!
//! An inbox-triage character-agent, built on the Echo/Watcher proactive pattern
//! (`crate::echo` + the daemon's `proactive` loop). This module is the Concierge's
//! **identity** (its flag, its self-knowledge descriptor) plus the **pure triage
//! core** (classification, drafting proposals, and the Decision-Inbox draft the
//! daemon records). The **behavior** — the interval loop that reads Gmail and
//! surfaces triage — lives in the daemon (`permagent-daemon`'s `concierge`
//! module), exactly like the split between `echo.rs` and `proactive.rs`.
//!
//! Three safety properties are load-bearing and enforced here, not merely
//! documented (see the ruled design in `docs/design/daily-life-roster.md`, #878,
//! and the ruling on #640):
//!
//! 1. **Flag-gated, default OFF** ([`is_enabled`], `PERMAGENT_CONCIERGE_ENABLED`).
//!    When off, the daemon loop never spawns AND [`SELF_KNOWLEDGE_FEATURE`] is
//!    hidden from the `permagent_self` brief (the Playbook render-gate pattern in
//!    `self_knowledge::worker_descriptor_visible`), so the canonical
//!    `prompt_manager` snapshots stay byte-for-byte identical. The Concierge reads
//!    *nothing* until deliberately enabled.
//! 2. **Local-tier pinned** ([`REASONING_WORKLOAD`]). All reasoning over mail
//!    content routes through [`crate::mesh::Workload::Interactive`], which
//!    `crate::mesh::resolve_route` resolves to an on-device
//!    [`crate::mesh::PrivacyApproach::LocalOnly`] endpoint **by construction** —
//!    interactive work never leaves the machine. Mail content is the most
//!    sensitive data in the product (D6), so it never touches a cloud tier.
//! 3. **Draft-only / read-only by construction.** The bundled Gmail MCP is
//!    `gmail.readonly` and exposes only [`GMAIL_TRIAGE_TOOLS`] (search/read/list) —
//!    there is no send or draft-write tool ([`is_mutating_gmail_tool`] rejects the
//!    whole mutation family). The Concierge triages and *proposes* a reply draft
//!    as an editable Decision-Inbox card; it can never send or mutate mail.

use crate::agents::self_knowledge::{FeatureCategory, FeatureDescriptor, StateSource};
use crate::decision_inbox::escalate::{
    DecisionDraft, DecisionKind, DecisionSink, RecordedDecision, ResumeMode,
};

// ── Flag gate (default OFF) ───────────────────────────────────────────────────

/// Env flag gating the entire Concierge. Default OFF: the character is an
/// early daily-life-roster slice reading the user's most sensitive data, so it
/// must be turned on deliberately (mirrors the Playbook rollout discipline).
pub const CONCIERGE_ENABLED_ENV: &str = "PERMAGENT_CONCIERGE_ENABLED";

/// The self-knowledge descriptor id — also the render-gate key that
/// [`crate::agents::self_knowledge`] checks so the descriptor is hidden from the
/// brief while the flag is off (canonical snapshots stay byte-identical).
pub const CONCIERGE_FEATURE_ID: &str = "concierge";

/// Pure truthiness for the flag value — unit-testable without the environment.
fn is_truthy(raw: Option<&str>) -> bool {
    matches!(
        raw.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("1") | Some("true") | Some("yes") | Some("on")
    )
}

/// Whether the Concierge is switched on. Read by the daemon loop (spawn gate)
/// and the self-knowledge brief (descriptor render gate) — one flag so the
/// capability the agent can DO is exactly the one it can DESCRIBE. Default OFF.
pub fn is_enabled() -> bool {
    is_truthy(std::env::var(CONCIERGE_ENABLED_ENV).ok().as_deref())
}

// ── Self-knowledge identity ───────────────────────────────────────────────────

/// The Concierge's name — one source of truth for framing + self-knowledge.
pub const CONCIERGE_NAME: &str = "the Concierge";

/// Self-knowledge descriptor for the Concierge. Its render presence in the
/// `permagent_self` brief is gated on [`is_enabled`] (see
/// `self_knowledge::worker_descriptor_visible`), so this early, flag-gated
/// character does not enter every user's Henry until deliberately enabled.
pub const SELF_KNOWLEDGE_FEATURE: FeatureDescriptor = FeatureDescriptor {
    id: CONCIERGE_FEATURE_ID,
    display_name: "The Concierge",
    category: FeatureCategory::Worker,
    what_it_does:
        "A background character that, when enabled, gently triages the user's email inbox — reading \
         it read-only, flagging what needs their attention (family, financial, anything that looks \
         urgent), and proposing an editable reply DRAFT — and surfaces it as a once-a-day triage \
         digest and as Decision-Inbox draft cards the user edits and approves. It reasons over mail \
         content strictly on the local, on-device model tier, so email never leaves the machine, \
         and it can never send or change mail — it only ever reads and proposes",
    why_it_matters:
        "It is the payoff of a permanent, local agent that lives on the user's machine: it can watch \
         the inbox and prepare the reply so the user only has to approve it. Safe by construction — \
         read-only Gmail access, draft-only (no send path exists), local-tier reasoning so the most \
         sensitive data in the product stays on-device, off by default, and gentle (one digest a \
         day, quiet hours)",
    // Workers are Queryable by invariant; the Concierge has no cheap live status
    // to merge, so `worker_live_state` returns None for its id and it renders
    // editorially (no `_(now: …)_` suffix) — like the Watcher and the Playbook.
    state_source: StateSource::Queryable,
    teaching: &[],
};

// ── Local-tier pin (sovereignty — mail content stays on-device) ────────────────

/// The workload class every Concierge reasoning call over mail content MUST use.
/// [`crate::mesh::Workload::Interactive`] is routed to an on-device
/// [`crate::mesh::PrivacyApproach::LocalOnly`] endpoint **unconditionally** by
/// `crate::mesh::resolve_route` — interactive work never leaves the device
/// (regardless of any mesh/trusted-pool configuration). This is the D6 privacy
/// ruling in code: the Concierge's reasoning cannot reach a cloud tier, because
/// the route it is pinned to is local by construction.
pub const REASONING_WORKLOAD: crate::mesh::Workload = crate::mesh::Workload::Interactive;

/// The concrete route the Concierge's reasoning resolves to — always the local
/// on-device endpoint (see [`REASONING_WORKLOAD`]).
pub fn reasoning_route() -> crate::mesh::InferenceRoute {
    crate::mesh::resolve_route(REASONING_WORKLOAD)
}

/// Whether the Concierge's reasoning is pinned to the on-device local tier.
/// Always true — the assertion the sovereignty guard rests on.
pub fn reasoning_is_local() -> bool {
    reasoning_route().approach == crate::mesh::PrivacyApproach::LocalOnly
}

// ── Draft-only / read-only safety (no send path exists) ────────────────────────

/// The ONLY Gmail tools the Concierge uses — every one read-only. The bundled
/// `permagent-gmail-mcp` is `gmail.readonly` and exposes exactly these; there is
/// no send or draft-write tool to call. Kept as an explicit allow-list so a test
/// pins the Concierge to read-only access even if the MCP ever grows a mutating
/// tool.
pub const GMAIL_TRIAGE_TOOLS: &[&str] = &[
    "gmail__search",
    "gmail__read",
    "gmail__list_labels",
    "gmail__list_threads",
];

/// Whether a Gmail tool name mutates mail (sends, drafts, modifies, deletes). The
/// Concierge must never call one. A conservative substring guard over the whole
/// mutation family, so a future MCP tool named anything in that family is
/// rejected by the safety test before it can be wired.
pub fn is_mutating_gmail_tool(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [
        "send",
        "draft",
        "create",
        "insert",
        "modify",
        "update",
        "delete",
        "trash",
        "untrash",
        "import",
        "batchmodify",
        "reply",
        "forward",
    ]
    .iter()
    .any(|m| n.contains(m))
}

// ── Pure triage core ───────────────────────────────────────────────────────────

/// A single inbox message the triage reasons over. Read-only projection of what
/// the Gmail read tools return — headers + snippet, never a mutation handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxMessage {
    pub id: String,
    pub thread_id: String,
    pub from: String,
    pub subject: String,
    pub snippet: String,
    pub unread: bool,
}

/// How much attention a message warrants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Urgency {
    /// Family / financial / time-critical — surface it.
    Urgent,
    /// A real message from a person that may want a reply.
    Normal,
    /// Newsletters, notifications, no-reply automation — archive-worthy noise.
    Noise,
}

/// The triage verdict for one message — the pure classification output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriagePlan {
    pub message_id: String,
    pub from: String,
    pub subject: String,
    pub urgency: Urgency,
    /// Whether the Concierge should propose a reply draft for this message.
    pub wants_reply: bool,
}

/// A concrete triage action the Concierge proposes. NEVER a send — a draft the
/// user edits and approves, or a flag on something needing their attention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriageProposal {
    /// Flag a message as needing the user's attention (no reply drafted).
    NeedsAttention {
        message_id: String,
        from: String,
        subject: String,
        reason: String,
    },
    /// A suggested reply DRAFT for the user to edit/approve. Draft-only: acting
    /// on it is a no-op to Gmail until an explicit, scope-gated send path is
    /// added in a later slice (#640 D3).
    DraftReply {
        message_id: String,
        to: String,
        subject: String,
        body: String,
    },
}

/// Lowercased keyword sets for the deterministic urgency heuristic. This is the
/// safe fallback (like `proactive`'s deterministic pick) used when no local model
/// is available; when one is, it refines the draft body, never the safety class.
const URGENT_HINTS: &[&str] = &[
    "urgent",
    "asap",
    "overdue",
    "payment",
    "invoice",
    "past due",
    "deadline",
    "final notice",
    "action required",
    "suspended",
    "fraud",
    "security alert",
    "mom",
    "dad",
    "family",
    "hospital",
    "school",
    "doctor",
];
const NOISE_HINTS: &[&str] = &[
    "unsubscribe",
    "newsletter",
    "no-reply",
    "noreply",
    "no_reply",
    "notifications@",
    "digest",
    "promotion",
    "sale",
    "% off",
    "marketing",
    "do-not-reply",
    "donotreply",
];

/// Classify a message's urgency with a deterministic, content-local heuristic.
/// Pure — no IO, no model — so it is fully unit-testable and is the safe default
/// the loop falls back to when the local model is unavailable.
pub fn classify_urgency(msg: &InboxMessage) -> Urgency {
    let hay = format!(
        "{} {} {}",
        msg.from.to_ascii_lowercase(),
        msg.subject.to_ascii_lowercase(),
        msg.snippet.to_ascii_lowercase()
    );
    if NOISE_HINTS.iter().any(|h| hay.contains(h)) {
        return Urgency::Noise;
    }
    if URGENT_HINTS.iter().any(|h| hay.contains(h)) {
        return Urgency::Urgent;
    }
    Urgency::Normal
}

/// Plan triage over a batch of messages: classify each and decide whether to
/// propose a reply. Pure. Only unread, non-noise mail from an addressable sender
/// gets a reply proposed — noise is flagged at most, never replied to.
pub fn plan_triage(messages: &[InboxMessage]) -> Vec<TriagePlan> {
    messages
        .iter()
        .filter(|m| m.unread)
        .map(|m| {
            let urgency = classify_urgency(m);
            let wants_reply = urgency != Urgency::Noise && looks_addressable(&m.from);
            TriagePlan {
                message_id: m.id.clone(),
                from: m.from.clone(),
                subject: m.subject.clone(),
                urgency,
                wants_reply,
            }
        })
        .collect()
}

/// Whether a `From` header looks like a real, replyable person/address (has an
/// `@`, not a no-reply automation address).
fn looks_addressable(from: &str) -> bool {
    let f = from.to_ascii_lowercase();
    f.contains('@')
        && !f.contains("no-reply")
        && !f.contains("noreply")
        && !f.contains("no_reply")
        && !f.contains("do-not-reply")
        && !f.contains("donotreply")
}

/// System prompt for the local reply-drafting model. Names the draft-only,
/// on-device contract so the model composes a reply, never an action.
pub const DRAFT_SYSTEM: &str = "You are the Concierge, a careful assistant drafting a reply the user \
     will review, edit, and decide whether to send. You never send anything yourself. Draft a short, \
     polite reply in the user's voice. Output only the reply body — no preamble, no subject line, no \
     signature block.";

/// Build the local-model prompt for drafting a reply to one message. The mail
/// content in this prompt only ever travels to the on-device local endpoint (see
/// [`REASONING_WORKLOAD`]).
pub fn build_draft_prompt(msg: &InboxMessage) -> String {
    format!(
        "Draft a brief reply to this email.\nFrom: {}\nSubject: {}\nBody: {}\n\nReply body:",
        msg.from,
        msg.subject,
        truncate(&msg.snippet, 4000)
    )
}

/// A safe deterministic reply body used when the local model is unavailable — a
/// short acknowledgement the user then edits. Never fabricates specifics.
pub fn template_reply_body(msg: &InboxMessage) -> String {
    format!(
        "Hi,\n\nThanks for your message about \"{}\". I'll take a look and get back to you shortly.\n\nBest,",
        msg.subject.trim()
    )
}

/// A reply subject: `Re: <subject>` unless it already carries a reply prefix.
fn reply_subject(subject: &str) -> String {
    let s = subject.trim();
    if s.to_ascii_lowercase().starts_with("re:") {
        s.to_string()
    } else {
        format!("Re: {s}")
    }
}

/// Turn a triage plan + a chosen reply body into a proposal. Urgent/normal mail
/// that wants a reply becomes a [`TriageProposal::DraftReply`]; urgent mail that
/// does not (e.g. an automated-but-important alert) becomes a
/// [`TriageProposal::NeedsAttention`] flag; ordinary noise yields nothing.
pub fn proposal_for(plan: &TriagePlan, reply_body: String) -> Option<TriageProposal> {
    if plan.wants_reply {
        return Some(TriageProposal::DraftReply {
            message_id: plan.message_id.clone(),
            to: plan.from.clone(),
            subject: reply_subject(&plan.subject),
            body: reply_body,
        });
    }
    if plan.urgency == Urgency::Urgent {
        return Some(TriageProposal::NeedsAttention {
            message_id: plan.message_id.clone(),
            from: plan.from.clone(),
            subject: plan.subject.clone(),
            reason: "looks time-sensitive".to_string(),
        });
    }
    None
}

/// The once-a-day triage digest line, surfaced as a `ProactiveNudge` through the
/// Notification Router. `None` when nothing cleared the bar (stay silent, like
/// the Watcher). Pure.
pub fn digest_message(proposals: &[TriageProposal]) -> Option<String> {
    if proposals.is_empty() {
        return None;
    }
    let drafts = proposals
        .iter()
        .filter(|p| matches!(p, TriageProposal::DraftReply { .. }))
        .count();
    let flags = proposals.len() - drafts;
    let mut parts = Vec::new();
    if flags > 0 {
        parts.push(format!(
            "{flags} message{} need your attention",
            plural(flags)
        ));
    }
    if drafts > 0 {
        parts.push(format!(
            "I drafted {drafts} repl{}",
            if drafts == 1 { "y" } else { "ies" }
        ));
    }
    Some(format!(
        "Inbox triage: {} — review them in your Decision Inbox.",
        parts.join(", ")
    ))
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

// ── Decision-Inbox draft card (the approval surface, #760) ─────────────────────

/// Build the Decision-Inbox draft for a proposal. A draft reply and an
/// attention flag both become an `approve_review` card (Tier 1, plain-language
/// headline, the draft body/reason in `detail`) that the user edits and
/// approves. Approving is a no-op to Gmail (draft-only) until a scope-gated send
/// path exists — the card never carries a send action.
pub fn draft_decision_from_proposal(proposal: &TriageProposal) -> DecisionDraft {
    let (headline, detail) = match proposal {
        TriageProposal::DraftReply {
            to, subject, body, ..
        } => (
            truncate_headline(&format!("Review a drafted reply to {}", sender_name(to))),
            format!("Suggested reply to \"{subject}\" (draft-only — nothing is sent):\n\n{body}"),
        ),
        TriageProposal::NeedsAttention {
            from,
            subject,
            reason,
            ..
        } => (
            truncate_headline(&format!(
                "Email from {} needs your attention",
                sender_name(from)
            )),
            format!("\"{subject}\" — {reason}. Read-only flag; nothing is sent or changed."),
        ),
    };
    DecisionDraft {
        kind: DecisionKind::ApproveReview,
        headline,
        detail,
        evidence_refs: Vec::new(),
        options: Vec::new(),
        resume: ResumeMode::Auto,
        minimum_tier: DecisionKind::ApproveReview.minimum_tier(),
        source_session_id: None,
        requested_by_display: CONCIERGE_NAME.to_string(),
        raw_payload: None,
        validation_errors: Vec::new(),
    }
}

/// Record proposals as Decision-Inbox draft cards through the given sink,
/// returning the created rows. The daemon passes the process-global
/// `SqlDecisionSink`; tests pass an in-memory sink. Never sends mail — the sink
/// only ever writes an `approve_review` decision row.
pub async fn record_proposals(
    sink: &dyn DecisionSink,
    proposals: &[TriageProposal],
) -> Vec<RecordedDecision> {
    let mut recorded = Vec::new();
    for p in proposals {
        match sink.record(draft_decision_from_proposal(p)).await {
            Ok(r) => recorded.push(r),
            Err(e) => tracing::warn!(
                target: "permagentd::concierge",
                "failed to record triage draft card: {e}"
            ),
        }
    }
    recorded
}

// ── Small helpers ──────────────────────────────────────────────────────────────

/// The display half of a `Name <addr>` / bare-address `From` header.
fn sender_name(from: &str) -> String {
    let f = from.trim();
    if let Some(idx) = f.find('<') {
        let name = f[..idx].trim().trim_matches('"').trim();
        if !name.is_empty() {
            return name.to_string();
        }
    }
    f.trim_matches(|c| c == '<' || c == '>').to_string()
}

/// Plain-language headline within the Decision-Inbox 80-char limit.
fn truncate_headline(s: &str) -> String {
    const MAX: usize = 80;
    if s.chars().count() <= MAX {
        s.to_string()
    } else {
        let head: String = s.chars().take(MAX - 1).collect();
        format!("{head}…")
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    let t = text.trim();
    if t.chars().count() <= max_chars {
        t.to_string()
    } else {
        let s: String = t.chars().take(max_chars).collect();
        format!("{s}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision_inbox::escalate::InMemoryDecisionSink;

    fn msg(id: &str, from: &str, subject: &str, snippet: &str) -> InboxMessage {
        InboxMessage {
            id: id.to_string(),
            thread_id: format!("t-{id}"),
            from: from.to_string(),
            subject: subject.to_string(),
            snippet: snippet.to_string(),
            unread: true,
        }
    }

    // ── Flag gate (default OFF) ──

    #[test]
    fn flag_is_off_by_default_and_truthy_values_enable() {
        assert!(!is_truthy(None));
        assert!(!is_truthy(Some("")));
        assert!(!is_truthy(Some("0")));
        assert!(!is_truthy(Some("false")));
        for on in ["1", "true", "TRUE", "yes", "on", " On "] {
            assert!(is_truthy(Some(on)), "{on:?} should enable");
        }
    }

    // ── Local-tier pin (sovereignty) ──

    #[test]
    fn reasoning_is_pinned_to_the_local_on_device_tier() {
        // Safe by construction: the workload the Concierge reasons on resolves to
        // a LocalOnly, on-device route — never a cloud tier — regardless of any
        // mesh/pool configuration. This is the D6 privacy ruling in code.
        assert_eq!(REASONING_WORKLOAD, crate::mesh::Workload::Interactive);
        let route = reasoning_route();
        assert_eq!(route.approach, crate::mesh::PrivacyApproach::LocalOnly);
        assert!(
            route.endpoint.contains("localhost") || route.endpoint.contains("127.0.0.1"),
            "reasoning endpoint must be on-device, got {}",
            route.endpoint
        );
        assert!(reasoning_is_local());
    }

    // ── Draft-only / read-only safety (no send path) ──

    #[test]
    fn concierge_uses_only_read_only_gmail_tools() {
        for t in GMAIL_TRIAGE_TOOLS {
            assert!(
                !is_mutating_gmail_tool(t),
                "{t} must be a read-only Gmail tool"
            );
        }
    }

    #[test]
    fn every_mail_mutating_tool_is_rejected() {
        for t in [
            "gmail__send",
            "gmail__create_draft",
            "gmail__drafts_create",
            "gmail__trash",
            "gmail__modify",
            "gmail__delete",
            "gmail__reply",
            "gmail__forward",
        ] {
            assert!(
                is_mutating_gmail_tool(t),
                "{t} must be rejected as a mutating tool"
            );
        }
    }

    // ── Classification (deterministic safe fallback) ──

    #[test]
    fn classifies_urgent_normal_and_noise() {
        assert_eq!(
            classify_urgency(&msg(
                "1",
                "bank@x.com",
                "Payment overdue",
                "your invoice is past due"
            )),
            Urgency::Urgent
        );
        assert_eq!(
            classify_urgency(&msg("2", "alice@x.com", "Lunch?", "want to grab lunch")),
            Urgency::Normal
        );
        assert_eq!(
            classify_urgency(&msg(
                "3",
                "news@promo.com",
                "50% off sale",
                "click to unsubscribe"
            )),
            Urgency::Noise
        );
    }

    #[test]
    fn plan_triage_skips_read_and_noise_and_only_replies_to_addressable() {
        let mut read = msg("r", "alice@x.com", "hi", "hello");
        read.unread = false;
        let messages = vec![
            read,
            msg("u", "bob@x.com", "Question", "can you confirm the time?"),
            msg(
                "n",
                "no-reply@service.com",
                "Receipt",
                "your newsletter digest",
            ),
            msg(
                "a",
                "alerts@no-reply.com",
                "URGENT: action required",
                "verify now",
            ),
        ];
        let plans = plan_triage(&messages);
        // The read message is excluded entirely.
        assert!(plans.iter().all(|p| p.message_id != "r"));
        let bob = plans.iter().find(|p| p.message_id == "u").unwrap();
        assert!(bob.wants_reply, "addressable person → reply proposed");
        let noise = plans.iter().find(|p| p.message_id == "n").unwrap();
        assert!(!noise.wants_reply, "noise → no reply");
        let alert = plans.iter().find(|p| p.message_id == "a").unwrap();
        assert_eq!(alert.urgency, Urgency::Urgent);
        assert!(
            !alert.wants_reply,
            "no-reply urgent alert → flagged, never replied to"
        );
    }

    // ── Proposals ──

    #[test]
    fn proposal_for_builds_draft_flag_or_nothing() {
        let plan = plan_triage(&[msg("u", "bob@x.com", "Question", "confirm the time?")])
            .pop()
            .unwrap();
        match proposal_for(&plan, "sure, 3pm works".to_string()).unwrap() {
            TriageProposal::DraftReply {
                to, subject, body, ..
            } => {
                assert_eq!(to, "bob@x.com");
                assert_eq!(subject, "Re: Question");
                assert_eq!(body, "sure, 3pm works");
            }
            other => panic!("expected a draft reply, got {other:?}"),
        }

        // Urgent but not addressable → attention flag.
        let alert = plan_triage(&[msg("a", "alerts@no-reply.com", "URGENT payment", "overdue")])
            .pop()
            .unwrap();
        assert!(matches!(
            proposal_for(&alert, String::new()),
            Some(TriageProposal::NeedsAttention { .. })
        ));

        // Ordinary noise → nothing.
        let noise_plan = TriagePlan {
            message_id: "n".into(),
            from: "news@promo.com".into(),
            subject: "sale".into(),
            urgency: Urgency::Noise,
            wants_reply: false,
        };
        assert!(proposal_for(&noise_plan, String::new()).is_none());
    }

    #[test]
    fn reply_subject_does_not_double_prefix() {
        assert_eq!(reply_subject("Question"), "Re: Question");
        assert_eq!(reply_subject("Re: Question"), "Re: Question");
    }

    #[test]
    fn digest_summarizes_drafts_and_flags_or_stays_silent() {
        assert!(digest_message(&[]).is_none());
        let props = vec![
            TriageProposal::DraftReply {
                message_id: "1".into(),
                to: "a@x.com".into(),
                subject: "Re: hi".into(),
                body: "ok".into(),
            },
            TriageProposal::NeedsAttention {
                message_id: "2".into(),
                from: "b@x.com".into(),
                subject: "urgent".into(),
                reason: "time-sensitive".into(),
            },
        ];
        let d = digest_message(&props).unwrap();
        assert!(d.contains("drafted 1 reply"));
        assert!(d.contains("1 message need") || d.contains("1 messages need"));
        assert!(d.contains("Decision Inbox"));
    }

    // ── Decision-Inbox draft card (wiring, draft-only) ──

    #[test]
    fn draft_card_is_a_tier1_approve_review_never_a_send() {
        let proposal = TriageProposal::DraftReply {
            message_id: "1".into(),
            to: "Alice Smith <alice@x.com>".into(),
            subject: "Re: lunch".into(),
            body: "sounds good".into(),
        };
        let draft = draft_decision_from_proposal(&proposal);
        assert_eq!(draft.kind, DecisionKind::ApproveReview);
        assert_eq!(draft.minimum_tier, 1);
        assert_eq!(draft.requested_by_display, CONCIERGE_NAME);
        assert!(draft.headline.chars().count() <= 80);
        assert!(draft.headline.contains("Alice Smith"));
        assert!(draft.detail.contains("sounds good"));
        assert!(draft.detail.contains("draft-only"));
    }

    #[tokio::test]
    async fn triage_proposes_a_draft_decision_when_enabled() {
        // The end-to-end proposal path against a real (in-memory) Decision sink:
        // an unread question from a person yields ONE approve_review draft card,
        // and nothing that could send or mutate mail.
        let sink = InMemoryDecisionSink::default();
        let messages = vec![msg("u", "bob@x.com", "Can you confirm?", "is 3pm ok?")];
        let plans = plan_triage(&messages);
        let proposals: Vec<TriageProposal> = plans
            .iter()
            .filter_map(|p| proposal_for(p, template_reply_body(&messages[0])))
            .collect();
        assert_eq!(proposals.len(), 1);

        let recorded = record_proposals(&sink, &proposals).await;
        assert_eq!(recorded.len(), 1, "one draft card recorded");

        let cards = sink.recorded();
        assert_eq!(cards.len(), 1);
        let (decision_id, draft) = &cards[0];
        assert!(decision_id.starts_with("pending-"));
        assert_eq!(draft.kind, DecisionKind::ApproveReview);
        assert!(draft.detail.contains("draft-only"));
    }

    #[test]
    fn sender_name_extracts_display_or_address() {
        assert_eq!(sender_name("Alice Smith <alice@x.com>"), "Alice Smith");
        assert_eq!(sender_name("\"Bob\" <bob@x.com>"), "Bob");
        assert_eq!(sender_name("carol@x.com"), "carol@x.com");
    }
}
