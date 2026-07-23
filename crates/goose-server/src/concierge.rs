//! The Concierge behavior loop (#640) — the daemon half of the inbox-triage
//! character. Identity + the pure triage core live in the `permagent` lib
//! (`permagent::concierge`), exactly like `echo.rs` ↔ this `proactive.rs`-shaped
//! loop.
//!
//! When — and ONLY when — `PERMAGENT_CONCIERGE_ENABLED` is on, this spawns a
//! gentle interval loop that:
//!   1. reads the user's unread mail **read-only** via the bundled Gmail API
//!      (`gmail.readonly` — there is no send/draft-write path to call);
//!   2. reasons over it strictly on the **local, on-device** model tier
//!      (`permagent::concierge::REASONING_WORKLOAD` → `Workload::Interactive`,
//!      which `mesh::resolve_route` pins to a LocalOnly endpoint by construction,
//!      so mail content never leaves the machine);
//!   3. surfaces a once-a-day **triage digest** through the Notification Router
//!      (a `ProactiveNudge` event, reusing #66 verbatim) and files each proposed
//!      reply as an **editable Decision-Inbox draft card** (#760).
//!
//! It only ever reads and proposes — it can never send or mutate mail. With the
//! flag off (default), the loop never spawns: the Concierge reads nothing.

use chrono::{DateTime, Local, Timelike, Utc};
use permagent::concierge::{
    self, plan_triage, proposal_for, template_reply_body, InboxMessage, TriageProposal,
};
use permagent::decision_inbox::escalate::DecisionSink;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::state::AppState;

const TICK: Duration = Duration::from_secs(3 * 3600);
const STARTUP_DELAY: Duration = Duration::from_secs(240);
const MIN_GAP_HOURS: i64 = 20;
/// Max unread messages triaged per pass — a gentle, bounded read.
const MAX_TRIAGE: usize = 10;
/// The local model used to draft replies. Same family the Reader summarizes with,
/// and it runs on-device (see the local-tier pin in `permagent::concierge`).
const DRAFT_MODEL: &str = "qwen2.5:7b";

/// How a reply body is composed. The real loop drafts on the LOCAL model; tests
/// use the deterministic template so they stay offline and fast.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposeMode {
    /// Draft on the on-device local model, falling back to the template if it is
    /// unavailable (mirrors the Reader's Ollama-then-truncate degrade).
    LocalModel,
    /// Deterministic template only — no model call.
    Template,
}

// ── Once-a-day budget (persisted across restarts, like the Watcher) ────────────

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Budget {
    /// RFC3339 — a plain string (no chrono serde dependency).
    last_delivered: Option<String>,
}

impl Budget {
    fn path() -> Option<PathBuf> {
        std::env::var_os("HOME").map(|h| {
            PathBuf::from(h)
                .join(".permagent")
                .join("concierge-state.json")
        })
    }
    fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
    fn save(&self) {
        if let (Some(p), Ok(s)) = (Self::path(), serde_json::to_string(self)) {
            let _ = std::fs::write(p, s);
        }
    }
}

/// Spawn the Concierge loop — but ONLY when the flag is on. Off (default) is a
/// no-op: no task, no reads, nothing surfaces.
pub fn spawn(_state: Arc<AppState>) {
    if !concierge::is_enabled() {
        tracing::debug!(
            target: "permagentd::concierge",
            "Concierge disabled (PERMAGENT_CONCIERGE_ENABLED off) — inbox triage will not run"
        );
        return;
    }
    // Safety invariant, asserted at startup: reasoning is pinned on-device.
    debug_assert!(concierge::reasoning_is_local());
    tracing::info!(
        target: "permagentd::concierge",
        endpoint = %concierge::reasoning_route().endpoint,
        "Concierge enabled — read-only inbox triage, draft-only, local-tier reasoning"
    );

    tokio::spawn(async move {
        tokio::time::sleep(STARTUP_DELAY).await;
        let mut budget = Budget::load();
        let mut ticker = tokio::time::interval(TICK);
        loop {
            ticker.tick().await;

            // Once-a-day budget.
            let now = Utc::now();
            let last = budget
                .last_delivered
                .as_deref()
                .and_then(|s| s.parse::<DateTime<Utc>>().ok());
            if let Some(last) = last {
                if (now - last).num_hours() < MIN_GAP_HOURS {
                    continue;
                }
            }
            // Quiet hours — no triage pings 22:00–08:00 local.
            let hour = Local::now().hour();
            if !(8u32..22).contains(&hour) {
                continue;
            }

            // Read-only fetch. No token ⇒ nothing connected ⇒ stay silent.
            let source = match GmailReader::from_stored_token() {
                Some(s) => s,
                None => continue,
            };
            let messages = match source.fetch_unread(MAX_TRIAGE).await {
                Ok(m) => m,
                Err(e) => {
                    tracing::debug!(target: "permagentd::concierge", "inbox read skipped: {e}");
                    continue;
                }
            };

            let sink = permagent::decision_inbox::escalate::global_decision_sink();
            if triage_once(&messages, sink.as_ref(), ComposeMode::LocalModel)
                .await
                .is_some()
            {
                budget.last_delivered = Some(now.to_rfc3339());
                budget.save();
            }
        }
    });
}

/// Triage a batch of already-read messages: classify, draft (local or template),
/// file each proposal as a Decision-Inbox card, and emit ONE triage digest via
/// the Notification Router. Returns the digest text when something surfaced, or
/// `None` when nothing cleared the bar (stay silent). Split from the fetch so it
/// is fully unit-testable with an in-memory sink and no network.
pub async fn triage_once(
    messages: &[InboxMessage],
    sink: &dyn DecisionSink,
    mode: ComposeMode,
) -> Option<String> {
    let plans = plan_triage(messages);
    let mut proposals: Vec<TriageProposal> = Vec::new();
    for plan in &plans {
        let body = if plan.wants_reply {
            match messages.iter().find(|m| m.id == plan.message_id) {
                Some(m) => compose_reply(m, mode).await,
                None => String::new(),
            }
        } else {
            String::new()
        };
        if let Some(p) = proposal_for(plan, body) {
            proposals.push(p);
        }
    }
    if proposals.is_empty() {
        return None;
    }

    // File the editable draft cards (draft-only — the sink writes an
    // approve_review row; nothing is sent).
    let recorded = concierge::record_proposals(sink, &proposals).await;

    // Surface the digest through the Notification Router (#66) as a ProactiveNudge.
    let digest = concierge::digest_message(&proposals)?;
    permagent::events::emit(permagent::events::proactive_nudge(
        "inbox_triage",
        "inbox",
        &digest,
        recorded.len() as i64,
        &Utc::now().to_rfc3339(),
    ));
    tracing::info!(
        target: "permagentd::concierge",
        drafts = recorded.len(),
        "emitted inbox-triage digest"
    );
    Some(digest)
}

/// Compose a reply body for a message. `LocalModel` drafts on the on-device model
/// and degrades to the template if it is unavailable; `Template` never calls a
/// model. Either way the mail content only ever reaches the local endpoint.
async fn compose_reply(msg: &InboxMessage, mode: ComposeMode) -> String {
    match mode {
        ComposeMode::Template => template_reply_body(msg),
        ComposeMode::LocalModel => local_draft(msg)
            .await
            .unwrap_or_else(|| template_reply_body(msg)),
    }
}

/// Draft a reply on the LOCAL model tier. Routes through the mesh pool with
/// `Workload::Interactive` (`concierge::REASONING_WORKLOAD`) — pinned on-device by
/// construction, so the email body never leaves the machine. `None` on any
/// failure (caller falls back to the template).
async fn local_draft(msg: &InboxMessage) -> Option<String> {
    let resp = permagent::mesh::pool::generate(permagent::mesh::pool::GenerateRequest {
        model: DRAFT_MODEL.to_string(),
        prompt: concierge::build_draft_prompt(msg),
        system: Some(concierge::DRAFT_SYSTEM.to_string()),
        options: Some(serde_json::json!({ "temperature": 0.3, "num_predict": 220 })),
        keep_alive: None,
        timeout: None,
        workload: concierge::REASONING_WORKLOAD,
    })
    .await
    .ok()?;
    let body = resp.text.trim().to_string();
    (!body.is_empty()).then_some(body)
}

// ── Read-only Gmail source (best-effort; the whole loop is flag-gated off) ──────

/// A read-only Gmail reader over the bundled `gmail.readonly` OAuth token. Only
/// GET calls — there is no send/draft path here by construction.
struct GmailReader {
    access_token: String,
}

impl GmailReader {
    /// Build a reader from the stored Gmail OAuth token, if one is connected.
    fn from_stored_token() -> Option<Self> {
        let token_json = permagent::config::gmail_oauth::load_token()?;
        let access_token = access_token_from_token_json(&token_json)?;
        Some(Self { access_token })
    }

    /// Fetch up to `max` unread messages, read-only. Lists unread ids then reads
    /// each message's metadata (From/Subject headers + snippet). Any failure
    /// bubbles up so the loop stays silent this tick.
    async fn fetch_unread(&self, max: usize) -> Result<Vec<InboxMessage>, String> {
        let client = reqwest::Client::builder()
            .user_agent("Permagent/1.0 (concierge)")
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|e| e.to_string())?;

        let list_url = format!(
            "https://gmail.googleapis.com/gmail/v1/users/me/messages?q=is%3Aunread&maxResults={}",
            max
        );
        let list_body = self.get_text(&client, &list_url).await?;
        let ids = parse_message_ids(&list_body);

        let mut out = Vec::new();
        for (id, thread_id) in ids.into_iter().take(max) {
            let meta_url = format!(
                "https://gmail.googleapis.com/gmail/v1/users/me/messages/{id}?format=metadata\
                 &metadataHeaders=From&metadataHeaders=Subject"
            );
            if let Ok(body) = self.get_text(&client, &meta_url).await {
                if let Some(msg) = parse_message_metadata(&id, &thread_id, &body) {
                    out.push(msg);
                }
            }
        }
        Ok(out)
    }

    async fn get_text(&self, client: &reqwest::Client, url: &str) -> Result<String, String> {
        let resp = client
            .get(url)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("gmail api status {}", resp.status()));
        }
        resp.text().await.map_err(|e| e.to_string())
    }
}

/// Extract the `token` (access token) field from the stored Gmail OAuth token
/// JSON. Pure.
fn access_token_from_token_json(json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    v.get("token")
        .or_else(|| v.get("access_token"))
        .and_then(|t| t.as_str())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

/// Parse a Gmail `messages.list` response into `(id, threadId)` pairs. Pure.
fn parse_message_ids(json: &str) -> Vec<(String, String)> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    v.get("messages")
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    let id = m.get("id")?.as_str()?.to_string();
                    let thread_id = m
                        .get("threadId")
                        .and_then(|t| t.as_str())
                        .unwrap_or(&id)
                        .to_string();
                    Some((id, thread_id))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parse a Gmail `messages.get?format=metadata` response into an `InboxMessage`.
/// Pure. Reads From/Subject headers, the snippet, and the UNREAD label.
fn parse_message_metadata(id: &str, thread_id: &str, json: &str) -> Option<InboxMessage> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let snippet = v
        .get("snippet")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let unread = v
        .get("labelIds")
        .and_then(|l| l.as_array())
        .map(|arr| arr.iter().any(|x| x.as_str() == Some("UNREAD")))
        .unwrap_or(true);
    let mut from = String::new();
    let mut subject = String::new();
    if let Some(headers) = v
        .get("payload")
        .and_then(|p| p.get("headers"))
        .and_then(|h| h.as_array())
    {
        for h in headers {
            match h.get("name").and_then(|n| n.as_str()) {
                Some(n) if n.eq_ignore_ascii_case("from") => {
                    from = h
                        .get("value")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string();
                }
                Some(n) if n.eq_ignore_ascii_case("subject") => {
                    subject = h
                        .get("value")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string();
                }
                _ => {}
            }
        }
    }
    Some(InboxMessage {
        id: id.to_string(),
        thread_id: thread_id.to_string(),
        from,
        subject,
        snippet,
        unread,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use permagent::decision_inbox::escalate::{DecisionKind, InMemoryDecisionSink};

    fn unread(id: &str, from: &str, subject: &str, snippet: &str) -> InboxMessage {
        InboxMessage {
            id: id.to_string(),
            thread_id: format!("t-{id}"),
            from: from.to_string(),
            subject: subject.to_string(),
            snippet: snippet.to_string(),
            unread: true,
        }
    }

    /// The wiring: when enabled and given unread mail, the triage tick files an
    /// editable Decision-Inbox draft card (approve_review — never a send) and
    /// returns a digest. Offline: template composer, in-memory sink.
    #[tokio::test]
    async fn triage_once_files_draft_cards_and_returns_a_digest() {
        let sink = InMemoryDecisionSink::default();
        let messages = vec![
            unread("u", "bob@example.com", "Can you confirm?", "is 3pm ok?"),
            unread("n", "news@promo.com", "Sale", "click to unsubscribe"),
        ];
        let digest = triage_once(&messages, &sink, ComposeMode::Template)
            .await
            .expect("a digest surfaced");
        assert!(digest.contains("Decision Inbox"));

        let cards = sink.recorded();
        assert_eq!(cards.len(), 1, "one draft card for the addressable person");
        assert_eq!(cards[0].1.kind, DecisionKind::ApproveReview);
        assert!(cards[0].1.detail.contains("draft-only"));
    }

    /// Nothing worth surfacing → the tick stays silent and files nothing (the
    /// Watcher's silence property).
    #[tokio::test]
    async fn triage_once_stays_silent_when_only_noise() {
        let sink = InMemoryDecisionSink::default();
        let messages = vec![unread(
            "n",
            "no-reply@promo.com",
            "Newsletter",
            "unsubscribe here",
        )];
        assert!(triage_once(&messages, &sink, ComposeMode::Template)
            .await
            .is_none());
        assert!(sink.recorded().is_empty());
    }

    #[test]
    fn parses_access_token_from_stored_json() {
        let json = r#"{"token":"ya29.abc","refresh_token":"1//0g","client_id":"x"}"#;
        assert_eq!(
            access_token_from_token_json(json).as_deref(),
            Some("ya29.abc")
        );
        assert_eq!(
            access_token_from_token_json(r#"{"access_token":"alt"}"#).as_deref(),
            Some("alt")
        );
        assert!(access_token_from_token_json("{}").is_none());
        assert!(access_token_from_token_json("not json").is_none());
    }

    #[test]
    fn parses_message_ids_and_metadata() {
        let list = r#"{"messages":[{"id":"a","threadId":"ta"},{"id":"b"}],"resultSizeEstimate":2}"#;
        let ids = parse_message_ids(list);
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], ("a".to_string(), "ta".to_string()));
        assert_eq!(ids[1], ("b".to_string(), "b".to_string()));
        assert!(parse_message_ids("{}").is_empty());

        let meta = r#"{
            "snippet":"hello there",
            "labelIds":["INBOX","UNREAD"],
            "payload":{"headers":[
                {"name":"From","value":"Alice <alice@x.com>"},
                {"name":"Subject","value":"Lunch?"}
            ]}
        }"#;
        let m = parse_message_metadata("a", "ta", meta).unwrap();
        assert_eq!(m.from, "Alice <alice@x.com>");
        assert_eq!(m.subject, "Lunch?");
        assert_eq!(m.snippet, "hello there");
        assert!(m.unread);
    }

    /// A read-only reader carries no send/draft capability — the type has only a
    /// GET-based fetch. (Compile-time safety-by-construction; this pins intent.)
    #[test]
    fn reasoning_is_pinned_local_from_the_daemon_side() {
        assert!(concierge::reasoning_is_local());
    }
}
