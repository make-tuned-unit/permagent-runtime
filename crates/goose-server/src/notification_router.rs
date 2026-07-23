//! Per-user notification routing (#66).
//!
//! Workflow code emits facts (`goal_state_changed`, `decision_created`, ...).
//! This daemon-side consumer is the single policy boundary which classifies a
//! fact and chooses delivery channels. Critical events can reach phone push,
//! warnings surface in-app, and informational events wait in a durable daily
//! digest. Channel thresholds are stored per user; NULL disables a channel.

use chrono::{Local, NaiveDate, Timelike};
use permagent::events::{self, PermagentEvent, PermagentEventType};
use sqlx::{Pool, Row, Sqlite};
use std::sync::Arc;
use std::time::Duration;

use crate::state::AppState;

const DEFAULT_USER: &str = "default";
const DIGEST_TICK: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info = 1,
    Warning = 2,
    Critical = 3,
}

impl Severity {
    fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Critical => "critical",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Push,
    InApp,
    Digest,
}

impl Channel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Push => "push",
            Self::InApp => "in_app",
            Self::Digest => "digest",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Preferences {
    push_min: Option<i64>,
    in_app_min: Option<i64>,
    digest_min: Option<i64>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            push_min: Some(3),
            in_app_min: Some(2),
            digest_min: Some(1),
        }
    }
}

impl Preferences {
    fn channels(self, severity: Severity) -> Vec<Channel> {
        let level = severity as i64;
        let mut channels = Vec::new();
        if self.push_min.is_some_and(|minimum| level >= minimum) {
            channels.push(Channel::Push);
        }
        if self.in_app_min.is_some_and(|minimum| level >= minimum) {
            channels.push(Channel::InApp);
        }
        if self.digest_min.is_some_and(|minimum| level >= minimum) {
            channels.push(Channel::Digest);
        }
        channels
    }
}

/// Deterministic severity policy. Events absent from this match are ordinary
/// liveness/telemetry and must never interrupt the user.
pub fn classify(event: &PermagentEvent) -> Option<Severity> {
    match &event.event_type {
        PermagentEventType::TaskFailed | PermagentEventType::IntegrationError => {
            Some(Severity::Critical)
        }
        PermagentEventType::DecisionCreated => match event.payload["tier"].as_i64() {
            Some(2..) => Some(Severity::Critical),
            Some(_) => Some(Severity::Warning),
            None => Some(Severity::Critical), // malformed approval fails noisy
        },
        PermagentEventType::GoalStateChanged => match event.payload["to"].as_str() {
            Some("failed" | "parked") => Some(Severity::Critical),
            Some("review" | "needs_attention" | "blocked") => Some(Severity::Warning),
            Some("complete" | "completed" | "deleted") => Some(Severity::Info),
            _ => None,
        },
        PermagentEventType::ProactiveNudge | PermagentEventType::TaskCompleted => {
            Some(Severity::Info)
        }
        // Routed events are deliberately ignored, preventing an event-bus loop.
        _ => None,
    }
}

pub fn spawn(_state: Arc<AppState>) {
    // Subscribe synchronously before spawning so startup cannot lose an event
    // in the gap between task creation and its first poll.
    let mut receiver = events::subscribe();
    tokio::spawn(async move {
        let pool = match permagent::session::SessionManager::instance()
            .pool_clone()
            .await
        {
            Ok(pool) => pool,
            Err(error) => {
                tracing::error!(target: "permagentd::notifications", %error, "notification router could not open the session database");
                return;
            }
        };
        let mut digest_tick = tokio::time::interval(DIGEST_TICK);
        loop {
            tokio::select! {
                received = receiver.recv() => match received {
                    Ok(event) => {
                        if let Err(error) = route(&pool, event).await {
                            tracing::warn!(target: "permagentd::notifications", %error, "notification routing failed");
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                        tracing::warn!(target: "permagentd::notifications", count, "notification router lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
                _ = digest_tick.tick() => {
                    if let Err(error) = deliver_due_digests(&pool).await {
                        tracing::warn!(target: "permagentd::notifications", %error, "daily digest delivery failed");
                    }
                }
            }
        }
    });
}

async fn preferences(pool: &Pool<Sqlite>, user_id: &str) -> anyhow::Result<Preferences> {
    sqlx::query(
        "INSERT OR IGNORE INTO notification_preferences
         (user_id, push_min_severity, in_app_min_severity, digest_min_severity)
         VALUES (?, 3, 2, 1)",
    )
    .bind(user_id)
    .execute(pool)
    .await?;
    let row = sqlx::query(
        "SELECT push_min_severity, in_app_min_severity, digest_min_severity
         FROM notification_preferences WHERE user_id = ?",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map_or_else(Preferences::default, |row| Preferences {
        push_min: row.get(0),
        in_app_min: row.get(1),
        digest_min: row.get(2),
    }))
}

async fn route(pool: &Pool<Sqlite>, event: PermagentEvent) -> anyhow::Result<()> {
    route_with_push_outcome(pool, event, None).await
}

async fn route_with_push_outcome(
    pool: &Pool<Sqlite>,
    event: PermagentEvent,
    forced_push_outcome: Option<bool>,
) -> anyhow::Result<()> {
    let Some(severity) = classify(&event) else {
        return Ok(());
    };
    let user_id = event.payload["user_id"].as_str().unwrap_or(DEFAULT_USER);
    let source_type = serde_json::to_value(&event.event_type)?
        .as_str()
        .unwrap_or("unknown")
        .to_owned();
    for channel in preferences(pool, user_id).await?.channels(severity) {
        match channel {
            Channel::Push => {
                let delivered = match forced_push_outcome {
                    Some(delivered) => delivered,
                    None => push(&source_type, severity, &event.payload).await,
                };
                if delivered {
                    break;
                }
            }
            Channel::InApp => {
                events::emit(events::notification_routed(
                    &event.id,
                    user_id,
                    severity.as_str(),
                    channel.as_str(),
                    &source_type,
                    &event.payload,
                ));
                break;
            }
            Channel::Digest => {
                sqlx::query(
                    "INSERT OR IGNORE INTO notification_digest_entries
                     (user_id, source_event_id, severity, source_type, payload_json)
                     VALUES (?, ?, ?, ?, ?)",
                )
                .bind(user_id)
                .bind(&event.id)
                .bind(severity as i64)
                .bind(&source_type)
                .bind(serde_json::to_string(&event.payload)?)
                .execute(pool)
                .await?;
                break;
            }
        }
    }
    Ok(())
}

async fn deliver_due_digests(pool: &Pool<Sqlite>) -> anyhow::Result<()> {
    let now = Local::now();
    deliver_due_digests_at(pool, now.hour() as i64, now.date_naive()).await
}

async fn deliver_due_digests_at(
    pool: &Pool<Sqlite>,
    hour: i64,
    today: NaiveDate,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT OR IGNORE INTO notification_preferences
         (user_id, push_min_severity, in_app_min_severity, digest_min_severity)
         SELECT DISTINCT user_id, 3, 2, 1 FROM notification_digest_entries
         WHERE delivered_at IS NULL",
    )
    .execute(pool)
    .await?;
    let today = today.format("%Y-%m-%d").to_string();
    let users = sqlx::query(
        "SELECT p.user_id FROM notification_preferences p
         WHERE p.digest_min_severity IS NOT NULL AND p.digest_hour_local <= ?
           AND (p.last_digest_delivery_date IS NULL
                OR p.last_digest_delivery_date <> ?)
           AND EXISTS (SELECT 1 FROM notification_digest_entries d
                       WHERE d.user_id = p.user_id AND d.delivered_at IS NULL)",
    )
    .bind(hour)
    .bind(&today)
    .fetch_all(pool)
    .await?;
    for user in users {
        let user_id: String = user.get(0);
        let mut tx = pool.begin().await?;
        let rows = sqlx::query(
            "SELECT id, source_event_id, severity, source_type, payload_json
             FROM notification_digest_entries
             WHERE user_id = ? AND delivered_at IS NULL ORDER BY created_at, id",
        )
        .bind(&user_id)
        .fetch_all(&mut *tx)
        .await?;
        if rows.is_empty() {
            tx.rollback().await?;
            continue;
        }
        let mut routed_events = Vec::with_capacity(rows.len());
        for row in &rows {
            let payload_json: String = row.get(4);
            let payload: serde_json::Value =
                serde_json::from_str(&payload_json).unwrap_or_default();
            let severity = match row.get::<i64, _>(2) {
                3 => "critical",
                2 => "warning",
                _ => "info",
            };
            routed_events.push(events::notification_routed(
                &row.get::<String, _>(1),
                &user_id,
                severity,
                "digest",
                &row.get::<String, _>(3),
                &payload,
            ));
        }
        sqlx::query(
            "UPDATE notification_digest_entries
             SET delivered_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
             WHERE user_id = ? AND delivered_at IS NULL",
        )
        .bind(&user_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE notification_preferences
             SET last_digest_delivery_date = ?,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
             WHERE user_id = ?",
        )
        .bind(&today)
        .bind(&user_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        for event in routed_events {
            events::emit(event);
        }
        events::emit(events::notification_digest_ready(
            &user_id,
            rows.len() as i64,
        ));
    }
    Ok(())
}

async fn push(source_type: &str, severity: Severity, payload: &serde_json::Value) -> bool {
    let Ok(topic) = std::env::var("PERMAGENT_NTFY_TOPIC") else {
        return false;
    };
    if topic.trim().is_empty() {
        return false;
    }
    let server =
        std::env::var("PERMAGENT_NTFY_SERVER").unwrap_or_else(|_| "https://ntfy.sh".to_owned());
    let url = format!("{}/{}", server.trim_end_matches('/'), topic.trim());
    let body = match source_type {
        "decision_created" => format!(
            "A decision needs your attention ({})",
            payload["kind"].as_str().unwrap_or("approval")
        ),
        "goal_state_changed" => format!(
            "Goal {} is {}",
            payload["goal_id"].as_str().unwrap_or(""),
            payload["to"].as_str().unwrap_or("waiting")
        ),
        _ => format!("Permagent reported {source_type}"),
    };
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
    else {
        return false;
    };
    match client
        .post(url)
        .header("Title", format!("Permagent {}", severity.as_str()))
        .header(
            "Priority",
            if severity == Severity::Critical {
                "urgent"
            } else {
                "default"
            },
        )
        .body(body)
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => true,
        Ok(response) => {
            tracing::warn!(target: "permagentd::notifications", status = %response.status(), "phone push rejected");
            false
        }
        Err(error) => {
            tracing::warn!(target: "permagentd::notifications", %error, "phone push failed");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: PermagentEventType, payload: serde_json::Value) -> PermagentEvent {
        PermagentEvent::new(kind, payload)
    }

    #[test]
    fn classifies_decisions_and_goal_outcomes() {
        assert_eq!(
            classify(&event(
                PermagentEventType::DecisionCreated,
                serde_json::json!({"tier": 2})
            )),
            Some(Severity::Critical)
        );
        assert_eq!(
            classify(&event(
                PermagentEventType::DecisionCreated,
                serde_json::json!({"tier": 1})
            )),
            Some(Severity::Warning)
        );
        assert_eq!(
            classify(&event(
                PermagentEventType::GoalStateChanged,
                serde_json::json!({"to": "failed"})
            )),
            Some(Severity::Critical)
        );
        assert_eq!(
            classify(&event(
                PermagentEventType::GoalStateChanged,
                serde_json::json!({"to": "review"})
            )),
            Some(Severity::Warning)
        );
        assert_eq!(
            classify(&event(
                PermagentEventType::GoalStateChanged,
                serde_json::json!({"to": "complete"})
            )),
            Some(Severity::Info)
        );
    }

    #[test]
    fn ignores_routed_and_liveness_events() {
        assert_eq!(
            classify(&event(
                PermagentEventType::NotificationRouted,
                serde_json::json!({})
            )),
            None
        );
        assert_eq!(
            classify(&event(
                PermagentEventType::AgentStateChanged,
                serde_json::json!({})
            )),
            None
        );
    }

    #[test]
    fn default_thresholds_route_by_urgency() {
        let prefs = Preferences::default();
        assert_eq!(prefs.channels(Severity::Info), vec![Channel::Digest]);
        assert_eq!(
            prefs.channels(Severity::Warning),
            vec![Channel::InApp, Channel::Digest]
        );
        assert_eq!(
            prefs.channels(Severity::Critical),
            vec![Channel::Push, Channel::InApp, Channel::Digest]
        );
    }

    async fn routing_pool() -> Pool<Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("CREATE TABLE users (id TEXT PRIMARY KEY)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO users VALUES ('default')")
            .execute(&pool)
            .await
            .unwrap();
        permagent::session::spectral_schema::apply_notification_routing_schema(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn wiring_emits_in_app_instruction_for_warning() {
        let pool = routing_pool().await;
        let mut receiver = events::subscribe();
        let source = event(
            PermagentEventType::DecisionCreated,
            serde_json::json!({"decision_id": "d-1", "kind": "choice", "tier": 1}),
        );
        route(&pool, source.clone()).await.unwrap();

        let routed = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let candidate = receiver.recv().await.unwrap();
                if candidate.event_type == PermagentEventType::NotificationRouted
                    && candidate.payload["source_event_id"] == source.id
                {
                    break candidate;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(routed.event_type, PermagentEventType::NotificationRouted);
        assert_eq!(routed.payload["source_event_id"], source.id);
        assert_eq!(routed.payload["severity"], "warning");
        assert_eq!(routed.payload["channel"], "in_app");
    }

    #[tokio::test]
    async fn unavailable_or_rejected_push_falls_through_to_in_app() {
        let pool = routing_pool().await;
        let mut receiver = events::subscribe();
        let source = event(
            PermagentEventType::TaskFailed,
            serde_json::json!({"task_id": "t-critical"}),
        );
        route_with_push_outcome(&pool, source.clone(), Some(false))
            .await
            .unwrap();
        let routed = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let candidate = receiver.recv().await.unwrap();
                if candidate.event_type == PermagentEventType::NotificationRouted
                    && candidate.payload["source_event_id"] == source.id
                {
                    break candidate;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(routed.payload["source_event_id"], source.id);
        assert_eq!(routed.payload["channel"], "in_app");
        assert_eq!(routed.payload["severity"], "critical");
    }

    #[tokio::test]
    async fn replayed_info_event_queues_digest_once() {
        let pool = routing_pool().await;
        let source = event(
            PermagentEventType::TaskCompleted,
            serde_json::json!({"task_id": "t-1"}),
        );
        route(&pool, source.clone()).await.unwrap();
        route(&pool, source.clone()).await.unwrap();
        let queued: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM notification_digest_entries WHERE source_event_id = ?",
        )
        .bind(source.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(queued, 1, "event replay cannot duplicate a digest entry");
    }

    #[tokio::test]
    async fn per_user_null_threshold_disables_channel() {
        let pool = routing_pool().await;
        sqlx::query(
            "UPDATE notification_preferences
             SET push_min_severity = NULL, in_app_min_severity = NULL
             WHERE user_id = 'default'",
        )
        .execute(&pool)
        .await
        .unwrap();
        let prefs = preferences(&pool, "default").await.unwrap();
        assert_eq!(prefs.channels(Severity::Critical), vec![Channel::Digest]);
    }

    #[tokio::test]
    async fn user_created_after_migration_gets_defaults_and_digest_delivery() {
        let pool = routing_pool().await;
        sqlx::query("INSERT INTO users VALUES ('later-user')")
            .execute(&pool)
            .await
            .unwrap();
        let source = event(
            PermagentEventType::TaskCompleted,
            serde_json::json!({"task_id": "late", "user_id": "later-user"}),
        );
        route(&pool, source).await.unwrap();
        let preference_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM notification_preferences WHERE user_id = 'later-user'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(preference_count, 1);

        deliver_due_digests_at(&pool, 23, NaiveDate::from_ymd_opt(2026, 7, 22).unwrap())
            .await
            .unwrap();
        let pending: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM notification_digest_entries
             WHERE user_id = 'later-user' AND delivered_at IS NULL",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(pending, 0);
    }

    #[tokio::test]
    async fn due_digest_emits_entries_once_and_marks_them_delivered() {
        let pool = routing_pool().await;
        sqlx::query(
            "UPDATE notification_preferences SET digest_hour_local = ?
             WHERE user_id = 'default'",
        )
        .bind(Local::now().hour() as i64)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO notification_digest_entries
             (user_id, source_event_id, severity, source_type, payload_json)
             VALUES ('default', 'digest-source', 1, 'task_completed', '{\"task_id\":\"t-1\"}')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let mut receiver = events::subscribe();
        deliver_due_digests(&pool).await.unwrap();

        let delivered = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let candidate = receiver.recv().await.unwrap();
                if candidate.event_type == PermagentEventType::NotificationRouted
                    && candidate.payload["source_event_id"] == "digest-source"
                {
                    break candidate;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(delivered.payload["channel"], "digest");
        let pending: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM notification_digest_entries WHERE delivered_at IS NULL",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(pending, 0);

        deliver_due_digests(&pool).await.unwrap();
        let delivered_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM notification_digest_entries WHERE delivered_at IS NOT NULL",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(delivered_count, 1, "a later tick cannot redeliver the row");
    }

    #[tokio::test]
    async fn overdue_digest_delivers_once_per_local_date() {
        let pool = routing_pool().await;
        sqlx::query(
            "UPDATE notification_preferences SET digest_hour_local = 8
             WHERE user_id = 'default'",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO notification_digest_entries
             (user_id, source_event_id, severity, source_type, payload_json)
             VALUES ('default', 'overdue', 1, 'task_completed', '{}')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 7, 22).unwrap();
        deliver_due_digests_at(&pool, 10, today).await.unwrap();
        let delivered: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM notification_digest_entries
             WHERE source_event_id = 'overdue' AND delivered_at IS NOT NULL",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let last_date: String = sqlx::query_scalar(
            "SELECT last_digest_delivery_date FROM notification_preferences
             WHERE user_id = 'default'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(delivered, 1);
        assert_eq!(last_date, "2026-07-22");

        sqlx::query(
            "INSERT INTO notification_digest_entries
             (user_id, source_event_id, severity, source_type, payload_json)
             VALUES ('default', 'same-day', 1, 'task_completed', '{}')",
        )
        .execute(&pool)
        .await
        .unwrap();
        deliver_due_digests_at(&pool, 12, today).await.unwrap();
        let still_pending: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM notification_digest_entries
             WHERE source_event_id = 'same-day' AND delivered_at IS NULL",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            still_pending, 1,
            "only one digest is delivered per local date"
        );
    }

    #[tokio::test]
    async fn failed_delivery_transaction_emits_no_digest_entry() {
        let pool = routing_pool().await;
        sqlx::query(
            "UPDATE notification_preferences SET digest_hour_local = 0
             WHERE user_id = 'default'",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO notification_digest_entries
             (user_id, source_event_id, severity, source_type, payload_json)
             VALUES ('default', 'must-not-emit', 1, 'task_completed', '{}')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TRIGGER reject_digest_delivery BEFORE UPDATE OF delivered_at
             ON notification_digest_entries BEGIN SELECT RAISE(ABORT, 'test failure'); END",
        )
        .execute(&pool)
        .await
        .unwrap();
        let mut receiver = events::subscribe();
        let result =
            deliver_due_digests_at(&pool, 23, NaiveDate::from_ymd_opt(2026, 7, 22).unwrap()).await;
        assert!(result.is_err());
        let emitted = tokio::time::timeout(Duration::from_millis(50), async {
            loop {
                let candidate = receiver.recv().await.unwrap();
                if candidate.event_type == PermagentEventType::NotificationRouted
                    && candidate.payload["source_event_id"] == "must-not-emit"
                {
                    break;
                }
            }
        })
        .await;
        assert!(emitted.is_err(), "rolled-back rows must not be emitted");
    }
}
