//! Watcher project insights (ruling 2026-07-28): once or twice a day per
//! project the Watcher quietly places a short, grounded observation onto the
//! project's Overview — no notification, no badge; you read them as you browse.
//!
//! Honesty law: an insight is composed ONLY from real per-project signals
//! (notes, kanban movement, recorded stack changes). No signals
//! or no LLM provider → silence, never filler. Storage rides
//! `projects.metadata_json.watcher_insights` (newest first, capped) — no
//! schema migration; the Overview card reads it straight off ProjectResponse.

use crate::state::AppState;
use permagent::conversation::message::Message;
use permagent::projects::{self, Project};
use sha2::{Digest, Sha256};
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
        let (signals, card_refs) = match gather_signals(&pool, &project.id).await {
            Ok(evidence) => evidence,
            Err(e) => {
                record_check(&pool, &project.id, "failed").await?;
                tracing::debug!(project_id = %project.id, "watcher evidence unavailable: {e}");
                continue;
            }
        };
        let evidence_key = evidence_key(&signals, &card_refs);
        if signals.is_empty() || already_observed(&project.metadata_json, &evidence_key) {
            record_check(&pool, &project.id, "unchanged").await?;
            continue; // no model call, memory entry or briefing for unchanged evidence
        }
        let Some(text) = compose(&project.name, &signals).await else {
            record_check(&pool, &project.id, "unavailable").await?;
            continue;
        };
        if text.is_empty() {
            // A successful review can find nothing worth interrupting the user
            // for. Remember it so the next tick does not ask the model again.
            record_reviewed(&pool, &project.id, &evidence_key).await?;
            continue;
        }
        if let Err(e) = append_insight(&pool, &project, &text, &card_refs, &evidence_key).await {
            tracing::debug!(project = %project.name, "watcher insight write failed: {e}");
        } else {
            tracing::info!(
                target: "permagentd::watcher",
                project = %project.name,
                "watcher insight placed on the project overview"
            );

            // Report up to Henry as well as onto the project overview. The
            // overview waits to be looked at; a briefing reaches Henry on his
            // next turn, so he can mention it without the user going hunting.
            //
            // `Info`, not `Attention`: the honesty law above means an insight
            // only exists when real signals fired, but it still asks nothing of
            // anyone. Severity is about what is REQUIRED, not how interesting
            // the Watcher found it.
            permagent::briefings::file_briefing(
                &pool,
                permagent::briefings::NewBriefing {
                    from_agent: "watcher".to_string(),
                    kind: "insight".to_string(),
                    severity: permagent::briefings::Severity::Info,
                    summary: format!("{}: {}", project.name, text),
                    detail: None,
                    ref_kind: Some("project".to_string()),
                    ref_id: Some(project.id.clone()),
                },
            )
            .await;
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

/// Cumulative counts and latest revisions stay stable as time passes. Rolling
/// windows alone changed when old events expired, manufacturing "new" evidence.
async fn gather_signals(
    pool: &Pool<Sqlite>,
    project_id: &str,
) -> Result<(Vec<String>, Vec<CardRef>), String> {
    let mut out = Vec::new();
    for (label, sql) in [
        ("Notes", "SELECT count(*), max(updated_at) FROM project_notes WHERE project_id = ?1"),
        ("Cards", "SELECT count(*), max(updated_at) FROM cards WHERE project_id = ?1 AND archived_at IS NULL"),
        ("Recorded stack services", "SELECT count(*), max(updated_at) FROM project_stack_entries WHERE project_id = ?1"),
    ] {
        let (count, latest): (i64, Option<String>) = sqlx::query_as(sql)
            .bind(project_id).fetch_one(pool).await.map_err(|e| e.to_string())?;
        if count > 0 {
            out.push(format!("{label}: {count}; latest recorded change {}", latest.as_deref().unwrap_or("unknown")));
        }
    }
    let stale_cards: Vec<(String, String)> = sqlx::query_as(STALE_CARDS_SQL)
        .bind(project_id)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
    if let Some(signal) = format_stale_signal(&stale_cards) {
        out.push(signal);
    }
    let refs = stale_cards
        .into_iter()
        .map(|(id, title)| CardRef { id, title })
        .collect();
    Ok((out, refs))
}

// Julian dates normalize both SQLite timestamps and RFC3339 timestamps. A
// textual comparison misclassifies same-day timestamps because of 'T' vs ' '.
const STALE_CARDS_SQL: &str = "SELECT c.id, c.title FROM cards c
    JOIN board_columns b ON b.id = c.column_id
    WHERE c.project_id = ?1 AND julianday(c.updated_at) < julianday('now', '-14 days')
      AND c.archived_at IS NULL
      AND coalesce(b.state_binding, '') NOT IN ('complete', 'cancelled')
      AND lower(b.name) NOT IN ('done', 'complete', 'completed', 'cancelled', 'canceled')
    ORDER BY julianday(c.updated_at) ASC, c.id ASC LIMIT 3";

fn evidence_key(signals: &[String], cards: &[CardRef]) -> String {
    let bytes = serde_json::to_vec(&(signals, cards)).expect("serializable evidence");
    hex::encode(Sha256::digest(bytes))
}

fn already_observed(metadata: &serde_json::Value, key: &str) -> bool {
    metadata
        .get("watcher_evidence_key")
        .and_then(|v| v.as_str())
        == Some(key)
}

async fn record_check(pool: &Pool<Sqlite>, project_id: &str, status: &str) -> Result<(), String> {
    let check =
        serde_json::json!({"status": status, "checked_at": chrono::Utc::now().to_rfc3339()});
    sqlx::query("UPDATE projects SET metadata_json = json_set(CASE WHEN json_valid(metadata_json) AND json_type(metadata_json) = 'object' THEN metadata_json ELSE '{}' END, '$.watcher_check', json(?1)) WHERE id = ?2")
        .bind(check.to_string()).bind(project_id).execute(pool).await.map_err(|e| e.to_string())?;
    Ok(())
}

async fn record_reviewed(
    pool: &Pool<Sqlite>,
    project_id: &str,
    evidence_key: &str,
) -> Result<(), String> {
    let check =
        serde_json::json!({"status": "reviewed", "checked_at": chrono::Utc::now().to_rfc3339()});
    sqlx::query("UPDATE projects SET metadata_json = json_set(CASE WHEN json_valid(metadata_json) AND json_type(metadata_json) = 'object' THEN metadata_json ELSE '{}' END, '$.watcher_check', json(?1), '$.watcher_evidence_key', ?2) WHERE id = ?3")
        .bind(check.to_string()).bind(evidence_key).bind(project_id).execute(pool).await.map_err(|e| e.to_string())?;
    Ok(())
}

/// Phrase the stalled-card signal so the composer can NAME what stalled.
///
/// Pure, so the one property that matters is testable without a pool: the
/// signal must carry titles, never just a number. A bare count is what produced
/// "One card stalled 14+ days" — true, unactionable, and indistinguishable from
/// the same sentence the day before.
fn format_stale_signal(cards: &[(String, String)]) -> Option<String> {
    if cards.is_empty() {
        return None;
    }
    let titles = cards
        .iter()
        .map(|(_, t)| format!("\"{t}\""))
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "{} card(s) untouched for 14+ days, oldest first: {titles}",
        cards.len()
    ))
}

/// A card an insight is ABOUT. Carried on the insight so the Overview can link
/// straight to it — the difference between "one card stalled" and a row the
/// reader can click.
#[derive(Clone, serde::Serialize)]
struct CardRef {
    id: String,
    title: String,
}

/// One short observation via the configured provider's fast model. None on
/// no-provider or malformed response; an empty string is a successful quiet review.
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
                  signals.\n\
                  \n\
                  NAME THE THING. When a signal quotes a card title, use that title in your \
                  sentence. \"One card stalled 14+ days\" is useless — the reader cannot tell \
                  which card, so there is nothing they can do. \"Onboarding copy has sat 3 weeks\" \
                  is the same length and actually actionable. Never write \"one card\", \"a card\" \
                  or \"some cards\" when you were given titles.\n\
                  \n\
                  Reply ONLY as JSON: {\"insight\": \"<text, or empty if nothing worth saying>\"}";
    let user = Message::user().with_text(format!(
        "Project: {project_name}\nRecorded evidence:\n- {}",
        signals.join("\n- ")
    ));
    let (response, _usage) = provider
        .complete_fast("watcher-insights", system, std::slice::from_ref(&user), &[])
        .await
        .ok()?;
    parse_insight(&response.as_concat_text())
}

fn parse_insight(text: &str) -> Option<String> {
    let (start, end) = (text.find('{')?, text.rfind('}')?);
    let v: serde_json::Value = serde_json::from_str(text.get(start..=end)?).ok()?;
    let insight = v.get("insight")?.as_str()?.trim().to_string();
    Some(insight)
}

async fn append_insight(
    pool: &Pool<Sqlite>,
    project: &Project,
    text: &str,
    cards: &[CardRef],
    evidence_key: &str,
) -> Result<(), String> {
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
            "id": evidence_key,
            "evidence": cards,
            "text": text,
            "created_at": chrono::Utc::now().to_rfc3339(),
            // The cards this insight is about. Older rows have no `cards` key;
            // the Overview treats absent and empty identically, so the panel
            // keeps rendering historical insights as plain text.
            "cards": cards,
        }),
    );
    list.truncate(MAX_KEPT);
    obj.insert(METADATA_KEY.to_string(), serde_json::Value::Array(list));
    // Update only Watcher-owned keys atomically; concurrent manual metadata
    // edits must not be overwritten by the snapshot used for this check.
    let check =
        serde_json::json!({"status": "changed", "checked_at": chrono::Utc::now().to_rfc3339()});
    sqlx::query("UPDATE projects SET metadata_json = json_set(CASE WHEN json_valid(metadata_json) AND json_type(metadata_json) = 'object' THEN metadata_json ELSE '{}' END, '$.watcher_insights', json(?1), '$.watcher_evidence_key', ?2, '$.watcher_check', json(?3)) WHERE id = ?4")
        .bind(obj.get(METADATA_KEY).expect("inserted").to_string())
        .bind(evidence_key).bind(check.to_string()).bind(&project.id)
        .execute(pool).await.map_err(|e| e.to_string())?;

    permagent::brain_feed::feed_for_project(
        pool,
        permagent::brain_feed::FeedSource::WatcherInsight,
        permagent::brain_feed::feed_key("watcher_insight", &project.id, evidence_key),
        &project.id,
        format!("Watcher insight for '{}': {}", project.name, text),
    )
    .await;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: &str, title: &str) -> (String, String) {
        (id.to_string(), title.to_string())
    }

    /// The regression this file exists to prevent: the signal used to be a bare
    /// `count(*)`, so the composer had nothing to name and could only ever
    /// write "One card stalled 14+ days".
    #[test]
    fn stale_signal_names_the_cards_it_counts() {
        let signal =
            format_stale_signal(&[card("c1", "Onboarding copy"), card("c2", "Pricing page")])
                .expect("two stale cards produce a signal");

        assert!(
            signal.contains("Onboarding copy"),
            "signal must name the card: {signal}"
        );
        assert!(
            signal.contains("Pricing page"),
            "signal must name every card: {signal}"
        );
        assert!(
            signal.contains('2'),
            "the count is still useful alongside the names"
        );
    }

    /// A single stalled card is the case that produced the useless message, so
    /// it gets its own assertion rather than riding on the plural one.
    #[test]
    fn a_single_stale_card_is_still_named() {
        let signal = format_stale_signal(&[card("c1", "Ship the changelog")]).unwrap();
        assert!(signal.contains("Ship the changelog"));
    }

    /// Nothing stalled means no signal at all — the Watcher's honesty law is
    /// silence over filler, so an empty list must not produce "0 card(s)".
    #[test]
    fn no_stale_cards_produces_no_signal() {
        assert!(format_stale_signal(&[]).is_none());
    }

    #[test]
    fn quiet_review_is_distinct_from_an_unavailable_or_malformed_response() {
        assert_eq!(parse_insight(r#"{"insight":""}"#), Some(String::new()));
        assert_eq!(
            parse_insight(r#"{"insight":" Check the launch task. "}"#),
            Some("Check the launch task.".into())
        );
        assert_eq!(parse_insight("provider unavailable"), None);
        assert_eq!(parse_insight(r#"{"other":"text"}"#), None);
    }

    #[test]
    fn unchanged_evidence_is_silent_but_a_revision_is_new() {
        let signals = vec!["Notes: 2; latest recorded change 2026-10-15".to_string()];
        let key = evidence_key(&signals, &[]);
        let metadata = serde_json::json!({"watcher_evidence_key": key});
        assert!(already_observed(&metadata, &evidence_key(&signals, &[])));
        assert!(!already_observed(
            &metadata,
            &evidence_key(
                &["Notes: 2; latest recorded change 2026-10-16".to_string()],
                &[]
            )
        ));
        assert!(!already_observed(&serde_json::json!({}), &key));
    }

    #[tokio::test]
    async fn stale_query_excludes_terminal_archived_recent_and_other_projects() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("CREATE TABLE board_columns(id TEXT, name TEXT, state_binding TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE cards(id TEXT, title TEXT, project_id TEXT, column_id TEXT, updated_at TEXT, archived_at TEXT)").execute(&pool).await.unwrap();
        for (id, name, state) in [
            ("active", "Ready", "ready"),
            ("complete", "Complete", "complete"),
            ("cancelled", "Cancelled", "cancelled"),
            ("legacy", "Done", ""),
        ] {
            sqlx::query("INSERT INTO board_columns VALUES (?1, ?2, ?3)")
                .bind(id)
                .bind(name)
                .bind(state)
                .execute(&pool)
                .await
                .unwrap();
        }
        for (id, column, project, date, archived) in [
            ("stale", "active", "p", "2000-01-01T12:00:00Z", None),
            ("done", "complete", "p", "2000-01-01", None),
            ("cancelled", "cancelled", "p", "2000-01-01", None),
            ("legacy", "legacy", "p", "2000-01-01", None),
            ("archived", "active", "p", "2000-01-01", Some("2000-02-01")),
            ("recent", "active", "p", "2999-01-01", None),
            ("other", "active", "elsewhere", "2000-01-01", None),
        ] {
            sqlx::query("INSERT INTO cards VALUES (?1, ?1, ?2, ?3, ?4, ?5)")
                .bind(id)
                .bind(project)
                .bind(column)
                .bind(date)
                .bind(archived)
                .execute(&pool)
                .await
                .unwrap();
        }
        let rows: Vec<(String, String)> = sqlx::query_as(STALE_CARDS_SQL)
            .bind("p")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(rows, vec![card("stale", "stale")]);
    }

    #[tokio::test]
    async fn failed_checks_preserve_manual_metadata_and_do_not_claim_clear() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("CREATE TABLE projects(id TEXT, metadata_json TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(r#"INSERT INTO projects VALUES ('p', '{"manual":"keep"}')"#)
            .execute(&pool)
            .await
            .unwrap();
        assert!(gather_signals(&pool, "p").await.is_err());
        record_check(&pool, "p", "failed").await.unwrap();
        let raw: String = sqlx::query_scalar("SELECT metadata_json FROM projects WHERE id='p'")
            .fetch_one(&pool)
            .await
            .unwrap();
        let data: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(data["manual"], "keep");
        assert_eq!(data["watcher_check"]["status"], "failed");
        assert!(data.get("watcher_insights").is_none());
        record_reviewed(&pool, "p", "quiet-evidence").await.unwrap();
        let raw: String = sqlx::query_scalar("SELECT metadata_json FROM projects WHERE id='p'")
            .fetch_one(&pool)
            .await
            .unwrap();
        let data: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(data["manual"], "keep");
        assert_eq!(data["watcher_check"]["status"], "reviewed");
        assert!(already_observed(&data, "quiet-evidence"));
        assert!(data.get("watcher_insights").is_none());
    }
}
