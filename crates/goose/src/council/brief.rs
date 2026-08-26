//! Deterministic portfolio snapshot the Council members all see.

use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};

const MAX_SECTION: usize = 4000;
const ACTIVITY_LIMIT: i64 = 40;
const BRIEFING_LIMIT: i64 = 12;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PortfolioBrief {
    pub assembled_at: String,
    pub extra_question: Option<String>,
    pub markdown: String,
}

pub async fn assemble(
    pool: &Pool<Sqlite>,
    extra_question: Option<&str>,
) -> Result<PortfolioBrief, String> {
    let assembled_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let mut sections = Vec::new();

    sections.push(projects_section(pool).await);
    sections.push(due_section(pool).await);
    sections.push(board_section(pool).await);
    sections.push(activity_section(pool).await);
    sections.push(briefings_section(pool).await);
    sections.push(decisions_section(pool).await);
    sections.push(watcher_section(pool).await);
    sections.push(analytics_section(pool).await);
    sections.push(forecaster_section(pool).await);
    sections.push(brain_section().await);

    if let Some(q) = extra_question.map(str::trim).filter(|s| !s.is_empty()) {
        sections.push(format!("## Extra question from Henry\n\n{q}"));
    }

    let markdown = sections
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    Ok(PortfolioBrief {
        assembled_at,
        extra_question: extra_question
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        markdown: cap(&markdown, 24_000),
    })
}

fn cap(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

fn cap_section(s: String) -> String {
    cap(&s, MAX_SECTION)
}

async fn projects_section(pool: &Pool<Sqlite>) -> String {
    let Ok(projects) = crate::projects::list_projects(pool, Some("active")).await else {
        return String::new();
    };
    if projects.is_empty() {
        return "## Projects\n\nNo active projects.".to_string();
    }
    let mut lines = vec!["## Projects".to_string()];
    for p in projects.iter().take(24) {
        let mut line = format!("- {} (`{}`, id {})", p.name, p.slug, p.id);
        if !p.description.trim().is_empty() {
            line.push_str(&format!(" — {}", cap(&p.description, 160)));
        }
        if let Some(insights) = p.metadata_json.get("watcher_insights") {
            if !insights.is_null() {
                line.push_str(&format!(" [watcher: {}]", cap(&insights.to_string(), 200)));
            }
        }
        lines.push(line);
    }
    cap_section(lines.join("\n"))
}

async fn due_section(pool: &Pool<Sqlite>) -> String {
    let Ok(due) = crate::cards::list_due_cards(pool).await else {
        return String::new();
    };
    if due.is_empty() {
        return "## Due cards\n\nNone.".to_string();
    }
    let mut lines = vec!["## Due cards".to_string()];
    for c in due.iter().take(30) {
        lines.push(format!(
            "- {} due {} on {} ({})",
            c.title, c.due_date, c.project_name, c.column_name
        ));
    }
    cap_section(lines.join("\n"))
}

async fn board_section(pool: &Pool<Sqlite>) -> String {
    match crate::agents::platform_extensions::orchestrator::format_board_summary(pool).await {
        Ok(s) if !s.trim().is_empty() => cap_section(s),
        _ => String::new(),
    }
}

async fn activity_section(pool: &Pool<Sqlite>) -> String {
    let Ok(items) = crate::activity_journal::page(pool, None, ACTIVITY_LIMIT, None, None).await
    else {
        return String::new();
    };
    if items.is_empty() {
        return "## Activity (7 days)\n\nNothing journaled.".to_string();
    }
    let week_ago = (chrono::Utc::now() - chrono::Duration::days(7)).to_rfc3339();
    let mut lines = vec!["## Activity (recent)".to_string()];
    for item in items.iter().filter(|i| i.ts >= week_ago).take(30) {
        lines.push(format!(
            "- [{}] {} — {} ({})",
            item.kind,
            item.actor,
            item.title,
            item.detail.as_deref().unwrap_or("")
        ));
    }
    if lines.len() == 1 {
        lines.push("Nothing in the last 7 days.".to_string());
    }
    cap_section(lines.join("\n"))
}

async fn briefings_section(pool: &Pool<Sqlite>) -> String {
    let Ok(items) = crate::briefings::try_unacknowledged(pool, BRIEFING_LIMIT).await else {
        return String::new();
    };
    if items.is_empty() {
        return "## Unread worker briefings\n\nNone.".to_string();
    }
    let mut lines = vec!["## Unread worker briefings".to_string()];
    for b in items {
        lines.push(format!(
            "- [{}] {} — {}",
            b.severity.render(),
            b.from_agent,
            b.summary
        ));
    }
    cap_section(lines.join("\n"))
}

async fn decisions_section(pool: &Pool<Sqlite>) -> String {
    let Ok(open) = crate::decisions::list_open_decisions(pool).await else {
        return String::new();
    };
    if open.is_empty() {
        return "## Open Decision Inbox\n\nNone.".to_string();
    }
    let mut lines = vec!["## Open Decision Inbox".to_string()];
    for item in open.iter().take(20) {
        lines.push(format!(
            "- [{}] {}",
            item.decision.kind, item.decision.headline
        ));
    }
    cap_section(lines.join("\n"))
}

async fn watcher_section(pool: &Pool<Sqlite>) -> String {
    let Ok(projects) = crate::projects::list_projects(pool, Some("active")).await else {
        return String::new();
    };
    let mut lines = vec!["## Watcher insights".to_string()];
    for p in projects {
        let Some(insights) = p.metadata_json.get("watcher_insights") else {
            continue;
        };
        if insights.is_null() {
            continue;
        }
        let text = if let Some(s) = insights.as_str() {
            s.to_string()
        } else {
            insights.to_string()
        };
        if text.trim().is_empty() {
            continue;
        }
        lines.push(format!("### {}\n{}", p.name, cap(&text, 600)));
    }
    if lines.len() == 1 {
        return String::new();
    }
    cap_section(lines.join("\n"))
}

async fn brain_section() -> String {
    let Some(brain) = crate::agents::platform_extensions::get_global_brain() else {
        return String::new();
    };
    match brain
        .recall("what we were working on", spectral::Visibility::Private)
        .await
    {
        Ok(result) if !result.memory_hits.is_empty() => {
            let mut lines = vec!["## Brain recall (what we were working on)".to_string()];
            for hit in result.memory_hits.iter().take(8) {
                let preview = cap(hit.content.as_str(), 220);
                if preview.trim().is_empty() {
                    continue;
                }
                lines.push(format!("- {preview}"));
            }
            if lines.len() == 1 {
                return String::new();
            }
            cap_section(lines.join("\n"))
        }
        _ => String::new(),
    }
}

async fn analytics_section(pool: &Pool<Sqlite>) -> String {
    let rows: Result<Vec<(String, i64, i64)>, _> = sqlx::query_as(
        "SELECT project_id,
                SUM(CASE WHEN kind = 'pageview' THEN 1 ELSE 0 END) AS pageviews,
                COUNT(*) AS events
         FROM analytics_events
         WHERE created_at >= datetime('now', '-7 days')
         GROUP BY project_id
         ORDER BY pageviews DESC
         LIMIT 12",
    )
    .fetch_all(pool)
    .await;
    let Ok(rows) = rows else {
        return String::new();
    };
    if rows.is_empty() {
        return String::new();
    }
    let mut lines = vec!["## First-party analytics (7 days)".to_string()];
    for (project_id, pageviews, events) in rows {
        let name = crate::projects::get_project(pool, &project_id)
            .await
            .ok()
            .flatten()
            .map(|p| p.name)
            .unwrap_or(project_id);
        lines.push(format!("- {name}: {pageviews} pageviews, {events} events"));
    }
    cap_section(lines.join("\n"))
}

async fn forecaster_section(pool: &Pool<Sqlite>) -> String {
    let rows: Result<Vec<(String, String, String)>, _> = sqlx::query_as(
        "SELECT b.project_id, b.summary, b.generated_at
         FROM forecaster_briefs b
         JOIN (
            SELECT project_id, MAX(generated_at) AS generated_at
            FROM forecaster_briefs GROUP BY project_id
         ) latest ON latest.project_id = b.project_id AND latest.generated_at = b.generated_at
         ORDER BY b.generated_at DESC
         LIMIT 12",
    )
    .fetch_all(pool)
    .await;
    let Ok(rows) = rows else {
        return String::new();
    };
    if rows.is_empty() {
        return String::new();
    }
    let mut lines = vec!["## Forecaster weekly briefs (direction only, not advice)".to_string()];
    for (project_id, summary, generated_at) in rows {
        let name = crate::projects::get_project(pool, &project_id)
            .await
            .ok()
            .flatten()
            .map(|p| p.name)
            .unwrap_or(project_id);
        lines.push(format!("- {name} ({generated_at}): {}", cap(&summary, 280)));
    }
    cap_section(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::spectral_schema::init_spectral_db;

    async fn pool() -> Pool<Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn assembles_from_an_empty_fixture_db() {
        let pool = pool().await;
        let brief = assemble(&pool, Some("Are we over-rotated?")).await.unwrap();
        assert!(brief.markdown.contains("## Projects"));
        assert!(brief.markdown.contains("Are we over-rotated?"));
        assert_eq!(
            brief.extra_question.as_deref(),
            Some("Are we over-rotated?")
        );
    }
}
