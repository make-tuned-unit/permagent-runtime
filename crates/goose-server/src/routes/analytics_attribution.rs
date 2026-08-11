//! Session traffic-source rollup for every drained project.
//!
//! Prefer the first `session_attribution` event in a browser session as
//! first-touch source/medium. When a project has not emitted that event yet,
//! fall back to the first pageview's `referrer` via the shared hostname map in
//! [`analytics_classify`]. Never require `utm_*` to populate Traffic sources.

use crate::routes::analytics_classify::{
    attribute_from_referrer, attribute_from_session_props, TrafficAttribution,
};
use sqlx::{Pool, Sqlite};
use std::collections::HashMap;

/// Named event emitters may send once per browser session.
pub const SESSION_ATTRIBUTION_EVENT: &str = "session_attribution";
pub const ANSWER_ENGINE_VISIT_EVENT: &str = "answer_engine_visit";

#[derive(Debug, Clone, Default)]
pub struct TrafficSourceRollup {
    /// Ranked `"source / medium"` → session (or sessionless hit) counts.
    pub top_sources: Vec<(String, i64)>,
    /// Sessions (plus sessionless hits) whose first-touch medium is `aeo`,
    /// unioned with explicit `answer_engine_visit` events.
    pub aeo_visits: i64,
    /// session_id → first-touch attribution (for funnel slicing).
    pub by_session: HashMap<String, TrafficAttribution>,
}

/// Load first-touch attribution for every session in the window, plus
/// sessionless pageviews, and rank Traffic sources.
pub async fn rollup_traffic_sources(
    pool: &Pool<Sqlite>,
    project_id: &str,
    since: &str,
    include_bots: bool,
) -> TrafficSourceRollup {
    let bot_filter = if include_bots { "" } else { " AND is_bot = 0" };

    // 1) Explicit session_attribution events — first per session wins.
    let attr_rows = sqlx::query_as::<_, (Option<String>, Option<String>, i64)>(&format!(
        "SELECT session_id, properties, id FROM analytics_events
         WHERE project_id = ?1 AND kind = 'event' AND name = '{SESSION_ATTRIBUTION_EVENT}'
           AND created_at >= datetime('now', ?2){bot_filter}
         ORDER BY id ASC"
    ))
    .bind(project_id)
    .bind(since)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let mut by_session: HashMap<String, TrafficAttribution> = HashMap::new();
    for (session_id, properties, _) in &attr_rows {
        let Some(sid) = session_id.as_deref().filter(|s| !s.is_empty()) else {
            continue;
        };
        if by_session.contains_key(sid) {
            continue;
        }
        let attr = match properties
            .as_deref()
            .and_then(|p| serde_json::from_str::<serde_json::Value>(p).ok())
        {
            Some(props) => attribute_from_session_props(&props),
            None => attribute_from_referrer(None),
        };
        by_session.insert(sid.to_string(), attr);
    }

    // 2) First pageview referrer per session — fill gaps only.
    let pv_rows = sqlx::query_as::<_, (Option<String>, Option<String>, i64)>(&format!(
        "SELECT session_id, referrer, id FROM analytics_events
         WHERE project_id = ?1 AND kind = 'pageview'
           AND created_at >= datetime('now', ?2){bot_filter}
         ORDER BY id ASC"
    ))
    .bind(project_id)
    .bind(since)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let mut session_counted: HashMap<String, bool> = HashMap::new();
    let mut counts: HashMap<String, i64> = HashMap::new();
    let mut aeo_keys: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (session_id, referrer, _) in &pv_rows {
        match session_id.as_deref().filter(|s| !s.is_empty()) {
            Some(sid) => {
                if session_counted.contains_key(sid) {
                    continue;
                }
                session_counted.insert(sid.to_string(), true);
                let attr = by_session
                    .entry(sid.to_string())
                    .or_insert_with(|| attribute_from_referrer(referrer.as_deref()))
                    .clone();
                *counts.entry(attr.label()).or_default() += 1;
                if attr.is_aeo() {
                    aeo_keys.insert(format!("s:{sid}"));
                }
            }
            None => {
                // Sessionless pageviews still contribute via referrer so legacy
                // relays without session ids are not invisible.
                let attr = attribute_from_referrer(referrer.as_deref());
                *counts.entry(attr.label()).or_default() += 1;
                if attr.is_aeo() {
                    aeo_keys.insert(format!(
                        "r:{}:{}",
                        referrer.as_deref().unwrap_or(""),
                        counts.get(&attr.label()).copied().unwrap_or(0)
                    ));
                }
            }
        }
    }

    // Sessions that sent session_attribution but no pageview in-window still
    // count (emit-before-pageview is the recommended order).
    for (sid, attr) in &by_session {
        if session_counted.contains_key(sid) {
            continue;
        }
        *counts.entry(attr.label()).or_default() += 1;
        if attr.is_aeo() {
            aeo_keys.insert(format!("s:{sid}"));
        }
    }

    // Explicit answer_engine_visit events — first-class AEO even without a
    // matching session_attribution medium.
    let aeo_event_rows = sqlx::query_as::<_, (Option<String>, Option<String>)>(&format!(
        "SELECT session_id, properties FROM analytics_events
         WHERE project_id = ?1 AND kind = 'event' AND name = '{ANSWER_ENGINE_VISIT_EVENT}'
           AND created_at >= datetime('now', ?2){bot_filter}"
    ))
    .bind(project_id)
    .bind(since)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    for (i, (session_id, properties)) in aeo_event_rows.iter().enumerate() {
        if let Some(sid) = session_id.as_deref().filter(|s| !s.is_empty()) {
            aeo_keys.insert(format!("s:{sid}"));
            by_session.entry(sid.to_string()).or_insert_with(|| {
                match properties
                    .as_deref()
                    .and_then(|p| serde_json::from_str::<serde_json::Value>(p).ok())
                {
                    Some(props) => {
                        let mut a = attribute_from_session_props(&props);
                        if !a.is_aeo() {
                            a.medium = "aeo".into();
                        }
                        a
                    }
                    None => TrafficAttribution::new("chatgpt", "aeo"),
                }
            });
        } else {
            aeo_keys.insert(format!("ae:{i}"));
        }
    }

    let mut top_sources: Vec<(String, i64)> = counts.into_iter().collect();
    top_sources.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    top_sources.truncate(10);

    TrafficSourceRollup {
        top_sources,
        aeo_visits: aeo_keys.len() as i64,
        by_session,
    }
}

/// Sessions whose first-touch matches an optional source and/or medium filter.
pub fn sessions_matching(
    by_session: &HashMap<String, TrafficAttribution>,
    source: Option<&str>,
    medium: Option<&str>,
) -> Option<std::collections::HashSet<String>> {
    if source.is_none() && medium.is_none() {
        return None;
    }
    let source = source.map(|s| s.to_ascii_lowercase());
    let medium = medium.map(|m| m.to_ascii_lowercase());
    let set = by_session
        .iter()
        .filter(|(_, attr)| {
            source
                .as_ref()
                .is_none_or(|s| attr.source.eq_ignore_ascii_case(s))
                && medium
                    .as_ref()
                    .is_none_or(|m| attr.medium.eq_ignore_ascii_case(m))
        })
        .map(|(sid, _)| sid.clone())
        .collect();
    Some(set)
}

#[cfg(test)]
mod tests {
    use super::*;
    use permagent::session::spectral_schema::apply_analytics_events_schema;

    async fn mem_pool() -> Pool<Sqlite> {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        apply_analytics_events_schema(&pool).await.unwrap();
        pool
    }

    async fn insert(
        pool: &Pool<Sqlite>,
        kind: &str,
        name: Option<&str>,
        referrer: Option<&str>,
        session_id: Option<&str>,
        properties: Option<&str>,
    ) {
        sqlx::query(
            "INSERT INTO analytics_events
                (project_id, kind, path, referrer, name, session_id, properties, is_bot, created_at)
             VALUES ('p1', ?1, '/', ?2, ?3, ?4, ?5, 0, strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        )
        .bind(kind)
        .bind(referrer)
        .bind(name)
        .bind(session_id)
        .bind(properties)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn referrer_fallback_fills_traffic_sources_without_utm() {
        let pool = mem_pool().await;
        insert(
            &pool,
            "pageview",
            None,
            Some("https://www.google.com/search?q=x"),
            Some("s1"),
            None,
        )
        .await;
        insert(
            &pool,
            "pageview",
            None,
            Some("https://chatgpt.com/"),
            Some("s2"),
            None,
        )
        .await;

        let rollup = rollup_traffic_sources(&pool, "p1", "-30 days", false).await;
        assert!(
            rollup
                .top_sources
                .iter()
                .any(|(n, c)| n == "google / organic" && *c == 1),
            "{:?}",
            rollup.top_sources
        );
        assert!(
            rollup
                .top_sources
                .iter()
                .any(|(n, c)| n == "chatgpt / aeo" && *c == 1),
            "{:?}",
            rollup.top_sources
        );
        assert_eq!(rollup.aeo_visits, 1);
    }

    #[tokio::test]
    async fn session_attribution_beats_later_referrer() {
        let pool = mem_pool().await;
        insert(
            &pool,
            "event",
            Some(SESSION_ATTRIBUTION_EVENT),
            None,
            Some("s1"),
            Some(r#"{"source":"reddit","medium":"social","referrer_raw":"https://reddit.com/"}"#),
        )
        .await;
        // SPA navigation re-reports an unrelated referrer — must not override.
        insert(
            &pool,
            "pageview",
            None,
            Some("https://www.google.com/"),
            Some("s1"),
            None,
        )
        .await;

        let rollup = rollup_traffic_sources(&pool, "p1", "-30 days", false).await;
        assert_eq!(
            rollup.by_session.get("s1").map(|a| a.label()),
            Some("reddit / social".into())
        );
        assert_eq!(rollup.top_sources[0].0, "reddit / social");
    }

    #[tokio::test]
    async fn answer_engine_visit_counts_as_aeo() {
        let pool = mem_pool().await;
        insert(
            &pool,
            "event",
            Some(ANSWER_ENGINE_VISIT_EVENT),
            None,
            Some("s9"),
            Some(r#"{"source":"chatgpt"}"#),
        )
        .await;
        insert(&pool, "pageview", None, None, Some("s9"), None).await;

        let rollup = rollup_traffic_sources(&pool, "p1", "-30 days", false).await;
        assert!(rollup.aeo_visits >= 1, "aeo={}", rollup.aeo_visits);
    }
}
