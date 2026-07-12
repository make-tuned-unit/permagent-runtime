//! Echo / the Watcher (#672) — proactive, gentle, rare nudges from the hub.
//!
//! A second brain's value is *rediscovery*, and a permanent agent's value is
//! *reaching out*. This background task periodically asks its signal sources for
//! the single most useful thing to surface, and — if the gentle budget allows —
//! emits ONE `proactive_nudge`. The frontend's notification stream turns that
//! into an in-app + (opt-in) OS notification, so it arrives even when the app is
//! backgrounded.
//!
//! Two signal sources, ranked timely-first:
//!   1. **project-news** — something is happening in the world around a subject
//!      you're actively working on (Google News over the entity's name). This is
//!      the timeliest, so it wins when there's a genuinely fresh story.
//!   2. **dormant-thread** — an entity you wove through many memories, then went
//!      quiet on while newer memories piled up elsewhere.
//!
//! Both are honest (real signals or nothing) and share one budget: at most ~once
//! a day, quiet hours, dedup, persisted across restarts. Analytics + phone push
//! (APNs) are the remaining #672 slices.

use crate::state::AppState;
use chrono::{DateTime, Local, Timelike, Utc};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const TICK: Duration = Duration::from_secs(3 * 3600);
const STARTUP_DELAY: Duration = Duration::from_secs(180);
const MIN_GAP_HOURS: i64 = 20;
const DAY_MS: i64 = 86_400_000;
/// A news item older than this isn't "happening" anymore.
const NEWS_FRESH_DAYS: i64 = 7;
/// A subject touched more recently than this counts as "active" (news-worthy).
const ACTIVE_WINDOW_DAYS: i64 = 30;

struct Nudge {
    kind: &'static str,
    subject: String,
    count: i64,
    last_ts: String,
    message: String,
}

/// Per-subject aggregate over the Brain's memories.
struct Subj {
    ids: HashSet<String>,
    last: i64,
    first: i64,
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Budget {
    /// RFC3339 — a plain string so we don't depend on chrono's serde feature.
    last_delivered: Option<String>,
    /// Dedup for the dormant source — never resurface the same thread twice.
    last_subject: Option<String>,
    /// Dedup for the news source — never nudge the same story twice.
    last_news_link: Option<String>,
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
            // No brain, nothing to work with.
            if state.brain.is_none() {
                continue;
            }

            // Timely-first: try news, then fall back to a dormant thread.
            let picked = match compute_news(budget.last_news_link.as_deref()).await {
                Some((nudge, link)) => Some((nudge, Some(link))),
                None => match compute_dormant().await {
                    // Don't resurface the same dormant thread twice running.
                    Some(n) if budget.last_subject.as_deref() != Some(n.subject.as_str()) => {
                        Some((n, None))
                    }
                    _ => None,
                },
            };
            let Some((nudge, news_link)) = picked else {
                continue;
            };

            permagent::events::emit(permagent::events::proactive_nudge(
                nudge.kind,
                &nudge.subject,
                &nudge.message,
                nudge.count,
                &nudge.last_ts,
            ));
            tracing::info!(
                target: "permagentd::echo",
                kind = nudge.kind,
                subject = %nudge.subject,
                "emitted proactive nudge"
            );
            // Opt-in phone push (no Apple cert needed) — arrives even with the
            // app closed. No-op unless the user has set a topic.
            push_to_phone(&nudge.message).await;
            budget.last_delivered = Some(now.to_rfc3339());
            budget.last_subject = Some(nudge.subject.clone());
            if let Some(link) = news_link {
                budget.last_news_link = Some(link);
            }
            budget.save();
        }
    });
}

/// Opt-in phone push via ntfy — real notifications on the phone (even with the
/// app closed) with NO Apple push cert. No-op unless `PERMAGENT_NTFY_TOPIC` is
/// set, so it's fully private by default; point `PERMAGENT_NTFY_SERVER` at a
/// self-hosted ntfy for zero third-party exposure. The hub just POSTs the nudge.
async fn push_to_phone(message: &str) {
    let Ok(topic) = std::env::var("PERMAGENT_NTFY_TOPIC") else {
        return;
    };
    let topic = topic.trim();
    if topic.is_empty() {
        return;
    }
    let server =
        std::env::var("PERMAGENT_NTFY_SERVER").unwrap_or_else(|_| "https://ntfy.sh".to_string());
    let url = format!("{}/{}", server.trim_end_matches('/'), topic);
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
    else {
        return;
    };
    // Fire-and-forget: a failed push never disturbs the loop.
    let _ = client
        .post(&url)
        .header("Title", "Henry noticed something")
        .header("Tags", "sparkles")
        .body(message.to_string())
        .send()
        .await;
}

// ── Subject aggregation (shared by both sources) ─────────────────────────────

/// Aggregate the Brain's memories by subject (annotation name). Returns the map
/// plus the newest memory timestamp (ms), or None if the Brain is unreachable.
fn aggregate_subjects() -> Option<(HashMap<String, Subj>, i64)> {
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

    let mut agg: HashMap<String, Subj> = HashMap::new();
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
                continue; // skip raw ids — a subject needs a readable name
            }
            let e = agg.entry(name.to_string()).or_insert_with(|| Subj {
                ids: HashSet::new(),
                last: i64::MIN,
                first: i64::MAX,
            });
            e.ids.insert(mem_id.clone());
            e.last = e.last.max(ts);
            e.first = e.first.min(ts);
        }
    }
    if newest == i64::MIN {
        return None;
    }
    Some((agg, newest))
}

// ── Source 1: project news ───────────────────────────────────────────────────

/// If something fresh is happening around the subject you're most actively
/// working on, return the nudge + the story link (for dedup). Network-tolerant:
/// any failure returns None and we fall back to a dormant thread.
async fn compute_news(last_link: Option<&str>) -> Option<(Nudge, String)> {
    let (agg, _newest) = tokio::task::spawn_blocking(aggregate_subjects)
        .await
        .ok()
        .flatten()?;

    // Pick the most-substantial *active* subject — what you're working on now.
    let now = Utc::now().timestamp_millis();
    let mut best: Option<(String, i64)> = None;
    for (name, s) in &agg {
        let count = s.ids.len() as i64;
        let recent_days = (now - s.last) / DAY_MS;
        if count < 3 || recent_days > ACTIVE_WINDOW_DAYS {
            continue;
        }
        if best.as_ref().map(|b| count > b.1).unwrap_or(true) {
            best = Some((name.clone(), count));
        }
    }
    let (name, count) = best?;

    // Google News RSS over the subject name. reqwest encodes the query.
    let client = reqwest::Client::builder()
        .user_agent("Permagent/1.0 (echo watcher)")
        .timeout(Duration::from_secs(12))
        .build()
        .ok()?;
    let resp = client
        .get("https://news.google.com/rss/search")
        .query(&[
            ("q", name.as_str()),
            ("hl", "en-US"),
            ("gl", "US"),
            ("ceid", "US:en"),
        ])
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body = resp.text().await.ok()?;
    let item = first_rss_item(&body)?;

    let pub_ms = parse_rfc2822_ms(&item.pub_date)?;
    if (now - pub_ms) / DAY_MS > NEWS_FRESH_DAYS {
        return None; // not "happening" anymore
    }
    if Some(item.link.as_str()) == last_link {
        return None; // already nudged this story
    }

    let message = format!("Something's happening around \"{}\": {}", name, item.title);
    Some((
        Nudge {
            kind: "project_news",
            subject: name,
            count,
            last_ts: item.pub_date,
            message,
        },
        item.link,
    ))
}

struct RssItem {
    title: String,
    link: String,
    pub_date: String,
}

/// Extract the first `<item>` (title, link, pubDate) from an RSS feed with plain
/// string ops — enough for Google News RSS, no XML crate needed.
fn first_rss_item(xml: &str) -> Option<RssItem> {
    let item = between(xml, "<item>", "</item>")?;
    let title = between(item, "<title>", "</title>")
        .map(clean_xml)
        .unwrap_or_default();
    let link = between(item, "<link>", "</link>")
        .map(clean_xml)
        .unwrap_or_default();
    let pub_date = between(item, "<pubDate>", "</pubDate>")
        .map(clean_xml)
        .unwrap_or_default();
    if title.is_empty() || pub_date.is_empty() {
        return None;
    }
    Some(RssItem {
        title,
        link,
        pub_date,
    })
}

fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let i = s.find(start)? + start.len();
    let rest = &s[i..];
    let j = rest.find(end)?;
    Some(&rest[..j])
}

fn clean_xml(s: &str) -> String {
    let t = s.trim();
    let t = t
        .strip_prefix("<![CDATA[")
        .and_then(|x| x.strip_suffix("]]>"))
        .unwrap_or(t);
    t.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .trim()
        .to_string()
}

fn parse_rfc2822_ms(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc2822(s.trim())
        .ok()
        .map(|d| d.timestamp_millis())
}

// ── Source 2: dormant thread ─────────────────────────────────────────────────

async fn compute_dormant() -> Option<Nudge> {
    tokio::task::spawn_blocking(compute_dormant_blocking)
        .await
        .ok()
        .flatten()
}

fn compute_dormant_blocking() -> Option<Nudge> {
    let (agg, newest) = aggregate_subjects()?;
    let mut best: Option<Nudge> = None;
    let mut best_score = 0.0_f64;
    for (name, s) in agg {
        let count = s.ids.len() as i64;
        if count < 3 {
            continue; // not substantial
        }
        let gap_days = (newest - s.last) as f64 / DAY_MS as f64;
        let span_days = (s.last - s.first) as f64 / DAY_MS as f64;
        if gap_days < 14.0 || span_days < 2.0 {
            continue; // still warm, or a one-off burst — not a dormant thread
        }
        let score = count as f64 * (1.0 + gap_days).ln();
        if score > best_score {
            best_score = score;
            let last_ts = DateTime::<Utc>::from_timestamp_millis(s.last)
                .map(|d| d.to_rfc3339())
                .unwrap_or_default();
            let message = format!(
                "You wove \"{}\" through {} memories, then it went quiet — last touched {}. \
                 Threads like this are where the good ideas hide.",
                name,
                count,
                rel_time(s.last)
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

// ── Small helpers ────────────────────────────────────────────────────────────

/// Parse the Brain's created_at (RFC3339 or `YYYY-MM-DD HH:MM:SS`) to epoch ms.
fn parse_ms(s: &str) -> Option<i64> {
    if let Ok(dt) = s.parse::<DateTime<Utc>>() {
        return Some(dt.timestamp_millis());
    }
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
        .ok()
        .map(|dt| dt.and_utc().timestamp_millis())
}

/// True for opaque ids (raw hex) we don't want to show as a subject.
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

    #[test]
    fn parses_first_rss_item() {
        let xml = r#"<rss><channel>
            <item><title><![CDATA[Acme raises $50M &amp; hires]]></title>
            <link>https://news.example/a</link>
            <pubDate>Tue, 07 Jul 2026 12:00:00 GMT</pubDate></item>
            <item><title>Second</title><link>https://news.example/b</link>
            <pubDate>Mon, 06 Jul 2026 12:00:00 GMT</pubDate></item>
            </channel></rss>"#;
        let item = first_rss_item(xml).expect("an item");
        assert_eq!(item.title, "Acme raises $50M & hires");
        assert_eq!(item.link, "https://news.example/a");
        assert!(parse_rfc2822_ms(&item.pub_date).is_some());
    }

    #[test]
    fn no_item_yields_none() {
        assert!(first_rss_item("<rss><channel></channel></rss>").is_none());
    }
}
