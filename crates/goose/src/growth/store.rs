//! Durable identity for growth actions, and their outcomes.
//!
//! The blocker the proposal opens with: both producers recompute their advice on
//! every load, so "an action has no identity, so nothing can be attached to it —
//! not 'I did this', not a baseline, not an outcome." The cache in
//! `projects.metadata_json["growth_actions"]` (growth_actions.rs:44) is
//! rewritten wholesale by `regenerate`'s `store()` call (growth_actions.rs:831),
//! so it can never hold status: one press of "Review again" would erase every
//! pre-registration. These two tables are the identity; the bag stays a render
//! cache for `evidence` and `steps`, which are not worth a column.

use super::metrics::{MetricWindow, TargetDir, TargetMetric};
use super::power::{Confounder, Judgement};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, Pool, Sqlite};

/// Lifecycle. `suggested` on first persist; the rest are user or job driven.
pub const STATUS_SUGGESTED: &str = "suggested";
pub const STATUS_DISMISSED: &str = "dismissed";
pub const STATUS_DONE: &str = "done";
pub const STATUS_VERIFIED: &str = "verified";
pub const STATUS_MEASURING: &str = "measuring";
pub const STATUS_JUDGED: &str = "judged";

pub const STATUSES: &[&str] = &[
    STATUS_SUGGESTED,
    STATUS_DISMISSED,
    STATUS_DONE,
    STATUS_VERIFIED,
    STATUS_MEASURING,
    STATUS_JUDGED,
];

/// How a change was confirmed to have landed. Shown on the card, because
/// "verified from a commit" and "you told me so" are different claims.
pub const VERIFIED_BY_GIT: &str = "git";
pub const VERIFIED_BY_CONTENT: &str = "content";
pub const VERIFIED_BY_EVENT: &str = "event";
pub const VERIFIED_BY_SELF: &str = "self";

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GrowthActionRow {
    pub id: String,
    pub project_id: String,
    pub fingerprint: String,
    pub title: String,
    pub recommendation: String,
    pub category: Option<String>,
    pub artifact_kind: Option<String>,
    pub artifact: Option<String>,
    pub target_metric: Option<String>,
    pub target_dir: Option<String>,
    pub baseline_json: Option<String>,
    pub status: String,
    pub verified_by: Option<String>,
    pub verified_at: Option<String>,
    pub created_at: String,
}

/// What the generator supplies. Everything else on the row is lifecycle state
/// the generator has no business setting.
#[derive(Debug, Clone)]
pub struct ActionSeed {
    pub title: String,
    pub recommendation: String,
    pub category: Option<String>,
    pub artifact_kind: Option<String>,
    pub artifact: Option<String>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutcomeRow {
    pub action_id: String,
    pub window_days: i64,
    pub before_json: String,
    pub after_json: String,
    pub delta_pct: Option<f64>,
    pub verdict: String,
    pub rationale: String,
    pub confounders: Option<String>,
    pub judged_at: String,
}

/// Collapse the incidental differences between two renderings of the same
/// advice: surrounding space, capitalisation, and the run-length of internal
/// whitespace.
///
/// This is what makes the fingerprint survive regeneration. The model rewords
/// whitespace and casing freely between calls, and `parse_actions` then truncates
/// at 120/600 chars (growth_actions.rs:408), so hashing the raw strings would
/// mint a new row — and orphan yesterday's outcomes — every time the panel is
/// refreshed. That duplicate-card failure is precisely what
/// `UNIQUE(project_id, fingerprint)` exists to prevent.
fn normalize(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// `sha256(project_id ␟ title ␟ recommendation)`, hex.
///
/// The unit separator (U+001F) is a real separator rather than a cosmetic one:
/// joining with a character that can occur in the text would let
/// `("ab", "c")` and `("a", "bc")` collide.
pub fn fingerprint(project_id: &str, title: &str, recommendation: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(project_id.as_bytes());
    hasher.update([0x1f]);
    hasher.update(normalize(title).as_bytes());
    hasher.update([0x1f]);
    hasher.update(normalize(recommendation).as_bytes());
    hex::encode(hasher.finalize())
}

/// Persist a generated action, or refresh the text of one already there.
///
/// The `WHERE status = 'suggested'` guard on the update is load-bearing: once a
/// user has marked an action done and pre-registered a metric against its text,
/// regeneration must not rewrite the claim the baseline refers to. Without it,
/// "Review again" would silently re-word the hypothesis after the experiment
/// started.
pub async fn upsert_suggested(
    pool: &Pool<Sqlite>,
    project_id: &str,
    seed: &ActionSeed,
) -> anyhow::Result<GrowthActionRow> {
    let fp = fingerprint(project_id, &seed.title, &seed.recommendation);
    // uuid v7 is time-ordered, so `ORDER BY id` doubles as creation order
    // (same reason projects.rs:138 uses it).
    let id = uuid::Uuid::now_v7().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO growth_actions
            (id, project_id, fingerprint, title, recommendation, category,
             artifact_kind, artifact, status, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'suggested', ?9)
         ON CONFLICT(project_id, fingerprint) DO UPDATE SET
            title          = excluded.title,
            recommendation = excluded.recommendation,
            category       = excluded.category,
            artifact_kind  = excluded.artifact_kind,
            artifact       = excluded.artifact
         WHERE growth_actions.status = 'suggested'",
    )
    .bind(&id)
    .bind(project_id)
    .bind(&fp)
    .bind(&seed.title)
    .bind(&seed.recommendation)
    .bind(seed.category.as_deref())
    .bind(seed.artifact_kind.as_deref())
    .bind(seed.artifact.as_deref())
    .bind(&now)
    .execute(pool)
    .await?;

    get_by_fingerprint(pool, project_id, &fp)
        .await?
        .ok_or_else(|| anyhow::anyhow!("growth action vanished immediately after upsert"))
}

pub async fn get_by_fingerprint(
    pool: &Pool<Sqlite>,
    project_id: &str,
    fingerprint: &str,
) -> anyhow::Result<Option<GrowthActionRow>> {
    Ok(sqlx::query_as::<_, GrowthActionRow>(
        "SELECT * FROM growth_actions WHERE project_id = ?1 AND fingerprint = ?2",
    )
    .bind(project_id)
    .bind(fingerprint)
    .fetch_optional(pool)
    .await?)
}

/// Scoped by project on purpose: an action id from one project must not be
/// addressable through another project's route.
pub async fn get(
    pool: &Pool<Sqlite>,
    project_id: &str,
    action_id: &str,
) -> anyhow::Result<Option<GrowthActionRow>> {
    Ok(sqlx::query_as::<_, GrowthActionRow>(
        "SELECT * FROM growth_actions WHERE project_id = ?1 AND id = ?2",
    )
    .bind(project_id)
    .bind(action_id)
    .fetch_optional(pool)
    .await?)
}

pub async fn list_for_project(
    pool: &Pool<Sqlite>,
    project_id: &str,
) -> anyhow::Result<Vec<GrowthActionRow>> {
    Ok(sqlx::query_as::<_, GrowthActionRow>(
        "SELECT * FROM growth_actions WHERE project_id = ?1 ORDER BY id DESC",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?)
}

/// Move an action's lifecycle state, optionally pre-registering what it claims
/// it will move.
///
/// `target_metric`/`target_dir` are typed rather than strings so an unmeasurable
/// target cannot reach the row: pre-registration that accepts anything is not
/// pre-registration.
pub async fn set_status(
    pool: &Pool<Sqlite>,
    project_id: &str,
    action_id: &str,
    status: &str,
    target: Option<(TargetMetric, TargetDir)>,
) -> anyhow::Result<Option<GrowthActionRow>> {
    if !STATUSES.contains(&status) {
        anyhow::bail!("unknown growth action status \"{status}\"");
    }
    // coalesce, not overwrite: re-marking an action done must not silently drop
    // a pre-registration made earlier.
    sqlx::query(
        "UPDATE growth_actions
            SET status        = ?3,
                target_metric = coalesce(?4, target_metric),
                target_dir    = coalesce(?5, target_dir)
          WHERE project_id = ?1 AND id = ?2",
    )
    .bind(project_id)
    .bind(action_id)
    .bind(status)
    .bind(target.map(|(m, _)| m.as_str()))
    .bind(target.map(|(_, d)| d.as_str()))
    .execute(pool)
    .await?;
    get(pool, project_id, action_id).await
}

/// Record that a change was confirmed to have landed, how, and what the metric
/// looked like at that moment.
pub async fn record_verification(
    pool: &Pool<Sqlite>,
    project_id: &str,
    action_id: &str,
    verified_by: &str,
    verified_at: &str,
    baseline_json: Option<&str>,
) -> anyhow::Result<Option<GrowthActionRow>> {
    sqlx::query(
        "UPDATE growth_actions
            SET status        = 'verified',
                verified_by   = ?3,
                verified_at   = ?4,
                baseline_json = coalesce(?5, baseline_json)
          WHERE project_id = ?1 AND id = ?2",
    )
    .bind(project_id)
    .bind(action_id)
    .bind(verified_by)
    .bind(verified_at)
    .bind(baseline_json)
    .execute(pool)
    .await?;
    get(pool, project_id, action_id).await
}

/// Other actions on the same project whose verification lands inside
/// `[start, end)` (dates, `YYYY-MM-DD`).
///
/// This is the confounding check. Two changes shipped into the same window
/// cannot be told apart at this traffic, and the proposal's position is that
/// naming both is the correct answer rather than crediting the one being
/// measured.
pub async fn verified_between(
    pool: &Pool<Sqlite>,
    project_id: &str,
    start: &str,
    end: &str,
    exclude_action_id: &str,
) -> anyhow::Result<Vec<Confounder>> {
    Ok(sqlx::query_as::<_, (String, String)>(
        "SELECT id, title FROM growth_actions
          WHERE project_id = ?1 AND id <> ?2 AND verified_at IS NOT NULL
            AND date(verified_at) >= ?3 AND date(verified_at) < ?4
          ORDER BY verified_at",
    )
    .bind(project_id)
    .bind(exclude_action_id)
    .bind(start)
    .bind(end)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|(id, title)| Confounder { id, title })
    .collect())
}

/// Write (or re-write) the verdict for one window.
///
/// `INSERT OR REPLACE` against `PRIMARY KEY(action_id, window_days)`: the
/// nightly sweep re-evaluates open windows, and a re-judge must overwrite that
/// window's verdict rather than append a second one.
#[allow(clippy::too_many_arguments)]
pub async fn upsert_outcome(
    pool: &Pool<Sqlite>,
    action_id: &str,
    window_days: u32,
    before: &MetricWindow,
    after: &MetricWindow,
    judgement: &Judgement,
    judged_at: &str,
) -> anyhow::Result<()> {
    let confounders = if judgement.confounders.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&judgement.confounders)?)
    };
    sqlx::query(
        "INSERT OR REPLACE INTO growth_action_outcomes
            (action_id, window_days, before_json, after_json, delta_pct,
             verdict, rationale, confounders, judged_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )
    .bind(action_id)
    .bind(i64::from(window_days))
    .bind(serde_json::to_string(before)?)
    .bind(serde_json::to_string(after)?)
    .bind(judgement.delta_pct)
    .bind(judgement.verdict.as_str())
    .bind(&judgement.rationale)
    .bind(confounders)
    .bind(judged_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn outcomes_for(pool: &Pool<Sqlite>, action_id: &str) -> anyhow::Result<Vec<OutcomeRow>> {
    Ok(sqlx::query_as::<_, OutcomeRow>(
        "SELECT * FROM growth_action_outcomes WHERE action_id = ?1 ORDER BY window_days",
    )
    .bind(action_id)
    .fetch_all(pool)
    .await?)
}

/// Every verified action still owed a verdict on at least one window.
///
/// Deliberately excludes unverified actions: the proposal's open decision 3 —
/// "without knowing the change landed, a delta is unattributable to anything".
pub async fn pending_measurement(pool: &Pool<Sqlite>) -> anyhow::Result<Vec<GrowthActionRow>> {
    Ok(sqlx::query_as::<_, GrowthActionRow>(
        "SELECT * FROM growth_actions
          WHERE status IN ('verified', 'measuring')
            AND verified_at IS NOT NULL
            AND target_metric IS NOT NULL
          ORDER BY verified_at",
    )
    .fetch_all(pool)
    .await?)
}

async fn get_any(pool: &Pool<Sqlite>, action_id: &str) -> anyhow::Result<Option<GrowthActionRow>> {
    Ok(
        sqlx::query_as::<_, GrowthActionRow>("SELECT * FROM growth_actions WHERE id = ?1")
            .bind(action_id)
            .fetch_optional(pool)
            .await?,
    )
}

/// Confirmed outcomes for a project, longest window first, for the generation
/// prompt.
///
/// Filters to what the proposal allows to feed learning: `inconclusive` and
/// `confounded` teach nothing and must not shift future ranking.
pub async fn learnable_outcomes(
    pool: &Pool<Sqlite>,
    project_id: &str,
    limit: u32,
) -> anyhow::Result<Vec<(GrowthActionRow, OutcomeRow)>> {
    // Two round trips rather than one join: sqlx derives `FromRow` per struct
    // and cannot split a joined row back into two of them, and `a.*, o.*` would
    // collide on nothing here but silently would on any future shared column.
    let outcomes = sqlx::query_as::<_, OutcomeRow>(
        "SELECT o.* FROM growth_action_outcomes o
           JOIN growth_actions a ON a.id = o.action_id
          WHERE a.project_id = ?1 AND o.verdict IN ('helped', 'hindered', 'no_effect')
          ORDER BY o.window_days DESC, o.judged_at DESC
          LIMIT ?2",
    )
    .bind(project_id)
    .bind(i64::from(limit))
    .fetch_all(pool)
    .await?;

    // ONE row per action, not per window.
    //
    // Each action produces three outcomes (7/14/28 day windows) measuring the
    // SAME change. Passing all three to the learning prompt reported "3
    // measured outcomes" for a single tried strategy — a 3x inflated sample
    // size (adversarial review, 2026-08-14). The whole reason the prompt
    // carries a count is so the model does not turn "worked once" into "works";
    // tripling it defeats that precisely.
    //
    // The longest window wins because it is the most settled: the ORDER BY
    // above is window_days DESC, so the first row seen for an action is its
    // longest, and later (shorter, noisier) windows are dropped.
    let mut out = Vec::with_capacity(outcomes.len());
    let mut seen_actions = std::collections::HashSet::new();
    for outcome in outcomes {
        if !seen_actions.insert(outcome.action_id.clone()) {
            continue;
        }
        if let Some(action) = get_any(pool, &outcome.action_id).await? {
            out.push((action, outcome));
        }
    }
    Ok(out)
}

/// Render the learnable outcomes as the compact record the proposal specifies
/// for injection into the generation prompt.
///
/// Carries the sample size on purpose: "Worked once" is not "works", and a
/// model shown one result without its count will over-generalise from it.
pub fn render_learning(rows: &[(GrowthActionRow, OutcomeRow)]) -> Option<String> {
    if rows.is_empty() {
        return None;
    }
    // "actions" not "outcomes": one action measured over three windows is one
    // thing tried, and the wording must not imply three independent results.
    let mut out = format!(
        "Previously tried on this project ({} measured action{}):\n",
        rows.len(),
        if rows.len() == 1 { "" } else { "s" }
    );
    for (action, outcome) in rows {
        let verdict = match outcome.verdict.as_str() {
            "helped" => "helped",
            "hindered" => "hindered",
            _ => "no detectable effect",
        };
        let delta = match outcome.delta_pct {
            Some(d) => format!(
                ", {}{:.0}%",
                if d >= 0.0 { "+" } else { "-" },
                d.abs() * 100.0
            ),
            None => String::new(),
        };
        out.push_str(&format!(
            "- \"{}\" ({}) -> {}{} over {}d, verified by {}\n",
            action.title,
            action.category.as_deref().unwrap_or("uncategorised"),
            verdict,
            delta,
            outcome.window_days,
            action.verified_by.as_deref().unwrap_or("unknown"),
        ));
    }
    out.push_str(
        "Treat these as weak evidence: they are one project's before/after readings, not \
         experiments. Do not suppress a whole category over one result.\n",
    );
    Some(out)
}

// NOT IMPLEMENTED HERE: the proposal's "Grading itself" section (calibration of
// the generator's own `impact`/`confidence` labels). Those are per-action
// predictions and the v42 table has no column for either — the proposal's own
// "Schema" block omits them — so scoring them would mean reading the
// regenerate-overwritten metadata cache (growth_actions.rs:831) and calling the
// result a grade. It needs a column first.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::spectral_schema::apply_growth_actions_schema;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        apply_growth_actions_schema(&pool).await.unwrap();
        pool
    }

    fn seed(title: &str, recommendation: &str) -> ActionSeed {
        ActionSeed {
            title: title.into(),
            recommendation: recommendation.into(),
            category: Some("seo".into()),
            artifact_kind: Some("prompt".into()),
            artifact: Some("do the thing".into()),
        }
    }

    /// The model rewords whitespace and casing between calls and the parser
    /// truncates, so the same advice must hash the same or every refresh mints
    /// a duplicate card and orphans yesterday's outcomes.
    #[test]
    fn the_fingerprint_survives_rewording_that_is_not_a_reword() {
        let a = fingerprint("p1", "Add FAQ schema", "Add a FAQPage block");
        let b = fingerprint("p1", "  add faq   schema ", "Add  a FAQPage\nblock");
        assert_eq!(a, b);
    }

    #[test]
    fn the_fingerprint_separates_projects_and_distinct_advice() {
        let base = fingerprint("p1", "Add FAQ schema", "Add a FAQPage block");
        assert_ne!(
            base,
            fingerprint("p2", "Add FAQ schema", "Add a FAQPage block")
        );
        assert_ne!(base, fingerprint("p1", "Add FAQ schema", "Something else"));
        // The separator must actually separate: without it these two collide.
        assert_ne!(fingerprint("p", "ab", "c"), fingerprint("p", "a", "bc"));
    }

    #[tokio::test]
    async fn regenerating_the_same_advice_resolves_to_one_row() {
        let pool = pool().await;
        let first = upsert_suggested(&pool, "p1", &seed("Add FAQ schema", "Add a FAQPage block"))
            .await
            .unwrap();
        let again = upsert_suggested(
            &pool,
            "p1",
            &seed("add faq schema", "Add  a FAQPage   block"),
        )
        .await
        .unwrap();
        assert_eq!(first.id, again.id, "same advice must keep its identity");
        assert_eq!(list_for_project(&pool, "p1").await.unwrap().len(), 1);
        // The refreshed text wins while the action is still merely suggested.
        assert_eq!(again.title, "add faq schema");
    }

    /// Once a metric is pre-registered against an action's text, regeneration
    /// must not rewrite the claim the baseline refers to.
    #[tokio::test]
    async fn regeneration_cannot_reword_an_action_already_underway() {
        let pool = pool().await;
        let row = upsert_suggested(&pool, "p1", &seed("Add FAQ schema", "Add a FAQPage block"))
            .await
            .unwrap();
        set_status(
            &pool,
            "p1",
            &row.id,
            STATUS_DONE,
            Some((TargetMetric::Sessions, TargetDir::Up)),
        )
        .await
        .unwrap();

        // Same fingerprint (whitespace only), different visible text.
        let after = upsert_suggested(
            &pool,
            "p1",
            &ActionSeed {
                title: "ADD FAQ  SCHEMA".into(),
                recommendation: "Add a FAQPage block".into(),
                category: Some("ux".into()),
                artifact_kind: None,
                artifact: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(after.id, row.id);
        assert_eq!(after.title, "Add FAQ schema", "text was rewritten underway");
        assert_eq!(after.category.as_deref(), Some("seo"));
        assert_eq!(after.status, STATUS_DONE);
        assert_eq!(after.target_metric.as_deref(), Some("sessions"));
    }

    #[tokio::test]
    async fn a_pre_registration_is_not_dropped_by_a_later_status_move() {
        let pool = pool().await;
        let row = upsert_suggested(&pool, "p1", &seed("t", "r"))
            .await
            .unwrap();
        set_status(
            &pool,
            "p1",
            &row.id,
            STATUS_DONE,
            Some((TargetMetric::BounceRate, TargetDir::Down)),
        )
        .await
        .unwrap();
        let moved = set_status(&pool, "p1", &row.id, STATUS_MEASURING, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(moved.target_metric.as_deref(), Some("bounce_rate"));
        assert_eq!(moved.target_dir.as_deref(), Some("down"));
    }

    #[tokio::test]
    async fn an_action_is_not_addressable_through_another_project() {
        let pool = pool().await;
        let row = upsert_suggested(&pool, "p1", &seed("t", "r"))
            .await
            .unwrap();
        assert!(get(&pool, "p2", &row.id).await.unwrap().is_none());
        assert!(set_status(&pool, "p2", &row.id, STATUS_DISMISSED, None)
            .await
            .unwrap()
            .is_none());
        assert_eq!(
            get(&pool, "p1", &row.id).await.unwrap().unwrap().status,
            STATUS_SUGGESTED
        );
    }

    #[tokio::test]
    async fn an_unknown_status_is_refused_rather_than_stored() {
        let pool = pool().await;
        let row = upsert_suggested(&pool, "p1", &seed("t", "r"))
            .await
            .unwrap();
        assert!(set_status(&pool, "p1", &row.id, "shipped", None)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn only_other_actions_verified_inside_the_window_confound_it() {
        let pool = pool().await;
        let mine = upsert_suggested(&pool, "p1", &seed("mine", "r1"))
            .await
            .unwrap();
        let inside = upsert_suggested(&pool, "p1", &seed("inside", "r2"))
            .await
            .unwrap();
        let outside = upsert_suggested(&pool, "p1", &seed("outside", "r3"))
            .await
            .unwrap();
        let elsewhere = upsert_suggested(&pool, "p2", &seed("elsewhere", "r4"))
            .await
            .unwrap();

        for (row, project, at) in [
            (&mine, "p1", "2026-08-12T09:00:00Z"),
            (&inside, "p1", "2026-08-15T09:00:00Z"),
            (&outside, "p1", "2026-08-30T09:00:00Z"),
            (&elsewhere, "p2", "2026-08-15T09:00:00Z"),
        ] {
            record_verification(&pool, project, &row.id, VERIFIED_BY_GIT, at, None)
                .await
                .unwrap();
        }

        let found = verified_between(&pool, "p1", "2026-08-12", "2026-08-19", &mine.id)
            .await
            .unwrap();
        assert_eq!(
            found.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(),
            vec!["inside"],
            "the window must not pick up itself, later work, or another project"
        );
    }

    #[tokio::test]
    async fn measurement_only_ever_considers_verified_pre_registered_actions() {
        let pool = pool().await;
        let unverified = upsert_suggested(&pool, "p1", &seed("a", "r1"))
            .await
            .unwrap();
        set_status(
            &pool,
            "p1",
            &unverified.id,
            STATUS_DONE,
            Some((TargetMetric::Sessions, TargetDir::Up)),
        )
        .await
        .unwrap();

        let unregistered = upsert_suggested(&pool, "p1", &seed("b", "r2"))
            .await
            .unwrap();
        record_verification(
            &pool,
            "p1",
            &unregistered.id,
            VERIFIED_BY_SELF,
            "2026-08-12T00:00:00Z",
            None,
        )
        .await
        .unwrap();

        let ready = upsert_suggested(&pool, "p1", &seed("c", "r3"))
            .await
            .unwrap();
        set_status(
            &pool,
            "p1",
            &ready.id,
            STATUS_DONE,
            Some((TargetMetric::Pageviews, TargetDir::Up)),
        )
        .await
        .unwrap();
        record_verification(
            &pool,
            "p1",
            &ready.id,
            VERIFIED_BY_GIT,
            "2026-08-12T00:00:00Z",
            None,
        )
        .await
        .unwrap();

        let pending = pending_measurement(&pool).await.unwrap();
        assert_eq!(
            pending.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(),
            vec!["c"]
        );
    }

    #[tokio::test]
    async fn only_confirmed_outcomes_reach_the_generation_prompt() {
        let pool = pool().await;
        let helped = upsert_suggested(&pool, "p1", &seed("Add FAQ schema", "r1"))
            .await
            .unwrap();
        let noisy = upsert_suggested(&pool, "p1", &seed("Rewrite hero copy", "r2"))
            .await
            .unwrap();
        for row in [&helped, &noisy] {
            record_verification(
                &pool,
                "p1",
                &row.id,
                VERIFIED_BY_GIT,
                "2026-08-12T00:00:00Z",
                None,
            )
            .await
            .unwrap();
        }
        write_outcome(&pool, &helped.id, 28, "helped", Some(0.34)).await;
        write_outcome(&pool, &noisy.id, 28, "inconclusive", Some(0.05)).await;

        let rows = learnable_outcomes(&pool, "p1", 10).await.unwrap();
        assert_eq!(rows.len(), 1);
        let rendered = render_learning(&rows).unwrap();
        assert!(rendered.contains("Add FAQ schema"), "{rendered}");
        assert!(!rendered.contains("Rewrite hero copy"), "{rendered}");
        assert!(rendered.contains("+34%"), "{rendered}");
        assert!(rendered.contains("1 measured action"), "{rendered}");
        assert!(rendered.contains("weak evidence"), "{rendered}");
        assert!(render_learning(&[]).is_none());
    }

    /// Adversarial review, 2026-08-14: one action produces three outcome rows
    /// (7/14/28 day windows) measuring the SAME change, and all three reached
    /// the learning prompt — reporting "3 measured outcomes" for one strategy
    /// tried once. The count exists so the model does not turn "worked once"
    /// into "works"; tripling it defeats exactly that.
    #[tokio::test]
    async fn three_windows_of_one_action_count_as_one_tried_strategy() {
        let pool = pool().await;
        let row = upsert_suggested(&pool, "p1", &seed("Add FAQ schema", "r1"))
            .await
            .unwrap();
        record_verification(
            &pool,
            "p1",
            &row.id,
            VERIFIED_BY_GIT,
            "2026-08-12T00:00:00Z",
            None,
        )
        .await
        .unwrap();

        // The real sweep writes all three windows for a single action.
        for days in [7, 14, 28] {
            write_outcome(&pool, &row.id, days, "helped", Some(0.34)).await;
        }

        let rows = learnable_outcomes(&pool, "p1", 10).await.unwrap();
        assert_eq!(rows.len(), 1, "one action is one measured strategy");
        assert_eq!(
            rows[0].1.window_days, 28,
            "the longest window is the settled one and must be the survivor"
        );
        let rendered = render_learning(&rows).unwrap();
        assert!(rendered.contains("1 measured action"), "{rendered}");
        assert!(!rendered.contains("3 measured"), "{rendered}");
    }

    async fn write_outcome(
        pool: &Pool<Sqlite>,
        action_id: &str,
        window_days: i64,
        verdict: &str,
        delta_pct: Option<f64>,
    ) {
        sqlx::query(
            "INSERT OR REPLACE INTO growth_action_outcomes
                (action_id, window_days, before_json, after_json, delta_pct, verdict,
                 rationale, confounders, judged_at)
             VALUES (?1, ?2, '{}', '{}', ?3, ?4, 'fixture rationale', NULL,
                     '2026-09-10T00:00:00Z')",
        )
        .bind(action_id)
        .bind(window_days)
        .bind(delta_pct)
        .bind(verdict)
        .execute(pool)
        .await
        .unwrap();
    }
}
