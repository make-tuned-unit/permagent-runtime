//! Echo / the Watcher (#672) — proactive, gentle, rare nudges from the hub.
//!
//! A second brain's value is *rediscovery*, and a permanent agent's value is
//! *reaching out*. This background task periodically asks a signal source for
//! the single most useful thing to surface, and — if the gentle budget allows —
//! emits ONE `proactive_nudge`. The frontend's notification stream turns that
//! into an in-app + (opt-in) OS notification, so it arrives even when the app is
//! backgrounded.
//!
//! Slice 1 ships the **dormant-thread** source: an entity you wove through many
//! memories, then went quiet on while newer memories piled up elsewhere. It is
//! honest — it only fires for a genuinely substantial, genuinely dormant thread,
//! with real numbers — and gentle: at most ~once a day, quiet hours, never the
//! same subject twice, budget persisted across restarts. Project-news and
//! analytics sources plug in behind the same emit path (see #672).

use crate::state::AppState;
use chrono::{DateTime, Local, Timelike, Utc};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// How often we wake to consider a nudge. The once-a-day budget below is what
/// actually paces delivery — this is just the polling grain.
const TICK: Duration = Duration::from_secs(3 * 3600);
/// Let the daemon settle before the first consideration (no nudge on boot).
const STARTUP_DELAY: Duration = Duration::from_secs(180);
/// Gentle & rare: at most one delivered nudge per ~day.
const MIN_GAP_HOURS: i64 = 20;
const DAY_MS: i64 = 86_400_000;

/// A composed nudge ready to emit.
struct Nudge {
    kind: &'static str,
    subject: String,
    count: i64,
    last_ts: String,
    message: String,
}

/// Delivery budget, persisted next to the rest of Permagent's state so "gentle
/// & rare" survives a daemon restart (otherwise a crash-loop could re-nudge).
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Budget {
    /// RFC3339 — a plain string so we don't depend on chrono's serde feature.
    last_delivered: Option<String>,
    last_subject: Option<String>,
}

impl Budget {
    fn path() -> Option<PathBuf> {
        std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join(".permagent").join("echo-state.json"))
    }
    fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
    fn save(&self) {
        if let (Some(p), Ok(s)) = (Self::path(), serde_json::to_string(self)) {
            let _ = std::fs::write(p, s);
        }
    }
}

/// Spawn the long-lived Watcher loop.
pub fn spawn(state: Arc<AppState>) {
    tokio::spawn(async move {
        tokio::time::sleep(STARTUP_DELAY).await;
        let mut budget = Budget::load();
        let mut ticker = tokio::time::interval(TICK);
        loop {
            ticker.tick().await;

            // Once-a-day budget.
            let now = Utc::now();
            let last_delivered = budget
                .last_delivered
                .as_deref()
                .and_then(|s| s.parse::<DateTime<Utc>>().ok());
            if let Some(last) = last_delivered {
                if (now - last).num_hours() < MIN_GAP_HOURS {
                    continue;
                }
            }
            // Quiet hours — no pings 22:00–08:00 local.
            let hour = Local::now().hour();
            if !(8u32..22).contains(&hour) {
                continue;
            }
            // No brain, nothing to resurface.
            if state.brain.is_none() {
                continue;
            }

            let Some(nudge) = compute_dormant().await else {
                continue;
            };
            // Never resurface the same subject twice running.
            if budget.last_subject.as_deref() == Some(nudge.subject.as_str()) {
                continue;
            }

            permagent::events::emit(permagent::events::proactive_nudge(
                nudge.kind,
                &nudge.subject,
                &nudge.message,
                nudge.count,
                &nudge.last_ts,
            ));
            tracing::info!(
                target: "permagentd::echo",
                subject = %nudge.subject,
                count = nudge.count,
                "emitted proactive nudge (dormant thread)"
            );
            budget.last_delivered = Some(now.to_rfc3339());
            budget.last_subject = Some(nudge.subject.clone());
            budget.save();
        }
    });
}

/// Find the single most substantial dormant thread, or None.
async fn compute_dormant() -> Option<Nudge> {
    tokio::task::spawn_blocking(compute_dormant_blocking)
        .await
        .ok()
        .flatten()
}

fn compute_dormant_blocking() -> Option<Nudge> {
    let conn = crate::brain_ops::read_only_brain_conn().ok()?;
    let mut stmt = conn
        .prepare(
            "SELECT m.id, m.created_at, a.who \
             FROM memories m JOIN memory_annotations a ON a.memory_id = m.id \
             WHERE m.created_at IS NOT NULL AND a.who IS NOT NULL",
        )
        .ok()?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .ok()?;

    // subject name -> (distinct memory ids, last_ms, first_ms)
    let mut agg: HashMap<String, (HashSet<String>, i64, i64)> = HashMap::new();
    let mut newest = i64::MIN;

    for (mem_id, created, who_json) in rows.flatten() {
        let Some(ts) = parse_ms(&created) else {
            continue;
        };
        newest = newest.max(ts);
        let Ok(refs) = serde_json::from_str::<Vec<serde_json::Value>>(&who_json) else {
            continue;
        };
        for r in refs {
            let Some(cid) = r.get("canonical_id").and_then(|v| v.as_str()) else {
                continue;
            };
            let name = cid
                .strip_prefix("term:")
                .or_else(|| cid.strip_prefix("cat:"))
                .unwrap_or(cid);
            if name.is_empty() || name.starts_with("e:") || is_hexish(name) {
                continue; // skip raw ids — a thread needs a readable name
            }
            let e = agg
                .entry(name.to_string())
                .or_insert_with(|| (HashSet::new(), i64::MIN, i64::MAX));
            e.0.insert(mem_id.clone());
            e.1 = e.1.max(ts);
            e.2 = e.2.min(ts);
        }
    }
    if newest == i64::MIN {
        return None;
    }

    let mut best: Option<Nudge> = None;
    let mut best_score = 0.0_f64;
    for (name, (ids, last, first)) in agg {
        let count = ids.len() as i64;
        if count < 3 {
            continue; // not substantial
        }
        let gap_days = (newest - last) as f64 / DAY_MS as f64;
        let span_days = (last - first) as f64 / DAY_MS as f64;
        if gap_days < 14.0 || span_days < 2.0 {
            continue; // still warm, or a one-off burst — not a dormant thread
        }
        let score = count as f64 * (1.0 + gap_days).ln();
        if score > best_score {
            best_score = score;
            let last_ts = DateTime::<Utc>::from_timestamp_millis(last)
                .map(|d| d.to_rfc3339())
                .unwrap_or_default();
            let message = format!(
                "You wove \"{}\" through {} memories, then it went quiet — last touched {}. \
                 Threads like this are where the good ideas hide.",
                name,
                count,
                rel_time(last)
            );
            best = Some(Nudge {
                kind: "dormant_thread",
                subject: name,
                count,
                last_ts,
                message,
            });
        }
    }
    best
}

/// Parse the Brain's created_at (RFC3339 or `YYYY-MM-DD HH:MM:SS`) to epoch ms.
fn parse_ms(s: &str) -> Option<i64> {
    if let Ok(dt) = s.parse::<DateTime<Utc>>() {
        return Some(dt.timestamp_millis());
    }
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
        .ok()
        .map(|dt| dt.and_utc().timestamp_millis())
}

/// True for opaque ids (raw hex) we don't want to show as a "thread".
fn is_hexish(s: &str) -> bool {
    s.len() >= 32 && s.chars().all(|c| c.is_ascii_hexdigit())
}

fn rel_time(ms: i64) -> String {
    let days = ((Utc::now().timestamp_millis() - ms) / DAY_MS).max(0);
    if days >= 365 {
        let y = days / 365;
        return if y > 1 {
            format!("{} years ago", y)
        } else {
            "a year ago".to_string()
        };
    }
    if days >= 45 {
        return format!("{} months ago", days / 30);
    }
    if days >= 25 {
        return "a month ago".to_string();
    }
    if days >= 12 {
        return format!("{} weeks ago", days / 7);
    }
    format!("{} days ago", days)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_both_timestamp_formats() {
        assert!(parse_ms("2026-01-02T03:04:05Z").is_some());
        assert!(parse_ms("2026-01-02 03:04:05").is_some());
        assert!(parse_ms("not a date").is_none());
    }

    #[test]
    fn hexish_ids_are_skipped_names_are_kept() {
        assert!(is_hexish(&"a".repeat(40)));
        assert!(!is_hexish("Spectral federation"));
        assert!(!is_hexish("sleep"));
    }

    #[test]
    fn rel_time_buckets() {
        let now = Utc::now().timestamp_millis();
        assert_eq!(rel_time(now - 3 * DAY_MS), "3 days ago");
        assert_eq!(rel_time(now - 21 * DAY_MS), "3 weeks ago");
        assert_eq!(rel_time(now - 400 * DAY_MS), "a year ago");
    }
}
