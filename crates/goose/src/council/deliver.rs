//! Brief Henry, nudge the human, file Decision Inbox actions.

use sqlx::{Pool, Sqlite};

use super::debate::{ChairAction, ChairReport, MAX_ACTIONS};
use crate::briefings::{self, NewBriefing, Severity};
use crate::decision_inbox::negatives;
use crate::decisions::{self, NewDecision};
use crate::events;

pub const KIND: &str = "council_action";

pub async fn file_briefing(
    pool: &Pool<Sqlite>,
    session_id: &str,
    headline: &str,
    n_actions: usize,
    verdict_missing: bool,
) -> Option<String> {
    briefings::file_briefing(
        pool,
        NewBriefing {
            from_agent: super::AGENT_ID.to_string(),
            kind: "weekly_report".to_string(),
            severity: Severity::Attention,
            summary: format!(
                "Council report: {headline} ({} action{}){}",
                n_actions,
                if n_actions == 1 { "" } else { "s" },
                if verdict_missing {
                    " — no verdict line; the chair did not rule"
                } else {
                    ""
                }
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
        // Retained negative: the user already declined this exact
        // recommendation. Re-filing it is re-litigation, so it is dropped here
        // rather than queued for a second refusal — the Initiative layer's
        // anti-nag guarantee, applied to the Council's actions.
        if negatives::was_declined(pool, KIND, &action.title).await {
            tracing::info!(
                target: "permagent::council",
                "action \"{}\" was already declined; not re-proposing", action.title
            );
            continue;
        }
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
    use crate::session::spectral_schema::{
        apply_briefings_schema, apply_council_schema, init_spectral_db,
    };

    async fn pool() -> Pool<Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        apply_council_schema(&pool).await.unwrap();
        apply_briefings_schema(&pool).await.unwrap();
        pool
    }

    async fn project(pool: &Pool<Sqlite>) -> String {
        crate::projects::create_project(
            pool,
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
        .unwrap()
        .id
    }

    fn report_with(actions: Vec<ChairAction>) -> ChairReport {
        ChairReport {
            headline: "H".into(),
            markdown: "# hi".into(),
            consensus: vec![],
            dissent: vec![],
            actions,
            verdict_missing: false,
        }
    }

    #[tokio::test]
    async fn files_at_most_five_council_actions() {
        let pool = pool().await;
        let project_id = project(&pool).await;
        let actions: Vec<ChairAction> = (0..8)
            .map(|i| ChairAction {
                project_id: project_id.clone(),
                project_name: "Permagent".into(),
                title: format!("Do thing {i}"),
                description: format!("Because {i}"),
            })
            .collect();
        let ids = file_actions(&pool, "sess-1", &report_with(actions))
            .await
            .unwrap();
        assert_eq!(ids.len(), MAX_ACTIONS);
        let open = crate::decisions::list_open_decisions(&pool).await.unwrap();
        assert_eq!(open.len(), MAX_ACTIONS);
        assert!(open.iter().all(|i| i.decision.kind == KIND));
    }

    /// Retained negatives: a recommendation the user already declined is not
    /// filed again, so the same argument is never re-litigated.
    #[tokio::test]
    async fn an_already_declined_action_is_not_re_proposed() {
        let pool = pool().await;
        let project_id = project(&pool).await;
        let action = |title: &str| ChairAction {
            project_id: project_id.clone(),
            project_name: "Permagent".into(),
            title: title.into(),
            description: "why".into(),
        };
        negatives::record_decline(&pool, KIND, "Rewrite the homepage").await;

        let ids = file_actions(
            &pool,
            "sess-2",
            &report_with(vec![
                // Different casing on purpose: the negative is case-folded.
                action("rewrite the HOMEPAGE"),
                action("Ship the pricing page"),
            ]),
        )
        .await
        .unwrap();

        assert_eq!(ids.len(), 1, "the declined action must not be re-filed");
        let open = crate::decisions::list_open_decisions(&pool).await.unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].decision.headline, "Ship the pricing page");
    }

    #[tokio::test]
    async fn the_briefing_says_when_the_chair_never_ruled() {
        let pool = pool().await;
        file_briefing(&pool, "sess-3", "Ship the card", 2, true).await;
        let items = crate::briefings::try_unacknowledged(&pool, 10)
            .await
            .unwrap();
        assert!(
            items.iter().any(|b| b.summary.contains("no verdict line")),
            "{items:#?}"
        );

        file_briefing(&pool, "sess-4", "Ship the card", 2, false).await;
        let items = crate::briefings::try_unacknowledged(&pool, 10)
            .await
            .unwrap();
        assert_eq!(
            items
                .iter()
                .filter(|b| b.summary.contains("no verdict line"))
                .count(),
            1,
            "a ruled report must not be flagged"
        );
    }
}
