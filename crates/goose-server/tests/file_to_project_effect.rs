//! file_to_project decision effect, end to end through the HTTP router
//! (call-notes/email MVP 2A).
//!
//! Drives the same axum router (`routes::decisions::routes`) with tower oneshot
//! requests against an AppState rooted at a throwaway PERMAGENT_PATH_ROOT — the
//! decisions_lifecycle.rs pattern. One test per integration binary (own
//! process): PERMAGENT_PATH_ROOT and the startup singletons are per-process.
//!
//! What this proves (the propose→confirm contract):
//! - Nothing persists before the answer; approve creates the project note
//!   through the shared composed path (durable row; here brainless, so
//!   memory_key stays None and the row still stands).
//! - People steps are address-less and never guess: an existing directory
//!   person is associated; an unknown person without a Brain mounted surfaces
//!   an HONEST effect warning instead of a silent drop.
//! - Reject persists nothing.
//! - The audit hash chain stays intact across all of it.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use permagent::decisions::{self, NewDecision};
use permagent::people::{self, PeopleFilter, PersonAttrs};
use permagent::projects::{self, CreateProject};
use permagent::{project_association, project_notes};

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

fn post_json(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn file_decision(project_id: &str, people: Vec<&str>) -> NewDecision {
    NewDecision {
        kind: "file_to_project".to_string(),
        project_id: Some(project_id.to_string()),
        headline: Some("File \"Email from Dana\" to project \"Acme\"".to_string()),
        detail: Some(
            "Source: email open in the embedded browser\n\nContent:\nHi — can we move the call?"
                .to_string(),
        ),
        payload: serde_json::json!({
            "project_id": project_id,
            "project_name": "Acme",
            "title": "Email from Dana",
            "body": "Hi — can we move the call to Thursday?\n\nDana",
            "content_origin": "email open in the embedded browser",
            "people": people,
        }),
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn file_to_project_effect_through_router() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("PERMAGENT_PATH_ROOT", tmp.path());

    let state = permagent_daemon::state::AppState::new(true).await.unwrap();
    let app = permagent_daemon::routes::decisions::routes(state.clone());
    let pool = state.session_manager().pool_clone().await.unwrap();

    let project = projects::create_project(
        &pool,
        CreateProject {
            name: "Acme".to_string(),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    // A person who already exists in the directory (seeded brainlessly through
    // the row-level upsert): the effect must ASSOCIATE, not re-mint.
    people::upsert_person(&pool, "sam vega", "Sam Vega", &PersonAttrs::default())
        .await
        .unwrap();

    // ── Approve: note filed; existing person associated; unknown person
    //    surfaces an honest warning (no Brain mounted in this test) ──
    let decision = decisions::create_decision(
        &pool,
        file_decision(&project.id, vec!["Sam Vega", "Dana Example"]),
    )
    .await
    .unwrap();
    assert_eq!(decision.kind, "file_to_project");
    assert_eq!(
        decision.tier, 2,
        "unseeded action class fails closed to Tier 2"
    );

    // Nothing persisted before the answer.
    assert!(project_notes::list_notes(&pool, &project.id)
        .await
        .unwrap()
        .is_empty());

    let resp = app
        .clone()
        .oneshot(post_json(
            &format!("/api/decisions/{}/answer", decision.id),
            serde_json::json!({"answer": "approve"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp).await;
    assert_eq!(v["decision"]["status"], "answered");
    assert_eq!(v["decision"]["answer"], "approve");
    let effect = v["effect"].as_str().unwrap();
    assert!(effect.contains("filed note"), "effect: {effect}");
    assert!(
        effect.contains("1 already in the directory"),
        "existing person must be associated: {effect}"
    );
    // The unknown person cannot be minted without a Brain — that is a warning,
    // not a silent drop and not a failed note.
    let warning = v["effectError"].as_str().unwrap();
    assert!(
        warning.contains("Dana Example") && warning.contains("Brain is not available"),
        "warning must name the person and the reason: {warning}"
    );

    // The note row is durable (brainless → no memory_key), body intact.
    let notes = project_notes::list_notes(&pool, &project.id).await.unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].title.as_deref(), Some("Email from Dana"));
    assert!(notes[0].body.contains("move the call to Thursday"));
    assert!(notes[0].memory_key.is_none());

    // Sam Vega is associated with the project; Dana Example was never minted.
    let assoc = project_association::list_project_people(&pool, &project.id)
        .await
        .unwrap();
    assert_eq!(assoc.len(), 1, "exactly the existing person: {assoc:?}");
    assert_eq!(assoc[0].person.display_name, "Sam Vega");
    let dana = people::list_people(
        &pool,
        &PeopleFilter {
            query: Some("Dana Example".to_string()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(dana.is_empty(), "no brainless person mint");

    // ── Reject: recorded, nothing else persists ──
    let decision = decisions::create_decision(&pool, file_decision(&project.id, vec![]))
        .await
        .unwrap();
    let resp = app
        .clone()
        .oneshot(post_json(
            &format!("/api/decisions/{}/answer", decision.id),
            serde_json::json!({"answer": "reject"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp).await;
    assert!(
        v["effect"]
            .as_str()
            .unwrap()
            .contains("nothing was persisted"),
        "{v}"
    );
    assert!(v["effectError"].is_null());
    assert_eq!(
        project_notes::list_notes(&pool, &project.id)
            .await
            .unwrap()
            .len(),
        1,
        "reject must not create a note"
    );

    // ── Audit hash chain intact across create/answer/effect-warning rows ──
    let report = decisions::verify_audit_chain(&pool).await.unwrap();
    assert!(report.intact, "{}", report.detail);

    std::env::remove_var("PERMAGENT_PATH_ROOT");
}
