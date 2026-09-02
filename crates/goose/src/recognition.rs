//! Recognition instrumentation — the AmbientFrame emit side.
//!
//! At every recall we mint a `retrieval_id` and persist a `recognition_events`
//! row plus its retrieved-set members UNCONDITIONALLY (the falsifiable
//! substrate). The retrieval-set memory IDs are not in scope at turn
//! resolution, so we never hold them to turn-end: association is by the
//! persisted key, and the outcome is written back later (seconds-to-minutes)
//! keyed on `retrieval_id`.
//!
//! Two outcome proxies feed the write-back:
//!   - PRIMARY: task resolution (`tasks::log_task_completed`), joined via
//!     `tasks.session_id` → `recognition_events.session_id`.
//!   - SECONDARY: decision approve/bounce (`routes/decisions::execute_effect`),
//!     joined 2-hop `goal_id` → `cards.metadata_json.worker_session_id` →
//!     `recognition_events.session_id`. ~0% volume until the orchestrator is
//!     enabled.
//!
//! All writes are best-effort: failures are logged, never propagated. These
//! tables live in permagent.db (the SessionStorage pool), distinct from
//! Spectral's own precursor `retrieval_events`.

use chrono::Utc;
use sqlx::{Pool, Sqlite};
use std::collections::HashSet;
use std::sync::Arc;
use tracing::{debug, warn};
use uuid::Uuid;

/// A single retrieved-set member: `(memory_id, signal_score, rank)`.
pub type SetMember = (String, f64, i64);

/// A memory that passed the prompt-injection filter. Its content is retained
/// only in memory until turn end so conservative exact-overlap citation
/// detection can run off the reply hot path; only the id is persisted.
#[derive(Debug, Clone)]
pub struct InjectedMemory {
    pub id: String,
    pub content: String,
}

/// Persistence handle for one recall. Dropping it detaches the initial write;
/// completing it after the reply chains citation detection behind that write.
pub struct PendingRecognition {
    pool: Pool<Sqlite>,
    retrieval_id: String,
    injected: Arc<[InjectedMemory]>,
    /// Flips to `true` when the `recognition_events` INSERT has finished.
    ///
    /// A `watch` rather than the `JoinHandle` this used to hold, because there
    /// are now TWO writers that must land after the row exists — the citation
    /// write-back at turn end and the recognize() verdict — and a `JoinHandle`
    /// can only be awaited once. Both are UPDATEs keyed on `retrieval_id`, so
    /// running either before the INSERT would silently affect zero rows.
    persisted: tokio::sync::watch::Receiver<bool>,
}

fn now_iso() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// Persist a recognition (recall) event plus its retrieved set, fire-and-forget.
///
/// Mints the `retrieval_id` join key, writes one `recognition_events` row and
/// one `recognition_set_members` row per `member`, UNCONDITIONALLY (even an
/// empty set). Spawns a detached task so it never blocks the reply path; all
/// errors are logged.
pub fn spawn_persist_recognition(
    pool: Pool<Sqlite>,
    recognition_ctx: spectral::graph::RecognitionContext,
    query: String,
    strategy: String,
    members: Vec<SetMember>,
    injected: Vec<InjectedMemory>,
) -> PendingRecognition {
    let retrieval_id = Uuid::now_v7().to_string();
    let task_pool = pool.clone();
    let task_retrieval_id = retrieval_id.clone();
    let injected: Arc<[InjectedMemory]> = injected.into();
    let task_injected = Arc::clone(&injected);
    let (persisted_tx, persisted) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        let injected_ids: Vec<String> = task_injected
            .iter()
            .map(|memory| memory.id.clone())
            .collect();
        if let Err(e) = persist_recognition_with_id(
            &task_pool,
            &task_retrieval_id,
            &recognition_ctx,
            &query,
            &strategy,
            &members,
            &injected_ids,
        )
        .await
        {
            warn!(
                target: "permagent::recognition",
                "Failed to persist recognition event: {}",
                e
            );
        }
        // Signal AFTER the write attempt, success or failure: the followers
        // are best-effort UPDATEs whose own zero-row / error handling is the
        // right place to notice a missing row, and holding them forever on a
        // failed INSERT would leak two tasks per recall.
        let _ = persisted_tx.send(true);
    });
    PendingRecognition {
        pool,
        retrieval_id,
        injected,
        persisted,
    }
}

async fn persist_recognition_with_id(
    pool: &Pool<Sqlite>,
    retrieval_id: &str,
    recognition_ctx: &spectral::graph::RecognitionContext,
    query: &str,
    strategy: &str,
    members: &[SetMember],
    injected_memory_ids: &[String],
) -> Result<(), sqlx::Error> {
    let now = now_iso();
    let rc_persona = recognition_ctx.persona.clone().unwrap_or_default();
    let rc_session_id = recognition_ctx.session_id.clone();
    let rc_focus_wing = recognition_ctx.focus_wing.clone();
    // Top-level session_id is NOT NULL; the recognition-context session is the
    // authoritative source, empty string only if a caller recalls session-less.
    let session_id = rc_session_id.clone().unwrap_or_default();
    let injected_json = serde_json::Value::Array(
        injected_memory_ids
            .iter()
            .cloned()
            .map(serde_json::Value::String)
            .collect(),
    )
    .to_string();

    let mut tx = pool.begin().await?;

    sqlx::query(
        "INSERT INTO recognition_events
            (retrieval_id, session_id, query, retrieved_at,
             rc_persona, rc_session_id, rc_focus_wing, strategy,
             injected_memory_ids, injected_memory_ids_source)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'recorded')",
    )
    .bind(retrieval_id)
    .bind(&session_id)
    .bind(query)
    .bind(&now)
    .bind(&rc_persona)
    .bind(&rc_session_id)
    .bind(&rc_focus_wing)
    .bind(strategy)
    .bind(&injected_json)
    .execute(&mut *tx)
    .await?;

    for (memory_id, signal_score, rank) in members {
        sqlx::query(
            "INSERT OR IGNORE INTO recognition_set_members
                (retrieval_id, memory_id, signal_score, rank)
             VALUES (?, ?, ?, ?)",
        )
        .bind(retrieval_id)
        .bind(memory_id)
        .bind(signal_score)
        .bind(rank)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    debug!(
        target: "permagent::recognition",
        "Persisted recognition event {} ({} members, strategy={})",
        retrieval_id,
        members.len(),
        strategy
    );
    Ok(())
}

#[cfg(test)]
async fn persist_recognition(
    pool: &Pool<Sqlite>,
    recognition_ctx: &spectral::graph::RecognitionContext,
    query: &str,
    strategy: &str,
    members: &[SetMember],
) -> Result<(), sqlx::Error> {
    persist_recognition_with_id(
        pool,
        &Uuid::now_v7().to_string(),
        recognition_ctx,
        query,
        strategy,
        members,
        &[],
    )
    .await
}

impl PendingRecognition {
    /// After the assistant reply ends, conservatively detect exact normalized
    /// five-word overlap with injected memory content and persist the cited ids.
    /// The join and overlap scan both run in this detached task, never inline on
    /// the reply path. Failures are logged and never propagated.
    pub fn spawn_record_reply_usage(self, assistant_reply: String) {
        if assistant_reply.trim().is_empty() {
            debug!(
                target: "permagent::recognition",
                "Skipping citation write-back for retrieval {}: assistant reply is empty",
                self.retrieval_id
            );
            return;
        }

        tokio::spawn(async move {
            let mut persisted = self.persisted;
            if let Err(e) = persisted.wait_for(|done| *done).await {
                warn!(
                    target: "permagent::recognition",
                    "Recognition persistence task failed before citation write-back: {}",
                    e
                );
                return;
            }

            let cited = cited_memories_by_content_overlap(&self.injected, &assistant_reply);
            if let Err(e) = record_reply_usage(&self.pool, &self.retrieval_id, &cited).await {
                warn!(
                    target: "permagent::recognition",
                    "Failed to persist cited memories for retrieval {}: {}",
                    self.retrieval_id,
                    e
                );
            }
        });
    }
}

const CITATION_SHINGLE_WORDS: usize = 5;
const CITATION_SHINGLE_MIN_CHARS: usize = 24;

fn normalized_words(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(|word| word.to_lowercase())
        .collect()
}

/// Which injected memories the reply actually drew on, by content overlap.
///
/// Public so the `Brain::turn` outcome path reports usage by the SAME rule the
/// recognition write-back already uses. Two different definitions of "used"
/// would make the turn corpus incomparable with the recognition data it is
/// meant to be validated against.
pub fn cited_memories_by_content_overlap(
    injected: &[InjectedMemory],
    assistant_reply: &str,
) -> Vec<String> {
    let reply_words = normalized_words(assistant_reply);
    let reply_shingles: HashSet<String> = reply_words
        .windows(CITATION_SHINGLE_WORDS)
        .map(|words| words.join(" "))
        .filter(|shingle| shingle.len() >= CITATION_SHINGLE_MIN_CHARS)
        .collect();

    if reply_shingles.is_empty() {
        return Vec::new();
    }

    injected
        .iter()
        .filter(|memory| {
            normalized_words(&memory.content)
                .windows(CITATION_SHINGLE_WORDS)
                .map(|words| words.join(" "))
                .any(|shingle| {
                    shingle.len() >= CITATION_SHINGLE_MIN_CHARS && reply_shingles.contains(&shingle)
                })
        })
        .map(|memory| memory.id.clone())
        .collect()
}

async fn record_reply_usage(
    pool: &Pool<Sqlite>,
    retrieval_id: &str,
    cited_memory_ids: &[String],
) -> Result<(), sqlx::Error> {
    let cited_json = serde_json::Value::Array(
        cited_memory_ids
            .iter()
            .cloned()
            .map(serde_json::Value::String)
            .collect(),
    )
    .to_string();
    let checked_at = now_iso();
    sqlx::query(
        "UPDATE recognition_events
            SET cited_memory_ids = ?,
                citation_checked_at = ?,
                outcome_label = CASE
                    WHEN outcome_polarity = 'Negative' THEN 'wrong'
                    WHEN outcome_polarity = 'Positive' AND ? THEN 'useful'
                    WHEN outcome_polarity = 'Positive' AND EXISTS (
                        SELECT 1 FROM recognition_set_members members
                         WHERE members.retrieval_id = recognition_events.retrieval_id
                    ) THEN 'ignored'
                    ELSE outcome_label
                END
          WHERE retrieval_id = ?",
    )
    .bind(cited_json)
    .bind(checked_at)
    .bind(!cited_memory_ids.is_empty())
    .bind(retrieval_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Write back a TASK-resolution outcome onto the still-unattributed recognition
/// events for a session (PRIMARY proxy). Attribute-by-key-later: every recall in
/// the session whose outcome is still NULL is resolved as `TaskResolved` /
/// `Positive`. Best-effort; no-op when the session has no open recognition rows.
pub async fn write_back_task_outcome(pool: &Pool<Sqlite>, session_id: &str) {
    if session_id.is_empty() {
        return;
    }
    write_back_outcome(pool, session_id, "TaskResolved", "Positive", "Task").await;
}

/// Write back a FAILED-task outcome (the negative half of the primary proxy).
///
/// Until this existed, every tool call — including the ones that errored — was
/// logged as a completed task, so [`write_back_task_outcome`] stamped
/// `Positive` across the session's recalls unconditionally and `outcome_label`
/// could essentially never become `wrong`. The recall-quality ground truth was
/// biased to success by construction, which makes it useless as a training or
/// evaluation signal. A tool call that failed is exactly the case where the
/// recalls that led into it deserve a negative label.
pub async fn write_back_task_failure(pool: &Pool<Sqlite>, session_id: &str) {
    if session_id.is_empty() {
        return;
    }
    write_back_outcome(pool, session_id, "TaskFailed", "Negative", "Task").await;
}

/// Write back a DECISION outcome (SECONDARY proxy) via the 2-hop join
/// `goal_id` → `cards.metadata_json.worker_session_id` → recognition events.
/// `approved` selects `DecisionApproved`/`Positive` vs `DecisionBounced`/
/// `Negative`. ~0% volume until the orchestrator is enabled. Best-effort.
pub async fn write_back_decision_outcome(pool: &Pool<Sqlite>, goal_id: &str, approved: bool) {
    let session_id = match resolve_worker_session_id(pool, goal_id).await {
        Some(sid) if !sid.is_empty() => sid,
        _ => {
            debug!(
                target: "permagent::recognition",
                "No worker_session_id for goal {}; skipping decision write-back",
                goal_id
            );
            return;
        }
    };
    let (kind, polarity) = if approved {
        ("DecisionApproved", "Positive")
    } else {
        ("DecisionBounced", "Negative")
    };
    write_back_outcome(pool, &session_id, kind, polarity, "Decision").await;
}

/// Mark an observation (an exact normalized command) as bounced so the
/// Initiative gate prunes it and never re-pitches it. Inserts a minimal
/// `Negative` `recognition_events` row keyed on `query = normalized`; because
/// [`seen_observation`] reads the MOST RECENT row for a query, this fresh row is
/// what [`RecognitionSeen::was_bounced`] then reports.
///
/// This is the direct-by-query counterpart to [`write_back_decision_outcome`]:
/// an initiative automation proposal has no worker session to 2-hop through
/// (nothing ran), so the decline records the bounce straight against the
/// observation. Called when a user declines an automation proposal on the
/// Decision Inbox. Best-effort — failures are logged, never propagated.
pub async fn mark_observation_bounced(pool: &Pool<Sqlite>, normalized: &str) {
    mark_observation_bounced_in_lane(pool, INITIATIVE_LANE, normalized).await
}

/// The lane the Initiative layer's declines are recorded under — the original
/// (and still the only pre-existing) caller of [`mark_observation_bounced`].
pub const INITIATIVE_LANE: &str = "initiative";

/// The same bounce, recorded under a named `lane`.
///
/// One decline mechanism, several surfaces: the Initiative layer bounces a
/// normalized command, the Council bounces a namespaced action key. The lane
/// becomes the row's synthetic `session_id`, its `strategy`, and its
/// `retrieval_id` prefix, so per-lane rows stay separable in the recognition
/// tables instead of every surface masquerading as the initiative loop.
/// `mark_observation_bounced` is `lane = "initiative"` and is byte-identical to
/// what it wrote before. Best-effort — failures are logged, never propagated.
pub async fn mark_observation_bounced_in_lane(pool: &Pool<Sqlite>, lane: &str, normalized: &str) {
    if normalized.is_empty() {
        return;
    }
    let lane = if lane.is_empty() {
        INITIATIVE_LANE
    } else {
        lane
    };
    let now = now_iso();
    let retrieval_id = format!("{lane}-decline:{}", Uuid::now_v7());
    let result = sqlx::query(
        "INSERT INTO recognition_events
            (retrieval_id, session_id, query, retrieved_at, rc_persona, strategy,
             outcome_kind, outcome_polarity, outcome_source, outcome_observed_at,
             outcome_label)
         VALUES (?, ?, ?, ?, 'henry', ?,
                 'DecisionBounced', 'Negative', 'Decision', ?, 'wrong')",
    )
    .bind(&retrieval_id)
    .bind(lane)
    .bind(normalized)
    .bind(&now)
    .bind(lane)
    .bind(&now)
    .execute(pool)
    .await;

    match result {
        Ok(_) => debug!(
            target: "permagent::recognition",
            "Marked observation bounced ({} decline): {}", lane, normalized
        ),
        Err(e) => warn!(
            target: "permagent::recognition",
            "Failed to mark observation bounced for '{}': {}", normalized, e
        ),
    }
}

/// Resolve a goal's worker session id from `cards.metadata_json`, preferring the
/// current `worker_session_id`, then the most recent of `worker_session_ids`.
async fn resolve_worker_session_id(pool: &Pool<Sqlite>, goal_id: &str) -> Option<String> {
    let meta_json: String = sqlx::query_scalar("SELECT metadata_json FROM cards WHERE id = ?")
        .bind(goal_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()?;
    let meta: serde_json::Value = serde_json::from_str(&meta_json).ok()?;
    if let Some(sid) = meta.get("worker_session_id").and_then(|v| v.as_str()) {
        return Some(sid.to_string());
    }
    meta.get("worker_session_ids")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.iter().rev().find_map(|v| v.as_str()))
        .map(String::from)
}

async fn write_back_outcome(
    pool: &Pool<Sqlite>,
    session_id: &str,
    kind: &str,
    polarity: &str,
    source: &str,
) {
    let now = now_iso();
    let result = sqlx::query(
        "UPDATE recognition_events
            SET outcome_kind = ?, outcome_polarity = ?, outcome_source = ?, outcome_observed_at = ?,
                outcome_label = CASE
                    WHEN ? = 'Negative' THEN 'wrong'
                    WHEN ? = 'Positive' AND citation_checked_at IS NOT NULL
                         AND cited_memory_ids <> '[]' THEN 'useful'
                    WHEN ? = 'Positive' AND citation_checked_at IS NOT NULL AND EXISTS (
                        SELECT 1 FROM recognition_set_members members
                         WHERE members.retrieval_id = recognition_events.retrieval_id
                    ) THEN 'ignored'
                    ELSE outcome_label
                END
          WHERE session_id = ? AND outcome_kind IS NULL",
    )
    .bind(kind)
    .bind(polarity)
    .bind(source)
    .bind(&now)
    .bind(polarity)
    .bind(polarity)
    .bind(polarity)
    .bind(session_id)
    .execute(pool)
    .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => {
            debug!(
                target: "permagent::recognition",
                "Wrote back {} outcome to {} recognition event(s) for session {}",
                kind,
                r.rows_affected(),
                session_id
            );
        }
        Ok(_) => {}
        Err(e) => {
            warn!(
                target: "permagent::recognition",
                "Failed to write back {} outcome for session {}: {}",
                kind,
                session_id,
                e
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Recognition verdicts (schema v22).
//
// `record_verdict` writes a recognize() verdict NEXT TO the outcome columns —
// the pairing that makes this table the recognition validation ground truth.
// Real verdicts flow here from `crate::recognition_sink::observe_recall_stimulus`,
// which calls `SafeBrain::recognize` alongside every recall (feature
// `spectral-recognition`). `spawn_log_tool_event` feeds the
// `recognition_tool_events` stream (the path-pursuit tracker input); its only
// caller is feature-gated. Both are best-effort, matching everything else in
// this module.
//
// Ordering matters: both writes are UPDATEs keyed on `retrieval_id`, and the
// row is INSERTed by a detached task. Go through [`VerdictWriteHandle`] rather
// than calling `record_verdict` directly on a recall that is still in flight.
// ---------------------------------------------------------------------------

/// Permission to attach a recognize() verdict to one in-flight recall,
/// chained behind that recall's own `recognition_events` INSERT.
///
/// Cheap to clone and to hold: the recognize() call it accompanies runs
/// detached, so nothing on the reply path waits for either.
#[derive(Clone)]
pub struct VerdictWriteHandle {
    pool: Pool<Sqlite>,
    retrieval_id: String,
    persisted: tokio::sync::watch::Receiver<bool>,
}

impl VerdictWriteHandle {
    /// The `recognition_events` row this verdict will be written onto.
    pub fn retrieval_id(&self) -> &str {
        &self.retrieval_id
    }

    /// Wait for the recall row to exist, then record the verdict on it.
    /// Best-effort throughout: a dead persistence task or a failed UPDATE is
    /// logged and dropped, never propagated.
    pub async fn record(self, verdict: &str, familiarity: f64) {
        let mut persisted = self.persisted;
        if let Err(e) = persisted.wait_for(|done| *done).await {
            warn!(
                target: "permagent::recognition",
                "Recognition persistence task failed before verdict write-back: {}",
                e
            );
            return;
        }
        record_verdict(&self.pool, &self.retrieval_id, verdict, familiarity).await;
    }
}

impl PendingRecognition {
    /// A handle for writing this recall's recognize() verdict, correctly
    /// ordered against the row's INSERT. Taken by reference: the caller keeps
    /// the `PendingRecognition` for turn-end citation detection.
    pub fn verdict_handle(&self) -> VerdictWriteHandle {
        VerdictWriteHandle {
            pool: self.pool.clone(),
            retrieval_id: self.retrieval_id.clone(),
            persisted: self.persisted.clone(),
        }
    }
}

/// Record a recognition verdict + familiarity on an already-persisted recall
/// event, keyed on `retrieval_id` (schema v22 columns). Best-effort.
pub async fn record_verdict(
    pool: &Pool<Sqlite>,
    retrieval_id: &str,
    verdict: &str,
    familiarity: f64,
) {
    let result = sqlx::query(
        "UPDATE recognition_events
            SET recognition_verdict = ?, familiarity = ?
          WHERE retrieval_id = ?",
    )
    .bind(verdict)
    .bind(familiarity)
    .bind(retrieval_id)
    .execute(pool)
    .await;

    if let Err(e) = result {
        warn!(
            target: "permagent::recognition",
            "Failed to record verdict for retrieval {}: {}",
            retrieval_id,
            e
        );
    }
}

/// Persist one timestamped tool-call event into `recognition_tool_events`,
/// fire-and-forget. Content-free by design: tool name, wing, coarse
/// args-class (the argument SHAPE hash, never argument values), session.
pub fn spawn_log_tool_event(
    pool: Pool<Sqlite>,
    tool_name: String,
    wing: Option<String>,
    args_class: Option<String>,
    session_id: Option<String>,
) {
    tokio::spawn(async move {
        let result = sqlx::query(
            "INSERT INTO recognition_tool_events
                (occurred_at, tool_name, wing, args_class, session_id)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(now_iso())
        .bind(&tool_name)
        .bind(&wing)
        .bind(&args_class)
        .bind(&session_id)
        .execute(&pool)
        .await;

        if let Err(e) = result {
            warn!(
                target: "permagent::recognition",
                "Failed to log tool event {}: {}",
                tool_name,
                e
            );
        }
    });
}

// ---------------------------------------------------------------------------
// Read side (the #360 initiative-layer glue).
//
// These are the ONLY query accessors over `recognition_events`. They expose the
// novelty + frequency signals the ambient goal-origination loop uses to prune
// timing and suppress already-declined proposals. Both are best-effort: a read
// failure degrades to "no signal" (count 0 / None), never an error path.
// ---------------------------------------------------------------------------

/// A prior recognition of an observation: when it was last seen and how it
/// resolved. `outcome_*` stay `None` until a write-back attributes the row.
#[derive(Debug, Clone)]
pub struct RecognitionSeen {
    pub retrieved_at: String,
    pub outcome_kind: Option<String>,
    pub outcome_polarity: Option<String>,
}

impl RecognitionSeen {
    /// True when a prior proposal for this observation was declined — the
    /// caller should suppress re-origination (the quality half of the flywheel).
    pub fn was_bounced(&self) -> bool {
        self.outcome_polarity.as_deref() == Some("Negative")
    }
}

/// Count recognition events for a session within the last `within_secs` (the
/// frequency signal). `retrieved_at` is fixed-format UTC ISO-8601, which sorts
/// lexically as chronologically, so a string lower-bound is exact. Returns 0
/// on empty session or any error.
pub async fn recent_recognition_count(
    pool: &Pool<Sqlite>,
    session_id: &str,
    within_secs: i64,
) -> i64 {
    if session_id.is_empty() {
        return 0;
    }
    let cutoff = (Utc::now() - chrono::Duration::seconds(within_secs.max(0)))
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM recognition_events
          WHERE session_id = ? AND retrieved_at >= ?",
    )
    .bind(session_id)
    .bind(&cutoff)
    .fetch_one(pool)
    .await
    .unwrap_or(0)
}

/// Look up whether an observation (exact `query` string) has been recognized
/// before, returning the MOST RECENT prior occurrence (the novelty + pruning
/// signal). `None` = never seen → novel, ripe to originate. A returned row with
/// `was_bounced()` means a prior proposal was declined. `None` on error.
pub async fn seen_observation(pool: &Pool<Sqlite>, query: &str) -> Option<RecognitionSeen> {
    use sqlx::Row;
    let row = sqlx::query(
        "SELECT retrieved_at, outcome_kind, outcome_polarity
           FROM recognition_events
          WHERE query = ?
          ORDER BY retrieved_at DESC
          LIMIT 1",
    )
    .bind(query)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()?;
    Some(RecognitionSeen {
        retrieved_at: row.get("retrieved_at"),
        outcome_kind: row.get("outcome_kind"),
        outcome_polarity: row.get("outcome_polarity"),
    })
}

/// How long recognition instrumentation is retained before pruning.
pub const RECOGNITION_RETENTION_DAYS: i64 = 90;

/// Delete recognition instrumentation older than `retention_days`.
///
/// `recognition_events` and `recognition_tool_events` are append-only
/// instrumentation written on every recall / tool call, and nothing else ever
/// deletes from them — so without a pruner they grow unbounded on a busy hub.
/// `recognition_set_members` is removed automatically via its
/// `ON DELETE CASCADE` FK on `recognition_events`. Returns the number of parent
/// rows deleted. Best-effort maintenance.
pub async fn prune_recognition_instrumentation(
    pool: &Pool<Sqlite>,
    retention_days: i64,
) -> Result<u64, sqlx::Error> {
    // Cutoff formatted in Rust in the SAME fixed-width UTC ISO-8601 shape the
    // rows are written with (see `now_iso`), so the lexical `<` compare is a
    // chronological compare — matching the existing `retrieved_at` queries.
    let cutoff = (Utc::now() - chrono::Duration::days(retention_days.max(0)))
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();
    let events = sqlx::query("DELETE FROM recognition_events WHERE retrieved_at < ?")
        .bind(&cutoff)
        .execute(pool)
        .await?
        .rows_affected();
    let tool_events = sqlx::query("DELETE FROM recognition_tool_events WHERE occurred_at < ?")
        .bind(&cutoff)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(events + tool_events)
}

/// Periodic pruner: prunes once shortly after boot (the first `interval` tick
/// fires immediately) and then daily, keeping [`RECOGNITION_RETENTION_DAYS`].
pub async fn recognition_prune_loop(pool: Pool<Sqlite>) {
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(24 * 60 * 60));
    loop {
        ticker.tick().await;
        match prune_recognition_instrumentation(&pool, RECOGNITION_RETENTION_DAYS).await {
            Ok(n) if n > 0 => tracing::info!(
                target: "recognition",
                "pruned {n} recognition instrumentation rows older than {RECOGNITION_RETENTION_DAYS}d"
            ),
            Ok(_) => {}
            Err(e) => tracing::warn!(target: "recognition", "recognition prune failed: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Row;

    /// A `persisted` signal that is already flipped — these tests INSERT the
    /// row themselves, so there is no detached write to wait behind.
    ///
    /// Dropping the sender here is deliberate and safe: `wait_for` evaluates
    /// the predicate before it looks at whether the channel closed, so an
    /// already-satisfied signal returns immediately either way. A sender that
    /// closes while the value is still `false` is the real failure case, and
    /// that one does report an error.
    fn already_persisted() -> tokio::sync::watch::Receiver<bool> {
        tokio::sync::watch::channel(true).1
    }

    async fn test_pool() -> Pool<Sqlite> {
        use crate::session::spectral_schema::init_spectral_db;
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        pool
    }

    fn ctx(session: &str) -> spectral::graph::RecognitionContext {
        spectral::graph::RecognitionContext::empty()
            .with_persona("henry")
            .with_session(session)
            .with_focus_wing("permagent")
    }

    #[tokio::test]
    async fn mark_observation_bounced_makes_it_seen_as_bounced() {
        let pool = test_pool().await;
        let cmd = "git status && git pull";
        // Never seen → novel.
        assert!(seen_observation(&pool, cmd).await.is_none());

        mark_observation_bounced(&pool, cmd).await;

        let seen = seen_observation(&pool, cmd)
            .await
            .expect("row now exists for the observation");
        assert!(seen.was_bounced(), "declined observation reads as bounced");
        let label: Option<String> =
            sqlx::query_scalar("SELECT outcome_label FROM recognition_events WHERE query = ?")
                .bind(cmd)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(label.as_deref(), Some("wrong"));
        // Empty command is a no-op (defensive).
        mark_observation_bounced(&pool, "").await;
    }

    #[tokio::test]
    async fn prune_removes_old_instrumentation_and_cascades() {
        let pool = test_pool().await;
        let old = "2000-01-01T00:00:00.000Z";

        // An old event, an old set member (should cascade), an old tool event.
        sqlx::query(
            "INSERT INTO recognition_events
                (retrieval_id, session_id, query, retrieved_at, rc_persona, strategy)
             VALUES ('old-1', 'sess', 'q', ?, 'henry', 'cascade')",
        )
        .bind(old)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO recognition_set_members (retrieval_id, memory_id, signal_score, rank)
             VALUES ('old-1', 'mem-a', 0.9, 0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO recognition_tool_events (occurred_at, tool_name, session_id)
             VALUES (?, 'do_thing', 'sess')",
        )
        .bind(old)
        .execute(&pool)
        .await
        .unwrap();

        // A fresh event via the normal path (retrieved_at = now).
        persist_recognition(
            &pool,
            &ctx("sess"),
            "recent",
            "cascade",
            &[("mem-b".into(), 0.8, 0)],
        )
        .await
        .unwrap();

        let deleted = prune_recognition_instrumentation(&pool, RECOGNITION_RETENTION_DAYS)
            .await
            .unwrap();
        assert_eq!(deleted, 2, "one old event + one old tool event pruned");

        let events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM recognition_events")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(events, 1, "only the fresh event remains");
        let members: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM recognition_set_members WHERE retrieval_id = 'old-1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(members, 0, "old set members cascaded away");
        let tool: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM recognition_tool_events")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(tool, 0, "old tool event pruned");
    }

    #[tokio::test]
    async fn persists_event_and_members_unconditionally() {
        let pool = test_pool().await;
        persist_recognition(
            &pool,
            &ctx("sess-1"),
            "how do I configure voice?",
            "cascade",
            &[("mem-a".into(), 0.9, 0), ("mem-b".into(), 0.8, 1)],
        )
        .await
        .unwrap();

        let row = sqlx::query(
            "SELECT retrieval_id, session_id, query, rc_persona, rc_focus_wing, strategy,
                    outcome_kind, cited_memory_ids, injected_memory_ids,
                    injected_memory_ids_source, citation_checked_at
               FROM recognition_events",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let retrieval_id: String = row.get("retrieval_id");
        assert!(!retrieval_id.is_empty(), "retrieval_id minted");
        assert_eq!(row.get::<String, _>("session_id"), "sess-1");
        assert_eq!(row.get::<String, _>("rc_persona"), "henry");
        assert_eq!(row.get::<String, _>("rc_focus_wing"), "permagent");
        assert_eq!(row.get::<String, _>("strategy"), "cascade");
        assert!(row.get::<Option<String>, _>("outcome_kind").is_none());
        assert_eq!(row.get::<String, _>("cited_memory_ids"), "[]");
        assert_eq!(row.get::<String, _>("injected_memory_ids"), "[]");
        assert_eq!(
            row.get::<Option<String>, _>("injected_memory_ids_source")
                .as_deref(),
            Some("recorded")
        );
        assert!(row
            .get::<Option<String>, _>("citation_checked_at")
            .is_none());

        let members: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM recognition_set_members WHERE retrieval_id = ?",
        )
        .bind(&retrieval_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(members, 2, "whole retrieved set persisted");
    }

    #[tokio::test]
    async fn persists_empty_set() {
        let pool = test_pool().await;
        persist_recognition(&pool, &ctx("sess-empty"), "q", "cascade", &[])
            .await
            .unwrap();
        let events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM recognition_events")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(events, 1, "event persists even with no hits");
    }

    #[tokio::test]
    async fn task_write_back_keys_on_session() {
        let pool = test_pool().await;
        persist_recognition(
            &pool,
            &ctx("sess-x"),
            "q",
            "cascade",
            &[("m".into(), 0.9, 0)],
        )
        .await
        .unwrap();
        // Unrelated session must stay untouched.
        persist_recognition(&pool, &ctx("sess-y"), "q2", "cascade", &[])
            .await
            .unwrap();

        write_back_task_outcome(&pool, "sess-x").await;

        let kind: Option<String> = sqlx::query_scalar(
            "SELECT outcome_kind FROM recognition_events WHERE session_id = 'sess-x'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(kind.as_deref(), Some("TaskResolved"));
        let label: Option<String> = sqlx::query_scalar(
            "SELECT outcome_label FROM recognition_events WHERE session_id = 'sess-x'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(
            label.is_none(),
            "positive outcome alone is not evidence that recall was ignored"
        );

        let retrieval_id: String = sqlx::query_scalar(
            "SELECT retrieval_id FROM recognition_events WHERE session_id = 'sess-x'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        record_reply_usage(&pool, &retrieval_id, &[]).await.unwrap();
        let measured = sqlx::query(
            "SELECT outcome_label, citation_checked_at FROM recognition_events
              WHERE session_id = 'sess-x'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            measured
                .get::<Option<String>, _>("outcome_label")
                .as_deref(),
            Some("ignored")
        );
        assert!(measured
            .get::<Option<String>, _>("citation_checked_at")
            .is_some());

        let untouched: Option<String> = sqlx::query_scalar(
            "SELECT outcome_kind FROM recognition_events WHERE session_id = 'sess-y'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(untouched.is_none(), "other sessions untouched");
    }

    #[tokio::test]
    async fn decision_write_back_resolves_2hop_join() {
        let pool = test_pool().await;
        let worker_session = "worker-sess-1";
        persist_recognition(&pool, &ctx(worker_session), "q", "cascade", &[])
            .await
            .unwrap();

        // Seed a goal card whose metadata carries the worker session id.
        sqlx::query(
            "INSERT INTO cards (id, project_id, card_type, title, column_id, metadata_json)
             VALUES ('goal-1', '00000000-0000-0000-0000-000000000001', 'goal', 'G',
                     'col-personal-backlog', ?)",
        )
        .bind(serde_json::json!({ "worker_session_id": worker_session }).to_string())
        .execute(&pool)
        .await
        .unwrap();

        write_back_decision_outcome(&pool, "goal-1", false).await;

        let row = sqlx::query(
            "SELECT outcome_kind, outcome_polarity, outcome_source, outcome_label
               FROM recognition_events WHERE session_id = ?",
        )
        .bind(worker_session)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            row.get::<Option<String>, _>("outcome_kind").as_deref(),
            Some("DecisionBounced")
        );
        assert_eq!(
            row.get::<Option<String>, _>("outcome_polarity").as_deref(),
            Some("Negative")
        );
        assert_eq!(
            row.get::<Option<String>, _>("outcome_source").as_deref(),
            Some("Decision")
        );
        assert_eq!(
            row.get::<Option<String>, _>("outcome_label").as_deref(),
            Some("wrong")
        );
    }

    #[test]
    fn citation_detection_requires_distinctive_exact_overlap() {
        let injected = vec![
            InjectedMemory {
                id: "used".into(),
                content: "The deployment token rotates every Tuesday at noon UTC.".into(),
            },
            InjectedMemory {
                id: "unused".into(),
                content: "The garden irrigation timer starts before sunrise each day.".into(),
            },
        ];
        assert_eq!(
            cited_memories_by_content_overlap(
                &injected,
                "Remember that the deployment token rotates every Tuesday at noon UTC."
            ),
            vec!["used"]
        );
        assert!(
            cited_memories_by_content_overlap(&injected, "The token rotates weekly.").is_empty(),
            "semantic guesses and short overlap are deliberately rejected"
        );
    }

    #[tokio::test]
    async fn empty_reply_leaves_usage_unmeasured() {
        let pool = test_pool().await;
        let retrieval_id = "empty-reply";
        persist_recognition_with_id(
            &pool,
            retrieval_id,
            &ctx("empty-reply"),
            "q",
            "cascade",
            &[],
            &[],
        )
        .await
        .unwrap();
        sqlx::query(
            "UPDATE recognition_events
                SET cited_memory_ids = '[\"prior\"]'
              WHERE retrieval_id = ?",
        )
        .bind(retrieval_id)
        .execute(&pool)
        .await
        .unwrap();

        PendingRecognition {
            pool: pool.clone(),
            retrieval_id: retrieval_id.into(),
            injected: Arc::from([]),
            persisted: already_persisted(),
        }
        .spawn_record_reply_usage(String::new());

        let row = sqlx::query(
            "SELECT cited_memory_ids, citation_checked_at
               FROM recognition_events WHERE retrieval_id = ?",
        )
        .bind(retrieval_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.get::<String, _>("cited_memory_ids"), "[\"prior\"]");
        assert!(row
            .get::<Option<String>, _>("citation_checked_at")
            .is_none());
    }

    #[tokio::test]
    async fn short_reply_records_measured_zero() {
        let pool = test_pool().await;
        let retrieval_id = "short-reply";
        persist_recognition_with_id(
            &pool,
            retrieval_id,
            &ctx("short-reply"),
            "q",
            "cascade",
            &[],
            &[],
        )
        .await
        .unwrap();
        sqlx::query(
            "UPDATE recognition_events
                SET cited_memory_ids = '[\"prior\"]'
              WHERE retrieval_id = ?",
        )
        .bind(retrieval_id)
        .execute(&pool)
        .await
        .unwrap();

        PendingRecognition {
            pool: pool.clone(),
            retrieval_id: retrieval_id.into(),
            injected: Arc::from([]),
            persisted: already_persisted(),
        }
        .spawn_record_reply_usage("Yes.".into());

        let row = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                let row = sqlx::query(
                    "SELECT cited_memory_ids, citation_checked_at
                       FROM recognition_events WHERE retrieval_id = ?",
                )
                .bind(retrieval_id)
                .fetch_one(&pool)
                .await
                .unwrap();
                if row
                    .get::<Option<String>, _>("citation_checked_at")
                    .is_some()
                {
                    break row;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("citation write-back should complete");
        assert_eq!(row.get::<String, _>("cited_memory_ids"), "[]");
        assert!(row
            .get::<Option<String>, _>("citation_checked_at")
            .is_some());
    }

    #[tokio::test]
    async fn citation_and_positive_outcome_derive_useful_in_either_order() {
        let pool = test_pool().await;
        for session in ["citation-first", "outcome-first"] {
            persist_recognition(
                &pool,
                &ctx(session),
                "q",
                "cascade",
                &[("m1".into(), 0.9, 0)],
            )
            .await
            .unwrap();
        }
        let citation_first: String = sqlx::query_scalar(
            "SELECT retrieval_id FROM recognition_events WHERE session_id = 'citation-first'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        record_reply_usage(&pool, &citation_first, &["m1".into()])
            .await
            .unwrap();
        write_back_task_outcome(&pool, "citation-first").await;

        let outcome_first: String = sqlx::query_scalar(
            "SELECT retrieval_id FROM recognition_events WHERE session_id = 'outcome-first'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        write_back_task_outcome(&pool, "outcome-first").await;
        record_reply_usage(&pool, &outcome_first, &["m1".into()])
            .await
            .unwrap();

        let labels: Vec<String> = sqlx::query_scalar(
            "SELECT outcome_label FROM recognition_events
              WHERE citation_checked_at IS NOT NULL ORDER BY session_id",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(labels, vec!["useful", "useful"]);
    }

    // ----- read side (#360 glue) -----

    #[tokio::test]
    async fn seen_observation_novel_is_none() {
        let pool = test_pool().await;
        assert!(
            seen_observation(&pool, "git status && git pull")
                .await
                .is_none(),
            "an unseen observation is novel"
        );
    }

    #[tokio::test]
    async fn seen_observation_returns_positive_outcome() {
        let pool = test_pool().await;
        let q = "git status && git pull --ff-only";
        persist_recognition(&pool, &ctx("sess-pos"), q, "cascade", &[])
            .await
            .unwrap();
        write_back_task_outcome(&pool, "sess-pos").await;

        let seen = seen_observation(&pool, q).await.expect("observation seen");
        assert_eq!(seen.outcome_kind.as_deref(), Some("TaskResolved"));
        assert_eq!(seen.outcome_polarity.as_deref(), Some("Positive"));
        assert!(!seen.was_bounced(), "positive outcome is not a bounce");
    }

    #[tokio::test]
    async fn seen_observation_flags_bounced() {
        let pool = test_pool().await;
        let worker_session = "worker-sess-bounce";
        let q = "npm run build:all";
        persist_recognition(&pool, &ctx(worker_session), q, "cascade", &[])
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO cards (id, project_id, card_type, title, column_id, metadata_json)
             VALUES ('goal-b', '00000000-0000-0000-0000-000000000001', 'goal', 'G',
                     'col-personal-backlog', ?)",
        )
        .bind(serde_json::json!({ "worker_session_id": worker_session }).to_string())
        .execute(&pool)
        .await
        .unwrap();
        write_back_decision_outcome(&pool, "goal-b", false).await;

        let seen = seen_observation(&pool, q).await.expect("observation seen");
        assert!(seen.was_bounced(), "declined proposal must read as bounced");
    }

    #[tokio::test]
    async fn seen_observation_returns_most_recent() {
        let pool = test_pool().await;
        let q = "cargo test -p permagent";
        // An older row (resolved) then a newer one (still open).
        sqlx::query(
            "INSERT INTO recognition_events
                (retrieval_id, session_id, query, retrieved_at, rc_persona, strategy, outcome_kind)
             VALUES ('r-old', 'sess-1', ?, '2020-01-01T00:00:00.000Z', '', 'cascade', 'TaskResolved')",
        )
        .bind(q)
        .execute(&pool)
        .await
        .unwrap();
        persist_recognition(&pool, &ctx("sess-1"), q, "cascade", &[])
            .await
            .unwrap();

        let seen = seen_observation(&pool, q).await.expect("seen");
        assert!(
            seen.outcome_kind.is_none(),
            "most-recent row wins (the still-open one)"
        );
    }

    #[tokio::test]
    async fn recent_count_windows_by_time() {
        let pool = test_pool().await;
        // Two fresh events in the session.
        persist_recognition(&pool, &ctx("sess-c"), "q1", "cascade", &[])
            .await
            .unwrap();
        persist_recognition(&pool, &ctx("sess-c"), "q2", "cascade", &[])
            .await
            .unwrap();
        // One ancient event in the same session, outside any sane window.
        sqlx::query(
            "INSERT INTO recognition_events
                (retrieval_id, session_id, query, retrieved_at, rc_persona, strategy)
             VALUES ('r-ancient', 'sess-c', 'q-old', '2020-01-01T00:00:00.000Z', '', 'cascade')",
        )
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(
            recent_recognition_count(&pool, "sess-c", 3600).await,
            2,
            "only the two fresh events fall inside the hour window"
        );
        assert_eq!(
            recent_recognition_count(&pool, "other", 3600).await,
            0,
            "unrelated session has none"
        );
        assert_eq!(
            recent_recognition_count(&pool, "", 3600).await,
            0,
            "empty session short-circuits"
        );
    }

    // ----- spectral-recognition prep (schema v22) -----

    #[tokio::test]
    async fn verdict_records_next_to_outcome() {
        let pool = test_pool().await;
        persist_recognition(&pool, &ctx("sess-v"), "q", "cascade", &[])
            .await
            .unwrap();
        let retrieval_id: String =
            sqlx::query_scalar("SELECT retrieval_id FROM recognition_events")
                .fetch_one(&pool)
                .await
                .unwrap();

        // Fresh init leaves the v22 columns NULL.
        let row = sqlx::query("SELECT recognition_verdict, familiarity FROM recognition_events")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(row
            .get::<Option<String>, _>("recognition_verdict")
            .is_none());
        assert!(row.get::<Option<f64>, _>("familiarity").is_none());

        // A verdict and an outcome coexist on the same row — the ground-truth
        // pairing the table exists for.
        record_verdict(&pool, &retrieval_id, "familiar", 0.62).await;
        write_back_task_outcome(&pool, "sess-v").await;

        let row = sqlx::query(
            "SELECT recognition_verdict, familiarity, outcome_kind FROM recognition_events",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            row.get::<Option<String>, _>("recognition_verdict")
                .as_deref(),
            Some("familiar")
        );
        assert_eq!(row.get::<Option<f64>, _>("familiarity"), Some(0.62));
        assert_eq!(
            row.get::<Option<String>, _>("outcome_kind").as_deref(),
            Some("TaskResolved")
        );
    }

    /// The verdict write is an UPDATE and the row it targets is INSERTed by a
    /// detached task, so the two can race. [`VerdictWriteHandle`] exists to
    /// order them; without it the UPDATE silently affects zero rows and the
    /// verdict column stays NULL forever, which looks exactly like "no
    /// recognition happened".
    #[tokio::test]
    async fn verdict_handle_lands_after_the_row_it_updates() {
        let pool = test_pool().await;
        let pending = spawn_persist_recognition(
            pool.clone(),
            ctx("sess-order"),
            "how do I configure voice?".into(),
            "cascade".into(),
            vec![],
            vec![],
        );
        let handle = pending.verdict_handle();
        assert!(!handle.retrieval_id().is_empty());

        // Issued immediately, while the INSERT is still in flight.
        handle.record("recognized", 0.91).await;

        let row = sqlx::query(
            "SELECT recognition_verdict, familiarity FROM recognition_events
              WHERE session_id = 'sess-order'",
        )
        .fetch_one(&pool)
        .await
        .expect("the recall row exists and the verdict found it");
        assert_eq!(
            row.get::<Option<String>, _>("recognition_verdict")
                .as_deref(),
            Some("recognized")
        );
        assert_eq!(row.get::<Option<f64>, _>("familiarity"), Some(0.91));
    }

    #[tokio::test]
    async fn migrate_v21_to_v22_adds_columns_and_is_idempotent() {
        use crate::session::spectral_schema::migrate_v21_to_v22;
        let pool = test_pool().await;
        // Fresh init already carries the columns; the migration must be a
        // harmless no-op there — and running it twice must also be safe.
        migrate_v21_to_v22(&pool).await.unwrap();
        migrate_v21_to_v22(&pool).await.unwrap();

        let cols: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pragma_table_info('recognition_events')
              WHERE name IN ('recognition_verdict', 'familiarity')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(cols, 2);
    }

    #[tokio::test]
    async fn tool_event_feed_persists_content_free_rows() {
        let pool = test_pool().await;
        spawn_log_tool_event(
            pool.clone(),
            "developer__shell".into(),
            Some("permagent".into()),
            Some("abc123".into()),
            Some("sess-t".into()),
        );
        // spawn is fire-and-forget; poll briefly for the row.
        let mut rows = 0i64;
        for _ in 0..50 {
            rows = sqlx::query_scalar("SELECT COUNT(*) FROM recognition_tool_events")
                .fetch_one(&pool)
                .await
                .unwrap();
            if rows > 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(rows, 1, "tool event landed");

        let row = sqlx::query(
            "SELECT tool_name, wing, args_class, session_id, occurred_at
               FROM recognition_tool_events",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.get::<String, _>("tool_name"), "developer__shell");
        assert_eq!(
            row.get::<Option<String>, _>("wing").as_deref(),
            Some("permagent")
        );
        assert_eq!(
            row.get::<Option<String>, _>("args_class").as_deref(),
            Some("abc123")
        );
        assert!(!row.get::<String, _>("occurred_at").is_empty());
    }
}
