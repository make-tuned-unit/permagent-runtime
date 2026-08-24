//! Watcher project insights (ruling 2026-07-28): once or twice a day per
//! project the Watcher quietly places a short, grounded observation onto the
//! project's Overview — no notification, no badge; you read them as you browse.
//!
//! Honesty law: an insight is composed ONLY from real per-project signals
//! (notes, kanban movement, stack changes, first-party analytics). No signals
//! or no LLM provider → silence, never filler. Storage rides
//! `projects.metadata_json.watcher_insights` (newest first, capped) — no
//! schema migration; the Overview card reads it straight off ProjectResponse.

use crate::agent_pass::{record_pass, Pass};
use crate::state::AppState;
use permagent::agent_runs::Trigger;
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
            if let Err(e) = run_once(&state, Trigger::Interval).await {
                tracing::debug!("watcher insights pass skipped: {e}");
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

/// One insight pass, recorded.
///
/// The pass itself lives in [`insight_pass`]; this wrapper exists so that
/// exactly ONE run row is written per invocation, with `started_at` stamped
/// before any work. Same shape as the Guard's and the Steward's loops.
///
/// The Watcher has no feature flag and no "is it due yet" gate — `spawn` calls
/// this on every tick — so unlike those two there is no path above this
/// function that declines to run, and every tick produces a row.
async fn run_once(state: &Arc<AppState>, trigger: Trigger) -> Result<(), String> {
    let started_at = chrono::Utc::now();
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| e.to_string())?;
    let pass = insight_pass(&pool).await;
    let _ = record_pass(
        &pool,
        permagent::echo::SELF_KNOWLEDGE_FEATURE.id,
        trigger,
        started_at,
        &pass.outcome,
    )
    .await;
    pass.result
}

/// Run the Watcher's insight pass now, because a person asked.
///
/// Called by `POST /api/agents/{id}/run`, which reports the run row this pass
/// records rather than its own account of what happened.
///
/// The same body as the interval pass — only the recorded trigger differs, so a
/// manual run lands in the same history rather than a parallel one. There is no
/// gate to re-check here: the Watcher's pass is unconditional, and its silence
/// comes from having nothing real to say, never from being switched off.
pub async fn run_pass_now(state: &Arc<AppState>) -> Result<(), String> {
    run_once(state, Trigger::Manual).await
}

/// The Watcher's actual pass.
///
/// Returns a [`Pass`] rather than returning early so `run_once` can record
/// whichever way it went. Note what the row can and cannot say: the per-project
/// `continue`s below are NOT pass-level skips — a project already at its daily
/// cap, or with no real signals, is the honesty law working, and the pass as a
/// whole still ran and examined it. So the common shape of a healthy row here
/// is `ok`, N examined, nothing produced.
async fn insight_pass(pool: &Pool<Sqlite>) -> Pass {
    let projects = match projects::list_projects(pool, Some("active")).await {
        Ok(projects) => projects,
        // Not a skip: the pass could not read its own worklist.
        Err(e) => return Pass::failed(None, e),
    };
    let observable = observable_projects(&projects);
    if observable.is_empty() {
        return Pass::skipped(
            "no active project to observe (the default Personal bucket is not one the Watcher \
             reports on)",
        );
    }
    // What this pass looked at. Counted before the loop because it is the
    // denominator the row is claiming — "examined 6, produced nothing" is a
    // very different sentence from "examined 0, produced nothing".
    let examined = observable.len() as i64;
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let mut placed: Vec<String> = Vec::new();

    for project in observable {
        if insights_on_day(project, &today) >= PER_DAY {
            continue;
        }
        let (signals, card_refs) = gather_signals(pool, &project.id).await;
        if signals.is_empty() {
            continue; // nothing real happened — the Watcher stays silent
        }
        let Some(text) = compose(&project.name, &signals).await else {
            continue;
        };
        if let Err(e) = append_insight(pool, project, &text, &card_refs).await {
            tracing::debug!(project = %project.name, "watcher insight write failed: {e}");
        } else {
            placed.push(project.name.clone());
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
                pool,
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
    Pass::completed(Some(examined), produced_line(&placed))
}

/// The projects this pass will consider.
///
/// The default bucket is not a project the Watcher reports on, so it is not
/// counted as examined either: `examined` has to be what the pass actually
/// looked at, or the number is decoration.
fn observable_projects(all: &[Project]) -> Vec<&Project> {
    all.iter()
        .filter(|p| p.id != projects::PERSONAL_PROJECT_ID)
        .collect()
}

/// The one-line `produced` summary for a completed pass.
///
/// `None` when no insight was placed, which is the normal result and the whole
/// reason this row exists: the Watcher's honesty law is silence over filler, so
/// most passes produce nothing, and until now a silent pass and an absent one
/// looked identical. The projects are NAMED for the same reason the stale-card
/// signal names its cards — a bare count sends the reader hunting.
fn produced_line(placed: &[String]) -> Option<String> {
    if placed.is_empty() {
        return None;
    }
    Some(format!(
        "{} insight(s) placed, on: {}",
        placed.len(),
        placed.join(", ")
    ))
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
async fn gather_signals(pool: &Pool<Sqlite>, project_id: &str) -> (Vec<String>, Vec<CardRef>) {
    let mut out = Vec::new();
    let mut refs: Vec<CardRef> = Vec::new();
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
    // Name the stalled cards, don't just count them.
    //
    // This was `SELECT count(*)`, which produced the signal "1 card(s)
    // untouched for 14+ days" — so the composer could only ever write "One card
    // stalled 14+ days", with no way for the reader to learn WHICH card. An
    // observation you cannot act on is not an observation, and repeated daily
    // it reads as noise. The titles cost nothing extra to fetch and make the
    // insight answer its own question.
    let stale_cards: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, title FROM cards \
         WHERE project_id = ?1 AND updated_at < datetime('now','-14 days') \
           AND archived_at IS NULL \
         ORDER BY updated_at ASC LIMIT 3",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    if let Some(signal) = format_stale_signal(&stale_cards) {
        out.push(signal);
        refs.extend(stale_cards.iter().map(|(id, title)| CardRef {
            id: id.clone(),
            title: title.clone(),
        }));
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
    (out, refs)
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

async fn append_insight(
    pool: &Pool<Sqlite>,
    project: &Project,
    text: &str,
    cards: &[CardRef],
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

    // ── Run recording ─────────────────────────────────────────────────────
    //
    // COVERED here: which projects a pass counts as examined, the pass-level
    // skip and its reason, the `produced` line, and the fact that each lands in
    // an `agent_runs` row under `watcher` with the trigger it was given.
    //
    // NOT covered: `insight_pass` end-to-end (it needs an `AppState`, a project
    // database and a configured LLM provider), and one thing the row genuinely
    // cannot say yet — a pass where every project had signals but `compose`
    // returned `None` because no provider is configured records `ok / examined
    // N / produced nothing`, identical to a pass where nothing was worth
    // saying. Attributing that to the provider from inside the loop would be a
    // guess, and a guessed reason is worse than none.

    use crate::test_support::project_fixture;
    use permagent::agent_runs::{recent_for_agent, Outcome};

    const WATCHER_ID: &str = permagent::echo::SELF_KNOWLEDGE_FEATURE.id;

    async fn runs_pool() -> Pool<Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        permagent::session::spectral_schema::apply_agent_runs_schema(&pool)
            .await
            .unwrap();
        pool
    }

    fn started() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp(1_760_000_000, 0).unwrap()
    }

    /// The row this record exists for. The Watcher's honesty law means most
    /// passes place nothing, so a silent pass and an absent one were the same
    /// thing from outside — for a worker whose whole job is silence, that made
    /// "is it running?" unanswerable.
    #[tokio::test]
    async fn a_pass_that_placed_no_insight_still_writes_one_ok_run_row() {
        let pool = runs_pool().await;
        let pass = Pass::completed(Some(4), produced_line(&[]));
        assert!(pass.result.is_ok());
        let _ = record_pass(
            &pool,
            WATCHER_ID,
            Trigger::Interval,
            started(),
            &pass.outcome,
        )
        .await;

        let runs = recent_for_agent(&pool, WATCHER_ID, 10).await.unwrap();
        assert_eq!(runs.len(), 1, "exactly one row per pass");
        assert_eq!(runs[0].outcome, Outcome::Ok);
        assert_eq!(
            runs[0].examined,
            Some(4),
            "the projects it looked at — 'examined 4, produced nothing' is a very \
             different sentence from 'examined 0, produced nothing'"
        );
        assert_eq!(runs[0].produced, None);
    }

    /// `examined` must be what the pass actually looked at. The default bucket
    /// is skipped before any work, so counting it would inflate the number the
    /// row is claiming.
    #[test]
    fn the_personal_bucket_is_not_counted_as_examined() {
        let projects = vec![
            project_fixture(
                projects::PERSONAL_PROJECT_ID,
                "Personal",
                None,
                serde_json::json!({}),
            ),
            project_fixture("a", "Alpha", None, serde_json::json!({})),
            project_fixture("b", "Beta", None, serde_json::json!({})),
        ];
        let observable = observable_projects(&projects);
        assert_eq!(observable.len(), 2);
        assert!(observable.iter().all(|p| p.name != "Personal"));
    }

    /// Nothing to observe is not the same as observing everything and finding
    /// nothing worth saying, and the row has to be able to tell them apart.
    #[tokio::test]
    async fn a_fleet_with_nothing_to_observe_records_a_skip_naming_the_reason() {
        let personal_only = vec![project_fixture(
            projects::PERSONAL_PROJECT_ID,
            "Personal",
            None,
            serde_json::json!({}),
        )];
        assert!(observable_projects(&personal_only).is_empty());

        // The reason the pass itself uses for this condition.
        let pass = Pass::skipped(
            "no active project to observe (the default Personal bucket is not one the Watcher \
             reports on)",
        );
        let pool = runs_pool().await;
        let _ = record_pass(
            &pool,
            WATCHER_ID,
            Trigger::Interval,
            started(),
            &pass.outcome,
        )
        .await;

        let runs = recent_for_agent(&pool, WATCHER_ID, 10).await.unwrap();
        assert_eq!(runs[0].outcome, Outcome::Skipped);
        assert!(
            runs[0]
                .reason
                .as_deref()
                .unwrap()
                .contains("Personal bucket"),
            "the reason must name what was excluded: {:?}",
            runs[0].reason
        );
        assert_eq!(
            runs[0].examined, None,
            "a pass that declined to observe examined nothing — never a fabricated 0"
        );
    }

    /// `produced` names the projects, for the same reason the stale-card signal
    /// names its cards: a bare count sends the reader hunting.
    #[test]
    fn produced_names_the_projects_it_counts() {
        assert_eq!(produced_line(&[]), None);
        let line = produced_line(&["Atlas".to_string(), "Beacon".to_string()]).unwrap();
        assert!(line.contains("Atlas") && line.contains("Beacon"), "{line}");
        assert!(
            line.contains('2'),
            "the count is still useful alongside: {line}"
        );
    }

    /// A manual run and a scheduled one land in the same history, each labelled
    /// for what it was. (`run_pass_now` binds `Trigger::Manual` and `spawn`
    /// binds `Trigger::Interval` in one line each; what is testable without an
    /// `AppState` is that the trigger handed to `record_pass` survives the
    /// round trip, which is what those two lines depend on.)
    #[tokio::test]
    async fn a_manual_pass_is_recorded_as_manual_beside_the_scheduled_ones() {
        let pool = runs_pool().await;
        let _ = record_pass(
            &pool,
            WATCHER_ID,
            Trigger::Interval,
            started(),
            &Pass::completed(Some(2), None).outcome,
        )
        .await;
        let _ = record_pass(
            &pool,
            WATCHER_ID,
            Trigger::Manual,
            started() + chrono::Duration::seconds(30),
            &Pass::completed(Some(2), None).outcome,
        )
        .await;

        let runs = recent_for_agent(&pool, WATCHER_ID, 10).await.unwrap();
        assert_eq!(runs.len(), 2, "one history, not two");
        assert_eq!(runs[0].trigger, Trigger::Manual);
        assert_eq!(runs[1].trigger, Trigger::Interval);
    }
}
