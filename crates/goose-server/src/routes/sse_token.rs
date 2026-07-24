use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{extract::State, routing::post, Json, Router};
use serde::Serialize;

use crate::state::AppState;

pub const STREAM_TOKEN_TTL_SECS: u64 = 120;

#[derive(Serialize)]
pub struct StreamTokenResponse {
    token: String,
    expires_in_secs: u64,
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/sse-token", post(mint_stream_token))
        .with_state(state)
}

async fn mint_stream_token(State(state): State<Arc<AppState>>) -> Json<StreamTokenResponse> {
    let token = hex::encode(rand::random::<[u8; 32]>());
    state.stream_tokens.insert(
        token.clone(),
        Instant::now() + Duration::from_secs(STREAM_TOKEN_TTL_SECS),
    );
    Json(StreamTokenResponse {
        token,
        expires_in_secs: STREAM_TOKEN_TTL_SECS,
    })
}

#[cfg(test)]
mod tests {
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use serial_test::serial;
    use tower::ServiceExt;

    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn mint_is_bearer_gated_and_token_is_stream_scoped() {
        crate::test_support::test_root();
        let state = AppState::new(true).await.unwrap();
        let daemon_token = state.daemon_token.clone().unwrap();
        let app = crate::routes::configure(state.clone());

        let unauthenticated = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/sse-token")
                    .method("POST")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);

        let minted = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/sse-token")
                    .method("POST")
                    .header("authorization", format!("Bearer {daemon_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(minted.status(), StatusCode::OK);
        let bytes = to_bytes(minted.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let scoped = body["token"].as_str().unwrap();
        assert_eq!(
            body["expires_in_secs"],
            serde_json::json!(STREAM_TOKEN_TTL_SECS)
        );
        assert!(state.stream_tokens.contains_unexpired(scoped));

        // The established daemon token and the freshly minted scoped token
        // both get past /events auth. With no upgrade headers Axum then
        // rejects the handshake as 400, proving auth did not return 401.
        for token in [daemon_token.as_str(), scoped] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/events?token={token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }

        // The per-session SSE route also admits both credential classes. The
        // unknown session reaches the handler and returns 404 rather than auth's
        // 401, proving the scoped credential is valid only on this GET.
        for token in [daemon_token.as_str(), scoped] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/sessions/unknown/events?token={token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }

        // A scoped stream token cannot reach agent-invoking or cancellation
        // handlers, whether presented in the query string or as a bearer.
        for path in ["reply", "cancel"] {
            for request in [
                Request::builder()
                    .uri(format!("/sessions/unknown/{path}?token={scoped}"))
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
                Request::builder()
                    .uri(format!("/sessions/unknown/{path}"))
                    .method("POST")
                    .header("authorization", format!("Bearer {scoped}"))
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            ] {
                let response = app.clone().oneshot(request).await.unwrap();
                assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            }
        }

        // Long-lived credentials still pass reply/cancel auth through both
        // established transports. An empty JSON body is rejected by the
        // handlers/extractors after auth, so any non-401 status proves access.
        for path in ["reply", "cancel"] {
            for request in [
                Request::builder()
                    .uri(format!("/sessions/unknown/{path}?token={daemon_token}"))
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
                Request::builder()
                    .uri(format!("/sessions/unknown/{path}"))
                    .method("POST")
                    .header("authorization", format!("Bearer {daemon_token}"))
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            ] {
                let response = app.clone().oneshot(request).await.unwrap();
                assert_ne!(response.status(), StatusCode::UNAUTHORIZED);
            }
        }

        // Scoped tokens never become bearer credentials for protected routes.
        let scoped_as_bearer = app
            .oneshot(
                Request::builder()
                    .uri("/sse-token")
                    .method("POST")
                    .header("authorization", format!("Bearer {scoped}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(scoped_as_bearer.status(), StatusCode::UNAUTHORIZED);
    }
}
