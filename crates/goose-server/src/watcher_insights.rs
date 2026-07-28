//! Watcher project insights (Jesse, 2026-07-28): once or twice a day per
//! project the Watcher quietly places a short, grounded observation onto the
//! project's Overview — no notification, no badge; you read them as you browse.
//!
//! Honesty law: an insight is composed ONLY from real per-project signals
//! (notes, kanban movement, stack changes, first-party analytics). No signals
//! or no LLM provider → silence, never filler. Storage rides
//! `projects.metadata_json.watcher_insights` (newest first, capped) — no
//! schema migration; the Overview card reads it straight off ProjectResponse.

use crate::state::AppState;
use permagent::conversation::message::Message;
use permagent::projects::{self, Project, UpdateProject};
use sqlx::{Pool, Sqlite};
use std::sync::Arc;
use std::time::Duration;

const METADATA_KEY: &str = "watcher_insights";
const MAX_KEPT: usize = 14;
const PER_DAY: usize = 2;
const TICK: Duration = Duration::from_secs(4 * 3600);

pub fn spawn(state: Arc<AppState>) {
    tokio::spawn(async move {
        // Let boot settle before the first pass.
        tokio::time::sleep(Duration::from_secs(180)).await;
        loop {
            if let Err(e) = run_once(&state).await {
                tracing::debug!("watcher insights pass skipped: {e}");
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

async fn run_once(state: &Arc<AppState>) -> Result<(), String> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| e.to_string())?;
    let projects = projects::list_projects(&pool, Some("active")).await?;
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();

    for project in projects {
        // The default Personal bucket isn't a project the Watcher reports on.
        if project.id == "00000000-0000-0000-0000-000000000001" {
            continue;
        }
        if insights_on_day(&project, &today) >= PER_DAY {
            continue;
        }
        let signals = gather_signals(&pool, &project.id).await;
        if signals.is_empty() {
            continue; // nothing real happened — the Watcher stays silent
        }
        let Some(text) = compose(&project.name, &signals).await else {
            continue;
        };
        if let Err(e) = append_insight(&pool, &project, &text).await {
            tracing::debug!(project = %project.name, "watcher insight write failed: {e}");
        } else {
            tracing::info!(
                target: "permagentd::watcher",
                project = %project.name,
                "watcher insight placed on the project overview"
            );
        }
    }
    Ok(())
}

fn insights_on_day(project: &Project, day: &str) -> usize {
    project
        .metadata_json
        .get(METADATA_KEY)
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .filter(|i| {
                    i.get("created_at")
                        .and_then(|c| c.as_str())
                        .map(|c| c.starts_with(day))
                        .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0)
}

/// Real 7-day activity signals, each phrased for the composer. Unknown /
/// missing tables and zero counts contribute nothing.
async fn gather_signals(pool: &Pool<Sqlite>, project_id: &str) -> Vec<String> {
    let mut out = Vec::new();
    let count = |sql: &str| {
        let sql = sql.to_string();
        let pool = pool.clone();
        let pid = project_id.to_string();
        async move {
            sqlx::query_scalar::<_, i64>(&sql)
                .bind(&pid)
                .fetch_one(&pool)
                .await
                .unwrap_or(0)
        }
    };

    let notes = count(
        "SELECT count(*) FROM project_notes WHERE project_id = ?1 AND created_at >= datetime('now','-7 days')",
    )
    .await;
    if notes > 0 {
        out.push(format!("{notes} note(s) added in the last 7 days"));
    }
    let cards_moved = count(
        "SELECT count(*) FROM cards WHERE project_id = ?1 AND updated_at >= datetime('now','-7 days')",
    )
    .await;
    if cards_moved > 0 {
        out.push(format!("{cards_moved} kanban card(s) touched this week"));
    }
    let cards_stale = count(
        "SELECT count(*) FROM cards WHERE project_id = ?1 AND updated_at < datetime('now','-14 days')",
    )
    .await;
    if cards_stale > 0 {
        out.push(format!("{cards_stale} card(s) untouched for 14+ days"));
    }
    let stack = count("SELECT count(*) FROM project_stack_entries WHERE project_id = ?1").await;
    if stack > 0 {
        out.push(format!("{stack} stack entr(ies) recorded"));
    }
    let pageviews = count(
        "SELECT count(*) FROM analytics_events WHERE project_id = ?1 AND kind = 'pageview' AND created_at >= datetime('now','-7 days')",
    )
    .await;
    if pageviews > 0 {
        out.push(format!("{pageviews} site pageview(s) in the last 7 days"));
    }
    out
}

/// One short observation via the configured provider's fast model. None on
/// no-provider, refusal, or an empty compose — silence over filler.
async fn compose(project_name: &str, signals: &[String]) -> Option<String> {
    let config = permagent::config::Config::global();
    let provider_name = config.get_goose_provider().ok()?;
    let model_name = config.get_goose_model().ok()?;
    if provider_name.trim().is_empty() || model_name.trim().is_empty() {
        return None;
    }
    let provider =
        permagent::providers::create_with_named_model(&provider_name, &model_name, Vec::new())
            .await
            .ok()?;

    let system = "You are the Watcher — a quiet observer who leaves one useful note a day on a \
                  project's overview. Given real activity signals, write ONE specific, grounded \
                  observation or gentle suggestion (max 25 words). Never invent facts beyond the \
                  signals. Reply ONLY as JSON: {\"insight\": \"<text, or empty if nothing worth saying>\"}";
    let user = Message::user().with_text(format!(
        "Project: {project_name}\nSignals this week:\n- {}",
        signals.join("\n- ")
    ));
    let (response, _usage) = provider
        .complete_fast("watcher-insights", system, std::slice::from_ref(&user), &[])
        .await
        .ok()?;
    let text = response.as_concat_text();
    let (start, end) = (text.find('{')?, text.rfind('}')?);
    let v: serde_json::Value = serde_json::from_str(text.get(start..=end)?).ok()?;
    let insight = v.get("insight")?.as_str()?.trim().to_string();
    if insight.is_empty() {
        return None;
    }
    Some(insight)
}

async fn append_insight(pool: &Pool<Sqlite>, project: &Project, text: &str) -> Result<(), String> {
    let mut metadata = if project.metadata_json.is_object() {
        project.metadata_json.clone()
    } else {
        serde_json::json!({})
    };
    let obj = metadata.as_object_mut().expect("object ensured");
    let mut list = obj
        .get(METADATA_KEY)
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    list.insert(
        0,
        serde_json::json!({
            "text": text,
            "created_at": chrono::Utc::now().to_rfc3339(),
        }),
    );
    list.truncate(MAX_KEPT);
    obj.insert(METADATA_KEY.to_string(), serde_json::Value::Array(list));
    projects::update_project(
        pool,
        &project.id,
        UpdateProject {
            metadata_json: Some(metadata),
            ..Default::default()
        },
    )
    .await?;
    Ok(())
}
