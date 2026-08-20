//! The daemon trust boundary, end to end through the COMPOSED router
//! (`routes::configure`) — the exact router the daemon serves.
//!
//! What this proves:
//! - a request with no token is refused, and a request with the token is not;
//! - the auth audit records what it claims to record: the refusal, the route's
//!   consequence class, the admitting principal, and the status the caller got;
//! - the class policy is ENFORCED at the middleware, not merely described — a
//!   status poll admitted with a valid token produces no row, while an admitted
//!   request to a consequential route does;
//! - the peer-verification gate, in its shipped (disabled) configuration, is a
//!   true no-op through the composed router.
//!
//! What this deliberately does NOT prove, because it is not true: that any of
//! the above prevents a same-user process from using the token. It cannot. See
//! `docs/design/daemon-trust-boundary.md`.
//!
//! One test per integration binary (own process): `PERMAGENT_PATH_ROOT` and the
//! startup singletons are per-process, so `#[serial]` has nothing to serialize
//! against (same note as `auth_plane.rs` / `decision_principal_audit.rs`).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use permagent::security::auth_audit::{self, RouteClass};
use permagent::sqlx::{Pool, Row, Sqlite};
use tower::ServiceExt;

fn get(uri: &str, bearer: Option<&str>) -> Request<Body> {
    let mut b = Request::builder().method("GET").uri(uri);
    if let Some(t) = bearer {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    b.body(Body::empty()).unwrap()
}

fn post_json(uri: &str, bearer: Option<&str>, body: &str) -> Request<Body> {
    let mut b = Request::builder().method("POST").uri(uri);
    if let Some(t) = bearer {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    b.header("content-type", "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap()
}

/// Every audit row for one path, newest first.
async fn rows_for(
    pool: &Pool<Sqlite>,
    path: &str,
) -> Vec<(String, String, String, Option<String>)> {
    permagent::sqlx::query(
        "SELECT outcome, class, credential, principal FROM daemon_auth_audit \
         WHERE path = ? ORDER BY ts DESC, id DESC",
    )
    .bind(path)
    .fetch_all(pool)
    .await
    .unwrap()
    .iter()
    .map(|r| {
        (
            r.get("outcome"),
            r.get("class"),
            r.get("credential"),
            r.get("principal"),
        )
    })
    .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_token_gates_the_control_plane_and_the_audit_records_the_use() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("PERMAGENT_PATH_ROOT", tmp.path());
    // The shipped configuration. Asserted rather than assumed, so this test
    // fails loudly if the flag ever starts defaulting on.
    std::env::remove_var(permagent_daemon::middleware::peer_identity::PEER_VERIFICATION_ENV);

    let state = permagent_daemon::state::AppState::new(true).await.unwrap();
    let master_token = state.daemon_token.clone().unwrap();
    let (device_token, device) = state.device_registry.pair("Trust boundary test device");
    let pool = state.session_manager().pool_clone().await.unwrap();

    assert!(
        !state.peer_gate.is_enforcing(),
        "peer verification must ship disabled: on TCP loopback it refuses everything"
    );

    let app = permagent_daemon::routes::configure(state);

    // ── 1. No token is refused ───────────────────────────────────────────────
    // `/config/read` is classified `secrets`: it is the route that reads a
    // provider API key back out of config.
    let response = app
        .clone()
        .oneshot(post_json("/config/read", None, r#"{"key":"x"}"#))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "a tokenless caller must not reach a secrets route"
    );

    // A wrong token is refused too, and is a DIFFERENT event from no token.
    let response = app
        .clone()
        .oneshot(post_json(
            "/config/read",
            Some("not-the-real-token"),
            r#"{"key":"x"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // ── 2. The audit recorded both refusals, with the route's real class ─────
    let refusals = rows_for(&pool, "/config/read").await;
    assert_eq!(refusals.len(), 2, "both refusals must be recorded");
    for (outcome, class, _, principal) in &refusals {
        assert_eq!(outcome, "denied");
        assert_eq!(
            class, "secrets",
            "a route that reads provider keys must be audited as `secrets`"
        );
        assert_eq!(*principal, None, "nothing was admitted, so nothing to name");
    }
    // Newest first: the wrong-token attempt, then the no-token one. A process
    // probing with a guessed token is the single most interesting row here, so
    // it must be distinguishable from a client that simply forgot the header.
    assert_eq!(refusals[0].2, "unrecognised");
    assert_eq!(refusals[1].2, "none");

    // ── 3. The token is accepted, and the use is attributed ──────────────────
    let response = app
        .clone()
        .oneshot(post_json(
            "/config/read",
            Some(&master_token),
            r#"{"key":"trust_boundary_probe"}"#,
        ))
        .await
        .unwrap();
    assert_ne!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "the daemon token must still admit its own app"
    );
    let admitted_status = response.status().as_u16();

    let rows = rows_for(&pool, "/config/read").await;
    assert_eq!(rows.len(), 3, "the admitted request must be recorded too");
    assert_eq!(
        (
            rows[0].0.as_str(),
            rows[0].1.as_str(),
            rows[0].2.as_str(),
            rows[0].3.as_deref()
        ),
        ("admitted", "secrets", "master", Some("master"))
    );
    let recorded_status: Option<i64> = permagent::sqlx::query(
        "SELECT status FROM daemon_auth_audit WHERE path = ? ORDER BY ts DESC, id DESC LIMIT 1",
    )
    .bind("/config/read")
    .fetch_one(&pool)
    .await
    .unwrap()
    .get("status");
    assert_eq!(
        recorded_status,
        Some(i64::from(admitted_status)),
        "the audit must record the status the caller actually got, not an assumed 200"
    );

    // ── 4. A device token is attributed to the DEVICE, not to `master` ───────
    // The iOS companion holds one of these. Pairing must keep working, and a
    // lost phone must be nameable in the log after the fact.
    let response = app
        .clone()
        .oneshot(post_json(
            "/config/read",
            Some(&device_token),
            r#"{"key":"trust_boundary_probe"}"#,
        ))
        .await
        .unwrap();
    assert_ne!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "a paired device must keep working"
    );
    let rows = rows_for(&pool, "/config/read").await;
    assert_eq!(
        (rows[0].2.as_str(), rows[0].3.as_deref()),
        ("device", Some(device.id.as_str())),
        "a device token must be attributed to its device id"
    );

    // ── 5. The class policy is ENFORCED, not merely recorded ────────────────
    // `/api/version` is a status route. The desktop app polls it; an append-only
    // table must not grow by a row per poll. Admitted status reads produce
    // NOTHING, and that is a property of the middleware, not of this comment.
    assert_eq!(
        auth_audit::classify("GET", "/api/version"),
        RouteClass::Status
    );
    let response = app
        .clone()
        .oneshot(get("/api/version", Some(&master_token)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        rows_for(&pool, "/api/version").await.is_empty(),
        "an admitted status poll must not produce an audit row"
    );

    // A read of user data is likewise not recorded on success...
    let before = rows_for(&pool, "/api/sessions").await.len();
    let response = app
        .clone()
        .oneshot(get("/api/sessions", Some(&master_token)))
        .await
        .unwrap();
    assert_ne!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        rows_for(&pool, "/api/sessions").await.len(),
        before,
        "an admitted read must not produce an audit row"
    );

    // ...but a REFUSED read is, whatever its class: a caller probing a read
    // route with a bad token is exactly what this log exists to surface.
    let response = app
        .clone()
        .oneshot(get("/api/sessions", Some("still-not-the-token")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let denied_reads = rows_for(&pool, "/api/sessions").await;
    assert_eq!(denied_reads.len(), before + 1);
    assert_eq!(
        (denied_reads[0].0.as_str(), denied_reads[0].1.as_str()),
        ("denied", "read")
    );

    // ── 6. Execute-class routes are audited as `execute` ────────────────────
    // `/agent/call_tool` dispatches straight through
    // `ExtensionManager::dispatch_tool_call`, bypassing the confirmation router
    // that gates model-initiated calls. It is the highest-consequence route
    // behind the token and must never be recorded as an ordinary mutation.
    let response = app
        .clone()
        .oneshot(post_json("/agent/call_tool", None, "{}"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let rows = rows_for(&pool, "/agent/call_tool").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].0.as_str(), rows[0].1.as_str()),
        ("denied", "execute")
    );

    // ── 7. The audit is append-only at the database ─────────────────────────
    assert!(
        permagent::sqlx::query("DELETE FROM daemon_auth_audit")
            .execute(&pool)
            .await
            .is_err(),
        "the auth audit must not be erasable through SQL"
    );

    // ── 8. The peer gate, as shipped, changed nothing above ─────────────────
    // Every assertion in this test ran through the composed router with the
    // peer-verification layer mounted. That it admitted all of them IS the
    // no-op proof for the real router; `middleware::peer_identity` proves the
    // enabled path refuses an unverifiable caller against a fixture verifier.
    let response = app
        .oneshot(get("/api/version", Some(&master_token)))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the disabled peer gate must not affect any request"
    );
}
