use crate::routes::errors::ErrorResponse;
use crate::state::AppState;
use axum::{extract::State, routing::post, Json, Router};
use permagent::permission::permission_confirmation::PrincipalType;
use permagent::permission::{Permission, PermissionConfirmation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use utoipa::ToSchema;

#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmToolActionRequest {
    id: String,
    #[serde(default = "default_principal_type")]
    principal_type: PrincipalType,
    action: Permission,
    session_id: String,
}

fn default_principal_type() -> PrincipalType {
    PrincipalType::Tool
}

#[utoipa::path(
    post,
    path = "/action-required/tool-confirmation",
    request_body = ConfirmToolActionRequest,
    responses(
        (status = 200, description = "Tool confirmation action is confirmed", body = Value),
        (status = 401, description = "Unauthorized - missing or invalid bearer token"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn confirm_tool_action(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ConfirmToolActionRequest>,
) -> Result<Json<Value>, ErrorResponse> {
    let agent = state.get_agent_for_route(request.session_id).await?;

    let delivered = agent
        .handle_confirmation(
            request.id.clone(),
            PermissionConfirmation {
                principal_type: request.principal_type,
                permission: request.action,
            },
        )
        .await;

    // Inbox mirror: this needs-approval call was also surfaced as a
    // `tool_approval` Decision-Inbox card keyed on the same request id.
    // Answered here first, that card is moot — close it as superseded with an
    // honest note instead of leaving a zombie open row forever. Only on a real
    // delivery: if nothing was waiting, this confirmation resolved nothing and
    // must not close anything. Best-effort — bookkeeping never fails the
    // confirmation the parked turn already received.
    if delivered {
        match state.session_manager().pool_clone().await {
            Ok(pool) => close_superseded_inbox_mirror(&pool, &request.id).await,
            Err(e) => tracing::warn!(
                "tool_approval mirror not closed for request {}: no DB pool ({})",
                request.id,
                e
            ),
        }
    }

    // Honest response: whether a live turn actually received this confirmation
    // (additive field — the previous body was an empty object).
    Ok(Json(serde_json::json!({ "delivered": delivered })))
}

/// Close the open `tool_approval` inbox card mirroring `request_id`, if any —
/// it was answered via the legacy per-tool prompt, so the card is moot.
/// `superseded` (with the note on the row and on a hash-chained audit entry)
/// is what the decisions state machine supports for "resolved elsewhere":
/// `answered` would forge a Tier-2 human answer and `expired` a timeout that
/// never happened. A concurrent inbox answer wins benignly (`Ok(false)`).
async fn close_superseded_inbox_mirror(
    pool: &permagent::sqlx::Pool<permagent::sqlx::Sqlite>,
    request_id: &str,
) {
    use permagent::decisions;
    match decisions::find_open_tool_approval_by_request_id(pool, request_id).await {
        Ok(Some(d)) => {
            match decisions::supersede_decision(
                pool,
                &d.id,
                "answered via the legacy per-tool prompt",
            )
            .await
            {
                Ok(true) => tracing::info!(
                    "tool_approval decision {} superseded (request {} answered via the legacy per-tool prompt)",
                    d.id,
                    request_id
                ),
                // Raced with an inbox answer — already resolved; nothing to do.
                Ok(false) => {}
                Err(e) => tracing::warn!(
                    "failed to supersede tool_approval decision {} for request {}: {}",
                    d.id,
                    request_id,
                    e
                ),
            }
        }
        Ok(None) => {}
        Err(e) => tracing::warn!(
            "tool_approval mirror lookup failed for request {}: {}",
            request_id,
            e
        ),
    }
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/action-required/tool-confirmation",
            post(confirm_tool_action),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    mod integration_tests {
        use super::*;
        use axum::{body::Body, http::Request};
        use http::StatusCode;
        use serial_test::serial;
        use tower::ServiceExt;

        // Builds a real AppState (global session pool) → #[serial] per the
        // standing rule, and pins PERMAGENT_PATH_ROOT to the shared,
        // process-lifetime root so the pool never outlives a per-test tempdir
        // (#858).
        #[tokio::test(flavor = "multi_thread")]
        #[serial]
        async fn test_tool_confirmation_endpoint() {
            crate::test_support::test_root();
            let state = AppState::new(true).await.unwrap();

            let app = routes(state);

            let request = Request::builder()
                .uri("/action-required/tool-confirmation")
                .method("POST")
                .header("content-type", "application/json")
                .header("x-secret-key", "test-secret")
                .body(Body::from(
                    serde_json::to_string(&ConfirmToolActionRequest {
                        id: "test-id".to_string(),
                        principal_type: PrincipalType::Tool,
                        action: Permission::AllowOnce,
                        session_id: "test-session".to_string(),
                    })
                    .unwrap(),
                ))
                .unwrap();

            let response = app.oneshot(request).await.unwrap();

            assert_eq!(response.status(), StatusCode::OK);

            // Honest response: no turn was parked on "test-id", so the endpoint
            // must say the confirmation was NOT delivered (previously an empty
            // object that all callers ignored — the field is additive).
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(body, serde_json::json!({ "delivered": false }));
        }
    }

    // ── Inbox-mirror supersede (legacy-first desync fix) ──
    //
    // The risk-bearing logic is `close_superseded_inbox_mirror` (request_id →
    // open tool_approval card → superseded-with-audit); exercised on an
    // in-memory decisions DB, no AppState — mirroring the sibling pattern in
    // routes/decisions.rs (the AppState session store is a process singleton).
    // The endpoint calls it only when `handle_confirmation` reports a real
    // delivery, which the endpoint test above pins on the false side.

    use permagent::decisions;
    use permagent::session::spectral_schema::init_spectral_db;
    use permagent::sqlx::{Pool, Sqlite};

    async fn memory_pool() -> Pool<Sqlite> {
        let pool = permagent::sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        pool
    }

    async fn create_tool_approval(pool: &Pool<Sqlite>, request_id: &str) -> decisions::Decision {
        let d = decisions::create_decision(
            pool,
            decisions::NewDecision {
                kind: "tool_approval".to_string(),
                headline: Some("Approve tool call: developer__shell".to_string()),
                detail: Some("run ls".to_string()),
                payload: serde_json::json!({
                    "session_id": "sess-1",
                    "request_id": request_id,
                    "tool_name": "developer__shell",
                    "arguments": {"command": "ls"},
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(d.kind, "tool_approval");
        assert_eq!(d.status, "open");
        d
    }

    /// Legacy-first: a successful per-tool-prompt answer closes the mirrored
    /// inbox card as superseded — honest note on the row, audit entry on the
    /// chain — so no zombie card lingers and a later inbox answer 409s.
    #[tokio::test]
    async fn legacy_delivery_supersedes_open_inbox_mirror() {
        let pool = memory_pool().await;
        let d = create_tool_approval(&pool, "req-legacy").await;

        close_superseded_inbox_mirror(&pool, "req-legacy").await;

        let after = decisions::get_decision(&pool, &d.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.status, "superseded");
        assert_eq!(
            after.answer_note.as_deref(),
            Some("answered via the legacy per-tool prompt")
        );
        assert_eq!(after.answer, None, "superseded is not a human answer");

        let outcome: String = permagent::sqlx::query_scalar(
            "SELECT outcome FROM decision_audit WHERE decision_id = ? ORDER BY seq DESC LIMIT 1",
        )
        .bind(&d.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            outcome,
            "superseded: answered via the legacy per-tool prompt"
        );

        // The dead card can no longer be answered from the inbox.
        let err = decisions::answer_decision(
            &pool,
            &d.id,
            &decisions::DecisionAnswer {
                answer: "approve".to_string(),
                ..Default::default()
            },
            decisions::ACTOR_JESSE,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            decisions::AnswerError::AlreadyResolved(ref s) if s == "superseded"
        ));
    }

    /// Inbox-first (or plain legacy traffic with no mirrored card): the mirror
    /// close is a benign no-op — nothing changes, nothing errors.
    #[tokio::test]
    async fn mirror_close_is_noop_without_open_mirror() {
        let pool = memory_pool().await;

        // No card at all for this request id.
        close_superseded_inbox_mirror(&pool, "req-none").await;

        // Card exists but the inbox already answered it — must stay 'answered'.
        let d = create_tool_approval(&pool, "req-answered").await;
        decisions::answer_decision(
            &pool,
            &d.id,
            &decisions::DecisionAnswer {
                answer: "approve".to_string(),
                ..Default::default()
            },
            decisions::ACTOR_JESSE,
        )
        .await
        .unwrap();

        close_superseded_inbox_mirror(&pool, "req-answered").await;

        let after = decisions::get_decision(&pool, &d.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.status, "answered");
        assert_eq!(after.answer.as_deref(), Some("approve"));
        assert_eq!(after.acted_by.as_deref(), Some(decisions::ACTOR_JESSE));
    }
}
