//! Read-only access to the independent typed Model Review ledger.
use crate::{routes::errors::ErrorResponse, state::AppState};
use axum::{extract::State, routing::get, Json, Router};
use permagent::model_review::{
    findings::Finding,
    report::{self, Report},
};
use std::sync::Arc;

async fn latest(State(state): State<Arc<AppState>>) -> Result<Json<Option<Report>>, ErrorResponse> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| ErrorResponse::internal(e.to_string()))?;
    report::latest(&pool)
        .await
        .map(Json)
        .map_err(|e| ErrorResponse::internal(e.to_string()))
}

async fn findings(State(state): State<Arc<AppState>>) -> Result<Json<Vec<Finding>>, ErrorResponse> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| ErrorResponse::internal(e.to_string()))?;
    report::ledger(&pool)
        .await
        .map(Json)
        .map_err(|e| ErrorResponse::internal(e.to_string()))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/model-review/latest", get(latest))
        .route("/api/model-review/findings", get(findings))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use serial_test::serial;
    use tower::ServiceExt;

    async fn get_json(app: &Router, uri: &str) -> serde_json::Value {
        let request = Request::builder().uri(uri).body(Body::empty()).unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// The mounted routes read the same session pool the scheduled Model
    /// Review writes into: a persisted report comes back from /latest and the
    /// ledger endpoint answers with an array.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn routes_read_the_persisted_report_and_ledger() {
        let root = crate::test_support::test_root();
        let home = tempfile::tempdir().unwrap();
        let _guard = env_lock::lock_env([
            ("HOME", Some(home.path().to_str().unwrap())),
            ("PERMAGENT_PATH_ROOT", Some(root.to_str().unwrap())),
        ]);
        let state = AppState::new(true).await.unwrap();
        let pool = state.session_manager().pool_clone().await.unwrap();
        // Far-future timestamp: the shared test pool may already hold reports,
        // and /latest orders by created_at.
        let mut report = report::build(Default::default(), chrono::Utc::now()).unwrap();
        report.created_at = "2999-01-01T00:00:00+00:00".into();
        report::persist_report(&pool, &report).await.unwrap();

        let app = routes(state);
        let latest = get_json(&app, "/api/model-review/latest").await;
        assert_eq!(latest["id"], report.id.as_str());
        assert_eq!(latest["discovery_mode"], "local-only");
        let findings = get_json(&app, "/api/model-review/findings").await;
        assert!(findings.is_array(), "{findings}");
    }
}
