//! Project stack organizer (#512) lifecycle, end to end through the HTTP
//! router.
//!
//! Substitutes for a curl demonstration against a live daemon (the sandbox
//! blocks long-running background processes): drives the exact axum router the
//! daemon mounts (`routes::projects::routes`) with tower oneshot requests
//! against an AppState rooted at a throwaway PERMAGENT_PATH_ROOT. Proves the
//! endpoints the StackPanel UI calls are really mounted and round-trip:
//! create project → add entries → list (grouped) → edit (incl. explicit-null
//! clear) → delete → 404s → reference-only rejection of secret-bearing bodies.
//!
//! Runs as its own integration-test binary (own process) so PERMAGENT_PATH_ROOT
//! and the startup singletons never touch the live data root. One test per
//! binary — #[serial] is a no-op across processes (the #695 lesson as ruled in
//! decisions_lifecycle.rs).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

fn json_req(method: &str, uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .method(method)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn delete(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .method("DELETE")
        .body(Body::empty())
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn stack_entry_lifecycle_through_router() {
    // Throwaway data root for the whole process (single test in this binary).
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("PERMAGENT_PATH_ROOT", tmp.path());

    let state = permagent_daemon::state::AppState::new(true).await.unwrap();
    let app = permagent_daemon::routes::projects::routes(state.clone());

    // ── 1. Create a project to hang the stack on ──
    let resp = app
        .clone()
        .oneshot(json_req(
            "POST",
            "/api/projects",
            serde_json::json!({ "name": "Kinrows" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let project = body_json(resp).await;
    let pid = project["id"].as_str().unwrap().to_string();

    // ── 2. Empty stack lists as [] ──
    let resp = app
        .clone()
        .oneshot(get(&format!("/api/projects/{pid}/stack")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await, serde_json::json!([]));

    // ── 3. Add entries (camelCase request, snake_case response) ──
    let resp = app
        .clone()
        .oneshot(json_req(
            "POST",
            &format!("/api/projects/{pid}/stack"),
            serde_json::json!({
                "serviceName": "Railway",
                "category": "hosting",
                "identity": "jesse+kinrows@gmail.com",
                "notes": "free tier, 2 services max",
                "dashboardUrl": "https://railway.app/dashboard",
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let railway = body_json(resp).await;
    assert_eq!(railway["service_name"], "Railway");
    assert_eq!(railway["category"], "hosting");
    assert_eq!(railway["identity"], "jesse+kinrows@gmail.com");
    assert_eq!(railway["dashboard_url"], "https://railway.app/dashboard");
    let railway_id = railway["id"].as_str().unwrap().to_string();

    // Minimal body: category defaults to "other", nullable fields null.
    let resp = app
        .clone()
        .oneshot(json_req(
            "POST",
            &format!("/api/projects/{pid}/stack"),
            serde_json::json!({ "serviceName": "Neon", "category": "database" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let neon = body_json(resp).await;
    assert_eq!(neon["identity"], serde_json::Value::Null);
    assert_eq!(neon["notes"], "");

    // ── 4. List comes back grouped (database before hosting) ──
    let resp = app
        .clone()
        .oneshot(get(&format!("/api/projects/{pid}/stack")))
        .await
        .unwrap();
    let list = body_json(resp).await;
    let names: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["service_name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["Neon", "Railway"]);

    // ── 5. Edit: change identity + notes, clear dashboardUrl with explicit
    //       null; untouched fields survive ──
    let resp = app
        .clone()
        .oneshot(json_req(
            "PATCH",
            &format!("/api/projects/{pid}/stack/{railway_id}"),
            serde_json::json!({
                "identity": "jesse.sharratt@gmail.com",
                "notes": "moved to the main account",
                "dashboardUrl": null,
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let updated = body_json(resp).await;
    assert_eq!(updated["service_name"], "Railway");
    assert_eq!(updated["identity"], "jesse.sharratt@gmail.com");
    assert_eq!(updated["dashboard_url"], serde_json::Value::Null);

    // ── 6. Validation: unknown category is a 400 ──
    let resp = app
        .clone()
        .oneshot(json_req(
            "POST",
            &format!("/api/projects/{pid}/stack"),
            serde_json::json!({ "serviceName": "Vercel", "category": "cloud" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // ── 7. Reference-only contract: a body smuggling a password/secret field
    //       is rejected (deny_unknown_fields → 422), never stored ──
    let resp = app
        .clone()
        .oneshot(json_req(
            "POST",
            &format!("/api/projects/{pid}/stack"),
            serde_json::json!({ "serviceName": "Vercel", "password": "hunter2" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let resp = app
        .clone()
        .oneshot(json_req(
            "PATCH",
            &format!("/api/projects/{pid}/stack/{railway_id}"),
            serde_json::json!({ "secret": "hunter2" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // ── 8. Delete round-trip + 404s ──
    let resp = app
        .clone()
        .oneshot(delete(&format!("/api/projects/{pid}/stack/{railway_id}")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let resp = app
        .clone()
        .oneshot(delete(&format!("/api/projects/{pid}/stack/{railway_id}")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let resp = app
        .clone()
        .oneshot(json_req(
            "PATCH",
            &format!("/api/projects/{pid}/stack/{railway_id}"),
            serde_json::json!({ "notes": "gone" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // Unknown project 404s too.
    let resp = app
        .clone()
        .oneshot(get("/api/projects/no-such-project/stack"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // ── 9. Neon survives; the stack answers "which account, at a glance" ──
    let resp = app
        .clone()
        .oneshot(get(&format!("/api/projects/{pid}/stack")))
        .await
        .unwrap();
    let list = body_json(resp).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["service_name"], "Neon");
}
