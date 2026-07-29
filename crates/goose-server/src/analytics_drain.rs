//! Analytics drain poller — the daemon half of relay-and-drain
//! (see `docs/architecture/LOCAL_FIRST_WEB_ANALYTICS.md`).
//!
//! A public site cannot beacon to this daemon: the browser would have to reach
//! a home machine behind NAT over HTTP from an HTTPS page. So the direction is
//! inverted. The site collects same-origin into its own database and exposes an
//! authenticated drain endpoint; this loop pulls outbound on a timer.
//!
//! Two properties fall out of that, and both are the point:
//!   * nothing inbound is exposed — no tunnel, no port-forward, no third party
//!     in the data path;
//!   * daemon downtime costs nothing. Events accumulate on the always-on side,
//!     and a machine that slept for a week catches up on wake.
//!
//! Correctness rests on two things the naive version gets wrong: rows carry the
//! source's row id into `source_event_id` (UNIQUE per project) so a retried or
//! overlapping drain is a no-op rather than double-counted traffic, and the
//! source's own timestamp is written to `created_at` rather than letting it
//! default to the fetch time, which would collapse history into one day.

use crate::state::AppState;
use permagent::projects;
use serde::Deserialize;
use sqlx::{Pool, Sqlite};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

const TICK: Duration = Duration::from_secs(120);
/// Let boot settle before the first pass (same courtesy as watcher_insights).
const BOOT_DELAY: Duration = Duration::from_secs(45);
const PAGE_LIMIT: u32 = 500;
/// Bound one project's catch-up per tick so a huge backlog can't monopolise a
/// pass; the next tick continues from the advanced cursor.
const MAX_PAGES_PER_TICK: u32 = 20;

fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .expect("reqwest client builds")
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DrainEvent {
    /// Source row id — monotonic, and the cursor. Accepts a number or string
    /// so a bigint/uuid-ish id survives JSON without the site having to care.
    #[serde(deserialize_with = "id_as_string")]
    id: String,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    referrer: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    visitor_hash: Option<String>,
    /// Event time, ISO-8601. Missing means the site is misimplemented; we fall
    /// back to now rather than dropping the event.
    #[serde(default)]
    at: Option<String>,
}

fn id_as_string<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    match v {
        serde_json::Value::String(s) => Ok(s),
        serde_json::Value::Number(n) => Ok(n.to_string()),
        other => Err(serde::de::Error::custom(format!(
            "drain event id must be a string or number, got {other}"
        ))),
    }
}

#[derive(Debug, Deserialize)]
struct DrainResponse {
    #[serde(default)]
    events: Vec<DrainEvent>,
}

pub fn spawn(state: Arc<AppState>) {
    tokio::spawn(async move {
        tokio::time::sleep(BOOT_DELAY).await;
        loop {
            if let Err(e) = run_once(&state).await {
                tracing::debug!(target: "analytics_drain", "drain pass skipped: {e}");
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

async fn run_once(state: &AppState) -> Result<(), String> {
    // Outbound HTTP is not on the audited egress path, so sovereign mode has to
    // be enforced here or it silently leaks (same contract as analytics.rs).
    if permagent::sovereignty::global_sovereign_mode() {
        return Ok(());
    }
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| e.to_string())?;
    let projects = projects::list_projects(&pool, Some("active")).await?;
    for project in projects {
        let Some(mut config) = crate::routes::first_party_analytics::drain_config(&project) else {
            continue;
        };
        let Some(drain_url) = config.drain_url.clone() else {
            continue;
        };
        let Some(secret) =
            crate::routes::first_party_analytics::stored_drain_secret_readonly(&project.id)
        else {
            continue;
        };

        let outcome = drain_project(&pool, &project.id, &drain_url, &secret, &mut config).await;
        // Persist the cursor/status regardless of outcome: a partial catch-up
        // must keep its progress, and a failure must be visible in the UI
        // rather than looking like a quiet traffic day.
        if let Err(e) =
            crate::routes::first_party_analytics::persist_drain_state(&pool, &project, &config)
                .await
        {
            tracing::warn!(target: "analytics_drain", "could not persist drain state: {e}");
        }
        match outcome {
            Ok(0) => {}
            Ok(n) => tracing::info!(
                target: "analytics_drain",
                project = %project.name, ingested = n, "analytics drained"
            ),
            Err(e) => tracing::warn!(
                target: "analytics_drain",
                project = %project.name, "analytics drain failed: {e}"
            ),
        }
    }
    Ok(())
}

async fn drain_project(
    pool: &Pool<Sqlite>,
    project_id: &str,
    drain_url: &str,
    secret: &str,
    config: &mut crate::routes::first_party_analytics::DrainState,
) -> Result<usize, String> {
    let mut total = 0usize;
    for _ in 0..MAX_PAGES_PER_TICK {
        let since = config.cursor.clone().unwrap_or_else(|| "0".to_string());
        let sep = if drain_url.contains('?') { '&' } else { '?' };
        let url = format!("{drain_url}{sep}since={since}&limit={PAGE_LIMIT}");

        let resp = http_client()
            .get(&url)
            .header("x-permagent-key", secret)
            .send()
            .await
            .map_err(|e| {
                let msg = format!("network error: {e}");
                config.last_error = Some(msg.clone());
                msg
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let msg = format!(
                "drain returned {status}: {}",
                body.chars().take(200).collect::<String>()
            );
            config.last_error = Some(msg.clone());
            return Err(msg);
        }

        let parsed: DrainResponse = resp.json().await.map_err(|e| {
            let msg = format!("malformed drain response: {e}");
            config.last_error = Some(msg.clone());
            msg
        })?;

        let batch = parsed.events.len();
        if batch == 0 {
            break;
        }
        for ev in &parsed.events {
            insert_event(pool, project_id, ev).await?;
            config.cursor = Some(ev.id.clone());
        }
        total += batch;
        if (batch as u32) < PAGE_LIMIT {
            break; // drained to the tip
        }
    }
    config.last_error = None;
    config.last_drain_at = Some(chrono::Utc::now().to_rfc3339());
    Ok(total)
}

async fn insert_event(
    pool: &Pool<Sqlite>,
    project_id: &str,
    ev: &DrainEvent,
) -> Result<(), String> {
    let kind = match ev.kind.as_deref() {
        Some("event") | Some("ev") => "event",
        _ => "pageview",
    };
    let path = ev.path.clone().unwrap_or_else(|| "/".to_string());
    let created_at = ev
        .at
        .clone()
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());

    // INSERT OR IGNORE against UNIQUE(project_id, source_event_id): re-draining
    // the same window is a no-op instead of inflating every count.
    sqlx::query(
        "INSERT OR IGNORE INTO analytics_events
            (project_id, kind, path, referrer, name, visitor_hash, created_at, source_event_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )
    .bind(project_id)
    .bind(kind)
    .bind(path)
    .bind(ev.referrer.as_deref())
    .bind(ev.name.as_deref())
    .bind(ev.visitor_hash.as_deref())
    .bind(created_at)
    .bind(&ev.id)
    .execute(pool)
    .await
    .map_err(|e| format!("insert failed: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem_pool() -> Pool<Sqlite> {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        permagent::session::spectral_schema::apply_analytics_events_schema(&pool)
            .await
            .unwrap();
        pool
    }

    fn ev(id: &str, at: &str) -> DrainEvent {
        DrainEvent {
            id: id.to_string(),
            kind: Some("pageview".into()),
            path: Some("/deals".into()),
            referrer: None,
            name: None,
            visitor_hash: Some("abc".into()),
            at: Some(at.to_string()),
        }
    }

    /// Re-draining an overlapping window must not double-count. Without the
    /// UNIQUE(project_id, source_event_id) + INSERT OR IGNORE pair, one retry
    /// silently inflates every number on the Analytics page.
    #[tokio::test]
    async fn redraining_the_same_events_is_a_no_op() {
        let pool = mem_pool().await;
        let e = ev("42", "2026-07-01T10:00:00Z");
        insert_event(&pool, "p1", &e).await.unwrap();
        insert_event(&pool, "p1", &e).await.unwrap();
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM analytics_events")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 1, "same source id must insert once");
    }

    /// The source's timestamp must be preserved. Letting created_at default to
    /// now() would stamp a week of backlog with the fetch time and collapse
    /// every chart into a single day.
    #[tokio::test]
    async fn event_time_is_preserved_not_ingest_time() {
        let pool = mem_pool().await;
        insert_event(&pool, "p1", &ev("1", "2026-07-01T10:00:00Z"))
            .await
            .unwrap();
        let at: String = sqlx::query_scalar("SELECT created_at FROM analytics_events")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(at.starts_with("2026-07-01"), "got {at}");
    }

    /// Distinct sources sharing an id must not collide (the unique index is
    /// per project, not global).
    #[tokio::test]
    async fn same_id_in_two_projects_both_land() {
        let pool = mem_pool().await;
        insert_event(&pool, "p1", &ev("7", "2026-07-01T10:00:00Z"))
            .await
            .unwrap();
        insert_event(&pool, "p2", &ev("7", "2026-07-01T10:00:00Z"))
            .await
            .unwrap();
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM analytics_events")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 2);
    }
}
