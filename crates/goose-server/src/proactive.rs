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
//! agent's voice. If no model is configured, only the deterministic
//! dormant-thread template may ship — news NEVER goes out unjudged. Either way
//! it's gentle: at most ~once a day, quiet hours, deduped, persisted across
//! restarts.

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
    /// `(project id, project name)` grounding this nudge, when it is about one
    /// of the user's active projects — judge context + client deep-link.
    project: Option<(String, String)>,
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
    /// Subjects the user muted ("stop nudging me about X") — honored alongside
    /// the `watcher_muted_subjects` config key. `default` so pre-existing
    /// echo-state.json files still deserialize.
    #[serde(default)]
    muted: Vec<String>,
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
            let days_since_last = last_delivered.map(|last| (now - last).num_days());
            let muted = budget.muted.clone();
            let mut candidates: Vec<Nudge> = Vec::new();
            if let Ok(pool) = state.session_manager().pool_clone().await {
                if let Some(n) = compute_news(
                    &pool,
                    budget.last_news_link.as_deref(),
                    budget.last_subject.as_deref(),
                    days_since_last,
                    &muted,
                )
                .await
                {
                    candidates.push(n);
                }
            }
            if let Some(n) = compute_dormant().await {
                if budget.last_subject.as_deref() != Some(n.subject.as_str())
                    && !is_muted(&n.subject, &muted)
                {
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
                    // No judge → no unjudged news, EVER. A dormant-thread
                    // recall is deterministic over the user's own memories and
                    // safe to template; a news headline is a relevance claim
                    // only the judge can make. Silence over noise.
                    match fallback_pick(&candidates) {
                        Some(i) => (i, candidates[i].message.clone()),
                        None => continue,
                    }
                }
            };
            let nudge = &candidates[idx];

            permagent::events::emit(permagent::events::proactive_nudge(
                nudge.kind,
                &nudge.subject,
                &message,
                nudge.count,
                &nudge.last_ts,
                // Deliver the link, don't just remember it. This was fetched
                // above, used for dedup below, and then dropped — so Henry
                // could say "there's a fresh piece worth reading" and give the
                // user no way to reach it.
                nudge.news_link.as_deref(),
                nudge
                    .project
                    .as_ref()
                    .map(|(id, name)| (id.as_str(), name.as_str())),
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

/// The model-unavailable fallback. Only judge-free kinds may ship: a
/// dormant-thread recall is a deterministic fact about the user's own
/// memories, while a news headline is a relevance CLAIM that needs the judge —
/// unjudged news never ships (returns `None` → silence).
fn fallback_pick(candidates: &[Nudge]) -> Option<usize> {
    candidates.iter().position(|n| n.kind == "dormant_thread")
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
        // The grounding project is part of the evidence the judge sees — "is
        // this really about their work" is unanswerable without the work.
        let project = c
            .project
            .as_ref()
            .map(|(_, name)| format!(" (their project: {name})"))
            .unwrap_or_default();
        list.push_str(&format!(
            "{}. [{}] {}{} — {}\n",
            i, c.kind, c.subject, project, c.detail
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

/// Config: subjects the user explicitly asked the Watcher to follow.
const WATCHER_TOPICS_KEY: &str = "watcher_topics";
/// Config: subjects the user muted ("stop nudging me about X").
const WATCHER_MUTED_KEY: &str = "watcher_muted_subjects";
/// How many feed items are considered per check. The lead item is often stale
/// or already delivered; refusing to look past it silenced whole subjects.
const NEWS_ITEM_CANDIDATES: usize = 5;
/// Days before the subject that headlined the last nudge may headline again.
const SAME_SUBJECT_SUPPRESS_DAYS: i64 = 7;

/// A chosen news subject: `(subject, memory count, grounding project)`.
type ChosenSubject = (String, i64, Option<(String, String)>);

async fn compute_news(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    last_link: Option<&str>,
    last_subject: Option<&str>,
    days_since_last: Option<i64>,
    budget_muted: &[String],
) -> Option<Nudge> {
    let agg = tokio::task::spawn_blocking(aggregate_subjects)
        .await
        .ok()
        .flatten()
        .map(|(agg, _newest)| agg)
        .unwrap_or_default();

    // The user's projects ground the news (audit 2026-08-11): a Brain subject
    // qualifies only when it intersects an active project's name, description,
    // or tags. Annotation counts alone kept surfacing entities from anywhere
    // in the user's life — which is how the Watcher earned "doesn't send me
    // relevant info".
    let projects = permagent::projects::list_projects(pool, Some("active"))
        .await
        .unwrap_or_default();

    let config = permagent::config::Config::global();
    let topics = config
        .get_param::<Vec<String>>(WATCHER_TOPICS_KEY)
        .unwrap_or_default();
    let mut muted = config
        .get_param::<Vec<String>>(WATCHER_MUTED_KEY)
        .unwrap_or_default();
    muted.extend(budget_muted.iter().cloned());

    let now = Utc::now().timestamp_millis();
    let suppressed = |subject: &str| {
        is_muted(subject, &muted) || same_subject_suppressed(subject, last_subject, days_since_last)
    };

    // An explicit topic outranks an inferred subject — the user declared it
    // relevant, which is grounding by fiat. Suppression rotates through
    // several configured topics rather than pinning the first forever.
    let chosen: Option<ChosenSubject> = topics
        .iter()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .find(|t| !suppressed(t))
        .map(|t| {
            let count = agg.get(t).map(|s| s.ids.len() as i64).unwrap_or(0);
            let project = subject_grounded_in(t, &projects).map(|p| (p.id.clone(), p.name.clone()));
            (t.to_string(), count, project)
        })
        .or_else(|| {
            let mut best: Option<ChosenSubject> = None;
            for (name, s) in &agg {
                let count = s.ids.len() as i64;
                let recent_days = (now - s.last) / DAY_MS;
                if count < 3 || recent_days > ACTIVE_WINDOW_DAYS || suppressed(name) {
                    continue;
                }
                let Some(project) = subject_grounded_in(name, &projects) else {
                    continue;
                };
                if best.as_ref().map(|b| count > b.1).unwrap_or(true) {
                    best = Some((
                        name.clone(),
                        count,
                        Some((project.id.clone(), project.name.clone())),
                    ));
                }
            }
            best
        });
    let (name, count, project) = chosen?;

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
    let item = pick_news_item(
        rss::parse_items(&body, NEWS_ITEM_CANDIDATES),
        now,
        last_link,
    )?;

    let detail = news_detail(
        &item,
        &name,
        count,
        project.as_ref().map(|(_, n)| n.as_str()),
    );
    Some(Nudge {
        kind: "project_news",
        message: format!("Something's happening around \"{}\": {}", name, item.title),
        detail,
        subject: name,
        count,
        last_ts: item.pub_date,
        news_link: Some(item.link),
        project,
    })
}

/// Ground a subject in the user's active projects: the subject must intersect
/// a project's name, description, or tags (case-insensitive; either direction
/// for name and tags). Pure, so relevance is testable without a Brain.
fn subject_grounded_in<'a>(
    subject: &str,
    projects: &'a [permagent::projects::Project],
) -> Option<&'a permagent::projects::Project> {
    let s = subject.trim().to_lowercase();
    if s.len() < 3 {
        return None; // a 1-2 char "subject" would intersect nearly everything
    }
    projects.iter().find(|p| {
        let name = p.name.to_lowercase();
        (!name.is_empty() && (name.contains(&s) || s.contains(&name)))
            || (!p.description.is_empty() && p.description.to_lowercase().contains(&s))
            || p.tags.iter().any(|t| {
                let t = t.trim().to_lowercase();
                !t.is_empty() && (t == s || t.contains(&s) || s.contains(&t))
            })
    })
}

/// Muted subjects never nudge (echo-state + `watcher_muted_subjects` both
/// feed this).
fn is_muted(subject: &str, muted: &[String]) -> bool {
    let s = subject.trim().to_lowercase();
    muted.iter().any(|m| {
        let m = m.trim().to_lowercase();
        !m.is_empty() && (s == m || s.contains(&m))
    })
}

/// Subject-level news dedupe: the subject that headlined the last nudge stays
/// quiet for a while (mirrors the dormant-thread same-subject check).
/// Link-only dedupe let the same subject re-nudge daily with a different
/// article forever. An unknown delivery age suppresses — conservative.
fn same_subject_suppressed(
    subject: &str,
    last_subject: Option<&str>,
    days_since_last: Option<i64>,
) -> bool {
    last_subject.is_some_and(|l| l.eq_ignore_ascii_case(subject.trim()))
        && days_since_last.is_none_or(|d| d < SAME_SUBJECT_SUPPRESS_DAYS)
}

/// The first item worth nudging about: parseable date (the freshness gate
/// needs one), fresh, and not the link already delivered. Pure over parsed
/// items. Parsing itself lives in `permagent::rss`, shared with the Grow
/// audience-listening tool so the two feed readers can't drift.
fn pick_news_item(
    items: Vec<rss::Item>,
    now_ms: i64,
    last_link: Option<&str>,
) -> Option<rss::Item> {
    items.into_iter().find(|item| {
        rss::parse_rfc2822_ms(&item.pub_date)
            .is_some_and(|pub_ms| (now_ms - pub_ms) / DAY_MS <= NEWS_FRESH_DAYS)
            && Some(item.link.as_str()) != last_link
    })
}

/// The judge's evidence line: headline, source domain, a slice of the item's
/// own description, and the grounding — enough to judge relevance honestly
/// instead of guessing from a bare title.
fn news_detail(item: &rss::Item, subject: &str, count: i64, project: Option<&str>) -> String {
    let mut detail = format!("fresh headline \"{}\"", item.title);
    if let Some(domain) = domain_of(&item.link) {
        detail.push_str(&format!(" ({domain})"));
    }
    let desc = truncate_chars(&strip_tags(&item.description), 200);
    if !desc.is_empty() {
        detail.push_str(&format!(" — {desc}"));
    }
    match project {
        Some(p) => detail.push_str(&format!(
            "; about \"{subject}\", which grounds in their project \"{p}\""
        )),
        None => detail.push_str(&format!(
            "; about \"{subject}\", a topic they asked the Watcher to follow"
        )),
    }
    if count > 0 {
        detail.push_str(&format!(" and appearing in {count} recent memories"));
    }
    detail
}

/// The host of an http(s) URL, for source attribution in the judge's evidence.
fn domain_of(url: &str) -> Option<String> {
    let rest = url.split("://").nth(1)?;
    let host = rest.split('/').next()?.split('?').next()?;
    (!host.is_empty()).then(|| host.to_string())
}

/// Drop `<tag>` spans — Google News descriptions are HTML after entity
/// unescape; the judge should see text, not markup.
fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.trim().to_string()
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
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
                project: None,
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
    fn picks_first_fresh_undelivered_item_not_just_the_lead() {
        let now = Utc::now().timestamp_millis();
        let fresh = |days_ago: i64| {
            DateTime::<Utc>::from_timestamp_millis(now - days_ago * DAY_MS)
                .unwrap()
                .to_rfc2822()
        };
        let item = |title: &str, link: &str, pub_date: String| rss::Item {
            title: title.into(),
            link: link.into(),
            description: String::new(),
            pub_date,
        };

        // Lead item is stale, second is the already-delivered link, third is
        // the one worth nudging about — the old first-item-only logic would
        // have gone silent here.
        let items = vec![
            item("stale", "https://news.example/stale", fresh(30)),
            item("seen", "https://news.example/seen", fresh(1)),
            item("good", "https://news.example/good", fresh(2)),
        ];
        let picked = pick_news_item(items, now, Some("https://news.example/seen")).expect("a pick");
        assert_eq!(picked.title, "good");

        // No parseable pub_date → not eligible (the freshness gate needs one).
        let dateless = vec![item("undated", "https://news.example/u", String::new())];
        assert!(pick_news_item(dateless, now, None).is_none());
    }

    #[test]
    fn news_subjects_must_ground_in_an_active_project() {
        fn project(name: &str, description: &str, tags: &[&str]) -> permagent::projects::Project {
            permagent::projects::Project {
                id: format!("id-{name}"),
                user_id: "default".into(),
                slug: name.to_lowercase(),
                name: name.into(),
                description: description.into(),
                status: "active".into(),
                root_path: None,
                site_url: None,
                repo_url: None,
                notes: String::new(),
                metadata_json: serde_json::Value::Null,
                graph_entity_id: None,
                tags: tags.iter().map(|t| t.to_string()).collect(),
                created_at: String::new(),
                updated_at: String::new(),
                last_opened_at: String::new(),
            }
        }
        let projects = vec![
            project(
                "Grocery Savers",
                "coupon site for weekly deals",
                &["retail"],
            ),
            project(
                "Polybot",
                "prediction-market trading agent",
                &["polymarket", "trading"],
            ),
        ];

        // Name intersection, either direction, case-insensitive.
        assert_eq!(
            subject_grounded_in("grocery savers launch", &projects).map(|p| p.name.as_str()),
            Some("Grocery Savers")
        );
        // Tag and description intersection.
        assert_eq!(
            subject_grounded_in("Polymarket", &projects).map(|p| p.name.as_str()),
            Some("Polybot")
        );
        assert_eq!(
            subject_grounded_in("prediction-market", &projects).map(|p| p.name.as_str()),
            Some("Polybot")
        );
        // A subject from elsewhere in the user's life does NOT qualify — this
        // is the project-blindness fix.
        assert!(subject_grounded_in("sleep", &projects).is_none());
        assert!(subject_grounded_in("Spectral federation", &projects).is_none());
        // Too-short subjects would intersect nearly everything.
        assert!(subject_grounded_in("ai", &projects).is_none());
    }

    #[test]
    fn unjudged_news_never_ships() {
        let nudge = |kind: &'static str| Nudge {
            kind,
            subject: "s".into(),
            count: 1,
            last_ts: String::new(),
            message: "m".into(),
            detail: String::new(),
            news_link: None,
            project: None,
        };
        // Only news pending + no judge → silence, never a templated headline.
        assert_eq!(fallback_pick(&[nudge("project_news")]), None);
        // A dormant thread is deterministic and may ship without the judge.
        assert_eq!(
            fallback_pick(&[nudge("project_news"), nudge("dormant_thread")]),
            Some(1)
        );
        assert_eq!(fallback_pick(&[]), None);
    }

    #[test]
    fn subject_dedupe_and_muting() {
        // Same subject within the window → suppressed (case-insensitive);
        // unknown delivery age suppresses conservatively.
        assert!(same_subject_suppressed("Polybot", Some("polybot"), Some(2)));
        assert!(same_subject_suppressed("Polybot", Some("Polybot"), None));
        // Window elapsed or a different subject → eligible again.
        assert!(!same_subject_suppressed(
            "Polybot",
            Some("Polybot"),
            Some(SAME_SUBJECT_SUPPRESS_DAYS)
        ));
        assert!(!same_subject_suppressed("Polybot", Some("other"), Some(1)));
        assert!(!same_subject_suppressed("Polybot", None, None));

        // Muting: exact and containing matches, case-insensitive; empty
        // entries never mute everything.
        let muted = vec!["polybot".to_string(), " ".to_string()];
        assert!(is_muted("Polybot", &muted));
        assert!(is_muted("Polybot revival", &muted));
        assert!(!is_muted("Grocery Savers", &muted));
        assert!(!is_muted("anything", &[String::new()]));
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
