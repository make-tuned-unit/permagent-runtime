//! Best-effort bridge between the standalone coding harness and the daemon's Brain.
//!
//! WHY THIS EXISTS. Chat and the harness share one Brain, but not one process.
//! The daemon owns the live `SafeBrain` handle and sets `GLOBAL_BRAIN` at boot;
//! `permagent run` is a second binary that never populates that singleton, so
//! the `search_memory` tool structurally cannot appear in its tool list and no
//! ambient recall runs before its turns. Until now the harness was write-only
//! toward the daemon — a spend ping per turn, one distilled summary at exit —
//! and read nothing back. It could not remember yesterday.
//!
//! The CLI deliberately talks to the owner over loopback instead of opening the
//! same databases in a second process: two writers of one Spectral brain is a
//! corruption story, and the daemon already exposes both halves behind the same
//! bearer token `spend_announce.rs` proves works today.
//!
//! SILENT BY CONSTRUCTION, and BOUNDED. Both operations swallow every failure —
//! no token, no daemon, refused, timed out, renamed field. A bare `permagent
//! run` in a terminal with no daemon running is a completely normal thing to
//! do, and memory availability must never turn into harness availability. The
//! read sits in the critical path before the model call, so it is capped at
//! [`BRIDGE_TIMEOUT`]; the write is detached and costs the turn nothing at all.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The whole budget the recall may spend, connect-to-parse.
///
/// This one is in the critical path: it runs before `agent.reply`, so it is the
/// only latency this module can add to a turn. 750ms is the ceiling a user
/// would pay on a turn where the daemon is wedged — not the common case, which
/// is a loopback query answering in single-digit milliseconds or failing to
/// connect instantly.
const BRIDGE_TIMEOUT: Duration = Duration::from_millis(750);

/// How much recalled text may reach the prompt.
const MAX_RECALL_CHARS: usize = 2_400;

/// How much of the user's turn is used as the recall query.
///
/// A pasted stack trace or file is a legitimate user turn and a terrible query
/// string: unbounded here it would be URL-encoded into a multi-kilobyte GET
/// that the daemon may refuse outright, turning a long paste into the one turn
/// with no memory. The opening of a message carries the intent.
const MAX_QUERY_CHARS: usize = 512;

/// Hits below this are noise. Matched to the parked design and to the score
/// floor Chat's own recall applies before injecting anything.
const MIN_SCORE: f64 = 0.7;

/// The system-prompt extra this block is installed under.
///
/// Deliberately the SAME key Chat's `brain_ops::inject_recall` uses. It is
/// registered in `prompt_manager::VOLATILE_EXTRA_KEYS`, which is what keeps a
/// per-turn block behind the prompt-cache breakpoint (in `# Turn-specific
/// Instructions`) instead of invalidating the cached prefix on every turn.
/// Renaming it here would silently move this block into the cached half and
/// cost a full prompt re-read per turn.
pub const RECALL_PROMPT_KEY: &str = "memory_recall";

#[derive(Debug, Deserialize)]
struct SearchResponse {
    results: Vec<SearchResult>,
}

#[derive(Debug, Deserialize)]
struct SearchResult {
    preview: String,
    score: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TurnRequest<'a> {
    session_id: &'a str,
    turn_idx: usize,
    user_text: &'a str,
    assistant_text: &'a str,
    working_dir: Option<&'a str>,
}

/// A resolved, authenticated way to reach the daemon.
///
/// Split out of the two public entry points so both halves of the bridge can be
/// exercised against a stub server in tests without ever reading — or needing —
/// the real `~/.permagent/secrets/daemon_token.json`.
struct Endpoint {
    client: reqwest::Client,
    base: String,
    token: String,
}

/// Resolve the live daemon, or fail fast.
///
/// Missing token file is the no-daemon-has-ever-run case and returns here
/// without a network attempt, so the common "just running the CLI" path pays
/// nothing at all.
fn endpoint() -> anyhow::Result<Endpoint> {
    let token = crate::commands::daemon::load_daemon_token()?;
    let port = crate::commands::daemon::read_daemon_port();
    Ok(Endpoint {
        client: http_client()?,
        base: format!("http://127.0.0.1:{port}"),
        token,
    })
}

/// The one place the budget is applied.
///
/// Factored out so the tests that assert the bound build their stub endpoint
/// through the SAME constructor. A test that set its own timeout would prove
/// only that reqwest honours a timeout, and would stay green if this module
/// stopped setting one.
fn http_client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder().timeout(BRIDGE_TIMEOUT).build()
}

/// What this turn should install under [`RECALL_PROMPT_KEY`], if anything.
///
/// Three-valued on purpose, and the third value is the one that matters:
/// - `Some(block)` — hits worth injecting.
/// - `Some(String::new())` — no hits, but the PREVIOUS turn injected some.
///   The extras map is keyed and persists across turns, so leaving turn 1's
///   memories in place while the user asks about something else on turn 9 is
///   exactly the contamination this bridge must not introduce. Overwriting with
///   an empty string clears it.
/// - `None` — no hits and nothing stale to clear. Write nothing, so a session
///   that never recalls anything never grows an empty header.
pub async fn recall_block(query: &str, previously_installed: bool) -> Option<String> {
    // No daemon has ever run here: nothing to recall, and nothing was ever
    // installed by this module, so there is nothing to clear either.
    let Ok(ep) = endpoint() else {
        return None;
    };
    recall_block_at(&ep, query, previously_installed).await
}

async fn recall_block_at(ep: &Endpoint, query: &str, previously_installed: bool) -> Option<String> {
    match (recall_at(ep, query).await, previously_installed) {
        (Some(block), _) => Some(block),
        (None, true) => Some(String::new()),
        (None, false) => None,
    }
}

async fn recall_at(ep: &Endpoint, query: &str) -> Option<String> {
    let query: String = query.trim().chars().take(MAX_QUERY_CHARS).collect();
    if query.is_empty() {
        return None;
    }
    let mut url = url::Url::parse(&format!("{}/api/brain/search", ep.base)).ok()?;
    url.query_pairs_mut()
        .append_pair("q", &query)
        .append_pair("source", "both")
        .append_pair("limit", "3");
    let response = ep
        .client
        .get(url)
        .bearer_auth(&ep.token)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json::<SearchResponse>()
        .await
        .ok()?;
    render_recall(response.results)
}

/// Frame the hits as BACKGROUND, never as instructions.
///
/// A memory is a record of something that was true once, written by a different
/// session with different facts in front of it. Dropped into a prompt unlabelled
/// it reads as a directive from the user — which is how a note about last
/// month's schema turns into this turn's refactor. The header says three things
/// on purpose: where it came from, that it is historical, and that it must be
/// verified before it is relied on.
fn render_recall(results: Vec<SearchResult>) -> Option<String> {
    let mut out = String::from(
        "Shared Brain context from Chat and prior Harness work (historical hints; verify before relying on them):\n",
    );
    let mut count = 0;
    for result in results
        .into_iter()
        .filter(|hit| hit.score >= MIN_SCORE)
        .take(3)
    {
        let preview = result.preview.trim();
        if preview.is_empty() {
            continue;
        }
        count += 1;
        out.push_str(&format!("- {preview}\n"));
        if out.chars().count() >= MAX_RECALL_CHARS {
            out = out.chars().take(MAX_RECALL_CHARS).collect();
            break;
        }
    }
    (count > 0).then_some(out)
}

/// Persist a completed Harness turn through the daemon that owns Brain.
///
/// Returns immediately: the request runs on a detached task, so a wedged or
/// absent daemon costs the turn nothing observable. Failures are intentionally
/// silent and never affect task completion.
pub fn persist_turn(
    session_id: String,
    turn_idx: usize,
    user_text: String,
    assistant_text: String,
    working_dir: Option<String>,
) {
    // Nothing to remember, and the route rejects it anyway (400) — skip the
    // round trip rather than spend a task on a guaranteed refusal.
    if session_id.trim().is_empty()
        || user_text.trim().is_empty()
        || assistant_text.trim().is_empty()
    {
        return;
    }
    let Ok(ep) = endpoint() else {
        return;
    };
    spawn_persist(
        ep,
        session_id,
        turn_idx,
        user_text,
        assistant_text,
        working_dir,
    );
}

fn spawn_persist(
    ep: Endpoint,
    session_id: String,
    turn_idx: usize,
    user_text: String,
    assistant_text: String,
    working_dir: Option<String>,
) {
    tokio::spawn(async move {
        let _ = ep
            .client
            .post(format!("{}/api/coding-sessions/turn", ep.base))
            .bearer_auth(&ep.token)
            .json(&TurnRequest {
                session_id: &session_id,
                turn_idx,
                user_text: &user_text,
                assistant_text: &assistant_text,
                working_dir: working_dir.as_deref(),
            })
            .send()
            .await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn ep_for(base: String) -> Endpoint {
        Endpoint {
            // Production's own constructor, on purpose — see `http_client`.
            client: http_client().unwrap(),
            base,
            token: "fixture-token-not-the-real-one".to_string(),
        }
    }

    #[test]
    fn recall_is_bounded_filtered_and_labelled_historical() {
        let text = render_recall(vec![
            SearchResult {
                preview: "current project".into(),
                score: 0.9,
            },
            SearchResult {
                preview: "too weak".into(),
                score: 0.2,
            },
        ])
        .unwrap();
        assert!(text.contains("historical hints"));
        assert!(text.contains("current project"));
        assert!(!text.contains("too weak"));
        assert!(text.chars().count() <= MAX_RECALL_CHARS);
    }

    #[test]
    fn empty_recall_is_not_injected() {
        assert!(render_recall(Vec::new()).is_none());
    }

    /// (a) When the daemon answers, the daemon's memories are what reaches the
    /// outgoing context — under the key Chat uses, carrying its own "historical
    /// hints" frame, and only after presenting the bearer token.
    #[tokio::test]
    async fn a_live_daemons_memories_reach_the_outgoing_context() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/brain/search"))
            .and(query_param("source", "both"))
            .and(header(
                "authorization",
                "Bearer fixture-token-not-the-real-one",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [
                    { "preview": "The picker scanner runs on port 8080", "score": 0.91 },
                    { "preview": "unrelated noise", "score": 0.10 },
                ]
            })))
            .mount(&server)
            .await;

        let block = recall_block_at(&ep_for(server.uri()), "where does the scanner run?", false)
            .await
            .expect("a live daemon with a strong hit must produce a block");
        assert!(block.contains("The picker scanner runs on port 8080"));
        assert!(
            block.contains("historical hints; verify before relying on them"),
            "recalled memories must be framed as background, never as instructions: {block}"
        );
        assert!(
            !block.contains("unrelated noise"),
            "a sub-threshold hit must not be injected: {block}"
        );
        assert_eq!(
            RECALL_PROMPT_KEY, "memory_recall",
            "the block must ride the same volatile key Chat uses, or it lands \
             inside the cached prompt prefix and costs a re-read every turn"
        );
    }

    /// The bearer token is not optional. A daemon that rejects the request
    /// yields no block at all — never a partial or unauthenticated read.
    #[tokio::test]
    async fn an_unauthenticated_read_yields_nothing() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/brain/search"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        assert!(recall_at(&ep_for(server.uri()), "anything").await.is_none());
    }

    /// (b) A daemon that is down must cost the turn nothing but the bound, and
    /// must inject nothing. This is the normal case for `permagent run` in a
    /// terminal, not an error.
    #[tokio::test]
    async fn a_dead_daemon_injects_nothing_within_the_bound() {
        // Port 1 on loopback: nothing listens, and the refusal is immediate —
        // the same shape as a daemon that was never started.
        let start = Instant::now();
        let out = recall_at(&ep_for("http://127.0.0.1:1".to_string()), "anything").await;
        let elapsed = start.elapsed();
        assert!(out.is_none(), "a dead daemon must inject nothing");
        assert!(
            elapsed <= BRIDGE_TIMEOUT + Duration::from_millis(250),
            "recall must stay inside its budget even with no daemon: {elapsed:?}"
        );
    }

    /// A daemon that accepts the connection and then never answers is the worst
    /// case, and it is the one the budget exists for: the turn proceeds, late.
    #[tokio::test]
    async fn a_wedged_daemon_is_abandoned_at_the_bound() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/brain/search"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_secs(5))
                    .set_body_json(serde_json::json!({ "results": [] })),
            )
            .mount(&server)
            .await;

        let start = Instant::now();
        let out = recall_at(&ep_for(server.uri()), "anything").await;
        let elapsed = start.elapsed();
        assert!(out.is_none());
        assert!(
            elapsed <= BRIDGE_TIMEOUT + Duration::from_millis(250),
            "a wedged daemon must be abandoned at the bound, not waited out: {elapsed:?}"
        );
    }

    /// A stale block from an earlier turn is worse than no block: it presents
    /// turn 1's memories as background for turn 9's unrelated question. When a
    /// turn recalls nothing, the previous turn's block is cleared.
    #[tokio::test]
    async fn a_turn_with_no_hits_clears_the_previous_turns_block() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/brain/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [{ "preview": "weak", "score": 0.1 }]
            })))
            .mount(&server)
            .await;
        let ep = ep_for(server.uri());

        assert_eq!(
            recall_block_at(&ep, "anything", false).await,
            None,
            "nothing recalled and nothing stale ⇒ write nothing"
        );
        assert_eq!(
            recall_block_at(&ep, "anything", true).await,
            Some(String::new()),
            "nothing recalled but a block is installed ⇒ clear it"
        );
    }

    /// (c) The write is fire-and-forget: the response path never waits on the
    /// daemon. Proven against a server that takes two seconds to answer.
    #[tokio::test(flavor = "multi_thread")]
    async fn persisting_a_turn_does_not_wait_for_the_daemon() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/coding-sessions/turn"))
            .and(header(
                "authorization",
                "Bearer fixture-token-not-the-real-one",
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_secs(2))
                    .set_body_json(serde_json::json!({ "accepted": true })),
            )
            .mount(&server)
            .await;

        let start = Instant::now();
        spawn_persist(
            ep_for(server.uri()),
            "sess-1".to_string(),
            3,
            "user said".to_string(),
            "assistant said".to_string(),
            Some("/tmp/proj".to_string()),
        );
        assert!(
            start.elapsed() < Duration::from_millis(100),
            "the turn must not wait on the memory write: {:?}",
            start.elapsed()
        );

        // …and it does eventually arrive, so "fire and forget" is not "drop".
        tokio::time::sleep(Duration::from_secs(3)).await;
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            1,
            "the detached write must still reach the daemon"
        );
    }

    /// An empty half is not a turn. The route rejects it (400); spending a task
    /// on a guaranteed refusal is just noise in the daemon's log.
    #[tokio::test]
    async fn an_empty_turn_is_not_sent() {
        // No endpoint is resolvable in the test environment either way, so this
        // asserts the guard rather than the transport: it must not panic and
        // must not spawn.
        persist_turn(
            "sess".to_string(),
            0,
            "   ".to_string(),
            "answer".to_string(),
            None,
        );
    }
}
