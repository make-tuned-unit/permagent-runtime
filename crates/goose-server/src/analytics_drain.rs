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

use crate::routes::analytics_behaviour;
use crate::routes::analytics_classify as classify;
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
/// How long collected events are kept. Beyond the dashboard's longest window,
/// so nothing displayable is ever removed.
const RETENTION_DAYS: u32 = 400;

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

    // ── v40 dimensions ──
    // All optional: a relay built against the earlier contract keeps working
    // and simply carries none of them, rather than failing to parse and
    // stalling the cursor.
    /// Flat scalar event payload. Re-sanitized here rather than trusted: the
    /// site's collector is written by a coding agent against a brief, and the
    /// clamps are ours to enforce.
    #[serde(default)]
    properties: Option<serde_json::Value>,
    #[serde(default)]
    is_bot: Option<bool>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    utm_source: Option<String>,
    #[serde(default)]
    utm_medium: Option<String>,
    #[serde(default)]
    utm_campaign: Option<String>,
    #[serde(default)]
    country: Option<String>,
    /// Raw user agent, if the relay sends it. Used ONLY to classify is_bot when
    /// the site did not, and never stored.
    #[serde(default)]
    user_agent: Option<String>,
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

/// Same number-or-string tolerance as `id_as_string`, for the optional
/// envelope ids. A relay whose ids are `bigIncrements` naturally emits them as
/// JSON numbers; without this, one such response fails to parse and stalls the
/// cursor for that project entirely.
fn opt_id_as_string<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    match Option::<serde_json::Value>::deserialize(d)? {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(s)) => Ok(Some(s)),
        Some(serde_json::Value::Number(n)) => Ok(Some(n.to_string())),
        Some(other) => Err(serde::de::Error::custom(format!(
            "drain envelope id must be a string or number, got {other}"
        ))),
    }
}

/// The drain envelope. **`camelCase`, like `DrainEvent`** — the install brief
/// tells the site to emit `latestId` / `firstAvailableId`, and without the
/// rename neither field ever bound: both v41 features (drain-lag reporting and
/// retention-gap detection) were reading `None` from every spec-compliant
/// relay and failing silently, which is exactly the class of hole the
/// retention detector was written to catch.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DrainResponse {
    #[serde(default)]
    events: Vec<DrainEvent>,
    /// Newest event id the relay currently holds (spec v41, optional). Lets
    /// the daemon report drain lag rather than infer health from silence.
    #[serde(default, deserialize_with = "opt_id_as_string")]
    latest_id: Option<String>,
    /// OLDEST event id the relay still holds (spec v41, optional). When this is
    /// greater than our cursor, the site's retention pruned rows the daemon
    /// never fetched — it slept past the window — and the resulting hole is
    /// otherwise INVISIBLE: the drain just resumes and under-reports forever.
    #[serde(default, deserialize_with = "opt_id_as_string")]
    first_available_id: Option<String>,
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
    // The global off-switch stays a plain early return: this loop wakes every
    // TICK, so auditing it here would write ~720 blocked rows a day per project
    // for requests that were never built. The per-project guard below is the
    // audited choke point, and it covers every request a pass actually makes.
    if permagent::sovereignty::global_sovereign_mode() {
        return Ok(());
    }
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| e.to_string())?;
    prune_old_events(&pool).await;
    let projects = projects::list_projects(&pool, Some("active")).await?;
    for project in &projects {
        let Some(mut config) = crate::routes::first_party_analytics::drain_config(project) else {
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

        // This is the audited choke point for outbound drain HTTP; a mid-pass
        // sovereign flip or unwritable audit skips the project rather than
        // egressing unaudited.
        if !permagent::sovereignty::guard_outbound_egress(
            permagent::sovereignty::EgressKind::Telemetry,
            &drain_url,
            &project.id,
        )
        .await
        {
            continue;
        }

        let outcome = drain_project(&pool, &project.id, &drain_url, &secret, &mut config).await;
        // Persist the cursor/status regardless of outcome: a partial catch-up
        // must keep its progress, and a failure must be visible in the UI
        // rather than looking like a quiet traffic day.
        if let Err(e) =
            crate::routes::first_party_analytics::persist_drain_state(&pool, project, &config).await
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
    sweep_behavioural_bots(&pool, &projects).await;
    Ok(())
}

/// Behavioural bot classification — the second half of bot detection.
///
/// The UA test runs at ingest and cannot see a headless browser, which sends a
/// stock Chrome UA (see `routes::analytics_behaviour` for the 2026-08-13/14
/// crawl this exists for). This pass reads behaviour out of what is already
/// stored instead, so it must run AFTER the drain: the rows that need it most
/// are the ones that just arrived in a backfill, days after the traffic
/// happened.
///
/// Best-effort and non-fatal, like the prune above.
///
/// It LOGS whenever it changes anything, because it changes numbers the user
/// reads — a guard that silently rewrites history is indistinguishable from a
/// bug in the dashboard. It stays silent when it changes nothing, which is
/// every tick but the rare one.
async fn sweep_behavioural_bots(pool: &Pool<Sqlite>, projects: &[projects::Project]) {
    match analytics_behaviour::flag_behavioural_bots(pool, analytics_behaviour::WINDOW_DAYS).await {
        Ok(sweep) if !sweep.is_empty() => {
            // Project names where we have them; a sweep can touch a project
            // that is no longer active, and an id is better than nothing.
            let named = sweep
                .projects
                .iter()
                .map(|id| {
                    projects
                        .iter()
                        .find(|p| &p.id == id)
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| id.clone())
                })
                .collect::<Vec<_>>()
                .join(", ");
            tracing::info!(
                target: "analytics_drain",
                rows = sweep.rows_flagged,
                device_days = sweep.device_days,
                projects = %named,
                "reclassified traffic as automated by behaviour \
                 (>= {} pageviews with a distinct session per pageview)",
                analytics_behaviour::MIN_PAGEVIEWS
            );
        }
        Ok(_) => {}
        Err(e) => tracing::warn!(
            target: "analytics_drain",
            "behavioural bot classification failed: {e}"
        ),
    }
}

/// Retention. `analytics_events` otherwise grows forever: the drain is a
/// read-only cursor that never deletes, so every pageview ever collected stays
/// in this database. The dashboard's longest window is a year, so anything
/// older cannot be displayed and is pure storage cost.
///
/// Best-effort and non-fatal: a failed prune must never stop a drain pass.
async fn prune_old_events(pool: &Pool<Sqlite>) {
    match sqlx::query("DELETE FROM analytics_events WHERE created_at < datetime('now', ?1)")
        .bind(format!("-{RETENTION_DAYS} days"))
        .execute(pool)
        .await
    {
        Ok(r) if r.rows_affected() > 0 => tracing::info!(
            target: "analytics_drain",
            pruned = r.rows_affected(),
            "pruned analytics events older than {RETENTION_DAYS} days"
        ),
        Ok(_) => {}
        Err(e) => tracing::warn!(target: "analytics_drain", "analytics prune failed: {e}"),
    }
}

async fn drain_project(
    pool: &Pool<Sqlite>,
    project_id: &str,
    drain_url: &str,
    secret: &str,
    config: &mut crate::routes::first_party_analytics::DrainState,
) -> Result<usize, String> {
    let mut total = 0usize;
    // The site's own host, for the self-referral drop in insert_event. The
    // drain URL IS the site's origin, so no extra configuration is needed.
    let site_host = classify::referrer_host(drain_url);
    // A retention gap is not an error — the drain succeeds, it just succeeds
    // over a hole. It therefore reached the success path at the bottom, which
    // clears `last_error`, and the one durable record that data was lost was
    // erased before the caller ever persisted it. Carried out of the loop so
    // the success path can keep it.
    let mut retention_warning: Option<String> = None;
    for _ in 0..MAX_PAGES_PER_TICK {
        let since = config.cursor.clone().unwrap_or_else(|| "0".to_string());
        // Strips any since/limit already baked into the configured URL — see
        // `drain_page_url` for the silent stall this prevents.
        let url =
            crate::routes::first_party_analytics::drain_page_url(drain_url, &since, PAGE_LIMIT);

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

        // Record what the relay says it holds, every page (last wins). Set
        // even on an empty page: "cursor == latest" is the proof of caught-up.
        if parsed.latest_id.is_some() {
            config.relay_latest_id = parsed.latest_id.clone();
        }

        // Retention hole: the relay's oldest surviving row is newer than where
        // we left off, so events were pruned before we ever fetched them. Say
        // so loudly and durably — a silent under-report is indistinguishable
        // from a quiet traffic week, which is exactly how this stays unnoticed.
        if let (Some(first), Some(cursor)) = (&parsed.first_available_id, &config.cursor) {
            if numeric_id_gt(first, cursor) {
                let msg = format!(
                    "retention gap: the site's oldest kept event is {first} but this hub last \
                     drained {cursor} — events in between were pruned before they were \
                     collected and cannot be recovered"
                );
                tracing::warn!(target: "analytics_drain", "{msg}");
                retention_warning = Some(msg.clone());
                // Also set now, so an early `?` return from ingest below still
                // carries the explanation out with it.
                config.last_error = Some(msg);
                // Skip forward: staying behind the retention floor would refetch
                // nothing forever while the backlog silently grew.
                config.cursor = Some(first.clone());
            }
        }

        let batch = parsed.events.len();
        if batch == 0 {
            break;
        }
        for ev in &parsed.events {
            insert_event(pool, project_id, ev, site_host.as_deref()).await?;
            config.cursor = Some(ev.id.clone());
        }
        total += batch;
        if (batch as u32) < PAGE_LIMIT {
            break; // drained to the tip
        }
    }
    // A clean pass clears a stale error; a pass that found a hole keeps saying
    // so, next to the cursor that skipped over it.
    config.last_error = retention_warning;
    config.last_drain_at = Some(chrono::Utc::now().to_rfc3339());
    Ok(total)
}

/// Compare two drain ids numerically when both parse as integers, else
/// lexically. Relay ids are `bigIncrements` in the reference implementation,
/// but a string-id relay must not make this comparison nonsense.
fn numeric_id_gt(a: &str, b: &str) -> bool {
    match (a.parse::<u64>(), b.parse::<u64>()) {
        (Ok(x), Ok(y)) => x > y,
        _ => a > b,
    }
}

async fn insert_event(
    pool: &Pool<Sqlite>,
    project_id: &str,
    ev: &DrainEvent,
    // The site's own host, derived from the drain URL. Used to drop
    // self-referrals; `None` disables the check rather than guessing.
    site_host: Option<&str>,
) -> Result<(), String> {
    let kind = match ev.kind.as_deref() {
        Some("event") | Some("ev") => "event",
        _ => "pageview",
    };
    let created_at = ev
        .at
        .clone()
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());

    // INSERT OR IGNORE against UNIQUE(project_id, source_event_id): re-draining
    // the same window is a no-op instead of inflating every count.
    // Campaign params ride in the path when the relay does not split them out,
    // so extract here too and store the path without its query — otherwise
    // every ?utm_source= variant is a separate "page".
    let raw_path = ev.path.as_deref().unwrap_or("/");
    let utm = classify::extract_utm(raw_path);
    let stored_path = classify::normalize_path(raw_path);
    // Trust the site's flag when it sent one; otherwise classify from the user
    // agent if the relay forwarded it. A relay that sends neither yields false,
    // which is the honest default — we cannot know.
    let is_bot = ev.is_bot.unwrap_or_else(|| {
        ev.user_agent
            .as_deref()
            .map(|ua| classify::is_bot(Some(ua)))
            .unwrap_or(false)
    }) as i64;
    let properties = ev
        .properties
        .as_ref()
        .and_then(classify::sanitize_properties);
    // Drop self-referrals at ingest. `document.referrer` is the site's own URL
    // on every hard internal load — a reload, an open-in-new-tab, or crossing a
    // locale boundary that is a full navigation — so storing it raw makes the
    // site the top "referrer" of itself. The snippet cannot fix this without
    // knowing its own host reliably; the daemon does, from the drain URL. Fixed
    // here rather than in each query so every downstream aggregate inherits it.
    // (Reported against reckonize.org, 2026-08-06.)
    // Normalize BOTH sides through referrer_host: it lowercases and strips
    // `www.`, so comparing a normalized host against a raw one silently never
    // matches (caught by CI — "www.reckonize.org" vs "reckonize.org"). Running
    // the site host through it too makes this correct whether the caller hands
    // us a bare host or a full URL, instead of resting on an implicit contract.
    let own_host = site_host.and_then(classify::referrer_host);
    let referrer = ev.referrer.as_deref().filter(|r| {
        match (classify::referrer_host(r), own_host.as_deref()) {
            (Some(h), Some(own)) => h != own,
            _ => true,
        }
    });

    // INSERT OR IGNORE against UNIQUE(project_id, source_event_id): re-draining
    // the same window is a no-op instead of inflating every count.
    sqlx::query(
        "INSERT OR IGNORE INTO analytics_events
            (project_id, kind, path, referrer, name, visitor_hash, created_at, source_event_id,
             properties, is_bot, session_id, utm_source, utm_medium, utm_campaign, country)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
    )
    .bind(project_id)
    .bind(kind)
    .bind(&stored_path)
    .bind(referrer)
    .bind(ev.name.as_deref())
    .bind(ev.visitor_hash.as_deref())
    .bind(created_at)
    .bind(&ev.id)
    .bind(&properties)
    .bind(is_bot)
    .bind(ev.session_id.as_deref())
    .bind(ev.utm_source.as_deref().or(utm.source.as_deref()))
    .bind(ev.utm_medium.as_deref().or(utm.medium.as_deref()))
    .bind(ev.utm_campaign.as_deref().or(utm.campaign.as_deref()))
    .bind(ev.country.as_deref())
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
            properties: None,
            is_bot: None,
            session_id: None,
            utm_source: None,
            utm_medium: None,
            utm_campaign: None,
            country: None,
            user_agent: None,
        }
    }

    /// Re-draining an overlapping window must not double-count. Without the
    /// UNIQUE(project_id, source_event_id) + INSERT OR IGNORE pair, one retry
    /// silently inflates every number on the Analytics page.
    #[tokio::test]
    async fn redraining_the_same_events_is_a_no_op() {
        let pool = mem_pool().await;
        let e = ev("42", "2026-07-01T10:00:00Z");
        insert_event(&pool, "p1", &e, None).await.unwrap();
        insert_event(&pool, "p1", &e, None).await.unwrap();
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM analytics_events")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 1, "same source id must insert once");
    }

    /// Self-referrals must not be stored. `document.referrer` is the site's own
    /// URL on every hard internal load (reload, new tab, locale switch), so
    /// storing it raw makes the site its own top referrer — reported live
    /// against reckonize.org, 2026-08-06.
    #[tokio::test]
    async fn same_host_referrer_is_dropped_but_foreign_referrers_are_kept() {
        let pool = mem_pool().await;
        let site = Some("www.reckonize.org");

        let mut own = ev("1", "2026-07-01T10:00:00Z");
        own.referrer = Some("https://www.reckonize.org/en/pricing".into());
        insert_event(&pool, "p1", &own, site).await.unwrap();

        let mut foreign = ev("2", "2026-07-01T10:00:00Z");
        foreign.referrer = Some("https://news.ycombinator.com/".into());
        insert_event(&pool, "p1", &foreign, site).await.unwrap();

        let stored: Vec<Option<String>> =
            sqlx::query_scalar("SELECT referrer FROM analytics_events ORDER BY source_event_id")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(stored[0], None, "a self-referral must not be stored");
        assert_eq!(
            stored[1].as_deref(),
            Some("https://news.ycombinator.com/"),
            "a genuine referrer must survive"
        );
    }

    /// The source's timestamp must be preserved. Letting created_at default to
    /// now() would stamp a week of backlog with the fetch time and collapse
    /// every chart into a single day.
    #[tokio::test]
    async fn event_time_is_preserved_not_ingest_time() {
        let pool = mem_pool().await;
        insert_event(&pool, "p1", &ev("1", "2026-07-01T10:00:00Z"), None)
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
        insert_event(&pool, "p1", &ev("7", "2026-07-01T10:00:00Z"), None)
            .await
            .unwrap();
        insert_event(&pool, "p2", &ev("7", "2026-07-01T10:00:00Z"), None)
            .await
            .unwrap();
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM analytics_events")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 2);
    }

    /// The envelope's own wire contract. Both v41 fields were declared
    /// snake_case while the install brief tells the site to emit camelCase, so
    /// neither ever bound — drain-lag reporting and retention-gap detection
    /// both read `None` from every correct relay and failed silently. Nothing
    /// caught it because nothing asserted on the envelope itself.
    #[test]
    fn the_drain_envelope_reads_the_camel_case_the_brief_asks_for() {
        let parsed: DrainResponse =
            serde_json::from_str(r#"{"events":[],"latestId":"5001","firstAvailableId":"5000"}"#)
                .expect("the documented shape parses");
        assert_eq!(parsed.latest_id.as_deref(), Some("5001"));
        assert_eq!(parsed.first_available_id.as_deref(), Some("5000"));
    }

    /// Relay ids are `bigIncrements` in the reference implementation, so a site
    /// emitting them as JSON numbers is the normal case, not an edge one — and
    /// a parse failure here stalls that project's cursor entirely.
    #[test]
    fn envelope_ids_survive_arriving_as_numbers() {
        let parsed: DrainResponse =
            serde_json::from_str(r#"{"events":[],"latestId":5001,"firstAvailableId":5000}"#)
                .expect("numeric ids parse");
        assert_eq!(parsed.latest_id.as_deref(), Some("5001"));
        assert_eq!(parsed.first_available_id.as_deref(), Some("5000"));
    }

    /// A relay built against the earlier contract carries neither field and
    /// must keep working rather than failing to parse and stalling.
    #[test]
    fn an_older_relay_without_the_v41_fields_still_parses() {
        let parsed: DrainResponse =
            serde_json::from_str(r#"{"events":[]}"#).expect("the pre-v41 shape parses");
        assert!(parsed.latest_id.is_none());
        assert!(parsed.first_available_id.is_none());
    }

    /// A retention gap is not an `Err`, so it reached the success path that
    /// clears `last_error` — and the one durable record that events were
    /// pruned before we collected them was erased before the caller persisted
    /// the state. A silent under-report is indistinguishable from a quiet
    /// traffic week, which is exactly how this would stay unnoticed.
    #[tokio::test]
    async fn a_retention_gap_survives_into_the_persisted_state() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        // We left off at 1200; the relay's oldest surviving row is 5000.
        Mock::given(method("GET"))
            .and(path("/api/permagent-analytics/drain"))
            .and(query_param("since", "1200"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "events": [
                    { "id": 5001, "kind": "pageview", "path": "/", "referrer": null,
                      "name": null, "visitorHash": "aa11", "at": "2026-08-06T09:00:00.000Z" }
                ],
                "latestId": "5001",
                "firstAvailableId": "5000"
            })))
            .mount(&server)
            .await;

        let pool = mem_pool().await;
        let mut state = crate::routes::first_party_analytics::DrainState {
            site_key: "k".into(),
            ingest_base: None,
            drain_url: None,
            cursor: Some("1200".into()),
            last_drain_at: None,
            last_error: None,
            relay_latest_id: None,
        };
        let url = format!("{}/api/permagent-analytics/drain", server.uri());

        let n = drain_project(&pool, "p1", &url, "SECRET123", &mut state)
            .await
            .expect("a gap is not a failure — the drain still succeeds");

        assert_eq!(n, 1, "the surviving event was still ingested");
        assert!(
            state
                .last_error
                .as_deref()
                .unwrap_or_default()
                .contains("retention gap"),
            "the gap was erased before it could be persisted: {:?}",
            state.last_error
        );
        assert_eq!(
            state.cursor.as_deref(),
            Some("5001"),
            "skipped forward over the hole and then advanced normally"
        );
        assert!(state.last_drain_at.is_some(), "freshness still recorded");
    }

    /// END-TO-END CONTRACT TEST. Stands up a mock site speaking exactly the
    /// JSON the install brief tells the coding agent to produce, and drains it.
    /// If the brief and the poller ever drift apart, this fails — which is the
    /// only thing standing between "the agent built it right" and silent zero.
    #[tokio::test]
    async fn drains_a_site_that_followed_the_install_brief() {
        use wiremock::matchers::{header, method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        // Page 1: exactly the shape documented in agent_prompt_for().
        Mock::given(method("GET"))
            .and(path("/api/permagent-analytics/drain"))
            .and(header("x-permagent-key", "SECRET123"))
            .and(query_param("since", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "events": [
                    { "id": 1, "kind": "pageview", "path": "/", "referrer": null,
                      "name": null, "visitorHash": "aa11", "at": "2026-07-20T09:00:00.000Z" },
                    { "id": 2, "kind": "event", "path": "/deals", "referrer": "https://google.com",
                      "name": "signup", "visitorHash": "bb22", "at": "2026-07-21T10:30:00.000Z" }
                ]
            })))
            .mount(&server)
            .await;
        // Page 2 (cursor advanced): empty — drained to the tip.
        Mock::given(method("GET"))
            .and(path("/api/permagent-analytics/drain"))
            .and(query_param("since", "2"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "events": [] })),
            )
            .mount(&server)
            .await;

        let pool = mem_pool().await;
        let mut state = crate::routes::first_party_analytics::DrainState {
            site_key: "k".into(),
            ingest_base: None,
            drain_url: None,
            cursor: None,
            last_drain_at: None,
            last_error: None,
            relay_latest_id: None,
        };
        let url = format!("{}/api/permagent-analytics/drain", server.uri());

        let n = drain_project(&pool, "p1", &url, "SECRET123", &mut state)
            .await
            .expect("drain succeeds");
        assert_eq!(n, 2, "both events ingested");
        assert_eq!(
            state.cursor.as_deref(),
            Some("2"),
            "cursor advanced to the tip"
        );
        assert!(state.last_error.is_none());
        assert!(
            state.last_drain_at.is_some(),
            "freshness recorded for the UI"
        );

        // Event identity survived the hop: kind, name, referrer, and — the one
        // that silently ruins charts — the ORIGINAL timestamps.
        let rows: Vec<(String, String, Option<String>, String)> = sqlx::query_as(
            "SELECT kind, path, name, created_at FROM analytics_events ORDER BY source_event_id",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, "pageview");
        assert_eq!(rows[1].0, "event");
        assert_eq!(rows[1].2.as_deref(), Some("signup"));
        assert!(rows[0].3.starts_with("2026-07-20"), "got {}", rows[0].3);
        assert!(rows[1].3.starts_with("2026-07-21"), "got {}", rows[1].3);

        // A second pass (daemon restarted, same window re-offered) must not
        // double-count — the failure mode that inflates every number.
        state.cursor = None;
        drain_project(&pool, "p1", &url, "SECRET123", &mut state)
            .await
            .unwrap();
        let total: i64 = sqlx::query_scalar("SELECT count(*) FROM analytics_events")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(total, 2, "re-drain must not duplicate");
    }

    /// A wrong/missing key must surface as a visible error, not silent zero.
    #[tokio::test]
    async fn rejected_key_reports_an_error() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(401).set_body_string("Unauthorized"))
            .mount(&server)
            .await;

        let pool = mem_pool().await;
        let mut state = crate::routes::first_party_analytics::DrainState {
            site_key: "k".into(),
            ingest_base: None,
            drain_url: None,
            cursor: None,
            last_drain_at: None,
            last_error: None,
            relay_latest_id: None,
        };
        let err = drain_project(&pool, "p1", &server.uri(), "WRONG", &mut state)
            .await
            .unwrap_err();
        assert!(err.contains("401"), "got {err}");
        assert!(
            state.last_error.as_deref().unwrap_or("").contains("401"),
            "the UI must be able to show why ingestion stopped"
        );
    }
}
