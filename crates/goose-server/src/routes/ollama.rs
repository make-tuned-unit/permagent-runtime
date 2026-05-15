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
}

impl Default for LibrarianSchedule {
    fn default() -> Self {
        Self {
            enabled: true,
            start_time: "02:00".to_string(),
            duration_minutes: 240,
            model: "qwen2.5:7b".to_string(),
            run_if_launched_in_window: true,
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

fn load_last_warmed_date() -> Option<chrono::NaiveDate> {
    let path = state_path();
    let contents = std::fs::read_to_string(path).ok()?;
    let obj: serde_json::Value = serde_json::from_str(&contents).ok()?;
    let date_str = obj.get("last_warmed_date")?.as_str()?;
    chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d").ok()
}

fn save_warmed_date(date: chrono::NaiveDate) {
    let path = state_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let obj = serde_json::json!({ "last_warmed_date": date.format("%Y-%m-%d").to_string() });
    // Write-temp-then-rename: protects against truncation on daemon crash mid-write.
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, serde_json::to_string_pretty(&obj).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

fn already_warmed_today() -> bool {
    let today = chrono::Local::now().date_naive();
    load_last_warmed_date().is_some_and(|d| d == today)
}

fn mark_warmed_today() {
    save_warmed_date(chrono::Local::now().date_naive());
}

/// Read-only query against the Brain's SQLite for total and described memory counts.
fn query_memory_counts() -> Result<(usize, usize), String> {
    let db_path = permagent::config::paths::Paths::brain_dir().join("memory.db");
    let conn = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
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
    }
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
