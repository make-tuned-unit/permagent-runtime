//! Brief Henry, nudge the human, file Decision Inbox actions.

use sqlx::{Pool, Sqlite};

use super::debate::{ChairAction, ChairReport, MAX_ACTIONS};
use crate::briefings::{self, NewBriefing, Severity};
use crate::decisions::{self, NewDecision};
use crate::events;

pub const KIND: &str = "council_action";

pub async fn file_briefing(
    pool: &Pool<Sqlite>,
    session_id: &str,
    headline: &str,
    n_actions: usize,
) -> Option<String> {
    briefings::file_briefing(
        pool,
        NewBriefing {
            from_agent: super::AGENT_ID.to_string(),
            kind: "weekly_report".to_string(),
            severity: Severity::Attention,
            summary: format!(
                "Council report: {headline} ({} action{})",
                n_actions,
                if n_actions == 1 { "" } else { "s" }
            ),
            detail: Some(format!("session {session_id}")),
            ref_kind: Some("council_session".to_string()),
            ref_id: Some(session_id.to_string()),
        },
    )
    .await
}

pub fn emit_nudge(headline: &str, n_actions: usize) {
    events::emit(events::proactive_nudge(
        "council_report",
        "Council of LLMs",
        &format!("{headline} — {n_actions} action(s) in the Decision Inbox"),
        n_actions as i64,
        &chrono::Utc::now().to_rfc3339(),
        None,
        None,
    ));
}

pub async fn file_actions(
    pool: &Pool<Sqlite>,
    session_id: &str,
    report: &ChairReport,
) -> Result<Vec<String>, String> {
    let mut ids = Vec::new();
    for action in report.actions.iter().take(MAX_ACTIONS) {
        match file_one(pool, session_id, action).await {
            Ok(id) => ids.push(id),
            Err(e) => tracing::warn!(target: "permagent::council", "action not filed: {e}"),
        }
    }
    Ok(ids)
}

async fn file_one(
    pool: &Pool<Sqlite>,
    session_id: &str,
    action: &ChairAction,
) -> Result<String, String> {
    let headline = decisions::truncate_for_headline(&action.title);
    let project_id = action.project_id.trim();
    let project_id = if project_id.is_empty() {
        None
    } else {
        Some(project_id.to_string())
    };
    let detail = if action.description.trim().is_empty() {
        format!("Council action for session {session_id}.")
    } else {
        action.description.clone()
    };
    let d = decisions::create_decision(
        pool,
        NewDecision {
            kind: KIND.to_string(),
            goal_id: None,
            project_id,
            headline: Some(headline),
            detail: Some(detail),
            payload: serde_json::json!({
                "session_id": session_id,
                "project_id": action.project_id,
                "project_name": action.project_name,
                "title": action.title,
                "description": action.description,
            }),
            rank: Some(0.6),
            action_class: Some(KIND.to_string()),
        },
    )
    .await?;
    Ok(d.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::spectral_schema::{apply_council_schema, init_spectral_db};

    async fn pool() -> Pool<Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        apply_council_schema(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn files_at_most_five_council_actions() {
        let pool = pool().await;
        let project = crate::projects::create_project(
            &pool,
            crate::projects::CreateProject {
                name: "Permagent".into(),
                slug: None,
                description: None,
                root_path: None,
                site_url: None,
                repo_url: None,
                notes: None,
                tags: None,
            },
        )
        .await
        .unwrap();
        let actions: Vec<ChairAction> = (0..8)
            .map(|i| ChairAction {
                project_id: project.id.clone(),
                project_name: "Permagent".into(),
                title: format!("Do thing {i}"),
                description: format!("Because {i}"),
            })
            .collect();
        let report = ChairReport {
            headline: "H".into(),
            markdown: "# hi".into(),
            consensus: vec![],
            dissent: vec![],
            actions,
        };
        let ids = file_actions(&pool, "sess-1", &report).await.unwrap();
        assert_eq!(ids.len(), MAX_ACTIONS);
        let open = crate::decisions::list_open_decisions(&pool).await.unwrap();
        assert_eq!(open.len(), MAX_ACTIONS);
        assert!(open.iter().all(|i| i.decision.kind == KIND));
    }
}
