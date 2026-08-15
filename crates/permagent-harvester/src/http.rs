use crate::db;
use axum::{
    extract::{Request, State},
    http::{header::AUTHORIZATION, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc};
use subtle::ConstantTimeEq;

#[derive(Clone)]
struct AppState {
    db_path: PathBuf,
    token: Arc<[u8]>,
}

pub fn router(db_path: PathBuf, token: String) -> Router {
    let state = AppState {
        db_path,
        token: Arc::from(token.into_bytes()),
    };
    Router::new()
        .route("/health", get(health))
        .route("/status", get(status))
        .fallback(not_found)
        // Auth wraps the entire router so newly added and unknown routes cannot become public accidentally.
        .layer(middleware::from_fn_with_state(state.clone(), auth))
        .with_state(state)
}

async fn health() -> Json<Value> {
    Json(json!({"status":"ok"}))
}

async fn status(State(state): State<AppState>) -> Result<Json<Value>, (StatusCode, String)> {
    let connection = db::open(&state.db_path).map_err(internal)?;
    let statuses = db::statuses(&connection).map_err(internal)?;
    Ok(Json(json!({"stages": statuses})))
}

async fn not_found() -> StatusCode {
    StatusCode::NOT_FOUND
}

async fn auth(State(state): State<AppState>, request: Request, next: Next) -> Response {
    if request.uri().path() == "/health" {
        return next.run(request).await;
    }
    let supplied = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::as_bytes);
    let valid = supplied.is_some_and(|bytes| {
        bytes.len() == state.token.len() && bool::from(bytes.ct_eq(&state.token))
    });
    if valid {
        next.run(request).await
    } else {
        StatusCode::UNAUTHORIZED.into_response()
    }
}

fn internal(error: anyhow::Error) -> (StatusCode, String) {
    tracing::error!(%error,"request failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal server error".into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tempfile::TempDir;
    use tower::ServiceExt;

    fn app() -> (TempDir, Router) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        db::open_and_migrate(&db_path).unwrap();
        (temp_dir, router(db_path, "secret".into()))
    }

    async fn response_status(path: &str, auth: Option<&str>) -> StatusCode {
        let (_temp_dir, app) = app();
        let mut request_builder = Request::builder().uri(path);
        if let Some(value) = auth {
            request_builder = request_builder.header(AUTHORIZATION, value)
        }
        app.oneshot(request_builder.body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status()
    }

    #[tokio::test]
    async fn health_is_public() {
        assert_eq!(response_status("/health", None).await, StatusCode::OK)
    }

    #[tokio::test]
    async fn status_requires_right_token() {
        assert_eq!(
            response_status("/status", None).await,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            response_status("/status", Some("Bearer wrong")).await,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            response_status("/status", Some("Bearer secret")).await,
            StatusCode::OK
        )
    }

    #[tokio::test]
    async fn unknown_path_is_protected() {
        assert_eq!(
            response_status("/nope", None).await,
            StatusCode::UNAUTHORIZED
        )
    }
}
