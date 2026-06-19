//! People (CRM) read routes.
//!
//! Endpoints:
//!   GET /api/people        — list / surface people, optionally filtered by attribute
//!
//! Serves the conversational read loop (query people by company/role/free-text)
//! from the typed `people` table in permagent.db — NOT Brain recall. Read-only:
//! no write op, no queue, no Spectral touch (v1). See [`permagent::people`].

use crate::state::AppState;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use permagent::people::{self, PeopleFilter, Person};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct PeopleQuery {
    /// Exact company match.
    #[serde(default)]
    pub company: Option<String>,
    /// Exact role match.
    #[serde(default)]
    pub role: Option<String>,
    /// Free-text substring over display_name / email / company.
    #[serde(default)]
    pub q: Option<String>,
}

async fn list_people_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<PeopleQuery>,
) -> Result<Json<Vec<Person>>, StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let filter = PeopleFilter {
        company: params.company,
        role: params.role,
        query: params.q,
    };

    let people = people::list_people(&pool, &filter)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(people))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/people", get(list_people_handler))
        .with_state(state)
}
