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
//!   [`spawn_periodic_drain`], started once the Brain is mounted, delivers
//!   them — both the startup backlog and any row whose retry backoff
//!   (`next_attempt_at`) has since elapsed — without depending on some other
//!   caller happening to call [`feed`] again.
//! - **Per-source status.** `brain_feed_status` counts deliveries per source
//!   with `last_fed_at` / `last_error`; [`feed_status`] joins it with outbox
//!   depth for `GET /api/brain/feed-status` and Settings → Memory.
//! - **Opt-out.** There was no pre-existing privacy gate on Brain writes (chat
//!   persistence writes unconditionally). The config key
//!   [`DISABLED_SOURCES_KEY`] (comma-separated short names, e.g.
//!   `finance,meeting`) stops a source at delivery: its queued rows are
//!   discarded without reaching the Brain. [`FeedSource::Chat`] is the one
//!   exception: [`disabled_sources`] strips it out even if it is listed
//!   (2026-09-25 ruling — a UI-only lock on the Settings toggle is not a
//!   guardrail), because conversations are the Brain's primary intake.

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
    /// The owner identity record (dagK owner-identity, 2026-09-26): fed once
    /// per real edit from `PUT /api/owner` so the agent has a durable memory
    /// of who its user is, instead of that fact living only in `owner.yaml`.
    Owner,
    /// A person's relationship-strength bucket crossed into "fading"/"dormant"
    /// (DAG K, `routes::relationship_strength`). Distinct from `Meeting`: this
    /// fires on the *derived score* transitioning, not on a single logged
    /// event, and is keyed per-person (`person:{id}:relationship`) rather than
    /// per-meeting.
    Person,
}

impl FeedSource {
    /// Every source, in the order Settings → Memory lists them.
    pub const ALL: [FeedSource; 19] = [
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
        FeedSource::Owner,
        FeedSource::Person,
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
            FeedSource::Owner => "permagent.owner",
            FeedSource::Person => "permagent.person",
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
            FeedSource::Owner => "owner",
            FeedSource::Person => "person",
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
            FeedSource::Owner => "Who you are, to the agent",
            FeedSource::Person => "Relationship changes",
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
    let mut disabled = parse_disabled(&raw);
    // Chat cannot be disabled from here, no matter what the config says. The
    // Settings → Memory toggle already locks this in the UI (product-level
    // guardrail), but a UI-only lock is not a guardrail — a hand-edited
    // config.yaml, an old value left over from before the lock existed, or a
    // future writer of this key would otherwise still switch off the Brain's
    // primary intake. Enforced here, at the one place every drain pass reads
    // this list, so nothing upstream of it can bypass it.
    if let Some(pos) = disabled.iter().position(|s| *s == FeedSource::Chat) {
        disabled.remove(pos);
        tracing::debug!(
            target: "permagentd::brain",
            "BRAIN_FEED_DISABLED_SOURCES listed 'chat' — ignored, conversations can never be disabled"
        );
    }
    disabled
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

/// How often [`spawn_periodic_drain`] sweeps the outbox on its own, with no
/// [`feed`] call to trigger it. A failed delivery's backoff
/// (`next_attempt_at = now + 30s`, see [`drain`]) is only honoured promptly if
/// this is comfortably tighter than that; a daily or hourly cadence (like
/// `analytics_drain`'s) would leave a retry — or a whole backlog enqueued
/// before the Brain mounted — waiting far longer than the failure itself.
const PERIODIC_DRAIN_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

/// Run [`drain`] on a fixed interval for as long as the process lives.
///
/// This is the ONLY periodic trigger for delivery: without it, a fact
/// enqueued before a Brain is mounted (daemon boot window, or a prior crash),
/// or one whose retry deadline has already passed, is only reclaimed by
/// whatever unrelated [`feed`] call happens to fire `spawn_drain` next —
/// which may be minutes, hours, or, in a short-lived process, never. Call
/// this once, right after the Brain becomes available (see `state.rs`); the
/// first pass runs immediately, so a startup backlog does not wait out the
/// full interval either.
pub fn spawn_periodic_drain(brain: crate::brain_handle::SafeBrain, pool: Pool<Sqlite>) {
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    tokio::spawn(async move {
        loop {
            if let Err(e) = drain(&brain, &pool).await {
                tracing::debug!(target: "permagentd::brain", "brain feed periodic drain failed: {e}");
            }
            tokio::time::sleep(PERIODIC_DRAIN_INTERVAL).await;
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

/// Finish one successful delivery: delete the claimed row, guarded by
/// `revision` so a row re-fed while in flight survives, and only count the
/// delivery (bump `fed_count`) if THIS call is the one that actually removed
/// it. `claim`'s stale-`running` recovery (rows stuck over 5 minutes) can
/// reclaim and redeliver a row whose original `remember_with` call is still
/// genuinely in flight — if both calls eventually succeed, the second one's
/// DELETE affects zero rows because the first already removed it, and must
/// not double-count `brain_feed_status.fed_count` for what is really one
/// fact. Returns whether this call won the delete.
async fn finish_delivered(
    pool: &Pool<Sqlite>,
    id: i64,
    revision: i64,
    source: Option<FeedSource>,
) -> anyhow::Result<bool> {
    let result = sqlx::query("DELETE FROM brain_feed_outbox WHERE id = ? AND revision = ?")
        .bind(id)
        .bind(revision)
        .execute(pool)
        .await?;
    let won = result.rows_affected() > 0;
    if won {
        if let Some(s) = source {
            record_fed(pool, s).await;
        }
    }
    Ok(won)
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
                if finish_delivered(pool, job.id, job.revision, source).await? {
                    delivered += 1;
                }
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

    /// dagK owner-identity (2026-09-26): the new `Owner` source is wired into
    /// every place a `FeedSource` variant has to appear — `ALL`, the inverse
    /// lookup, and (implicitly, via a non-empty `as_str`/`short_name`/`label`)
    /// the three per-variant match arms. `from_tag` round-tripping is the one
    /// check that would have caught a variant added to the enum but left out
    /// of `ALL` (a `match` omission fails to compile; a missed `ALL` entry
    /// does not).
    #[test]
    fn owner_source_is_wired_into_all_and_round_trips_by_tag() {
        assert!(FeedSource::ALL.contains(&FeedSource::Owner));
        assert_eq!(FeedSource::Owner.as_str(), "permagent.owner");
        assert_eq!(FeedSource::Owner.short_name(), "owner");
        assert!(!FeedSource::Owner.label().is_empty());
        assert_eq!(
            FeedSource::from_tag("permagent.owner"),
            Some(FeedSource::Owner)
        );
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

    /// The Settings → Memory panel's per-source Toggle writes
    /// `BRAIN_FEED_DISABLED_SOURCES` and expects it to take effect on the
    /// daemon's NEXT drain pass, no restart — that promise rests entirely on
    /// [`disabled_sources`] never caching what it read at boot.
    /// `Config::get_param` checks an env-var override before the config file
    /// on every call (`crates/goose/src/config/base.rs`), so toggling the env
    /// var here is a faithful, filesystem-free stand-in for editing
    /// config.yaml: if this passes, a live config-file edit takes effect the
    /// same way, with no code path in between that could cache the old value.
    #[test]
    #[serial_test::serial(brain_feed_disabled_sources_env)]
    fn disabled_sources_reads_live_not_cached_at_boot() {
        let env_key = DISABLED_SOURCES_KEY.to_uppercase();
        std::env::remove_var(&env_key);
        assert!(
            disabled_sources().is_empty(),
            "nothing disabled before the env var is ever set"
        );

        std::env::set_var(&env_key, "finance,meeting");
        assert_eq!(
            disabled_sources(),
            vec![FeedSource::Finance, FeedSource::Meeting],
            "the very next read sees the new value — no boot-time cache"
        );

        // Flip it again, same process, no restart: still live.
        std::env::set_var(&env_key, "skill");
        assert_eq!(disabled_sources(), vec![FeedSource::Skill]);

        std::env::remove_var(&env_key);
        assert!(
            disabled_sources().is_empty(),
            "removing it re-enables everything on the next read too"
        );
    }

    /// A UI-only lock is not a guardrail (coordinator ruling, 2026-09-25): even
    /// if `BRAIN_FEED_DISABLED_SOURCES` names `"chat"` — a hand-edited
    /// config.yaml, a stale value from before the lock existed, anything —
    /// [`disabled_sources`] must still never return [`FeedSource::Chat`]. This
    /// is the exact list [`drain`] filters deliveries against, so proving chat
    /// never appears here proves chat is never skipped there: conversations
    /// still feed no matter what this key says.
    #[test]
    #[serial_test::serial(brain_feed_disabled_sources_env)]
    fn chat_cannot_be_disabled_even_when_listed() {
        let env_key = DISABLED_SOURCES_KEY.to_uppercase();
        std::env::set_var(&env_key, "chat");
        assert!(
            !disabled_sources().contains(&FeedSource::Chat),
            "chat must never be in the disabled list, no matter what the config says"
        );

        std::env::set_var(&env_key, "chat,finance");
        let disabled = disabled_sources();
        assert!(
            !disabled.contains(&FeedSource::Chat),
            "chat still not disabled alongside another source"
        );
        assert!(
            disabled.contains(&FeedSource::Finance),
            "the other listed source is unaffected"
        );

        std::env::remove_var(&env_key);
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

    /// #1: before `spawn_periodic_drain` existed, nothing but a fresh `feed()`
    /// call ever triggered delivery — `state.rs` mounted the Brain and never
    /// drained `brain_feed_outbox` at all (unlike the chat-memory outbox,
    /// which gets an explicit startup drain right next to it). This backlog
    /// row is inserted directly, the way a row lands when it is enqueued
    /// before any Brain is mounted (see `feed_without_brain_queues_and_never_errors`),
    /// and the test calls neither `feed`, `drain`, nor `spawn_drain` itself —
    /// only the periodic loop may deliver it.
    #[tokio::test]
    #[serial_test::serial(brain_feed_permagent_path_root)]
    async fn spawn_periodic_drain_delivers_a_backlog_with_no_feed_call() {
        let root = tempfile::tempdir().unwrap();
        let root_str = root.path().display().to_string();
        // `remember_with`'s post-write entity linking reaches
        // `Paths::brain_dir()`; without this, a background task would read
        // and write the real ~/.permagent/brain instead of this tempdir.
        let _env = env_lock::lock_env([("PERMAGENT_PATH_ROOT", Some(root_str.as_str()))]);

        let pool = test_pool().await;
        let key = feed_key("card", "c1", "status");
        sqlx::query("INSERT INTO brain_feed_outbox (memory_key, source, content) VALUES (?, ?, ?)")
            .bind(&key)
            .bind(FeedSource::Card.as_str())
            .bind("Card 'Ship it' moved to Done.")
            .execute(&pool)
            .await
            .unwrap();

        let raw = spectral::Brain::open(root.path().join("brain")).unwrap();
        let brain = crate::brain_handle::SafeBrain::new(raw);
        spawn_periodic_drain(brain, pool.clone());

        let mut delivered = false;
        for _ in 0..150 {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            if outbox(&pool).await.is_empty() {
                delivered = true;
                break;
            }
        }
        assert!(
            delivered,
            "a periodic drain must deliver a pre-existing backlog on its own, \
             with no external feed() call ever made"
        );
        let status = feed_status(&pool, &[]).await.unwrap();
        let card = status.iter().find(|s| s.source == "card").unwrap();
        assert_eq!(card.count, 1);
    }

    /// #16: `claim()` can reclaim a row stuck in `status = 'running'` for over
    /// five minutes even though the original `remember_with` call is still
    /// genuinely in flight (e.g. slow embedding under load), not dead. If
    /// both the original and the reclaiming delivery go on to succeed, only
    /// the one that actually deletes the row may count as delivered — the
    /// other's `DELETE ... WHERE id = ? AND revision = ?` affects zero rows
    /// because the row is already gone, and must not still bump
    /// `brain_feed_status.fed_count` for what is really one fact.
    #[tokio::test]
    async fn redundant_redelivery_of_an_already_completed_job_does_not_double_count() {
        let pool = test_pool().await;
        let key = feed_key("card", "c1", "status");
        feed(&pool, FeedSource::Card, key.clone(), None, "v1".into()).await;

        // `claim()` doesn't touch `revision`, so two deliveries of the same
        // row in flight at once (the stalled-reclaim race) share it.
        let job = claim(&pool).await.unwrap().unwrap();

        let first_won = finish_delivered(&pool, job.id, job.revision, Some(FeedSource::Card))
            .await
            .unwrap();
        assert!(
            first_won,
            "the first delivery to finish must win the delete"
        );

        let second_won = finish_delivered(&pool, job.id, job.revision, Some(FeedSource::Card))
            .await
            .unwrap();
        assert!(
            !second_won,
            "a redundant redelivery of an already-completed job must not win"
        );

        let status = feed_status(&pool, &[]).await.unwrap();
        let card = status.iter().find(|s| s.source == "card").unwrap();
        assert_eq!(
            card.count, 1,
            "fed_count must reflect exactly one delivered fact, not two"
        );
    }
}
