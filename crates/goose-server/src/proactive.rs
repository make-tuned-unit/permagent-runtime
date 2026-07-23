//! Echo / the Watcher (#672) — proactive, gentle, rare nudges from the hub.
//!
//! A second brain's value is *rediscovery*, and a permanent agent's value is
//! *reaching out*. This background task periodically gathers candidate signals,
//! lets the model exercise **taste** over them, and — if the gentle budget
//! allows and something is genuinely worth it — emits ONE `proactive_nudge`. The
//! frontend's notification stream turns that into an in-app + (opt-in) OS
//! notification and (opt-in) a phone push, so it arrives even when you're away.
//!
//! Signal sources:
//!   - **project-news** — something happening in the world around a subject
//!     you're actively working on (Google News over the entity's name).
//!   - **dormant-thread** — an entity you wove through many memories, then went
//!     quiet on while newer memories piled up elsewhere.
//!
//! The pick is not a heuristic: the candidates go to the model (the "Watcher"
//! reasoning), which judges relevance honestly — for news, only if the headline
//! is really about your work, not a name coincidence — chooses the single one
//! worth interrupting for (or NONE, staying silent), and writes it in the
//! agent's voice. If no model is configured, a deterministic pick + template is
//! the fallback. Either way it's gentle: at most ~once a day, quiet hours,
//! deduped, persisted across restarts.

use crate::state::AppState;
use chrono::{DateTime, Local, Timelike, Utc};
use permagent::conversation::message::Message;
use permagent::providers::base::Provider;
use permagent::rss;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const TICK: Duration = Duration::from_secs(3 * 3600);
const STARTUP_DELAY: Duration = Duration::from_secs(180);
const MIN_GAP_HOURS: i64 = 20;
const DAY_MS: i64 = 86_400_000;
const NEWS_FRESH_DAYS: i64 = 7;
const ACTIVE_WINDOW_DAYS: i64 = 30;

/// A candidate nudge — carries both a deterministic template `message` (the
/// fallback) and a one-line `detail` the model uses to judge and voice it.
struct Nudge {
    kind: &'static str,
    subject: String,
    count: i64,
    last_ts: String,
    message: String,
    detail: String,
    news_link: Option<String>,
}

struct Subj {
    ids: HashSet<String>,
    last: i64,
    first: i64,
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Budget {
    /// RFC3339 — a plain string so we don't depend on chrono's serde feature.
    last_delivered: Option<String>,
    last_subject: Option<String>,
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
            if state.brain.is_none() {
                continue;
            }

            // Gather candidates (dedup so we never re-nudge the same thing).
            let mut candidates: Vec<Nudge> = Vec::new();
            if let Some(n) = compute_news(budget.last_news_link.as_deref()).await {
                candidates.push(n);
            }
            if let Some(n) = compute_dormant().await {
                if budget.last_subject.as_deref() != Some(n.subject.as_str()) {
                    candidates.push(n);
                }
            }
            if candidates.is_empty() {
                continue;
            }

            // The agent's name — for the voice.
            let agent_name = {
                let p = state.persona.read().await;
                if p.first_name.is_empty() {
                    "Aria".to_string()
                } else {
                    p.first_name.clone()
                }
            };

            // Let the Watcher exercise taste: pick one + voice it, decline, or
            // (if no model) fall back to a deterministic pick + template.
            let (idx, message) = match reason(&agent_name, &candidates).await {
                Reasoned::Silence => continue, // the model judged nothing worth it
                Reasoned::Pick(i, msg) => (i, msg),
                Reasoned::Unavailable => {
                    // Timely-first: prefer a news item, else the first candidate.
                    let i = candidates
                        .iter()
                        .position(|n| n.kind == "project_news")
                        .unwrap_or(0);
                    (i, candidates[i].message.clone())
                }
            };
            let nudge = &candidates[idx];

            permagent::events::emit(permagent::events::proactive_nudge(
                nudge.kind,
                &nudge.subject,
                &message,
                nudge.count,
                &nudge.last_ts,
            ));
            tracing::info!(
                target: "permagentd::echo",
                kind = nudge.kind,
                subject = %nudge.subject,
                "emitted proactive nudge"
            );
            budget.last_delivered = Some(now.to_rfc3339());
            budget.last_subject = Some(nudge.subject.clone());
            if let Some(link) = nudge.news_link.clone() {
                budget.last_news_link = Some(link);
            }
            budget.save();
        }
    });
}

// ── The Watcher's reasoning: taste over the candidates ───────────────────────

enum Reasoned {
    /// The model picked candidate `usize` and voiced it.
    Pick(usize, String),
    /// The model judged nothing worth interrupting for — stay silent.
    Silence,
    /// No model configured / call failed — caller uses the deterministic pick.
    Unavailable,
}

async fn reason(agent_name: &str, candidates: &[Nudge]) -> Reasoned {
    let Some(provider) = resolve_provider().await else {
        return Reasoned::Unavailable;
    };
    let system = format!(
        "You are {name}, a permanent AI companion with a long memory. At most once a day you may \
         gently reach out about the ONE thing genuinely worth someone's attention — or stay silent \
         if nothing truly is. You have taste and you protect their focus: never interrupt for the \
         trivial or the irrelevant. For a news item, only pick it if the headline is really about \
         their work, not a name coincidence. Reply ONLY as compact JSON: \
         {{\"pick\": <0-based index into the list, or -1 for none>, \"message\": \"<one warm, \
         specific sentence in your own voice; empty if none>\"}}.",
        name = agent_name
    );
    let mut list = String::new();
    for (i, c) in candidates.iter().enumerate() {
        list.push_str(&format!(
            "{}. [{}] {} — {}\n",
            i, c.kind, c.subject, c.detail
        ));
    }
    let user = Message::user().with_text(format!(
        "Candidates you could mention today:\n{list}\nWhich single one is worth a gentle nudge \
         right now? Judge honestly; -1 if none clears the bar."
    ));
    let Ok((response, _usage)) = provider
        .complete_fast("echo-watcher", &system, std::slice::from_ref(&user), &[])
        .await
    else {
        return Reasoned::Unavailable;
    };
    parse_reason(&response.as_concat_text(), candidates.len())
}

fn parse_reason(text: &str, n: usize) -> Reasoned {
    let (Some(start), Some(end)) = (text.find('{'), text.rfind('}')) else {
        return Reasoned::Unavailable;
    };
    let Some(slice) = text.get(start..=end) else {
        return Reasoned::Unavailable;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(slice) else {
        return Reasoned::Unavailable;
    };
    let Some(pick) = v.get("pick").and_then(|p| p.as_i64()) else {
        return Reasoned::Unavailable;
    };
    if pick < 0 {
        return Reasoned::Silence;
    }
    let idx = pick as usize;
    if idx >= n {
        return Reasoned::Unavailable;
    }
    let msg = v
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .trim();
    if msg.is_empty() {
        return Reasoned::Unavailable; // fall back to the template
    }
    Reasoned::Pick(idx, msg.to_string())
}

/// The provider for the Watcher's judgment. Uses the configured default provider
/// (complete_fast routes to its cheap fast model). None → deterministic pick.
async fn resolve_provider() -> Option<Arc<dyn Provider>> {
    let config = permagent::config::Config::global();
    let provider_name = config.get_goose_provider().ok()?;
    let model_name = config.get_goose_model().ok()?;
    if provider_name.trim().is_empty() || model_name.trim().is_empty() {
        return None;
    }
    permagent::providers::create_with_named_model(&provider_name, &model_name, Vec::new())
        .await
        .ok()
}

// ── Subject aggregation (shared by both sources) ─────────────────────────────

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
                continue;
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

// ── Source: project news ─────────────────────────────────────────────────────

async fn compute_news(last_link: Option<&str>) -> Option<Nudge> {
    let (agg, _newest) = tokio::task::spawn_blocking(aggregate_subjects)
        .await
        .ok()
        .flatten()?;

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

    let client = reqwest::Client::builder()
        .user_agent("Permagent/1.0 (echo watcher)")
        .timeout(Duration::from_secs(12))
        .build()
        .ok()?;
    // The URL is built by hand in crate::rss — this reqwest is configured
    // without the `query` helper (default-features = false), same as browser.rs.
    let url = rss::google_news_search_url(&name);
    let resp = client.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body = resp.text().await.ok()?;
    let item = first_rss_item(&body)?;

    let pub_ms = rss::parse_rfc2822_ms(&item.pub_date)?;
    if (now - pub_ms) / DAY_MS > NEWS_FRESH_DAYS {
        return None;
    }
    if Some(item.link.as_str()) == last_link {
        return None;
    }

    Some(Nudge {
        kind: "project_news",
        message: format!("Something's happening around \"{}\": {}", name, item.title),
        detail: format!(
            "fresh headline \"{}\" about a subject they've touched in {} memories recently",
            item.title, count
        ),
        subject: name,
        count,
        last_ts: item.pub_date,
        news_link: Some(item.link),
    })
}

/// The freshest feed item, requiring a `pub_date` (the news freshness gate needs
/// one). Parsing itself lives in `permagent::rss`, shared with the Grow
/// audience-listening tool so the two feed readers can't drift.
fn first_rss_item(xml: &str) -> Option<rss::Item> {
    let item = rss::first_item(xml)?;
    (!item.pub_date.is_empty()).then_some(item)
}

// ── Source: dormant thread ───────────────────────────────────────────────────

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
            continue;
        }
        let gap_days = (newest - s.last) as f64 / DAY_MS as f64;
        let span_days = (s.last - s.first) as f64 / DAY_MS as f64;
        if gap_days < 14.0 || span_days < 2.0 {
            continue;
        }
        let score = count as f64 * (1.0 + gap_days).ln();
        if score > best_score {
            best_score = score;
            let last_ts = DateTime::<Utc>::from_timestamp_millis(s.last)
                .map(|d| d.to_rfc3339())
                .unwrap_or_default();
            best = Some(Nudge {
                kind: "dormant_thread",
                message: format!(
                    "You wove \"{}\" through {} memories, then it went quiet — last touched {}. \
                     Threads like this are where the good ideas hide.",
                    name,
                    count,
                    rel_time(s.last)
                ),
                detail: format!(
                    "{} memories, went deep then quiet — last touched {}",
                    count,
                    rel_time(s.last)
                ),
                subject: name,
                count,
                last_ts,
                news_link: None,
            });
        }
    }
    best
}

// ── Small helpers ────────────────────────────────────────────────────────────

fn parse_ms(s: &str) -> Option<i64> {
    if let Ok(dt) = s.parse::<DateTime<Utc>>() {
        return Some(dt.timestamp_millis());
    }
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
        .ok()
        .map(|dt| dt.and_utc().timestamp_millis())
}

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
            </channel></rss>"#;
        let item = first_rss_item(xml).expect("an item");
        assert_eq!(item.title, "Acme raises $50M & hires");
        assert_eq!(item.link, "https://news.example/a");
        assert!(rss::parse_rfc2822_ms(&item.pub_date).is_some());
    }

    #[test]
    fn reason_parse_handles_pick_silence_and_garbage() {
        match parse_reason("{\"pick\": 1, \"message\": \"hey\"}", 2) {
            Reasoned::Pick(1, m) => assert_eq!(m, "hey"),
            _ => panic!("expected Pick(1)"),
        }
        assert!(matches!(
            parse_reason("noise {\"pick\": -1, \"message\": \"\"} tail", 2),
            Reasoned::Silence
        ));
        assert!(matches!(parse_reason("not json", 2), Reasoned::Unavailable));
        // out-of-range index falls back
        assert!(matches!(
            parse_reason("{\"pick\": 9, \"message\": \"x\"}", 2),
            Reasoned::Unavailable
        ));
    }
}
