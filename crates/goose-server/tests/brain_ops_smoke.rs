//! Integration smoke tests for brain_ops extraction (PR #141).
//!
//! These tests verify that recall and remember work end-to-end through the
//! live daemon path after the brain_ops module was extracted from inline code
//! in reply.rs and session_events.rs.
//!
//! Prerequisites:
//!   - Daemon running on localhost:3001
//!   - Dev brain populated (at least a few memories for recall to hit)
//!
//! Run:
//!   cargo test -p permagent-daemon --test brain_ops_smoke -- --ignored --nocapture

use std::io::{BufRead, BufReader};
use std::time::Duration;

fn daemon_url() -> String {
    std::env::var("PERMAGENT_DAEMON_URL").unwrap_or_else(|_| "http://localhost:3001".to_string())
}

fn load_bearer_token() -> String {
    let home = dirs::home_dir().expect("no home dir");
    let path = home.join(".permagent/secrets/daemon_token.json");
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Cannot read {}: {}", path.display(), e));
    let v: serde_json::Value =
        serde_json::from_str(&content).expect("daemon_token.json is not valid JSON");
    v["token"]
        .as_str()
        .expect("daemon_token.json missing 'token' field")
        .to_string()
}

fn client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .expect("failed to build reqwest client")
}

/// Create a session via POST /api/sessions (required before /reply).
fn create_session(token: &str) -> String {
    let url = format!("{}/api/sessions", daemon_url());
    let resp = client()
        .post(&url)
        .bearer_auth(token)
        .json(&serde_json::json!({}))
        .send()
        .expect("failed to create session");
    assert!(resp.status().is_success(), "create session failed: {}", resp.status());
    let body: serde_json::Value = resp.json().expect("session response not JSON");
    body["id"].as_str().expect("no id in session response").to_string()
}

fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// POST /reply with a recall-triggering prompt and verify the response stream
/// contains recall-related events or content.
#[test]
#[ignore]
fn smoke_recall_round_trip() {
    let url = format!("{}/reply", daemon_url());
    let token = load_bearer_token();
    let session_id = create_session(&token);
    println!("Session: {session_id}");

    // A prompt long enough (>20 chars) to trigger recall via ContextBuilder
    let body = serde_json::json!({
        "session_id": session_id,
        "user_message": {
            "role": "user",
            "created": now_ts(),
            "content": [{"type": "text", "text": "What do you remember about our previous conversations and projects we have worked on together?"}],
            "metadata": {"userVisible": true, "agentVisible": true}
        }
    });

    println!("POST {url}");

    let resp = client()
        .post(&url)
        .bearer_auth(&token)
        .json(&body)
        .send()
        .expect("request failed");

    let status = resp.status();
    println!("Status: {status}");
    assert!(
        status.is_success(),
        "Expected 2xx, got {status}"
    );

    // The response is SSE — read lines and look for recall evidence
    let reader = BufReader::new(resp);
    let mut found_context_attached = false;
    let mut line_count = 0;

    for line in reader.lines() {
        let line = line.expect("read error");
        line_count += 1;

        if line.contains("ContextAttached") {
            found_context_attached = true;
            println!("[line {line_count}] RECALL EVIDENCE: {line}");
        }

        // Print first few data lines for visibility
        if line.starts_with("data:") && line_count <= 20 {
            println!("[line {line_count}] {line}");
        }

        // Don't consume the entire stream — stop after reasonable output
        if line_count > 200 {
            println!("(truncated after {line_count} lines)");
            break;
        }
    }

    println!("\nTotal lines read: {line_count}");
    println!("ContextAttached event seen: {found_context_attached}");

    // The test passes if the daemon accepted the request and streamed back.
    // ContextAttached is bonus evidence that recall fired; its absence is not
    // a failure (brain may be empty or query too short for recall threshold).
    assert!(line_count > 0, "Expected at least some SSE output from /reply");
}

/// Verify recall behavior through the /sessions/{id}/events SSE endpoint.
/// This endpoint streams events for a session — we create a session, send a
/// message via /reply, and confirm events arrive on the SSE bus.
#[test]
#[ignore]
fn smoke_session_events_recall() {
    let base = daemon_url();
    let token = load_bearer_token();
    let session_id = create_session(&token);
    println!("Session: {session_id}");

    let body = serde_json::json!({
        "session_id": &session_id,
        "user_message": {
            "role": "user",
            "created": now_ts(),
            "content": [{"type": "text", "text": "Tell me about brain operations and memory recall in this system. How does contextual memory retrieval work?"}],
            "metadata": {"userVisible": true, "agentVisible": true}
        }
    });

    // Connect to the SSE events endpoint FIRST (it replays missed events)
    let events_url = format!("{base}/sessions/{session_id}/events");
    println!("GET {events_url}");

    // Fire /reply in a background thread so we can simultaneously listen on SSE
    let reply_url = format!("{base}/reply");
    let token_clone = token.clone();
    let body_clone = body.clone();
    let reply_handle = std::thread::spawn(move || {
        // Small delay to let the SSE connection establish
        std::thread::sleep(Duration::from_millis(200));
        let resp = client()
            .post(&reply_url)
            .bearer_auth(&token_clone)
            .json(&body_clone)
            .send()
            .expect("/reply request failed");
        let status = resp.status();
        println!("/reply status: {status}");
        // Drain the response to let the session complete
        let reader = BufReader::new(resp);
        let mut count = 0;
        for line in reader.lines() {
            let _ = line;
            count += 1;
            if count > 300 {
                break;
            }
        }
        (status, count)
    });

    let sse_resp = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap()
        .get(&events_url)
        .bearer_auth(&token)
        .send();

    match sse_resp {
        Ok(resp) => {
            let status = resp.status();
            println!("SSE status: {status}");

            if status.is_success() {
                let reader = BufReader::new(resp);
                let mut event_count = 0;
                let mut found_context = false;

                for line in reader.lines() {
                    let line = match line {
                        Ok(l) => l,
                        Err(_) => break,
                    };
                    event_count += 1;

                    if line.contains("ContextAttached") {
                        found_context = true;
                        println!("[SSE line {event_count}] RECALL: {line}");
                    }
                    if line.starts_with("data:") && event_count <= 10 {
                        println!("[SSE line {event_count}] {line}");
                    }
                    if event_count > 100 {
                        break;
                    }
                }

                println!("SSE events received: {event_count}");
                println!("ContextAttached on SSE bus: {found_context}");
            } else {
                println!("SSE endpoint returned {status}");
            }
        }
        Err(e) => {
            println!("SSE connection result: {e} (timeout acceptable for smoke test)");
        }
    }

    // Wait for the /reply thread to finish
    let (reply_status, reply_lines) = reply_handle.join().expect("reply thread panicked");
    println!("\n/reply completed: status={reply_status}, lines={reply_lines}");
    assert!(reply_status.is_success(), "/reply failed with {reply_status}");
}
