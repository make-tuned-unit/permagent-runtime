//! Ollama status and control routes.
//! Proxies to the local Ollama instance for model state queries and warm-load.

use crate::routes::errors::ErrorResponse;
use crate::state::AppState;
use axum::{
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

const OLLAMA_BASE: &str = "http://localhost:11434";

// ── Response types ──────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OllamaModelInfo {
    pub name: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub digest: String,
    #[serde(default)]
    pub modified_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaTagsResponse {
    #[serde(default)]
    models: Vec<OllamaModelInfo>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OllamaRunningModel {
    pub name: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub size_vram: u64,
    #[serde(default)]
    pub digest: String,
    #[serde(default)]
    pub expires_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaPsResponse {
    #[serde(default)]
    models: Vec<OllamaRunningModel>,
}

/// Combined status for the frontend
#[derive(Debug, Serialize)]
pub struct OllamaStatus {
    pub reachable: bool,
    pub installed: Vec<OllamaModelInfo>,
    pub running: Vec<OllamaRunningModel>,
}

#[derive(Debug, Deserialize)]
pub struct WarmLoadRequest {
    pub model: String,
    /// How long to keep the model loaded, in seconds
    pub keep_alive_secs: u64,
}

#[derive(Debug, Serialize)]
pub struct WarmLoadResponse {
    pub success: bool,
    pub model: String,
    pub keep_alive_secs: u64,
}

// ── Handlers ────────────────────────────────────────────────────────

/// GET /api/ollama/status — combined installed + running state
async fn ollama_status() -> Json<OllamaStatus> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap_or_default();

    let tags = client.get(format!("{}/api/tags", OLLAMA_BASE)).send().await;
    let ps = client.get(format!("{}/api/ps", OLLAMA_BASE)).send().await;

    let installed = match tags {
        Ok(resp) => resp
            .json::<OllamaTagsResponse>()
            .await
            .map(|r| r.models)
            .unwrap_or_default(),
        Err(_) => vec![],
    };

    let running = match ps {
        Ok(resp) => resp
            .json::<OllamaPsResponse>()
            .await
            .map(|r| r.models)
            .unwrap_or_default(),
        Err(_) => vec![],
    };

    let reachable = !installed.is_empty() || {
        // If tags returned empty but didn't error, Ollama is reachable
        client
            .get(format!("{}/api/tags", OLLAMA_BASE))
            .send()
            .await
            .is_ok()
    };

    Json(OllamaStatus {
        reachable,
        installed,
        running,
    })
}

/// POST /api/ollama/warm — warm-load a model with keep_alive duration
async fn ollama_warm(
    Json(req): Json<WarmLoadRequest>,
) -> Result<Json<WarmLoadResponse>, ErrorResponse> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| ErrorResponse::internal(format!("HTTP client error: {}", e)))?;

    let body = serde_json::json!({
        "model": req.model,
        "prompt": "ok",
        "stream": false,
        "keep_alive": format!("{}s", req.keep_alive_secs),
        "options": { "num_predict": 1 }
    });

    let resp = client
        .post(format!("{}/api/generate", OLLAMA_BASE))
        .json(&body)
        .send()
        .await
        .map_err(|e| ErrorResponse::internal(format!("Ollama unreachable: {}", e)))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(ErrorResponse::internal(format!(
            "Ollama warm-load failed ({}): {}",
            status, text
        )));
    }

    tracing::info!(model = %req.model, keep_alive = req.keep_alive_secs, "Ollama model warm-loaded");

    Ok(Json(WarmLoadResponse {
        success: true,
        model: req.model,
        keep_alive_secs: req.keep_alive_secs,
    }))
}

// ── Librarian schedule config ───────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LibrarianSchedule {
    pub enabled: bool,
    /// Daily start time in "HH:MM" 24-hour local format
    pub start_time: String,
    /// Window duration in minutes (15..=720)
    pub duration_minutes: u32,
    /// Model name to warm-load for the Librarian
    pub model: String,
    /// If true, start Librarian if app launches mid-window
    pub run_if_launched_in_window: bool,
    /// If true, the daily consolidation pass also prunes noise memories.
    /// Off by default — consolidation preserves signal, pruning deletes.
    /// Different risk profiles, different defaults.
    #[serde(default)]
    pub pruning_enabled: bool,
}

impl Default for LibrarianSchedule {
    fn default() -> Self {
        Self {
            enabled: true,
            start_time: "02:00".to_string(),
            duration_minutes: 240,
            model: "qwen2.5:7b".to_string(),
            run_if_launched_in_window: true,
            pruning_enabled: false,
        }
    }
}

fn schedule_path() -> std::path::PathBuf {
    permagent::config::paths::Paths::in_data_dir("librarian_schedule.json")
}

fn load_schedule() -> LibrarianSchedule {
    let path = schedule_path();
    match std::fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => LibrarianSchedule::default(),
    }
}

fn save_schedule(schedule: &LibrarianSchedule) -> Result<(), String> {
    let path = schedule_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(schedule).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(())
}

/// GET /api/librarian/schedule
async fn get_librarian_schedule() -> Json<LibrarianSchedule> {
    Json(load_schedule())
}

/// PUT /api/librarian/schedule
async fn set_librarian_schedule(
    Json(schedule): Json<LibrarianSchedule>,
) -> Result<Json<LibrarianSchedule>, ErrorResponse> {
    // Validate
    if schedule.duration_minutes < 15 || schedule.duration_minutes > 720 {
        return Err(ErrorResponse::bad_request(
            "Duration must be between 15 and 720 minutes".to_string(),
        ));
    }
    // Validate HH:MM format
    let parts: Vec<&str> = schedule.start_time.split(':').collect();
    if parts.len() != 2 {
        return Err(ErrorResponse::bad_request(
            "start_time must be HH:MM".to_string(),
        ));
    }
    let hour: u32 = parts[0]
        .parse()
        .map_err(|_| ErrorResponse::bad_request("Invalid hour".to_string()))?;
    let minute: u32 = parts[1]
        .parse()
        .map_err(|_| ErrorResponse::bad_request("Invalid minute".to_string()))?;
    if hour >= 24 || minute >= 60 {
        return Err(ErrorResponse::bad_request(
            "start_time out of range".to_string(),
        ));
    }

    save_schedule(&schedule)
        .map_err(|e| ErrorResponse::internal(format!("Failed to save: {}", e)))?;
    tracing::info!(
        enabled = schedule.enabled,
        start = %schedule.start_time,
        duration = schedule.duration_minutes,
        model = %schedule.model,
        "Librarian schedule updated"
    );
    Ok(Json(schedule))
}

// ── Librarian warm-load scheduler ────────────────────────────────────
// Behavior A: warm for the full window duration, then let Ollama unload.
// TODO(Behavior C): Once the Librarian exposes a "queue empty / done"
// signal, switch to: warm at window start, run jobs, unload as soon as
// queue is empty. Tracked as Librarian Phase 2 work.

/// Mutex serializing batch runs. Both the scheduled window and "Run now"
/// acquire this before calling run_batch. Second caller awaits the first.
static BATCH_MUTEX: std::sync::LazyLock<tokio::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

fn parse_schedule_time(schedule: &LibrarianSchedule) -> Option<(u32, u32, u32)> {
    let parts: Vec<&str> = schedule.start_time.split(':').collect();
    if parts.len() != 2 {
        return None;
    }
    let h: u32 = parts[0].parse().ok()?;
    let m: u32 = parts[1].parse().ok()?;
    Some((h, m, schedule.duration_minutes))
}

fn is_in_window(schedule: &LibrarianSchedule) -> bool {
    let (sh, sm, dur) = match parse_schedule_time(schedule) {
        Some(v) => v,
        None => return false,
    };
    let now = chrono::Local::now();
    let today = now.date_naive();
    let naive_start = today.and_hms_opt(sh, sm, 0).unwrap_or_default();
    let tz = now.timezone();
    if let Some(start) = naive_start.and_local_timezone(tz).single() {
        let end = start + chrono::Duration::minutes(dur as i64);
        now >= start && now < end
    } else {
        false
    }
}

// ── Persisted warm date ─────────────────────────────────────────────
// Replaces the old AtomicBool which reset on daemon restart. The warm
// date is stored in ~/.permagent/data/librarian_state.json so the
// scheduler knows not to re-warm if the daemon restarts mid-window.

fn state_path() -> std::path::PathBuf {
    permagent::config::paths::Paths::in_data_dir("librarian_state.json")
}

/// Persisted scheduler state. Extends beyond the original warmed_date to track
/// timestamps for co-retrieval rebuild and consolidation jobs.
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct SchedulerState {
    #[serde(default)]
    last_warmed_date: Option<String>,
    #[serde(default)]
    last_co_retrieval_rebuild: Option<String>,
    #[serde(default)]
    last_consolidated: Option<String>,
}

fn load_scheduler_state() -> SchedulerState {
    let path = state_path();
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_scheduler_state(state: &SchedulerState) {
    let path = state_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, serde_json::to_string_pretty(state).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

fn load_last_warmed_date() -> Option<chrono::NaiveDate> {
    let state = load_scheduler_state();
    state
        .last_warmed_date
        .and_then(|s| chrono::NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok())
}

fn already_warmed_today() -> bool {
    let today = chrono::Local::now().date_naive();
    load_last_warmed_date().is_some_and(|d| d == today)
}

fn mark_warmed_today() {
    let mut state = load_scheduler_state();
    state.last_warmed_date = Some(chrono::Local::now().date_naive().format("%Y-%m-%d").to_string());
    save_scheduler_state(&state);
}

fn co_retrieval_rebuild_due() -> bool {
    let state = load_scheduler_state();
    match state.last_co_retrieval_rebuild {
        None => true,
        Some(ts) => chrono::DateTime::parse_from_rfc3339(&ts)
            .map(|dt| chrono::Utc::now() - dt.with_timezone(&chrono::Utc) > chrono::Duration::hours(1))
            .unwrap_or(true),
    }
}

fn mark_co_retrieval_rebuilt() {
    let mut state = load_scheduler_state();
    state.last_co_retrieval_rebuild = Some(chrono::Utc::now().to_rfc3339());
    save_scheduler_state(&state);
}

fn consolidation_due() -> bool {
    let state = load_scheduler_state();
    match state.last_consolidated {
        None => true,
        Some(ts) => chrono::DateTime::parse_from_rfc3339(&ts)
            .map(|dt| chrono::Utc::now() - dt.with_timezone(&chrono::Utc) > chrono::Duration::days(1))
            .unwrap_or(true),
    }
}

fn mark_consolidated() {
    let mut state = load_scheduler_state();
    state.last_consolidated = Some(chrono::Utc::now().to_rfc3339());
    save_scheduler_state(&state);
}

/// Read-only query against the Brain's SQLite for total and described memory counts.
fn query_memory_counts() -> Result<(usize, usize), String> {
    let conn = crate::brain_ops::read_only_brain_conn()
        .map_err(|e| format!("Failed to open brain DB: {}", e))?;

    let total: usize = conn
        .query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
        .map_err(|e| format!("Count query failed: {}", e))?;
    let described: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE description IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .map_err(|e| format!("Described count query failed: {}", e))?;

    Ok((total, described))
}

/// Warm-load a model then run the batch, holding the BATCH_MUTEX.
async fn warm_and_run(schedule: &LibrarianSchedule, keep_alive_secs: u64) -> Result<usize, String> {
    use permagent::agents::platform_extensions::librarian_state;

    let _guard = BATCH_MUTEX.lock().await;

    let brain =
        permagent::agents::platform_extensions::get_global_brain().ok_or("Brain not available")?;

    // Query actual memory counts from the Brain's SQLite DB.
    let (total, described) = query_memory_counts()?;
    librarian_state::set_warming(total, described);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| {
            librarian_state::set_error(&format!("HTTP client error: {}", e));
            format!("HTTP client error: {}", e)
        })?;

    let body = serde_json::json!({
        "model": schedule.model,
        "prompt": "ok",
        "stream": false,
        "keep_alive": format!("{}s", keep_alive_secs),
        "options": { "num_predict": 1 }
    });

    let resp = client
        .post(format!("{}/api/generate", OLLAMA_BASE))
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            librarian_state::set_error(&format!("Ollama unreachable: {}", e));
            format!("Ollama unreachable: {}", e)
        })?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        let msg = format!("Warm-load failed: {}", text);
        librarian_state::set_error(&msg);
        return Err(msg);
    }

    tracing::info!(model = %schedule.model, keep_alive = keep_alive_secs, "Librarian model warm-loaded");

    // Annotation backfill runs at daemon startup (state.rs), not here.

    let result =
        permagent::agents::platform_extensions::librarian::run_batch(&brain, 20, &schedule.model)
            .await;

    if let Err(ref e) = result {
        librarian_state::set_error(e);
    }

    result
}

/// Background loop: ticks once per minute, warm-loads if in window.
pub async fn librarian_scheduler_loop() {
    let schedule = load_schedule();
    let brain_db = permagent::config::paths::Paths::brain_dir().join("memory.db");
    tracing::info!(
        model = %schedule.model,
        enabled = schedule.enabled,
        schedule_window = %format!("{} + {}min", schedule.start_time, schedule.duration_minutes),
        brain_db = %brain_db.display(),
        "Librarian scheduler started"
    );

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
    loop {
        interval.tick().await;

        let schedule = load_schedule();
        if !schedule.enabled {
            continue;
        }

        let in_window = is_in_window(&schedule);

        if in_window && !already_warmed_today() {
            tracing::info!(
                model = %schedule.model,
                duration = schedule.duration_minutes,
                "Librarian window active — warm-loading model and running batch"
            );

            let keep_alive_secs = schedule.duration_minutes as u64 * 60;
            match warm_and_run(&schedule, keep_alive_secs).await {
                Ok(n) => {
                    mark_warmed_today();
                    tracing::info!(described = n, "Librarian scheduled batch complete");
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Librarian scheduled batch failed");
                }
            }
        }

        // ── Co-retrieval rebuild (hourly) ───────────────────────────────
        // Converts accumulated retrieval_events into co_retrieval_pairs.
        // Full recompute, atomic replace, idempotent. No Ollama needed.
        if co_retrieval_rebuild_due() {
            if let Some(brain) = permagent::agents::platform_extensions::get_global_brain() {
                match tokio::task::spawn_blocking(move || brain.rebuild_co_retrieval_index()).await {
                    Ok(Ok(n)) => {
                        mark_co_retrieval_rebuilt();
                        tracing::info!(
                            target: "permagentd::librarian",
                            pairs = n,
                            "Co-retrieval index rebuilt"
                        );
                    }
                    Ok(Err(e)) => {
                        tracing::warn!(
                            target: "permagentd::librarian",
                            error = %e,
                            "Co-retrieval index rebuild failed"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "permagentd::librarian",
                            error = %e,
                            "Co-retrieval rebuild panicked"
                        );
                    }
                }
            }
        }

        // ── Consolidation scan + optional pruning (daily) ────────────────
        if consolidation_due() {
            let pruning_enabled = schedule.pruning_enabled;
            if let Some(brain) = permagent::agents::platform_extensions::get_global_brain() {
                match tokio::task::spawn_blocking(move || {
                    let consolidation = run_consolidation_scan(&brain);
                    let pruned = if pruning_enabled {
                        run_pruning_pass().unwrap_or(0)
                    } else {
                        0
                    };
                    consolidation.map(|(c, o)| (c, o, pruned))
                }).await {
                    Ok(Ok((clusters, originals, pruned))) => {
                        mark_consolidated();
                        tracing::info!(
                            target: "permagentd::librarian",
                            clusters,
                            originals,
                            pruned,
                            "Consolidation scan complete"
                        );
                    }
                    Ok(Err(e)) => {
                        tracing::warn!(
                            target: "permagentd::librarian",
                            error = %e,
                            "Consolidation scan failed"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "permagentd::librarian",
                            error = %e,
                            "Consolidation scan panicked"
                        );
                    }
                }
            }
        }
    }
}

// ── Consolidation scan ────────────────────────────────────────────
// Detects redundant memory clusters and merges them into representative
// summaries. Uses direct SQL against the Brain's memory.db — Spectral
// has no delete/archive API (append-only by design).

/// Find groups of memories with identical content that haven't been consolidated yet.
/// Returns Vec<(content, count)> — each entry is a cluster of exact duplicates.
pub fn find_exact_duplicate_clusters(
    conn: &rusqlite::Connection,
) -> Result<Vec<(String, usize)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT content, COUNT(*) as cnt FROM memories \
             WHERE _pm_consolidated_into IS NULL \
             GROUP BY content HAVING cnt > 1 ORDER BY cnt DESC LIMIT 50",
        )
        .map_err(|e| format!("Prepare failed: {e}"))?;
    let clusters: Vec<(String, usize)> = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?)))
        .map_err(|e| format!("Query failed: {e}"))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(clusters)
}

/// Find clusters of browser navigation memories grouped by domain.
/// Returns Vec<(domain, count, first_visit, last_visit)> for domains with 3+ entries.
pub fn find_domain_clusters(
    conn: &rusqlite::Connection,
) -> Result<Vec<(String, usize, String, String)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT \
               CASE \
                 WHEN content LIKE 'Navigated to https://%' THEN \
                   substr(content, 16, instr(substr(content, 16), '/') - 1) \
                 WHEN content LIKE 'Navigated to http://%' THEN \
                   substr(content, 15, instr(substr(content, 15), '/') - 1) \
                 ELSE NULL \
               END as domain, \
               COUNT(*) as cnt, \
               MIN(created_at) as first_visit, \
               MAX(created_at) as last_visit \
             FROM memories \
             WHERE source = 'permagent.activity' \
               AND _pm_consolidated_into IS NULL \
               AND content LIKE 'Navigated to http%' \
             GROUP BY domain \
             HAVING cnt >= 3 AND domain IS NOT NULL \
             ORDER BY cnt DESC \
             LIMIT 30",
        )
        .map_err(|e| format!("Domain query failed: {e}"))?;

    let clusters: Vec<(String, usize, String, String)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, usize>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|e| format!("Domain query map failed: {e}"))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(clusters)
}

/// Build the SQL UPDATE that marks a duplicate as consolidated into the keeper.
/// Returns the statement string and params. Does NOT execute.
pub fn build_consolidation_update_sql() -> &'static str {
    "UPDATE memories SET _pm_consolidated_into = ?1 WHERE id = ?2"
}

fn run_consolidation_scan(brain: &spectral::Brain) -> Result<(usize, usize), String> {
    let db_path = permagent::config::paths::Paths::brain_dir().join("memory.db");
    let conn = rusqlite::Connection::open(&db_path)
        .map_err(|e| format!("Failed to open brain DB: {e}"))?;

    // Ensure Permagent-side column exists (idempotent)
    conn.execute_batch(
        "ALTER TABLE memories ADD COLUMN _pm_consolidated_into TEXT DEFAULT NULL;"
    ).ok(); // Ignore "duplicate column" error

    let mut total_clusters = 0usize;
    let mut total_originals = 0usize;

    // ── Strategy 1: Exact content duplicates ──
    let dup_clusters = find_exact_duplicate_clusters(&conn)?;

    for (content, count) in &dup_clusters {
        // Keep the oldest, mark the rest
        let ids: Vec<(String, String)> = conn
            .prepare(
                "SELECT id, key FROM memories \
                 WHERE content = ?1 AND _pm_consolidated_into IS NULL \
                 ORDER BY created_at ASC",
            )
            .and_then(|mut s| {
                s.query_map([content], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
            })
            .unwrap_or_default();

        if ids.len() < 2 {
            continue;
        }
        let keeper_id = &ids[0].0;
        for (id, key) in &ids[1..] {
            conn.execute(
                build_consolidation_update_sql(),
                rusqlite::params![keeper_id, id],
            )
            .ok();
            tracing::debug!(
                target: "permagentd::librarian",
                key,
                keeper = keeper_id,
                "Consolidated duplicate"
            );
        }
        total_clusters += 1;
        total_originals += count - 1;
    }

    // ── Strategy 2: Same-domain browser navigations with 3+ entries ──
    let domain_clusters = find_domain_clusters(&conn)?;

    for (domain, count, first_visit, last_visit) in &domain_clusters {
        let summary = format!(
            "{domain} — visited {count} times between {first} and {last}",
            first = first_visit.split('T').next().unwrap_or(first_visit),
            last = last_visit.split('T').next().unwrap_or(last_visit),
        );
        let key = format!("consolidated:browser:{domain}");

        // Create the summary memory via Brain API
        let result = brain.remember_with(
            &key,
            &summary,
            spectral::RememberOpts {
                source: Some("librarian.consolidation".into()),
                visibility: spectral::Visibility::Private,
                ..Default::default()
            },
        );

        if let Ok(r) = result {
            // Mark originals as consolidated
            conn.execute(
                "UPDATE memories SET _pm_consolidated_into = ?1 \
                 WHERE source = 'permagent.activity' \
                   AND _pm_consolidated_into IS NULL \
                   AND content LIKE ?2",
                rusqlite::params![r.memory_id, format!("Navigated to%{domain}%")],
            )
            .ok();

            total_clusters += 1;
            total_originals += count;

            tracing::debug!(
                target: "permagentd::librarian",
                domain,
                count,
                summary_id = r.memory_id,
                "Consolidated browser navigation cluster"
            );
        }
    }

    Ok((total_clusters, total_originals))
}

// ── Pruning pass ──────────────────────────────────────────────────
// Conservative noise removal. Only runs when pruning_enabled = true.
// ALL conditions must be true to prune a memory.

fn run_pruning_pass() -> Result<usize, String> {
    let db_path = permagent::config::paths::Paths::brain_dir().join("memory.db");
    let conn = rusqlite::Connection::open(&db_path)
        .map_err(|e| format!("Failed to open brain DB: {e}"))?;

    // Ensure columns exist
    conn.execute_batch(
        "ALTER TABLE memories ADD COLUMN _pm_consolidated_into TEXT DEFAULT NULL;"
    ).ok();

    let annotation_table_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='memory_annotations'",
            [],
            |r| r.get::<_, usize>(0),
        )
        .unwrap_or(0) > 0;

    // Find candidates: short content, activity source, never recalled, not consolidated
    let mut stmt = conn
        .prepare(
            "SELECT m.id, m.key, m.content FROM memories m \
             WHERE length(m.content) < 20 \
               AND m.source = 'permagent.activity' \
               AND m.last_reinforced_at IS NULL \
               AND m._pm_consolidated_into IS NULL",
        )
        .map_err(|e| format!("Prune query failed: {e}"))?;

    let candidates: Vec<(String, String, String)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| format!("Prune query map failed: {e}"))?
        .filter_map(|r| r.ok())
        .collect();

    let mut pruned = 0usize;
    for (id, key, content) in &candidates {
        // Check: zero entity connections
        if annotation_table_exists {
            let ann_count: usize = conn
                .query_row(
                    "SELECT COUNT(*) FROM memory_annotations WHERE memory_id = ?1",
                    [id],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            if ann_count > 0 {
                continue;
            }
        }

        // All conditions met — prune (hard delete)
        if conn
            .execute("DELETE FROM memories WHERE id = ?1", [id])
            .is_ok()
        {
            tracing::info!(
                target: "permagentd::librarian",
                key,
                content,
                reason = "short content, no entities, no recall, activity source",
                "Pruned noise memory"
            );
            pruned += 1;
        }
    }

    Ok(pruned)
}

/// POST /api/librarian/run-now — manual trigger for warm-load + run.
/// Returns immediately with 409 if a batch is already running.
async fn run_librarian_now() -> Result<Json<WarmLoadResponse>, ErrorResponse> {
    let guard = BATCH_MUTEX.try_lock();
    if guard.is_err() {
        return Err(ErrorResponse::conflict(
            "A Librarian batch is already running. Try again later.".to_string(),
        ));
    }
    // Drop the guard — warm_and_run will re-acquire. We only used try_lock
    // to check availability. This creates a tiny race window but is fine:
    // worst case, the scheduled batch finishes between our check and the
    // re-acquire, and we run a second batch (idempotent via describe_one).
    drop(guard);

    let schedule = load_schedule();
    let keep_alive_secs: u64 = 1800;

    tracing::info!(model = %schedule.model, "Librarian manual run triggered");

    match warm_and_run(&schedule, keep_alive_secs).await {
        Ok(n) => {
            tracing::info!(described = n, "Librarian manual batch complete");
            Ok(Json(WarmLoadResponse {
                success: true,
                model: schedule.model,
                keep_alive_secs,
            }))
        }
        Err(e) => Err(ErrorResponse::internal(format!(
            "Librarian run failed: {}",
            e
        ))),
    }
}

// ── Librarian status endpoint ────────────────────────────────────────

#[derive(Debug, Serialize)]
struct LibrarianStatusResponse {
    state: String,
    current_task: String,
    current_memory: Option<serde_json::Value>,
    schedule: serde_json::Value,
    session_stats: serde_json::Value,
    lifetime_stats: serde_json::Value,
    model: String,
    provider: String,
    error_message: Option<String>,
}

/// GET /api/librarian/status
async fn get_librarian_status() -> Json<LibrarianStatusResponse> {
    use permagent::agents::platform_extensions::librarian_state;

    let rt_state = librarian_state::get_state();
    let schedule = load_schedule();

    let current_memory = rt_state.current_memory.map(|m| {
        serde_json::json!({ "key": m.key, "content_preview": m.content_preview })
    });

    let next_window = compute_next_window_start(&schedule);

    // Always read real counts from the DB for lifetime stats.
    let (total, described) = query_memory_counts().unwrap_or((0, 0));
    let pending = total.saturating_sub(described);

    Json(LibrarianStatusResponse {
        state: serde_json::to_value(&rt_state.phase)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "unknown".to_string()),
        current_task: rt_state.current_task,
        current_memory,
        schedule: serde_json::json!({
            "next_window_start": next_window,
            "window_duration_min": schedule.duration_minutes,
        }),
        session_stats: serde_json::json!({
            "batch_started_at": rt_state.session_stats.batch_started_at,
            "memories_described_this_session": rt_state.session_stats.memories_described_this_session,
            "avg_seconds_per_memory": rt_state.session_stats.avg_seconds_per_memory,
        }),
        lifetime_stats: serde_json::json!({
            "total_memories": total,
            "described": described,
            "pending": pending,
        }),
        model: schedule.model,
        provider: "ollama".to_string(),
        error_message: rt_state.error_message,
    })
}

/// Compute the next window start time as ISO 8601 string.
fn compute_next_window_start(schedule: &LibrarianSchedule) -> Option<String> {
    let (sh, sm, dur) = parse_schedule_time(schedule)?;
    let now = chrono::Local::now();
    let today = now.date_naive();
    let tz = now.timezone();

    let naive_start = today.and_hms_opt(sh, sm, 0)?;
    let start_today = naive_start.and_local_timezone(tz).single()?;
    let end_today = start_today + chrono::Duration::minutes(dur as i64);

    if now < start_today {
        Some(start_today.to_rfc3339())
    } else if now < end_today {
        let tomorrow_start = start_today + chrono::Duration::days(1);
        Some(tomorrow_start.to_rfc3339())
    } else {
        let tomorrow_start = start_today + chrono::Duration::days(1);
        Some(tomorrow_start.to_rfc3339())
    }
}

// ── Router ──────────────────────────────────────────────────────────

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/ollama/status", get(ollama_status))
        .route("/api/ollama/warm", post(ollama_warm))
        .route("/api/librarian/schedule", get(get_librarian_schedule))
        .route(
            "/api/librarian/schedule",
            axum::routing::put(set_librarian_schedule),
        )
        .route("/api/librarian/run-now", post(run_librarian_now))
        .route("/api/librarian/status", get(get_librarian_status))
        .with_state(state)
}
