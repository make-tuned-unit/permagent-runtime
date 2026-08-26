//! Tell the daemon a turn just finished, so the Build tab's cost meter moves.
//!
//! WHY THIS EXISTS. The harness and the daemon are two processes sharing one
//! `permagent.db`. Cost is written by the harness, in-process, through the same
//! `append_cost_ledger` the daemon uses — so the numbers are already correct
//! and already durable the instant a turn ends. What is missing is not data,
//! it is NOTIFICATION: the daemon's event bus lives in the daemon's process, so
//! a `session_spend_changed` emitted here would reach nobody, and the browser
//! has never been told that this session id exists at all. The harness mints
//! its own session (`get_or_create_session_id`, "CLI Session") and the Build
//! tab's meter subscribes to the browser's chat session, which is idle for the
//! whole time the user is coding. That is the $0.00.
//!
//! So this sends the smallest possible thing: an id and a "look again". The
//! daemon re-reads the rollup the harness just wrote and announces it on the
//! bus. No figures cross this boundary, because a second writer of
//! `accumulated_cost_usd` would double every number on the meter it feeds.
//!
//! SILENT BY CONSTRUCTION. A bare `permagent run` in a terminal with no daemon
//! running is a completely normal thing to do, and it is not the cost meter's
//! business to interrupt it. Every failure here — no daemon, no token, refused,
//! timed out — is swallowed. The harness prints its own cost line from the same
//! ledger regardless (`output::display_session_cost`), so the user is never
//! left without the number; only the other window's copy of it is missed.

use std::time::Duration;

/// How long a turn's announcement may take before it is abandoned.
///
/// Short on purpose. This runs on a detached task, so it cannot stall the REPL,
/// but an unbounded request against a wedged daemon would leak one task per
/// turn for the life of the session.
const ANNOUNCE_TIMEOUT: Duration = Duration::from_secs(3);

/// Announce, in the background, that `session_id` has spent more.
///
/// Returns immediately. `final_turn` marks the session's last word, so the
/// meter can hold a finished session's total rather than letting it decay.
pub fn announce(session_id: &str, final_turn: bool) {
    let session_id = session_id.to_string();
    let working_dir = std::env::current_dir()
        .ok()
        .map(|p| p.display().to_string());
    tokio::spawn(async move {
        let _ = post(&session_id, working_dir.as_deref(), final_turn).await;
    });
}

/// Announce and WAIT.
///
/// For the closing announcement only: `announce` detaches, and a detached task
/// is dropped when the process exits — which is every time the session ends, so
/// the final total would be the one announcement guaranteed never to arrive.
/// Still silent, and still bounded by [`ANNOUNCE_TIMEOUT`], so a wedged daemon
/// delays the exit by seconds rather than hanging it.
pub async fn announce_now(session_id: &str, final_turn: bool) {
    let working_dir = std::env::current_dir()
        .ok()
        .map(|p| p.display().to_string());
    let _ = post(session_id, working_dir.as_deref(), final_turn).await;
}

/// The request itself, separated so it can be awaited (and tested) directly.
async fn post(session_id: &str, working_dir: Option<&str>, final_turn: bool) -> anyhow::Result<()> {
    let port = crate::commands::daemon::read_daemon_port();
    let token = crate::commands::daemon::load_daemon_token()?;
    let client = reqwest::Client::builder()
        .timeout(ANNOUNCE_TIMEOUT)
        .build()?;
    client
        .post(format!("http://127.0.0.1:{port}/api/coding-sessions/spend"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "sessionId": session_id,
            "workingDir": working_dir,
            "finalTurn": final_turn,
        }))
        .send()
        .await?;
    Ok(())
}

/// The session's closing line: what the whole session cost.
///
/// The per-turn line (`display_session_cost`) already says "$X this turn · $Y
/// session", but it is printed BEFORE the next prompt — so the last turn's copy
/// scrolls away under whatever the user does next, and a session that ends with
/// `/exit` never prints one at all. A session whose total is only recoverable
/// by scrolling back is a session whose total was not reported.
pub fn format_session_total(session_usd: Option<f64>, total_tokens: i64) -> Option<String> {
    let total = session_usd?;
    // A local model spends tokens and no money. Saying "$0.00" for that reads
    // as a broken meter; saying nothing about the money and reporting the
    // tokens is the honest version — the same distinction `format_cost_line`
    // draws for the per-turn line.
    if total == 0.0 && total_tokens > 0 {
        return Some(format!(
            "Session total: {total_tokens} tokens · no API spend"
        ));
    }
    Some(format!(
        "Session total: ${total:.2} · {total_tokens} tokens"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_local_session_reports_tokens_rather_than_a_zero_dollar_bill() {
        assert_eq!(
            format_session_total(Some(0.0), 12_400).unwrap(),
            "Session total: 12400 tokens · no API spend"
        );
    }

    #[test]
    fn a_paid_session_reports_what_it_cost() {
        assert_eq!(
            format_session_total(Some(1.2345), 98_000).unwrap(),
            "Session total: $1.23 · 98000 tokens"
        );
    }

    /// No ledger reading at all is not "$0.00" — it is nothing to say. A meter
    /// that invents a zero is indistinguishable from one reporting a free run.
    #[test]
    fn an_unknown_total_says_nothing() {
        assert!(format_session_total(None, 0).is_none());
        assert!(format_session_total(None, 500).is_none());
    }

    /// A session that truly spent nothing and used nothing still reports, so a
    /// session that closed immediately does not look like a reporting failure.
    #[test]
    fn an_empty_session_still_reports_zero() {
        assert_eq!(
            format_session_total(Some(0.0), 0).unwrap(),
            "Session total: $0.00 · 0 tokens"
        );
    }
}
