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
use std::collections::BTreeSet;

/// Lifecycle. `suggested` on first persist; the rest are user or job driven.
pub const STATUS_SUGGESTED: &str = "suggested";
pub const STATUS_DISMISSED: &str = "dismissed";
pub const STATUS_DONE: &str = "done";
pub const STATUS_VERIFIED: &str = "verified";
pub const STATUS_MEASURING: &str = "measuring";
pub const STATUS_JUDGED: &str = "judged";
/// The user's shelf, not deletion. An archived action leaves the active board,
/// keeps feeding learning (`learnable_outcomes` never filtered on status), and
/// is still measured while it owes a window (`pending_measurement` below) — and
/// it is the ONLY thing that releases an action's text for re-proposal, because
/// [`board`] is both what the generator is shown and what a new suggestion is
/// checked against. That coupling is why archiving is load-bearing rather than
/// cosmetic: without it a project's early advice would block that advice
/// forever.
///
/// No DDL is needed for it: `growth_actions.status` is `TEXT NOT NULL` with no
/// CHECK constraint (spectral_schema.rs), so the allowlist below is the only
/// gate there has ever been.
pub const STATUS_ARCHIVED: &str = "archived";

pub const STATUSES: &[&str] = &[
    STATUS_SUGGESTED,
    STATUS_DISMISSED,
    STATUS_DONE,
    STATUS_VERIFIED,
    STATUS_MEASURING,
    STATUS_JUDGED,
    STATUS_ARCHIVED,
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
    /// What the agent expects this action to move, and which way — its
    /// PREDICTION, recorded when the action is suggested.
    ///
    /// This used to be absent, so the only place a target could be set was the
    /// verify form, which asked the USER "what should move, and which way?".
    /// That inverted the loop. The agent is the one recommending the strategy,
    /// so the agent is the one making a claim about what it will do; a user
    /// filling that in is answering a question they came here to be advised on,
    /// and there is no prediction left to grade the agent against.
    ///
    /// Pre-registration is preserved and still enforced — the target must exist
    /// before any measurement is taken (`growth_actions.rs` refuses a verify
    /// without one) so a metric cannot be chosen once the result is visible.
    /// The change is only WHO writes it, and when: the agent, at suggestion
    /// time, before anything has happened at all.
    ///
    /// `None` stays legal. Actions suggested before this field existed have no
    /// prediction, and an agent that genuinely cannot predict an effect should
    /// say so rather than invent one — both fall back to asking the user.
    pub target_metric: Option<String>,
    pub target_dir: Option<String>,
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

/// The content words of an action, for the near-duplicate check below.
///
/// Lowercased, split on anything that is not an ASCII alphanumeric (so
/// `/blog/canadian-grocery-stores` contributes `blog`, `canadian`, `grocery`
/// and `stores` rather than one unmatchable token), with tokens under three
/// characters and a short grammatical stopword list removed. The stopwords are
/// function words only — nothing domain-specific — because dropping a domain
/// word would be dropping the thing that distinguishes two actions.
fn content_tokens(title: &str, recommendation: &str) -> BTreeSet<String> {
    const STOPWORDS: &[&str] = &[
        "the", "and", "for", "with", "your", "its", "this", "that", "are", "from",
    ];
    let mut out = BTreeSet::new();
    for word in format!("{title} {recommendation}").split(|c: char| !c.is_ascii_alphanumeric()) {
        let word = word.to_ascii_lowercase();
        if word.len() < 3 || STOPWORDS.contains(&word.as_str()) {
            continue;
        }
        out.insert(word);
    }
    out
}

/// How much of two token sets is the same, symmetrically: the Dice
/// coefficient, `2|a∩b| / (|a|+|b|)`. `0.0` when either side is empty.
///
/// The design called for `|a∩b| / min(|a|,|b|)` — the overlap coefficient — on
/// the reasoning that a long recommendation restated tersely is still a
/// restatement. It cannot be used: with `min` in the denominator, a SHORT new
/// action scores against a long board row on shared vocabulary alone. The
/// design's own counter-example proves it — "Add an FAQ section to the pricing
/// page" / "Add FAQPage schema to the grocery post" shares six tokens with the
/// grocery-post row out of its own nine, which is 0.67 and would have been
/// dropped as a restatement of advice it has nothing to do with. Scoring both
/// sizes puts that pair at 0.55 and the genuine reword at 0.69, which is a gap
/// a threshold can sit in.
fn overlap(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let shared = a.intersection(b).count() as f64;
    2.0 * shared / (a.len() + b.len()) as f64
}

/// How much of two actions' full text must match before one is a restatement of
/// the other.
///
/// Calibrated against real rows, not invented ones. The closest thing to a
/// false positive anywhere in the corpus below is "Add an FAQ section to the
/// pricing page" / "Add FAQPage schema to the grocery post" scored against the
/// grocery-post row, at 0.545. The genuine reword scores 0.692. This sits in
/// that gap, nearer the negative, because a false positive here withholds
/// advice silently and a false negative only lets a duplicate through to a
/// guard the user can see.
pub const RESTATEMENT_OVERLAP: f64 = 0.6;

/// How much of two actions' TITLES must match, when the recommendation was
/// rewritten at length but the headline was not.
///
/// This was 0.8, which caught nothing real. Scored against the seven rows the
/// 2026-08-19 incident actually left in the database, the reworded funnel pair
/// — "Instrument missing conversion funnel events to understand why 95%
/// bounce…" and "…to diagnose the 99% bounce…", the same advice twice — scores
/// 0.449 on full text and 0.600 on titles. Both were under the old floors, so
/// the guard was a no-op on the only duplication it was ever built for.
///
/// 0.55 is where the real corpus separates: every genuine restatement in it
/// scores 0.600 on titles, and the highest-scoring non-restatement is 0.444
/// ("Expand the coupon-codes post" vs "Expand the Canadian grocery stores
/// post"). The next-highest pair of unrelated live rows is 0.261. Moving this
/// floor without re-running it against those rows is how the guard goes quiet
/// again.
pub const RESTATEMENT_TITLE_OVERLAP: f64 = 0.55;

/// The board row this new suggestion merely restates, if any.
///
/// The exact-text [`fingerprint`] above only survives whitespace and casing. On
/// 2026-08-19 the generator produced three actions for one project that were
/// genuine rewords of three it had produced on 2026-08-14 — different wording,
/// different fingerprints, the same advice — so every one of them minted a new
/// row and pushed the in-flight originals down the panel. Nothing in the
/// pipeline noticed, because nothing in the pipeline had ever been shown the
/// open board.
///
/// Token overlap is the mechanism because it is deterministic, needs no
/// provider, runs offline in a unit test, and can be reasoned about from the
/// code. The two floors (four content tokens combined, three in a title) stop a
/// two-word action tripping on a single shared word, where any ratio is
/// meaningless.
///
/// What this catches, measured against those three real pairs and NOT against
/// invented ones: the funnel pair, which is a reword close enough to score
/// 0.600 on titles. What it does not catch: "Rewrite the homepage to drive
/// search and direct traffic into event discovery" against "Rewrite the
/// homepage (/) to reduce 13-pageview entry bounce and funnel users to category
/// or search" — the same advice in almost disjoint words, scoring 0.173 on full
/// text and 0.261 on titles. Nothing lexical can reach that without also
/// dropping the coupon-codes negative at 0.444, so this function is deliberately
/// NOT the mechanism for semantic rewords. The board in the generation prompt
/// is: the model is now shown every open action and told not to restate one.
/// This is the deterministic backstop under it, and calling it the fix would be
/// claiming a guarantee it cannot give.
///
/// A false positive here silently withholds advice, so the caller counts the
/// drops and names both titles in the log rather than dropping them quietly.
pub fn restates<'a>(
    title: &str,
    recommendation: &str,
    board: &'a [GrowthActionRow],
) -> Option<&'a GrowthActionRow> {
    let new_all = content_tokens(title, recommendation);
    let new_title = content_tokens(title, "");
    board.iter().find(|row| {
        let row_all = content_tokens(&row.title, &row.recommendation);
        let row_title = content_tokens(&row.title, "");
        let combined = new_all.len().min(row_all.len()) >= 4
            && overlap(&new_all, &row_all) >= RESTATEMENT_OVERLAP;
        let headline = new_title.len().min(row_title.len()) >= 3
            && overlap(&new_title, &row_title) >= RESTATEMENT_TITLE_OVERLAP;
        combined || headline
    })
}

/// Persist a generated action, or refresh the text of one already there.
///
/// The `WHERE status = 'suggested'` guard on the update is load-bearing: once a
/// user has marked an action done and pre-registered a metric against its text,
/// regeneration must not rewrite the claim the baseline refers to. Without it,
/// "Review again" would silently re-word the hypothesis after the experiment
/// started.
///
/// An archived row that was never measured is the one other case the update may
/// touch, and it is the case that makes the archive mean what this module says
/// it means. `UNIQUE(project_id, fingerprint)` means re-proposed text can never
/// become a second row, so with `suggested` as the only updatable status the
/// insert was absorbed, the update was refused, and `get_by_fingerprint`
/// handed back the still-archived row: the caller recorded a success, the card
/// never reached the panel, and no counter or log mentioned it. Archiving
/// claimed to release the text for re-proposal and instead swallowed it.
///
/// `verified_at IS NULL` is the limit on the resurrection. An archived action
/// that was measured owns outcome rows and a frozen baseline pivot; flipping it
/// back to `suggested` would put a completed experiment back on the board with
/// its verdict still attached. Those stay archived, and [`get_by_fingerprint`]
/// returning an archived row is the caller's signal to say so out loud.
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
             artifact_kind, artifact, target_metric, target_dir, status, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'suggested', ?11)
         ON CONFLICT(project_id, fingerprint) DO UPDATE SET
            title          = excluded.title,
            recommendation = excluded.recommendation,
            category       = excluded.category,
            artifact_kind  = excluded.artifact_kind,
            artifact       = excluded.artifact,
            -- Re-suggesting refreshes the prediction, but only while the action
            -- is still `suggested`: the WHERE below already blocks any update
            -- once it is verified, so a target can never be rewritten after
            -- measurement has begun. `coalesce` keeps an existing prediction
            -- when a later suggestion omits one, rather than silently clearing
            -- the pre-registered target and reopening the verify form.
            target_metric  = coalesce(excluded.target_metric, growth_actions.target_metric),
            target_dir     = coalesce(excluded.target_dir, growth_actions.target_dir),
            -- Re-proposing a never-measured archived action puts it back on the
            -- board, which is what archiving promised. created_at moves with it
            -- because it is the window the verify strategies look for work in --
            -- commits since the action was issued -- and leaving the original
            -- date would credit the new suggestion with old commits.
            status         = CASE WHEN growth_actions.status = 'archived'
                                  THEN 'suggested' ELSE growth_actions.status END,
            created_at     = CASE WHEN growth_actions.status = 'archived'
                                  THEN excluded.created_at ELSE growth_actions.created_at END
         WHERE growth_actions.status = 'suggested'
            OR (growth_actions.status = 'archived'
                AND growth_actions.verified_at IS NULL)",
    )
    .bind(&id)
    .bind(project_id)
    .bind(&fp)
    .bind(&seed.title)
    .bind(&seed.recommendation)
    .bind(seed.category.as_deref())
    .bind(seed.artifact_kind.as_deref())
    .bind(seed.artifact.as_deref())
    .bind(seed.target_metric.as_deref())
    .bind(seed.target_dir.as_deref())
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

/// Everything on this project's board — every action that has not been
/// archived, newest first.
///
/// One function serves two callers on purpose: this is what the generator is
/// SHOWN, and it is the set a freshly proposed action is CHECKED against
/// ([`restates`]). If they came from two queries they could disagree, and the
/// disagreement would look exactly like the bug this exists to fix — a model
/// told about an action it is then allowed to duplicate.
///
/// This returns the WHOLE board, unwindowed. It used to be `LIMIT 20`, which
/// silently broke the guarantee above: the prompt and the check read the same
/// rows, so capping the query capped the CHECK too, and on a project with more
/// than twenty open actions the oldest work became invisible to `restates` —
/// re-opening the exact duplication this exists to close, on precisely the
/// long-lived projects that have accumulated the most to duplicate.
///
/// Windowing belongs on the prompt alone, where it is safe: showing the model
/// fewer rows than the guard checks can only cost a retry, while checking fewer
/// rows than the model was shown mints a duplicate. See [`BOARD_PROMPT_ROWS`].
pub async fn board(pool: &Pool<Sqlite>, project_id: &str) -> anyhow::Result<Vec<GrowthActionRow>> {
    Ok(sqlx::query_as::<_, GrowthActionRow>(
        "SELECT * FROM growth_actions
          WHERE project_id = ?1 AND status <> 'archived'
          ORDER BY id DESC",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?)
}

/// How many board rows the generation prompt is shown, newest first.
///
/// A budget on the prompt, not on the guard: [`restates`] is checked against
/// every row [`board`] returns regardless of this number.
pub const BOARD_PROMPT_ROWS: usize = 20;

/// The board as the generation prompt sees it. `None` when the board is empty,
/// so the caller adds no block at all rather than an empty heading.
///
/// Every line carries the action's STATUS, because "suggested, not started" and
/// "verified, being measured" call for different behaviour from the model: the
/// first may genuinely be worth replacing, the second is live work whose
/// measurement a duplicate card would corrupt.
///
/// Only the newest [`BOARD_PROMPT_ROWS`] are rendered, and the count in the
/// heading is the TRUE total so the model is never told the board is smaller
/// than it is. The window lives here rather than in [`board`] because the
/// caller checks restatements against every row, windowed or not.
pub fn render_board(rows: &[GrowthActionRow]) -> Option<String> {
    if rows.is_empty() {
        return None;
    }
    let shown = rows.len().min(BOARD_PROMPT_ROWS);
    let mut out = format!(
        "Already on this project's board ({}). Do NOT restate any of these:\n",
        rows.len()
    );
    for row in &rows[..shown] {
        let phrase = match row.status.as_str() {
            STATUS_SUGGESTED => "suggested, not started",
            STATUS_DONE => "marked done, awaiting verification",
            STATUS_VERIFIED => "verified, being measured",
            STATUS_MEASURING => "being measured",
            STATUS_JUDGED => "measured, verdict recorded",
            STATUS_DISMISSED => "dismissed by the user",
            other => other,
        };
        let predicts = match (row.target_metric.as_deref(), row.target_dir.as_deref()) {
            (Some(metric), Some(dir)) => format!(", predicts {metric} {dir}"),
            _ => String::new(),
        };
        out.push_str(&format!(
            "- \"{}\" ({}) — {phrase}{predicts}\n",
            row.title,
            row.category.as_deref().unwrap_or("uncategorised"),
        ));
    }
    out.push_str("If the only strong moves left are already here, return fewer actions or none.\n");
    Some(out)
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
///
/// Archived actions are included while they still owe a window. Archiving is
/// the user filing a card away, not a stop order: an action archived on day
/// three of its 28-day window would otherwise have its remaining windows
/// silently destroyed, and those windows are the data point the archive exists
/// to keep. The outcome-count clause is what ends it — once all three windows
/// are written there is nothing left to measure, and without the clause every
/// pass would re-read every archived action the project has ever had.
pub async fn pending_measurement(pool: &Pool<Sqlite>) -> anyhow::Result<Vec<GrowthActionRow>> {
    Ok(sqlx::query_as::<_, GrowthActionRow>(
        "SELECT * FROM growth_actions
          WHERE (status IN ('verified', 'measuring')
                 OR (status = 'archived'
                     AND (SELECT count(*) FROM growth_action_outcomes o
                           WHERE o.action_id = growth_actions.id) < ?1))
            AND verified_at IS NOT NULL
            AND target_metric IS NOT NULL
          ORDER BY verified_at",
    )
    .bind(crate::growth::metrics::WINDOW_DAYS.len() as i64)
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
///
/// It has never filtered on the action's STATUS and must not start: an archived
/// action is exactly one the user has finished with, which is the point at
/// which its measurement is most settled. That property is what makes archiving
/// safe — it takes a card off the board without taking its result out of the
/// agent's memory.
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
            target_metric: None,
            target_dir: None,
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
                target_metric: None,
                target_dir: None,
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

    /// The board row the 2026-08-19 duplication was a reword of.
    fn grocery_board_row() -> GrowthActionRow {
        GrowthActionRow {
            id: "a1".into(),
            project_id: "p1".into(),
            fingerprint: "f1".into(),
            title: "Expand the Canadian grocery stores post".into(),
            recommendation: "Expand and interlink the /blog/canadian-grocery-stores post with an \
                             FAQ section and schema.org FAQPage markup"
                .into(),
            category: Some("aeo".into()),
            artifact_kind: Some("prompt".into()),
            artifact: None,
            target_metric: Some("pageviews".into()),
            target_dir: Some("up".into()),
            baseline_json: None,
            status: STATUS_MEASURING.into(),
            verified_by: Some(VERIFIED_BY_GIT.into()),
            verified_at: Some("2026-08-14T00:00:00Z".into()),
            created_at: "2026-08-14T00:00:00Z".into(),
        }
    }

    /// REGRESSION. On 2026-08-19 the generator produced three actions that were
    /// rewords of three it had produced on 2026-08-14 for the same project.
    /// Nothing caught them: their normalised text differs, so `fingerprint`
    /// gives three different hashes and `upsert_suggested` mints three new
    /// rows. Before this function existed there was no second check at all, so
    /// this test could not even be written — that is the failure it pins.
    #[test]
    fn a_reworded_restatement_of_an_open_action_is_recognised() {
        let board = vec![grocery_board_row()];
        let title = "Grow the grocery-stores blog post";
        let recommendation = "Add an FAQ section and FAQPage structured data to \
                              /blog/canadian-grocery-stores and interlink it";
        // The fingerprints differ, which is exactly why the old path minted a row.
        assert_ne!(
            fingerprint("p1", title, recommendation),
            fingerprint("p1", &board[0].title, &board[0].recommendation)
        );
        let hit = restates(title, recommendation, &board).expect("a reword is a restatement");
        assert_eq!(hit.id, "a1");
    }

    /// The three actions the 2026-08-19 incident actually produced, and the
    /// three from 2026-08-14 they were rewords of, copied verbatim out of the
    /// database the incident left behind. These are the fixtures that matter:
    /// every other restatement test in this file uses text someone wrote to
    /// make a point.
    fn incident_08_14() -> Vec<GrowthActionRow> {
        [
            (
                "Instrument missing conversion funnel events to understand why 95% bounce without \
                 action",
                "No conversion events are being recorded, so the funnel cannot be diagnosed. Add \
                 events for search, category browse and event detail views.",
            ),
            (
                "Add structured data (schema.org Event + FAQPage) to event detail pages to enable \
                 answer-engine visibility",
                "Event detail pages carry no structured data, so answer engines cannot read them.",
            ),
            (
                "Rewrite the homepage (/) to reduce 13-pageview entry bounce and funnel users to \
                 category or search",
                "The homepage does not send entering users anywhere. Give it category and search \
                 entry points.",
            ),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (title, recommendation))| {
            let mut row = grocery_board_row();
            row.id = format!("i{i}");
            row.title = title.into();
            row.recommendation = recommendation.into();
            row
        })
        .collect()
    }

    /// REGRESSION, and the one that matters most. The guard shipped calibrated
    /// at `RESTATEMENT_TITLE_OVERLAP = 0.8` and `RESTATEMENT_OVERLAP = 0.6`,
    /// which the synthetic grocery fixture above clears at 0.692 — so the suite
    /// was green while the guard caught NOTHING from the incident it cites as
    /// its reason for existing. This pair scores 0.449 on full text and 0.600
    /// on titles: under the old title floor, over the new one. Restore 0.8 and
    /// this test fails, which is the only thing standing between the guard and
    /// being decorative again.
    #[test]
    fn the_reword_that_caused_the_incident_is_caught() {
        let board = incident_08_14();
        let hit = restates(
            "Instrument missing conversion funnel events to diagnose the 99% bounce and sub-1% \
             CTA engagement",
            "Conversion events are still not recorded, so the 99% bounce cannot be explained. \
             Instrument search, category and detail-view events.",
            &board,
        )
        .expect("the funnel reword restates the 2026-08-14 funnel action");
        assert_eq!(hit.id, "i0");
    }

    /// The honest limit, pinned so nobody claims more for this function than it
    /// delivers. The homepage pair IS the same advice, and lexical overlap
    /// cannot see it: 0.173 on full text, 0.261 on titles, against a genuine
    /// non-restatement ("Expand the coupon-codes post") that scores 0.444. Any
    /// threshold low enough to catch this drops that. The board in the
    /// generation prompt is what addresses semantic rewords; this test exists
    /// so that a future reader finds the boundary written down rather than
    /// assuming the guard covers it.
    #[test]
    fn a_semantic_reword_is_beyond_what_token_overlap_can_see() {
        let board = incident_08_14();
        assert!(
            restates(
                "Rewrite the homepage to drive search and direct traffic into event discovery, \
                 not a single event detail page",
                "Entering users land on one event and leave. Route them into search and category \
                 browse instead.",
                &board,
            )
            .is_none(),
            "if this starts passing, re-check the coupon-codes negative below before celebrating"
        );
    }

    /// The guard is the one change that can silently withhold advice, so it
    /// must not swallow advice that merely shares a vocabulary with the board.
    #[test]
    fn genuinely_different_advice_is_not_a_restatement() {
        let board = vec![grocery_board_row()];
        // Same domain words (FAQ, FAQPage, schema, grocery, post), different page
        // and different change.
        assert!(restates(
            "Add an FAQ section to the pricing page",
            "Add FAQPage schema to the grocery post",
            &board
        )
        .is_none());
        // A different post entirely.
        assert!(restates(
            "Expand the coupon-codes post",
            "Expand the grocery-stores post",
            &board
        )
        .is_none());
        // Two words sharing one: any ratio over sets this small is noise, which
        // is what the token floors exist for.
        assert!(restates("Expand pricing", "", &board).is_none());
    }

    /// Pins the coupling between the open-board guard and the archive: the
    /// archive is the ONLY thing that hands an action's text back.
    #[tokio::test]
    async fn the_archive_is_what_releases_an_actions_text() {
        let pool = pool().await;
        let row = upsert_suggested(
            &pool,
            "p1",
            &seed(
                "Expand the Canadian grocery stores post",
                "Expand and interlink the /blog/canadian-grocery-stores post with an FAQ section \
                 and schema.org FAQPage markup",
            ),
        )
        .await
        .unwrap();

        let title = "Grow the grocery-stores blog post";
        let recommendation = "Add an FAQ section and FAQPage structured data to \
                              /blog/canadian-grocery-stores and interlink it";
        let open = board(&pool, "p1").await.unwrap();
        assert!(restates(title, recommendation, &open).is_some());

        set_status(&pool, "p1", &row.id, STATUS_ARCHIVED, None)
            .await
            .unwrap();
        let after = board(&pool, "p1").await.unwrap();
        assert!(after.is_empty(), "an archived action is off the board");
        assert!(restates(title, recommendation, &after).is_none());
    }

    #[tokio::test]
    async fn the_board_is_every_action_that_is_not_archived() {
        let pool = pool().await;
        for &status in STATUSES {
            let row = upsert_suggested(&pool, "p1", &seed(status, "r"))
                .await
                .unwrap();
            set_status(&pool, "p1", &row.id, status, None)
                .await
                .unwrap();
        }
        let titles: Vec<String> = board(&pool, "p1")
            .await
            .unwrap()
            .into_iter()
            .map(|r| r.title)
            .collect();
        assert!(!titles.contains(&STATUS_ARCHIVED.to_string()));
        for status in [
            STATUS_SUGGESTED,
            STATUS_DISMISSED,
            STATUS_DONE,
            STATUS_VERIFIED,
            STATUS_MEASURING,
            STATUS_JUDGED,
        ] {
            assert!(titles.contains(&status.to_string()), "{status} missing");
        }
        // Newest first: uuid v7 ids are time-ordered, so the last seeded row
        // that is still on the board leads.
        assert_eq!(titles.first().map(String::as_str), Some(STATUS_JUDGED));
    }

    /// REGRESSION. `board` was `LIMIT 20`, and `board` is BOTH what the prompt
    /// is shown and what `restates` is checked against. On a project with more
    /// than twenty open actions the oldest fell out of the check as well as out
    /// of the prompt, so the duplication guard went blind on exactly the
    /// long-lived projects with the most history to duplicate. The check now
    /// sees everything; only the prompt text is windowed.
    #[tokio::test]
    async fn the_guard_sees_the_whole_board_even_when_the_prompt_cannot() {
        let pool = pool().await;
        for i in 0..(BOARD_PROMPT_ROWS + 5) {
            upsert_suggested(&pool, "p1", &seed(&format!("action number {i}"), "r"))
                .await
                .unwrap();
        }
        let rows = board(&pool, "p1").await.unwrap();
        assert_eq!(
            rows.len(),
            BOARD_PROMPT_ROWS + 5,
            "the restatement check must see every open action"
        );

        let text = render_board(&rows).unwrap();
        assert_eq!(
            text.lines().filter(|l| l.starts_with("- ")).count(),
            BOARD_PROMPT_ROWS,
            "the prompt stays windowed"
        );
        // The heading reports the true total, not the windowed one, so the model
        // is never told the board is smaller than it is.
        assert!(
            text.contains(&format!("({})", BOARD_PROMPT_ROWS + 5)),
            "{text}"
        );
        // Newest first: the oldest action is the one the prompt drops.
        assert!(!text.contains("\"action number 0\""), "{text}");
    }

    /// REGRESSION. `UNIQUE(project_id, fingerprint)` means re-proposed text
    /// cannot become a second row, and the upsert would only update a
    /// `suggested` one — so re-proposing an archived action inserted nothing,
    /// updated nothing, and returned the archived row. The caller counted a
    /// success and the card never appeared. This module's own docs promise
    /// archiving is what releases the text for re-proposal; before this it was
    /// what buried it.
    #[tokio::test]
    async fn re_proposing_an_archived_action_puts_it_back_on_the_board() {
        let pool = pool().await;
        let row = upsert_suggested(&pool, "p1", &seed("Add FAQ schema", "r1"))
            .await
            .unwrap();
        set_status(&pool, "p1", &row.id, STATUS_DISMISSED, None)
            .await
            .unwrap();
        set_status(&pool, "p1", &row.id, STATUS_ARCHIVED, None)
            .await
            .unwrap();
        assert!(board(&pool, "p1").await.unwrap().is_empty());

        let again = upsert_suggested(&pool, "p1", &seed("Add FAQ schema", "r1"))
            .await
            .unwrap();
        assert_eq!(
            again.id, row.id,
            "the fingerprint is unique, so it is one row"
        );
        assert_eq!(again.status, STATUS_SUGGESTED);
        assert_eq!(board(&pool, "p1").await.unwrap().len(), 1);
    }

    /// The limit on that resurrection. A measured action owns outcome rows and
    /// a frozen baseline pivot, so putting it back on the board would re-open a
    /// finished experiment with its verdict attached. It stays archived, and
    /// the caller is left able to see that it did.
    #[tokio::test]
    async fn re_proposing_a_measured_archived_action_leaves_it_archived() {
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
        set_status(&pool, "p1", &row.id, STATUS_ARCHIVED, None)
            .await
            .unwrap();

        let again = upsert_suggested(&pool, "p1", &seed("Add FAQ schema", "r1"))
            .await
            .unwrap();
        assert_eq!(again.status, STATUS_ARCHIVED);
        assert!(board(&pool, "p1").await.unwrap().is_empty());
    }

    #[test]
    fn the_board_render_names_status_target_and_forbids_restating() {
        let mut row = grocery_board_row();
        row.target_metric = Some("bounce_rate".into());
        row.target_dir = Some("down".into());
        let text = render_board(std::slice::from_ref(&row)).unwrap();
        assert!(
            text.contains("Expand the Canadian grocery stores post"),
            "{text}"
        );
        assert!(text.contains("being measured"), "{text}");
        assert!(text.contains("bounce_rate"), "{text}");
        assert!(text.contains("down"), "{text}");
        assert!(text.contains("Do NOT restate"), "{text}");
        assert!(render_board(&[]).is_none());
    }

    /// REGRESSION for the archive: filing a card away must not destroy the data
    /// point it exists to keep. `learnable_outcomes` never filtered on status,
    /// and this test is what stops a future "tidy up the learning query" from
    /// adding that filter.
    #[tokio::test]
    async fn archiving_keeps_the_action_as_a_learning_data_point() {
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
        write_outcome(&pool, &row.id, 28, "helped", Some(0.34)).await;
        set_status(&pool, "p1", &row.id, STATUS_ARCHIVED, None)
            .await
            .unwrap();

        let rows = learnable_outcomes(&pool, "p1", 10).await.unwrap();
        assert_eq!(rows.len(), 1, "an archived action still teaches");
        assert!(render_learning(&rows).unwrap().contains("Add FAQ schema"));
        assert_eq!(
            list_for_project(&pool, "p1").await.unwrap().len(),
            1,
            "archiving is not deletion"
        );
    }

    /// REGRESSION. `pending_measurement` used to read `status IN
    /// ('verified','measuring')` only, so archiving an action mid-window
    /// dropped it out of the sweep and its remaining windows were never
    /// written — the archive quietly destroying the result it was supposed to
    /// preserve.
    #[tokio::test]
    async fn an_archived_action_still_owed_a_window_is_still_measured() {
        let pool = pool().await;
        let row = upsert_suggested(&pool, "p1", &seed("a", "r"))
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
        set_status(&pool, "p1", &row.id, STATUS_ARCHIVED, None)
            .await
            .unwrap();

        for days in [7, 14] {
            write_outcome(&pool, &row.id, days, "helped", Some(0.1)).await;
        }
        let pending = pending_measurement(&pool).await.unwrap();
        assert_eq!(
            pending.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec![row.id.as_str()],
            "a window is still owed, so it is still measured"
        );

        write_outcome(&pool, &row.id, 28, "helped", Some(0.1)).await;
        assert!(
            pending_measurement(&pool).await.unwrap().is_empty(),
            "nothing is owed once every window is written"
        );
    }

    /// Fails before `archived` joined `STATUSES`: `set_status` bails on an
    /// unknown status, so the archive route had nothing to write.
    #[tokio::test]
    async fn archived_is_a_known_status() {
        let pool = pool().await;
        let row = upsert_suggested(&pool, "p1", &seed("t", "r"))
            .await
            .unwrap();
        let updated = set_status(&pool, "p1", &row.id, STATUS_ARCHIVED, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.status, STATUS_ARCHIVED);
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
