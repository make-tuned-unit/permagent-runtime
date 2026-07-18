//! Agent control plane authentication (C1/C2 launch audit), end to end
//! through the COMPOSED router (`routes::configure`) — the exact router the
//! daemon serves, including the bearer/stream-token middleware, the origin
//! guard, and the access log.
//!
//! Proves:
//! - `/sessions/{id}/reply` + `/sessions/{id}/cancel` (agent invocation, C1)
//!   reject without a valid token and admit with one (header or `?token=`).
//! - `/sessions/{id}/events` (per-session SSE, C2) rejects without a token.
//! - `/events` (global bus WS, C2) rejects the upgrade without a token and
//!   accepts either the `?token=` query or the bearer header (CLI parity) —
//!   over a REAL TCP socket, like `events_wire.rs`.
//! - The origin guard 403s foreign browser origins on every rail — even with
//!   a valid token — while the Tauri origin, same-origin Host matches, and
//!   Origin-less native clients pass.
//! - `/diagnostics/{session_id}` (logs+config zip) is no longer public.
//! - `/voice` rejects a tokenless upgrade (fail-closed after H3).
//!
//! Fail-closed-when-no-token (H3) is covered at the unit level in
//! `middleware::auth` (`validate_token_value`): `AppState::new` always
//! generates a token under the test root, so the None case cannot be built
//! through the public constructor — by design.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

fn reply_body() -> serde_json::Value {
    serde_json::json!({
        "request_id": "req-auth-plane-1",
        "user_message": {
            "role": "user",
            "created": 0,
            "content": [{ "type": "text", "text": "hi" }]
        }
    })
}

/// Build a request; `bearer`/`origin`/`host` are optional headers.
fn req(
    method: &str,
    uri: &str,
    bearer: Option<&str>,
    origin: Option<&str>,
    host: Option<&str>,
    body: Option<serde_json::Value>,
) -> Request<Body> {
    let mut b = Request::builder().method(method).uri(uri);
    if let Some(t) = bearer {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    if let Some(o) = origin {
        b = b.header("origin", o);
    }
    if let Some(h) = host {
        b = b.header("host", h);
    }
    match body {
        Some(v) => b
            .header("content-type", "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => b.body(Body::empty()).unwrap(),
    }
}

// One test per integration binary (own process): PERMAGENT_PATH_ROOT and the
// startup singletons are per-process, so #[serial] has nothing to serialize
// against (same note as events_wire.rs / decisions_lifecycle.rs).
#[tokio::test(flavor = "multi_thread")]
async fn control_plane_requires_token_and_our_origin() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("PERMAGENT_PATH_ROOT", tmp.path());

    let state = permagent_daemon::state::AppState::new(true).await.unwrap();
    let token = state
        .daemon_token
        .clone()
        .expect("test AppState should have generated a daemon token");
    let app = permagent_daemon::routes::configure(state.clone());

    // ── Public baseline: /status stays reachable with no credentials ──
    let resp = app
        .clone()
        .oneshot(req("GET", "/status", None, None, None, None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "/status must stay public");

    // ── C1: POST /sessions/{id}/reply — reject anonymous & wrong tokens ──
    for (name, request) in [
        (
            "no token",
            req(
                "POST",
                "/sessions/ghost/reply",
                None,
                None,
                None,
                Some(reply_body()),
            ),
        ),
        (
            "wrong bearer",
            req(
                "POST",
                "/sessions/ghost/reply",
                Some("not-the-token"),
                None,
                None,
                Some(reply_body()),
            ),
        ),
        (
            "wrong query token",
            req(
                "POST",
                "/sessions/ghost/reply?token=not-the-token",
                None,
                None,
                None,
                Some(reply_body()),
            ),
        ),
    ] {
        let resp = app.clone().oneshot(request).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "reply must 401 with {name}"
        );
    }

    // With the real token the auth layer admits the request (whatever the
    // handler then says about a nonexistent session is its own contract).
    let resp = app
        .clone()
        .oneshot(req(
            "POST",
            "/sessions/ghost/reply",
            Some(&token),
            None,
            None,
            Some(reply_body()),
        ))
        .await
        .unwrap();
    assert!(
        resp.status() != StatusCode::UNAUTHORIZED
            && resp.status() != StatusCode::FORBIDDEN
            && resp.status() != StatusCode::SERVICE_UNAVAILABLE,
        "reply with valid token must clear auth, got {}",
        resp.status()
    );

    // ── C1: POST /sessions/{id}/cancel ──
    let resp = app
        .clone()
        .oneshot(req(
            "POST",
            "/sessions/ghost/cancel",
            None,
            None,
            None,
            Some(serde_json::json!({"request_id": "r1"})),
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "cancel must 401 without a token"
    );

    let resp = app
        .clone()
        .oneshot(req(
            "POST",
            "/sessions/ghost/cancel",
            Some(&token),
            None,
            None,
            Some(serde_json::json!({"request_id": "r1"})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "cancel with token → 200");
    let v = body_json(resp).await;
    assert_eq!(v["cancelled"], false, "nothing to cancel → honest false");

    // ── C2: GET /sessions/{id}/events (SSE) ──
    let resp = app
        .clone()
        .oneshot(req("GET", "/sessions/ghost/events", None, None, None, None))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "session SSE must 401 without a token"
    );

    // `?token=` clears auth (EventSource cannot set headers); the unknown
    // session then 404s — proof the request reached the handler.
    let resp = app
        .clone()
        .oneshot(req(
            "GET",
            &format!("/sessions/ghost/events?token={token}"),
            None,
            None,
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "session SSE with ?token= must clear auth (404 = handler ran)"
    );

    // The bearer header works on the SSE route too (native consumers).
    let resp = app
        .clone()
        .oneshot(req(
            "GET",
            "/sessions/ghost/events",
            Some(&token),
            None,
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // ── Diagnostics zip is no longer public ──
    let resp = app
        .clone()
        .oneshot(req("GET", "/diagnostics/ghost", None, None, None, None))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "/diagnostics/{{id}} must require the token"
    );

    // ── Origin guard: foreign browser origins are 403 on every rail ──
    // Even WITH a valid token: a malicious page that somehow stole one still
    // cannot drive the daemon cross-origin from a browser.
    let resp = app
        .clone()
        .oneshot(req(
            "POST",
            "/sessions/ghost/cancel",
            Some(&token),
            Some("https://evil.example"),
            Some("127.0.0.1:3001"),
            Some(serde_json::json!({"request_id": "r1"})),
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "foreign Origin must be rejected even with a valid token"
    );

    let resp = app
        .clone()
        .oneshot(req(
            "GET",
            "/status",
            None,
            Some("https://evil.example"),
            Some("127.0.0.1:3001"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "foreign Origin must be rejected on public routes too"
    );

    // Our origins pass: the Tauri webview, and a same-origin companion device
    // reaching the daemon-served UI over the tailnet.
    let resp = app
        .clone()
        .oneshot(req(
            "GET",
            "/status",
            None,
            Some("tauri://localhost"),
            Some("127.0.0.1:3001"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "Tauri origin must pass");

    let resp = app
        .clone()
        .oneshot(req(
            "GET",
            "/status",
            None,
            Some("http://100.64.0.7:3001"),
            Some("100.64.0.7:3001"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "same-origin (Origin == Host) must pass"
    );

    // ── /events WS over a REAL TCP socket (events_wire.rs pattern) ──
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let served = permagent_daemon::routes::configure(state.clone());
    tokio::spawn(async move {
        axum::serve(
            listener,
            served.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });

    use tokio_tungstenite::tungstenite;

    // Tokenless upgrade → HTTP 401, no 101.
    match tokio_tungstenite::connect_async(&format!("ws://{addr}/events")).await {
        Err(tungstenite::Error::Http(resp)) => {
            assert_eq!(resp.status(), 401, "tokenless /events upgrade must 401")
        }
        other => panic!("tokenless /events upgrade must fail with 401, got {other:?}"),
    }

    // Wrong token → 401.
    match tokio_tungstenite::connect_async(&format!("ws://{addr}/events?token=wrong")).await {
        Err(tungstenite::Error::Http(resp)) => assert_eq!(resp.status(), 401),
        other => panic!("wrong-token /events upgrade must fail with 401, got {other:?}"),
    }

    // Valid ?token= → 101 (browser path).
    let (ws, resp) = tokio_tungstenite::connect_async(&format!("ws://{addr}/events?token={token}"))
        .await
        .expect("valid ?token= must upgrade");
    assert_eq!(resp.status(), 101);
    drop(ws);

    // Valid bearer header → 101 (CLI `activity tail` path).
    let request = tungstenite::http::Request::builder()
        .uri(format!("ws://{addr}/events"))
        .header("Authorization", format!("Bearer {token}"))
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header(
            "Sec-WebSocket-Key",
            tungstenite::handshake::client::generate_key(),
        )
        .header("Host", "127.0.0.1")
        .body(())
        .unwrap();
    let (ws, resp) = tokio_tungstenite::connect_async(request)
        .await
        .expect("bearer-header /events upgrade must succeed");
    assert_eq!(resp.status(), 101);
    drop(ws);

    // Foreign Origin on the WS handshake → 403 before any upgrade, token or not.
    let request = tungstenite::http::Request::builder()
        .uri(format!("ws://{addr}/events?token={token}"))
        .header("Origin", "https://evil.example")
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header(
            "Sec-WebSocket-Key",
            tungstenite::handshake::client::generate_key(),
        )
        .header("Host", "127.0.0.1")
        .body(())
        .unwrap();
    match tokio_tungstenite::connect_async(request).await {
        Err(tungstenite::Error::Http(resp)) => {
            assert_eq!(resp.status(), 403, "foreign-Origin WS must be forbidden")
        }
        other => panic!("foreign-Origin WS must fail with 403, got {other:?}"),
    }

    // ── /voice fails closed too: tokenless upgrade must NOT be served ──
    match tokio_tungstenite::connect_async(&format!("ws://{addr}/voice")).await {
        Err(tungstenite::Error::Http(resp)) => {
            assert_eq!(resp.status(), 401, "tokenless /voice upgrade must 401")
        }
        other => panic!("tokenless /voice upgrade must fail with 401, got {other:?}"),
    }

    eprintln!("✓ control plane rejects anonymous + foreign-origin access and admits the daemon token on every rail.");
}
