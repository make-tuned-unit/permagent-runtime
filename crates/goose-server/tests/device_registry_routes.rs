//! Device registry (#628), end to end through the COMPOSED router
//! (`routes::configure`) — the exact router the daemon serves, including the
//! bearer middleware and origin guard.
//!
//! Proves the full pairing lifecycle:
//! - `/api/devices*` management routes require a bearer token;
//! - `POST /api/devices/pair` mints a claim code (no token in that response);
//! - the public `POST /pair/claim` exchanges it exactly once for a fresh
//!   device token — the only response that ever carries the token value;
//! - the device token then authenticates protected routes (and last-seen
//!   appears), while the legacy master token keeps working unchanged;
//! - rename + revoke work over HTTP, and a revoked device token is 401
//!   immediately;
//! - the device list never echoes token material.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

fn req(
    method: &str,
    uri: &str,
    bearer: Option<&str>,
    body: Option<serde_json::Value>,
) -> Request<Body> {
    let mut b = Request::builder().method(method).uri(uri);
    if let Some(t) = bearer {
        b = b.header("authorization", format!("Bearer {t}"));
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
// against (same note as auth_plane.rs / events_wire.rs).
#[tokio::test(flavor = "multi_thread")]
async fn device_pairing_lifecycle_over_http() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("PERMAGENT_PATH_ROOT", tmp.path());

    let state = permagent_daemon::state::AppState::new(true).await.unwrap();
    let master = state
        .daemon_token
        .clone()
        .expect("test AppState should have generated a daemon token");
    let app = permagent_daemon::routes::configure(state.clone());

    // ── Management routes are bearer-protected ──
    for (method, uri, body) in [
        ("GET", "/api/devices", None),
        (
            "POST",
            "/api/devices/pair",
            Some(serde_json::json!({"name": "iPhone"})),
        ),
        ("POST", "/api/devices/ghost/revoke", None),
    ] {
        let resp = app
            .clone()
            .oneshot(req(method, uri, None, body))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "{method} {uri} must 401 without a token"
        );
    }

    // ── Empty registry lists cleanly with the master token ──
    let resp = app
        .clone()
        .oneshot(req("GET", "/api/devices", Some(&master), None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await, serde_json::json!([]));

    // ── Pair: mint a claim code (hub side, master-authenticated) ──
    let resp = app
        .clone()
        .oneshot(req(
            "POST",
            "/api/devices/pair",
            Some(&master),
            Some(serde_json::json!({"name": "iPhone"})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let pair = body_json(resp).await;
    let claim_code = pair["claim_code"].as_str().expect("claim_code").to_string();
    assert!(pair["expires_at"].is_string());
    assert!(
        pair.get("token").is_none(),
        "pairing response must carry a claim code, never a token"
    );

    // Blank names are rejected.
    let resp = app
        .clone()
        .oneshot(req(
            "POST",
            "/api/devices/pair",
            Some(&master),
            Some(serde_json::json!({"name": "  "})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // ── Claim: the public exchange — token appears exactly here, once ──
    let resp = app
        .clone()
        .oneshot(req(
            "POST",
            "/pair/claim",
            None,
            Some(serde_json::json!({"code": claim_code})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "claim must be public");
    let claimed = body_json(resp).await;
    let device_token = claimed["token"].as_str().expect("token").to_string();
    let device_id = claimed["device"]["id"].as_str().expect("id").to_string();
    assert_eq!(claimed["device"]["name"], "iPhone");

    // Single-use: the same code is dead now.
    let resp = app
        .clone()
        .oneshot(req(
            "POST",
            "/pair/claim",
            None,
            Some(serde_json::json!({"code": claim_code})),
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "claim codes are single-use"
    );

    // Garbage codes: same answer (no oracle).
    let resp = app
        .clone()
        .oneshot(req(
            "POST",
            "/pair/claim",
            None,
            Some(serde_json::json!({"code": "totally-wrong"})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // ── The device token authenticates protected routes ──
    let resp = app
        .clone()
        .oneshot(req("GET", "/api/devices", Some(&device_token), None))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "device token must clear auth"
    );
    let list = body_json(resp).await;
    let listed = &list.as_array().expect("array")[0];
    assert_eq!(listed["id"], device_id.as_str());
    assert!(
        listed["last_seen"].is_string(),
        "the authenticated device request itself must stamp last_seen"
    );
    // The list must never echo token material.
    let raw = list.to_string();
    assert!(!raw.contains(&device_token));
    assert!(!raw.contains("token_hash"));

    // Legacy master token still works alongside (zero-breakage).
    let resp = app
        .clone()
        .oneshot(req("GET", "/api/devices", Some(&master), None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // ── Rename ──
    let resp = app
        .clone()
        .oneshot(req(
            "PATCH",
            &format!("/api/devices/{device_id}"),
            Some(&master),
            Some(serde_json::json!({"name": "Jesse's iPhone"})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["name"], "Jesse's iPhone");

    let resp = app
        .clone()
        .oneshot(req(
            "PATCH",
            "/api/devices/ghost",
            Some(&master),
            Some(serde_json::json!({"name": "x"})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // ── Revoke: the device token dies immediately ──
    let resp = app
        .clone()
        .oneshot(req(
            "POST",
            &format!("/api/devices/{device_id}/revoke"),
            Some(&master),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["revoked"], true);

    let resp = app
        .clone()
        .oneshot(req("GET", "/api/devices", Some(&device_token), None))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "a revoked device token must be rejected immediately"
    );

    // Master remains untouched by device revocation.
    let resp = app
        .clone()
        .oneshot(req("GET", "/api/devices", Some(&master), None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    eprintln!("✓ device pairing lifecycle: claim-code mint → one-time exchange → device-token auth + last-seen → rename → revoke, with the legacy master token intact.");
}
