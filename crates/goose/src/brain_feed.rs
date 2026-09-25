//! Brain feed — the one path by which Permagent's own structured data (goal
//! outcomes, decisions, cards, projects, meetings, growth, finance facts,
//! insights, reports, skills, incidents …) becomes Spectral memory.
//!
//! Before this module only chat turns, notes, Reader documents and
//! user-answered decisions reached the Brain; everything the product *did*
//! was invisible to recall. Every store now calls [`feed`] (or
//! [`feed_for_project`]) right after its own durable write.
//!
//! ## Contract
//!
//! - **Stable keys.** Each fact is keyed by what it is about, not when it was
//!   written (`card:{id}:status`, `goal:{id}:outcome`, …) — see [`feed_key`].
//!   A repeat feed of the same key replaces the pending payload in the outbox
//!   and Spectral's keyed upsert replaces the memory in place, so a card that
//!   moves ten times is one memory holding its latest status, not ten.
//! - **Source tagging.** Every memory carries `RememberOpts.source` =
//!   [`FeedSource::as_str`] (`permagent.card`, `permagent.goal`, …). None of
//!   them is `permagent.activity`, so nightly pruning and consolidation (which
//!   only sweep that source) leave these facts alone.
//! - **Best-effort.** [`feed`] never returns an error and never fails the
//!   caller's write: an outbox insert that fails is logged at debug and
//!   dropped. Callers invoke it AFTER their own write has committed.
//! - **Durable.** The payload lands in `brain_feed_outbox` (same session DB as
//!   the stores) before delivery, mirroring the chat-memory outbox: a daemon
//!   restart or a Brain hiccup does not lose it; rows are deleted once
//!   Spectral confirms, and go `dead` after [`MAX_ATTEMPTS`].
//! - **No Brain, no-op.** Without a mounted Brain (tests, early boot) rows
//!   simply stay pending; nothing panics and nothing is written anywhere else.
//!   The startup drain delivers them once a Brain is mounted.
//! - **Per-source status.** `brain_feed_status` counts deliveries per source
//!   with `last_fed_at` / `last_error`; [`feed_status`] joins it with outbox
//!   depth for `GET /api/brain/feed-status` and Settings → Memory.
//! - **Opt-out.** There was no pre-existing privacy gate on Brain writes (chat
//!   persistence writes unconditionally). The config key
//!   [`DISABLED_SOURCES_KEY`] (comma-separated short names, e.g.
//!   `finance,meeting`) stops a source at delivery: its queued rows are
//!   discarded without reaching the Brain.

use sqlx::{Pool, Row, Sqlite};

/// Config key: comma-separated [`FeedSource::short_name`]s whose facts must
/// never be written to the Brain.
pub const DISABLED_SOURCES_KEY: &str = "BRAIN_FEED_DISABLED_SOURCES";

/// Delivery attempts before a row is parked as `dead`.
pub const MAX_ATTEMPTS: i64 = 5;

/// Hard cap on a single fact's content. Feeds are short facts; anything longer
/// is a caller bug, truncated on a character boundary rather than rejected.
pub const MAX_CONTENT_CHARS: usize = 2_000;

/// Rows delivered per drain pass.
const DRAIN_BATCH: usize = 32;

/// Every kind of Permagent data that feeds the Brain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FeedSource {
    Chat,
    Goal,
    DecisionAsked,
    DecisionAuto,
    Card,
    Project,
    Meeting,
    Growth,
    Analytics,
    Finance,
    WatcherInsight,
    Council,
    Briefing,
    Skill,
    Lesson,
    Incident,
    ModelReview,
}

impl FeedSource {
    /// Every source, in the order Settings → Memory lists them.
    pub const ALL: [FeedSource; 17] = [
        FeedSource::Chat,
        FeedSource::Goal,
        FeedSource::DecisionAsked,
        FeedSource::DecisionAuto,
        FeedSource::Card,
        FeedSource::Project,
        FeedSource::Meeting,
        FeedSource::Growth,
        FeedSource::Analytics,
        FeedSource::Finance,
        FeedSource::WatcherInsight,
        FeedSource::Council,
        FeedSource::Briefing,
        FeedSource::Skill,
        FeedSource::Lesson,
        FeedSource::Incident,
        FeedSource::ModelReview,
    ];

    /// The `RememberOpts.source` tag. `Chat` keeps the tag chat persistence
    /// has always written (`"chat"`), so existing memories stay one class.
    pub fn as_str(self) -> &'static str {
        match self {
            FeedSource::Chat => "chat",
            FeedSource::Goal => "permagent.goal",
            FeedSource::DecisionAsked => "permagent.decision.asked",
            FeedSource::DecisionAuto => "permagent.decision.auto",
            FeedSource::Card => "permagent.card",
            FeedSource::Project => "permagent.project",
            FeedSource::Meeting => "permagent.meeting",
            FeedSource::Growth => "permagent.growth",
            FeedSource::Analytics => "permagent.analytics",
            FeedSource::Finance => "permagent.finance",
            FeedSource::WatcherInsight => "permagent.watcher_insight",
            FeedSource::Council => "permagent.council",
            FeedSource::Briefing => "permagent.briefing",
            FeedSource::Skill => "permagent.skill",
            FeedSource::Lesson => "permagent.lesson",
            FeedSource::Incident => "permagent.incident",
            FeedSource::ModelReview => "permagent.model_review",
        }
    }

    /// Short name used in [`DISABLED_SOURCES_KEY`] and the status API.
    pub fn short_name(self) -> &'static str {
        match self {
            FeedSource::Chat => "chat",
            FeedSource::Goal => "goal",
            FeedSource::DecisionAsked => "decision_asked",
            FeedSource::DecisionAuto => "decision_auto",
            FeedSource::Card => "card",
            FeedSource::Project => "project",
            FeedSource::Meeting => "meeting",
            FeedSource::Growth => "growth",
            FeedSource::Analytics => "analytics",
            FeedSource::Finance => "finance",
            FeedSource::WatcherInsight => "watcher_insight",
            FeedSource::Council => "council",
            FeedSource::Briefing => "briefing",
            FeedSource::Skill => "skill",
            FeedSource::Lesson => "lesson",
            FeedSource::Incident => "incident",
            FeedSource::ModelReview => "model_review",
        }
    }

    /// Human label for Settings → Memory.
    pub fn label(self) -> &'static str {
        match self {
            FeedSource::Chat => "Conversations",
            FeedSource::Goal => "Goal outcomes",
            FeedSource::DecisionAsked => "Decisions asked",
            FeedSource::DecisionAuto => "Decisions answered by policy or agents",
            FeedSource::Card => "Cards and tasks",
            FeedSource::Project => "Project briefs and strategy",
            FeedSource::Meeting => "Calendar meetings",
            FeedSource::Growth => "Growth actions and outcomes",
            FeedSource::Analytics => "Daily analytics rollups",
            FeedSource::Finance => "Finance facts",
            FeedSource::WatcherInsight => "Watcher insights",
            FeedSource::Council => "Council reports",
            FeedSource::Briefing => "Agent briefings",
            FeedSource::Skill => "Skills",
            FeedSource::Lesson => "Lessons",
            FeedSource::Incident => "Incidents",
            FeedSource::ModelReview => "Model reviews",
        }
    }

    /// Inverse of [`Self::as_str`].
    pub fn from_tag(tag: &str) -> Option<FeedSource> {
        FeedSource::ALL.into_iter().find(|s| s.as_str() == tag)
    }
}

/// Where a fact's wing comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
enum WingRef {
    None,
    Slug(String),
    /// A project id, resolved to its slug at enqueue time.
    Project(String),
}

/// Stable memory key: `{kind}:{id}:{facet}`. The id is sanitised the same way
/// decision keys are (colons/whitespace → `-`), so the three-part structure
/// stays parseable. Identical inputs always yield the identical key — that is
/// what turns a repeat feed into an upsert.
pub fn feed_key(kind: &str, id: &str, facet: &str) -> String {
    format!(
        "{kind}:{}:{facet}",
        crate::decision_inbox::learn::sanitize_key_part(id)
    )
}

/// The `RememberOpts` every fed fact is written with. Pure, so the tagging is
/// unit-testable without a Brain.
pub fn feed_remember_opts(
    source: FeedSource,
    key: &str,
    wing: Option<String>,
) -> spectral::RememberOpts {
    spectral::RememberOpts {
        source: Some(source.as_str().to_string()),
        confidence: Some(1.0),
        visibility: spectral::Visibility::Private,
        // The fact's key is its episode: re-feeds of the same card/goal/…
        // are one evolving event, never merged with unrelated writes by the
        // write-gap heuristic.
        episode_id: Some(format!("feed-episode:{key}")),
        wing,
        ..Default::default()
    }
}

/// Bound a fact's content on a character boundary.
pub fn bounded_content(content: &str) -> String {
    let trimmed = content.trim();
    if trimmed.chars().count() <= MAX_CONTENT_CHARS {
        trimmed.to_string()
    } else {
        let mut s: String = trimmed.chars().take(MAX_CONTENT_CHARS - 1).collect();
        s.push('…');
        s
    }
}

/// Today's date as `YYYY-MM-DD` (UTC) — the date stamp facts carry so a
/// future agent can tell when something happened.
pub fn today() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

/// Create the outbox + status tables. Idempotent; called from
/// `init_spectral_db` and the version-independent boot pass.
pub async fn apply_schema(pool: &Pool<Sqlite>) -> anyhow::Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS brain_feed_outbox (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            memory_key      TEXT NOT NULL UNIQUE,
            source          TEXT NOT NULL,
            wing            TEXT,
            content         TEXT NOT NULL,
            revision        INTEGER NOT NULL DEFAULT 0,
            status          TEXT NOT NULL DEFAULT 'pending'
                            CHECK (status IN ('pending','running','dead')),
            attempts        INTEGER NOT NULL DEFAULT 0,
            last_error      TEXT,
            next_attempt_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
            created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
            updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_brain_feed_outbox_drain
         ON brain_feed_outbox(status, next_attempt_at, id)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS brain_feed_status (
            source        TEXT PRIMARY KEY,
            fed_count     INTEGER NOT NULL DEFAULT 0,
            last_fed_at   TEXT,
            last_error    TEXT,
            last_error_at TEXT
        )",
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Feed one fact to the Brain. Best-effort: never errors, never blocks on the
/// Brain (only on one local outbox upsert). Call AFTER the caller's own write
/// has committed — never inside an open transaction on the same pool.
pub async fn feed(
    pool: &Pool<Sqlite>,
    source: FeedSource,
    key: String,
    wing: Option<String>,
    content: String,
) {
    let wing = match wing {
        Some(w) if !w.trim().is_empty() => WingRef::Slug(w),
        _ => WingRef::None,
    };
    feed_inner(pool, source, key, wing, content).await;
}

/// [`feed`] for a fact that belongs to a project: the project's slug becomes
/// the wing (the implicit `personal` project stays unwinged, like notes).
pub async fn feed_for_project(
    pool: &Pool<Sqlite>,
    source: FeedSource,
    key: String,
    project_id: &str,
    content: String,
) {
    feed_inner(
        pool,
        source,
        key,
        WingRef::Project(project_id.to_string()),
        content,
    )
    .await;
}

/// A project's display name, for fact prose. Falls back to the id.
pub async fn project_name(pool: &Pool<Sqlite>, project_id: &str) -> String {
    sqlx::query_scalar::<_, String>("SELECT name FROM projects WHERE id = ?")
        .bind(project_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| project_id.to_string())
}

async fn resolve_wing(pool: &Pool<Sqlite>, wing: WingRef) -> Option<String> {
    match wing {
        WingRef::None => None,
        WingRef::Slug(s) => Some(s),
        WingRef::Project(id) => {
            match sqlx::query_scalar::<_, String>("SELECT slug FROM projects WHERE id = ?")
                .bind(&id)
                .fetch_optional(pool)
                .await
            {
                Ok(Some(slug)) if slug == crate::session_wing::PERSONAL_SLUG => None,
                Ok(Some(slug)) => Some(slug),
                _ => None,
            }
        }
    }
}

async fn feed_inner(
    pool: &Pool<Sqlite>,
    source: FeedSource,
    key: String,
    wing: WingRef,
    content: String,
) {
    let content = bounded_content(&content);
    if key.trim().is_empty() || content.is_empty() {
        return;
    }
    let wing = resolve_wing(pool, wing).await;
    let result = sqlx::query(
        "INSERT INTO brain_feed_outbox (memory_key, source, wing, content)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(memory_key) DO UPDATE SET
           source = excluded.source,
           wing = excluded.wing,
           content = excluded.content,
           revision = brain_feed_outbox.revision + 1,
           status = 'pending',
           attempts = 0,
           last_error = NULL,
           next_attempt_at = strftime('%Y-%m-%dT%H:%M:%fZ','now'),
           updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')",
    )
    .bind(&key)
    .bind(source.as_str())
    .bind(wing.as_deref())
    .bind(&content)
    .execute(pool)
    .await;
    if let Err(e) = result {
        tracing::debug!(
            target: "permagentd::brain",
            source = source.as_str(),
            key = %key,
            error = %e,
            "brain feed enqueue skipped"
        );
        return;
    }
    if let Some(brain) = crate::agents::platform_extensions::get_global_brain() {
        spawn_drain(brain, pool.clone());
    }
}

/// Record one successful delivery for `source` (also used by the chat outbox,
/// whose rows live in their own table).
pub async fn record_fed(pool: &Pool<Sqlite>, source: FeedSource) {
    let r = sqlx::query(
        "INSERT INTO brain_feed_status (source, fed_count, last_fed_at, last_error, last_error_at)
         VALUES (?, 1, strftime('%Y-%m-%dT%H:%M:%fZ','now'), NULL, NULL)
         ON CONFLICT(source) DO UPDATE SET
           fed_count = brain_feed_status.fed_count + 1,
           last_fed_at = excluded.last_fed_at,
           last_error = NULL,
           last_error_at = NULL",
    )
    .bind(source.as_str())
    .execute(pool)
    .await;
    if let Err(e) = r {
        tracing::debug!(target: "permagentd::brain", "brain feed status update skipped: {e}");
    }
}

/// Record a delivery failure for `source` (count unchanged).
pub async fn record_error(pool: &Pool<Sqlite>, source: FeedSource, error: &str) {
    let r = sqlx::query(
        "INSERT INTO brain_feed_status (source, fed_count, last_error, last_error_at)
         VALUES (?, 0, ?, strftime('%Y-%m-%dT%H:%M:%fZ','now'))
         ON CONFLICT(source) DO UPDATE SET
           last_error = excluded.last_error,
           last_error_at = excluded.last_error_at",
    )
    .bind(source.as_str())
    .bind(error)
    .execute(pool)
    .await;
    if let Err(e) = r {
        tracing::debug!(target: "permagentd::brain", "brain feed status update skipped: {e}");
    }
}

/// Sources disabled by [`DISABLED_SOURCES_KEY`].
pub fn disabled_sources() -> Vec<FeedSource> {
    let raw = crate::config::Config::global()
        .get_param::<String>(DISABLED_SOURCES_KEY)
        .unwrap_or_default();
    parse_disabled(&raw)
}

/// Parse a comma-separated short-name list. Unknown names are ignored.
pub fn parse_disabled(raw: &str) -> Vec<FeedSource> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|name| {
            FeedSource::ALL
                .into_iter()
                .find(|s| s.short_name().eq_ignore_ascii_case(name))
        })
        .collect()
}

/// Start a bounded background drain of the outbox.
pub fn spawn_drain(brain: crate::brain_handle::SafeBrain, pool: Pool<Sqlite>) {
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    tokio::spawn(async move {
        if let Err(e) = drain(&brain, &pool).await {
            tracing::debug!(target: "permagentd::brain", "brain feed drain failed: {e}");
        }
    });
}

struct FeedJob {
    id: i64,
    memory_key: String,
    source: String,
    wing: Option<String>,
    content: String,
    revision: i64,
    attempts: i64,
}

async fn claim(pool: &Pool<Sqlite>) -> anyhow::Result<Option<FeedJob>> {
    let row = sqlx::query(
        "UPDATE brain_feed_outbox
         SET status = 'running', attempts = attempts + 1,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
         WHERE id = (
           SELECT id FROM brain_feed_outbox
           WHERE (status = 'pending' OR
                  (status = 'running' AND updated_at < strftime('%Y-%m-%dT%H:%M:%fZ','now','-5 minutes')))
             AND next_attempt_at <= strftime('%Y-%m-%dT%H:%M:%fZ','now')
           ORDER BY id LIMIT 1
         )
         RETURNING id, memory_key, source, wing, content, revision, attempts",
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|row| FeedJob {
        id: row.get("id"),
        memory_key: row.get("memory_key"),
        source: row.get("source"),
        wing: row.get("wing"),
        content: row.get("content"),
        revision: row.get("revision"),
        attempts: row.get("attempts"),
    }))
}

/// Deliver pending rows to the Brain. A row re-fed while in flight (revision
/// bumped) survives its old delivery and is redelivered with the new content.
pub async fn drain(
    brain: &crate::brain_handle::SafeBrain,
    pool: &Pool<Sqlite>,
) -> anyhow::Result<usize> {
    let disabled = disabled_sources();
    let mut delivered = 0usize;
    for _ in 0..DRAIN_BATCH {
        let Some(job) = claim(pool).await? else {
            break;
        };
        let source = FeedSource::from_tag(&job.source);
        if source.is_some_and(|s| disabled.contains(&s)) {
            sqlx::query("DELETE FROM brain_feed_outbox WHERE id = ? AND revision = ?")
                .bind(job.id)
                .bind(job.revision)
                .execute(pool)
                .await?;
            continue;
        }
        let opts = match source {
            Some(s) => feed_remember_opts(s, &job.memory_key, job.wing.clone()),
            None => spectral::RememberOpts {
                source: Some(job.source.clone()),
                visibility: spectral::Visibility::Private,
                wing: job.wing.clone(),
                ..Default::default()
            },
        };
        match brain
            .remember_with(&job.memory_key, &job.content, opts)
            .await
        {
            Ok(_) => {
                sqlx::query("DELETE FROM brain_feed_outbox WHERE id = ? AND revision = ?")
                    .bind(job.id)
                    .bind(job.revision)
                    .execute(pool)
                    .await?;
                if let Some(s) = source {
                    record_fed(pool, s).await;
                }
                delivered += 1;
            }
            Err(e) => {
                let message = e.to_string();
                let status = if job.attempts >= MAX_ATTEMPTS {
                    "dead"
                } else {
                    "pending"
                };
                sqlx::query(
                    "UPDATE brain_feed_outbox
                     SET status = ?, last_error = ?,
                         next_attempt_at = strftime('%Y-%m-%dT%H:%M:%fZ','now','+30 seconds'),
                         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
                     WHERE id = ? AND revision = ?",
                )
                .bind(status)
                .bind(&message)
                .bind(job.id)
                .bind(job.revision)
                .execute(pool)
                .await?;
                if let Some(s) = source {
                    record_error(pool, s, &message).await;
                }
                tracing::debug!(
                    target: "permagentd::brain",
                    key = %job.memory_key,
                    attempts = job.attempts,
                    error = %message,
                    "brain feed delivery failed"
                );
            }
        }
    }
    Ok(delivered)
}

/// One source's ingestion status.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct FeedSourceStatus {
    /// Short name (`card`, `goal`, …).
    pub source: String,
    /// The `RememberOpts.source` tag memories carry.
    pub tag: String,
    pub label: String,
    /// Facts delivered to the Brain.
    pub count: i64,
    pub last_fed_at: Option<String>,
    pub last_error: Option<String>,
    pub last_error_at: Option<String>,
    /// Queued, not yet delivered.
    pub pending: i64,
    /// Gave up after [`MAX_ATTEMPTS`].
    pub dead: i64,
    /// False when [`DISABLED_SOURCES_KEY`] lists this source.
    pub enabled: bool,
}

/// Status for every [`FeedSource`], including ones never fed (count 0).
/// `disabled` is passed in so the query stays config-free for tests.
pub async fn feed_status(
    pool: &Pool<Sqlite>,
    disabled: &[FeedSource],
) -> anyhow::Result<Vec<FeedSourceStatus>> {
    let status_rows = sqlx::query(
        "SELECT source, fed_count, last_fed_at, last_error, last_error_at FROM brain_feed_status",
    )
    .fetch_all(pool)
    .await?;
    let depth_rows = sqlx::query(
        "SELECT source,
                SUM(CASE WHEN status IN ('pending','running') THEN 1 ELSE 0 END) AS pending,
                SUM(CASE WHEN status = 'dead' THEN 1 ELSE 0 END) AS dead
         FROM brain_feed_outbox GROUP BY source",
    )
    .fetch_all(pool)
    .await?;

    let mut out = Vec::with_capacity(FeedSource::ALL.len());
    for s in FeedSource::ALL {
        let tag = s.as_str();
        let st = status_rows
            .iter()
            .find(|r| r.get::<String, _>("source") == tag);
        let depth = depth_rows
            .iter()
            .find(|r| r.get::<String, _>("source") == tag);
        out.push(FeedSourceStatus {
            source: s.short_name().to_string(),
            tag: tag.to_string(),
            label: s.label().to_string(),
            count: st.map(|r| r.get::<i64, _>("fed_count")).unwrap_or(0),
            last_fed_at: st.and_then(|r| r.get::<Option<String>, _>("last_fed_at")),
            last_error: st.and_then(|r| r.get::<Option<String>, _>("last_error")),
            last_error_at: st.and_then(|r| r.get::<Option<String>, _>("last_error_at")),
            pending: depth
                .and_then(|r| r.get::<Option<i64>, _>("pending"))
                .unwrap_or(0),
            dead: depth
                .and_then(|r| r.get::<Option<i64>, _>("dead"))
                .unwrap_or(0),
            enabled: !disabled.contains(&s),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_pool() -> Pool<Sqlite> {
        // One connection: an in-memory SQLite database is per-connection.
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        apply_schema(&pool).await.unwrap();
        sqlx::query("CREATE TABLE projects (id TEXT PRIMARY KEY, name TEXT, slug TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    async fn outbox(pool: &Pool<Sqlite>) -> Vec<(String, String, Option<String>, String)> {
        sqlx::query_as::<_, (String, String, Option<String>, String)>(
            "SELECT memory_key, source, wing, content FROM brain_feed_outbox ORDER BY id",
        )
        .fetch_all(pool)
        .await
        .unwrap()
    }

    #[test]
    fn keys_are_stable_and_sanitised() {
        assert_eq!(feed_key("card", "c-1", "status"), "card:c-1:status");
        assert_eq!(
            feed_key("card", "c-1", "status"),
            feed_key("card", "c-1", "status")
        );
        // Colons and whitespace inside an id cannot break the structure.
        assert_eq!(feed_key("goal", "a:b c", "outcome"), "goal:a-b-c:outcome");
    }

    #[test]
    fn remember_opts_carry_the_source_tag() {
        let opts = feed_remember_opts(FeedSource::Card, "card:x:status", Some("evntally".into()));
        assert_eq!(opts.source.as_deref(), Some("permagent.card"));
        assert_eq!(opts.wing.as_deref(), Some("evntally"));
        assert_eq!(opts.visibility, spectral::Visibility::Private);
        assert_eq!(
            opts.episode_id.as_deref(),
            Some("feed-episode:card:x:status")
        );
    }

    #[test]
    fn no_source_is_the_prunable_activity_class() {
        for s in FeedSource::ALL {
            assert_ne!(s.as_str(), "permagent.activity", "{s:?} would be pruned");
            assert_eq!(FeedSource::from_tag(s.as_str()), Some(s));
        }
    }

    #[test]
    fn disabled_list_parses_short_names() {
        assert_eq!(
            parse_disabled(" finance, MEETING ,bogus,,"),
            vec![FeedSource::Finance, FeedSource::Meeting]
        );
        assert!(parse_disabled("").is_empty());
    }

    #[test]
    fn content_is_bounded_on_char_boundary() {
        let long = "é".repeat(MAX_CONTENT_CHARS + 50);
        let b = bounded_content(&long);
        assert_eq!(b.chars().count(), MAX_CONTENT_CHARS);
        assert!(b.ends_with('…'));
    }

    #[tokio::test]
    async fn feed_without_brain_queues_and_never_errors() {
        // No global Brain is mounted in unit tests: feed must no-op cleanly
        // (row queued for a later drain, nothing delivered, no panic).
        assert!(crate::agents::platform_extensions::get_global_brain().is_none());
        let pool = test_pool().await;
        feed(
            &pool,
            FeedSource::Card,
            feed_key("card", "c1", "status"),
            None,
            "Card 'Ship it' moved to Done.".into(),
        )
        .await;
        let rows = outbox(&pool).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1, "permagent.card");
        let status = feed_status(&pool, &[]).await.unwrap();
        let card = status.iter().find(|s| s.source == "card").unwrap();
        assert_eq!(card.count, 0);
        assert_eq!(card.pending, 1);
        assert!(card.last_fed_at.is_none());
    }

    #[tokio::test]
    async fn repeat_feed_of_same_key_upserts() {
        let pool = test_pool().await;
        let key = feed_key("card", "c1", "status");
        feed(&pool, FeedSource::Card, key.clone(), None, "v1".into()).await;
        feed(&pool, FeedSource::Card, key.clone(), None, "v2".into()).await;
        let rows = outbox(&pool).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].3, "v2");
    }

    #[tokio::test]
    async fn feed_for_project_resolves_slug_and_skips_personal() {
        let pool = test_pool().await;
        sqlx::query("INSERT INTO projects (id, name, slug) VALUES ('p1','Evntally','evntally'), ('p2','Personal',?)")
            .bind(crate::session_wing::PERSONAL_SLUG)
            .execute(&pool)
            .await
            .unwrap();
        feed_for_project(
            &pool,
            FeedSource::Goal,
            "goal:g1:outcome".into(),
            "p1",
            "x".into(),
        )
        .await;
        feed_for_project(
            &pool,
            FeedSource::Goal,
            "goal:g2:outcome".into(),
            "p2",
            "y".into(),
        )
        .await;
        let rows = outbox(&pool).await;
        assert_eq!(rows[0].2.as_deref(), Some("evntally"));
        assert_eq!(rows[1].2, None);
        assert_eq!(project_name(&pool, "p1").await, "Evntally");
        assert_eq!(project_name(&pool, "nope").await, "nope");
    }

    #[tokio::test]
    async fn feed_on_a_pool_without_the_table_is_silent() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        // Must not panic or error — the caller's write already succeeded.
        feed(
            &pool,
            FeedSource::Goal,
            "goal:x:outcome".into(),
            None,
            "z".into(),
        )
        .await;
    }

    #[tokio::test]
    async fn status_lists_every_source_with_counts_and_errors() {
        let pool = test_pool().await;
        record_fed(&pool, FeedSource::Goal).await;
        record_fed(&pool, FeedSource::Goal).await;
        record_error(&pool, FeedSource::Finance, "brain down").await;
        let status = feed_status(&pool, &[FeedSource::Finance]).await.unwrap();
        assert_eq!(status.len(), FeedSource::ALL.len());
        let goal = status.iter().find(|s| s.source == "goal").unwrap();
        assert_eq!(goal.count, 2);
        assert!(goal.last_fed_at.is_some());
        let fin = status.iter().find(|s| s.source == "finance").unwrap();
        assert_eq!(fin.count, 0);
        assert_eq!(fin.last_error.as_deref(), Some("brain down"));
        assert!(!fin.enabled);
        // A success clears the error.
        record_fed(&pool, FeedSource::Finance).await;
        let status = feed_status(&pool, &[]).await.unwrap();
        let fin = status.iter().find(|s| s.source == "finance").unwrap();
        assert_eq!(fin.count, 1);
        assert!(fin.last_error.is_none());
    }
}
