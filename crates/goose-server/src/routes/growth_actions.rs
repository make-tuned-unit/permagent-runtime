//! Growth actions — what to actually DO about the analytics.
//!
//! The Analytics lens answers "what happened". This answers "so what". It reads
//! the project's real first-party analytics and asks a model for ranked,
//! specific moves across conversion, retention, churn and UX.
//!
//! Two rules govern the whole module, both learned from the analytics work:
//!
//! * **Grounded or silent.** Every action must cite a figure that is actually
//!   in the data. With no traffic there is nothing to advise on, and generic
//!   growth advice dressed as analysis is worse than an empty panel — it reads
//!   as insight while being unfalsifiable. No provider, no data, or a refusal
//!   all produce an empty list with an honest reason, never filler.
//! * **The numbers are already qualified.** `deviceSignatures` undercounts and
//!   bots are excluded; the prompt says so, so the model cannot present a
//!   device count as a headcount or explain away a gap the filter created.
//!
//! Results are cached in the project metadata bag so opening the tab does not
//! spend a model call every time, and so the actions are stable enough to act
//! on rather than reshuffling on every render.

use crate::routes::analytics_attribution as attribution;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::post,
    Json, Router,
};
use permagent::conversation::message::Message;
use permagent::projects::{self, Project, UpdateProject};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};
use std::sync::Arc;

/// Metadata bag key holding the cached actions.
const METADATA_KEY: &str = "growth_actions";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GrowthAction {
    /// Short imperative headline.
    pub title: String,
    /// What in the data prompted this. MUST reference a real figure.
    pub evidence: String,
    /// The concrete change to make.
    pub recommendation: String,
    /// The ordered steps to carry it out. An action nobody knows how to start
    /// is a observation wearing an action's clothes.
    #[serde(default)]
    pub steps: Vec<String>,
    /// What `artifact` is, so the UI can label and offer it correctly:
    /// prompt (paste into a coding harness) | post (social copy) | none.
    #[serde(default)]
    pub artifact_kind: String,
    /// Ready-to-use text — a coding-harness prompt or drafted post. This is
    /// what turns "improve your SEO" into something that gets done.
    #[serde(default)]
    pub artifact: Option<String>,
    /// conversion | retention | churn | ux | acquisition | measurement |
    /// content | seo | aeo
    pub category: String,
    /// high | medium | low
    pub impact: String,
    /// high | medium | low — how confident the evidence makes this.
    pub confidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GrowthActionsData {
    pub actions: Vec<GrowthAction>,
    pub generated_at: Option<String>,
    /// Why the list is empty, when it is. Shown verbatim — an empty panel with
    /// no explanation is indistinguishable from a broken one.
    pub reason: Option<String>,
    /// The window the actions were derived from.
    pub period_days: Option<u32>,
}

/// The analytics shape the generator reasons over. Deliberately a plain struct
/// rather than the HTTP response type, so the summary the model sees is
/// explicit and testable.
#[derive(Debug, Clone, Default)]
pub struct AnalyticsSummary {
    pub pageviews: i64,
    pub device_signatures: i64,
    pub sessions: i64,
    pub bounce_rate: Option<f64>,
    pub pages_per_session: Option<f64>,
    pub bots_excluded: i64,
    pub top_pages: Vec<(String, i64)>,
    pub top_sources: Vec<(String, i64)>,
    pub top_referrers: Vec<(String, i64)>,
    pub top_campaigns: Vec<(String, i64)>,
    pub top_entry_pages: Vec<(String, i64)>,
    pub top_events: Vec<(String, i64)>,
    /// Answer-engine first-touch / answer_engine_visit count.
    pub aeo_visits: i64,
    pub days_with_traffic: usize,
    pub period_days: u32,
}

/// Below this there is not enough signal to say anything grounded.
pub const MIN_PAGEVIEWS: i64 = 20;

/// Is there enough here to advise on? Returns the reason when there is not.
pub fn readiness(summary: &AnalyticsSummary) -> Result<(), String> {
    if summary.pageviews == 0 {
        return Err(
            "No analytics data yet. Once the relay is installed and traffic arrives, actions \
             appear here."
                .to_string(),
        );
    }
    if summary.pageviews < MIN_PAGEVIEWS {
        return Err(format!(
            "Only {} pageviews so far — too little to draw conclusions from. Actions appear once \
             there are at least {MIN_PAGEVIEWS}.",
            summary.pageviews
        ));
    }
    Ok(())
}

/// Render the summary as the compact factual brief the model reasons over.
///
/// Every qualification the data carries is stated here, so the model cannot
/// silently upgrade a device count into a headcount or read a bot-filtered
/// figure as a traffic drop.
pub fn render_summary(project_name: &str, s: &AnalyticsSummary) -> String {
    let list = |label: &str, rows: &[(String, i64)]| -> String {
        if rows.is_empty() {
            return format!("{label}: none recorded\n");
        }
        let body = rows
            .iter()
            .take(8)
            .map(|(name, count)| format!("  {name} — {count}"))
            .collect::<Vec<_>>()
            .join("\n");
        format!("{label}:\n{body}\n")
    };
    let pct = |v: Option<f64>| match v {
        Some(v) => format!("{:.0}%", v * 100.0),
        None => "not measurable (no sessions recorded)".to_string(),
    };
    let num = |v: Option<f64>| match v {
        Some(v) => format!("{v:.1}"),
        None => "not measurable".to_string(),
    };

    let mut out = format!(
        "Project: {project_name}\nWindow: last {} days ({} days had traffic)\n\n\
         Pageviews: {}\n\
         Device signatures: {} (NOT a headcount — the hash merges people sharing a browser \
         build, OS and language, so this UNDERCOUNTS, badly on mobile)\n\
         Sessions: {}\n\
         Bounce rate: {}\n\
         Pages per session: {}\n\
         Bot hits excluded from all of the above: {}\n\n",
        s.period_days,
        s.days_with_traffic,
        s.pageviews,
        s.device_signatures,
        s.sessions,
        pct(s.bounce_rate),
        num(s.pages_per_session),
        s.bots_excluded,
    );
    out.push_str(&list("Top pages", &s.top_pages));
    out.push_str(&list("Entry pages", &s.top_entry_pages));
    out.push_str(&list("Traffic sources", &s.top_sources));
    out.push_str(&list("Referrers", &s.top_referrers));
    out.push_str(&list("Campaigns", &s.top_campaigns));
    out.push_str(&list("Product events", &s.top_events));
    if s.top_events.is_empty() {
        out.push_str(
            "\nNOTE: no product events are instrumented, so conversion and retention cannot be \
             measured at all — only traffic shape is visible.\n",
        );
    }
    if s.sessions == 0 {
        out.push_str(
            "\nNOTE: no session ids recorded, so bounce rate, pages per session and entry pages \
             are unavailable. The site's relay predates session support.\n",
        );
    }

    // Content and answer-engine signals, called out explicitly. Buried in a
    // referrer list a single chatgpt.com hit reads as noise; named, it is the
    // strongest AEO signal a small site gets — proof the content is being cited
    // by an answer engine and worth making more quotable.
    let answer_engines: Vec<&str> = ANSWER_ENGINES
        .iter()
        .copied()
        .filter(|host| {
            s.top_referrers
                .iter()
                .any(|(r, _)| r.to_ascii_lowercase().contains(host))
                || s.top_sources.iter().any(|(n, _)| {
                    let n = n.to_ascii_lowercase();
                    n.contains(host) || n.contains(" / aeo")
                })
        })
        .collect();
    if s.aeo_visits > 0 || !answer_engines.is_empty() {
        let engines = if answer_engines.is_empty() {
            "aeo".to_string()
        } else {
            answer_engines.join(", ")
        };
        out.push_str(&format!(
            "\nAEO SIGNAL: {} answer-engine visit(s); sources include ({}). The content is being \
             cited in generated answers — worth making more quotable and structured.\n",
            s.aeo_visits, engines,
        ));
    }

    let content_pages: Vec<&(String, i64)> = s
        .top_pages
        .iter()
        .filter(|(p, _)| {
            let p = p.to_ascii_lowercase();
            p.contains("/blog")
                || p.contains("/guide")
                || p.contains("/article")
                || p.contains("/post")
        })
        .collect();
    if !content_pages.is_empty() {
        let total_content: i64 = content_pages.iter().map(|(_, c)| *c).sum();
        out.push_str(&format!(
            "\nCONTENT: {} of {} pageviews land on written content ({}). Expanding, \
             interlinking and structuring the strongest of these is available as an action.\n",
            total_content,
            s.pageviews,
            content_pages
                .iter()
                .map(|(p, c)| format!("{p} — {c}"))
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    out
}

/// Referrer hosts that indicate a generated answer cited this site. Distinct
/// from ordinary search: it means the content was quoted, not merely ranked.
const ANSWER_ENGINES: &[&str] = &[
    "chatgpt.com",
    "chat.openai.com",
    "perplexity.ai",
    "claude.ai",
    "copilot.microsoft.com",
    "gemini.google.com",
    "you.com",
    "phind.com",
];

const SYSTEM: &str = "You are a growth analyst reviewing one product's own first-party web \
    analytics. Propose concrete moves that would strengthen the product: increase conversion, \
    reduce churn, improve retention, improve UX, or grow qualified traffic through content, SEO \
    and AEO (answer-engine optimisation — being cited by ChatGPT, Perplexity, Claude, Copilot).\n\n\
    HARD RULES:\n\
    - Ground EVERY action in a figure from the data. Quote the number in `evidence`.\n\
    - Never invent metrics that are not present. If something cannot be measured, an action \
      that says so — e.g. instrumenting a missing event — is more valuable than a guess.\n\
    - Do not treat device signatures as a headcount; they undercount.\n\
    - Do not explain a number by traffic the bot filter already removed.\n\
    - Prefer few strong actions over many weak ones. Between two and five.\n\
    - If the data genuinely does not support any specific action, return an empty list.\n\n\
    EVERY ACTION MUST BE DOABLE. Give ordered `steps`, and wherever the work is writing code or \
    writing copy, include a ready-to-use `artifact`:\n\
    - artifactKind \"prompt\": a complete, self-contained instruction to paste into a coding \
      agent working in that repo. Name the concrete change — the route, the meta tags, the \
      schema.org type, the heading structure. Assume the agent cannot see this dashboard, so \
      restate what it needs.\n\
    - artifactKind \"post\": the actual drafted post text, ready to publish, in the product's \
      voice. Not a description of a post.\n\
    - artifactKind \"none\": only when the work is a human decision, not a deliverable.\n\n\
    Content, SEO and AEO specifics worth acting on when the data shows them: a blog post pulling \
    disproportionate traffic is worth expanding and interlinking; search referrals concentrated \
    on one engine suggest the others are unindexed; a referral from an ANSWER ENGINE means the \
    content is being cited and is worth making more quotable (clear headings, direct answers, \
    FAQ and structured data); an entry page with high bounce needs its first screen rewritten.\n\n\
    Reply ONLY with JSON:\n\
    {\"actions\":[{\"title\":\"...\",\"evidence\":\"...\",\"recommendation\":\"...\",\
    \"steps\":[\"...\"],\"artifactKind\":\"prompt|post|none\",\"artifact\":\"...\",\
    \"category\":\"conversion|retention|churn|ux|acquisition|measurement|content|seo|aeo\",\
    \"impact\":\"high|medium|low\",\"confidence\":\"high|medium|low\"}]}";

/// Parse and sanitize the model's reply.
///
/// Drops any action missing evidence or a recommendation: an ungrounded action
/// is exactly the failure mode this module exists to avoid, and a plausible
/// one is harder to spot than an absent one.
pub fn parse_actions(text: &str) -> Vec<GrowthAction> {
    let Some(start) = text.find('{') else {
        return Vec::new();
    };
    let Some(end) = text.rfind('}') else {
        return Vec::new();
    };
    let Some(slice) = text.get(start..=end) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(slice) else {
        return Vec::new();
    };
    let Some(items) = v.get("actions").and_then(|a| a.as_array()) else {
        return Vec::new();
    };

    let norm = |value: Option<&serde_json::Value>, allowed: &[&str], fallback: &str| -> String {
        let raw = value
            .and_then(|v| v.as_str())
            .unwrap_or(fallback)
            .trim()
            .to_ascii_lowercase();
        if allowed.contains(&raw.as_str()) {
            raw
        } else {
            fallback.to_string()
        }
    };

    items
        .iter()
        .filter_map(|item| {
            let get = |k: &str| {
                item.get(k)
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            };
            let title = get("title")?;
            let evidence = get("evidence")?;
            let recommendation = get("recommendation")?;
            let steps: Vec<String> = item
                .get("steps")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str())
                        .map(str::trim)
                        .filter(|x| !x.is_empty())
                        .map(|x| x.chars().take(300).collect::<String>())
                        .take(8)
                        .collect()
                })
                .unwrap_or_default();
            let artifact = get("artifact").map(|a| a.chars().take(4000).collect::<String>());
            // An artifact kind with nothing attached would render an empty
            // "copy" affordance, so the two are reconciled here rather than in
            // the UI.
            let artifact_kind = if artifact.is_some() {
                norm(item.get("artifactKind"), &["prompt", "post"], "prompt")
            } else {
                "none".to_string()
            };
            Some(GrowthAction {
                title: title.chars().take(120).collect(),
                evidence: evidence.chars().take(400).collect(),
                recommendation: recommendation.chars().take(600).collect(),
                steps,
                artifact_kind,
                artifact,
                category: norm(
                    item.get("category"),
                    &[
                        "conversion",
                        "retention",
                        "churn",
                        "ux",
                        "acquisition",
                        "measurement",
                        "content",
                        "seo",
                        "aeo",
                    ],
                    "ux",
                ),
                impact: norm(item.get("impact"), &["high", "medium", "low"], "medium"),
                confidence: norm(item.get("confidence"), &["high", "medium", "low"], "medium"),
            })
        })
        .take(5)
        .collect()
}

/// Rank so the most actionable surface first: impact, then confidence.
pub fn rank(mut actions: Vec<GrowthAction>) -> Vec<GrowthAction> {
    let weight = |s: &str| match s {
        "high" => 0,
        "medium" => 1,
        _ => 2,
    };
    actions.sort_by_key(|a| (weight(&a.impact), weight(&a.confidence)));
    actions
}

async fn generate(
    project_name: &str,
    summary: &AnalyticsSummary,
) -> Result<Vec<GrowthAction>, String> {
    let config = permagent::config::Config::global();
    let provider_name = config
        .get_goose_provider()
        .map_err(|_| "No model provider configured — connect one in Settings.".to_string())?;
    let model_name = config
        .get_goose_model()
        .map_err(|_| "No model configured — choose one in Settings.".to_string())?;
    if provider_name.trim().is_empty() || model_name.trim().is_empty() {
        return Err("No model provider configured — connect one in Settings.".to_string());
    }
    let provider =
        permagent::providers::create_with_named_model(&provider_name, &model_name, Vec::new())
            .await
            .map_err(|e| format!("Could not reach the model provider: {e}"))?;

    let user = Message::user().with_text(render_summary(project_name, summary));
    let (response, _usage) = provider
        .complete_fast("growth-actions", SYSTEM, std::slice::from_ref(&user), &[])
        .await
        .map_err(|e| format!("The model call failed: {e}"))?;
    Ok(rank(parse_actions(&response.as_concat_text())))
}

// ── HTTP ─────────────────────────────────────────────────────────────────────

async fn load_summary(pool: &Pool<Sqlite>, project_id: &str, period_days: u32) -> AnalyticsSummary {
    let since = format!("-{period_days} days");
    let rows = |sql: String| {
        let pool = pool.clone();
        let id = project_id.to_string();
        let since = since.clone();
        async move {
            sqlx::query_as::<_, (String, i64)>(&sql)
                .bind(id)
                .bind(since)
                .fetch_all(&pool)
                .await
                .unwrap_or_default()
        }
    };

    let (pageviews, device_signatures): (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(DISTINCT visitor_hash) FROM analytics_events
         WHERE project_id = ?1 AND kind = 'pageview' AND is_bot = 0
           AND created_at >= datetime('now', ?2)",
    )
    .bind(project_id)
    .bind(&since)
    .fetch_one(pool)
    .await
    .unwrap_or((0, 0));

    let (sessions, session_views, bounced): (i64, i64, i64) = sqlx::query_as(
        "SELECT count(*), coalesce(sum(views), 0), coalesce(sum(views = 1), 0) FROM (
           SELECT session_id, count(*) AS views FROM analytics_events
           WHERE project_id = ?1 AND kind = 'pageview' AND is_bot = 0
             AND session_id IS NOT NULL AND created_at >= datetime('now', ?2)
           GROUP BY session_id)",
    )
    .bind(project_id)
    .bind(&since)
    .fetch_one(pool)
    .await
    .unwrap_or((0, 0, 0));

    let bots_excluded: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM analytics_events
         WHERE project_id = ?1 AND is_bot = 1 AND created_at >= datetime('now', ?2)",
    )
    .bind(project_id)
    .bind(&since)
    .fetch_one(pool)
    .await
    .unwrap_or(0);

    let days_with_traffic: i64 = sqlx::query_scalar(
        "SELECT count(DISTINCT date(created_at)) FROM analytics_events
         WHERE project_id = ?1 AND is_bot = 0 AND created_at >= datetime('now', ?2)",
    )
    .bind(project_id)
    .bind(&since)
    .fetch_one(pool)
    .await
    .unwrap_or(0);

    let base = "FROM analytics_events WHERE project_id = ?1 AND is_bot = 0                 AND created_at >= datetime('now', ?2)";
    let traffic = attribution::rollup_traffic_sources(pool, project_id, &since, false).await;
    AnalyticsSummary {
        pageviews,
        device_signatures,
        sessions,
        bounce_rate: (sessions > 0).then(|| bounced as f64 / sessions as f64),
        pages_per_session: (sessions > 0).then(|| session_views as f64 / sessions as f64),
        bots_excluded,
        top_pages: rows(format!(
            "SELECT path, count(*) {base} AND kind = 'pageview' GROUP BY path ORDER BY count(*) DESC LIMIT 8"
        ))
        .await,
        top_sources: traffic.top_sources,
        top_referrers: rows(format!(
            "SELECT referrer, count(*) {base} AND kind = 'pageview' AND referrer IS NOT NULL              AND referrer <> '' GROUP BY referrer ORDER BY count(*) DESC LIMIT 8"
        ))
        .await,
        top_campaigns: rows(format!(
            "SELECT coalesce(utm_campaign, utm_source), count(*) {base}              AND (utm_campaign IS NOT NULL OR utm_source IS NOT NULL)              GROUP BY coalesce(utm_campaign, utm_source) ORDER BY count(*) DESC LIMIT 8"
        ))
        .await,
        top_entry_pages: rows(format!(
            "SELECT path, count(*) FROM (SELECT path, session_id,              row_number() OVER (PARTITION BY session_id ORDER BY id) AS rn              {base} AND kind = 'pageview' AND session_id IS NOT NULL) WHERE rn = 1              GROUP BY path ORDER BY count(*) DESC LIMIT 8"
        ))
        .await,
        top_events: rows(format!(
            "SELECT coalesce(name, '(unnamed)'), count(*) {base} AND kind = 'event'              GROUP BY name ORDER BY count(*) DESC LIMIT 8"
        ))
        .await,
        aeo_visits: traffic.aeo_visits,
        days_with_traffic: days_with_traffic as usize,
        period_days,
    }
}

fn cached(project: &Project) -> Option<GrowthActionsData> {
    project
        .metadata_json
        .get(METADATA_KEY)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
}

async fn store(
    pool: &Pool<Sqlite>,
    project: &Project,
    data: &GrowthActionsData,
) -> Result<(), String> {
    let mut metadata = if project.metadata_json.is_object() {
        project.metadata_json.clone()
    } else {
        serde_json::json!({})
    };
    // Never index-assign into a serde_json::Map — it panics when the key is
    // absent, and panic=abort takes the daemon with it.
    if let Some(obj) = metadata.as_object_mut() {
        obj.insert(
            METADATA_KEY.to_string(),
            serde_json::to_value(data).map_err(|e| e.to_string())?,
        );
    }
    projects::update_project(
        pool,
        &project.id,
        UpdateProject {
            metadata_json: Some(metadata),
            ..Default::default()
        },
    )
    .await
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// GET returns the cached list; POST regenerates.
async fn get_actions(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
) -> Result<Json<GrowthActionsData>, StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let project = projects::get_project_by_id_or_slug(&pool, &project_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(cached(&project).unwrap_or_default()))
}

async fn regenerate(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
) -> Result<Json<GrowthActionsData>, StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let project = projects::get_project_by_id_or_slug(&pool, &project_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let period_days = 30;
    let summary = load_summary(&pool, &project.id, period_days).await;
    let now = chrono::Utc::now().to_rfc3339();

    let data = match readiness(&summary) {
        Err(reason) => GrowthActionsData {
            actions: Vec::new(),
            generated_at: Some(now),
            reason: Some(reason),
            period_days: Some(period_days),
        },
        Ok(()) => match generate(&project.name, &summary).await {
            Ok(actions) if actions.is_empty() => GrowthActionsData {
                actions,
                generated_at: Some(now),
                reason: Some(
                    "The data does not support a specific action right now — nothing here is \
                     strong enough to act on."
                        .to_string(),
                ),
                period_days: Some(period_days),
            },
            Ok(actions) => GrowthActionsData {
                actions,
                generated_at: Some(now),
                reason: None,
                period_days: Some(period_days),
            },
            Err(reason) => GrowthActionsData {
                actions: Vec::new(),
                generated_at: Some(now),
                reason: Some(reason),
                period_days: Some(period_days),
            },
        },
    };

    if let Err(e) = store(&pool, &project, &data).await {
        tracing::warn!(target: "permagentd::growth", "could not cache growth actions: {e}");
    }
    Ok(Json(data))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/api/projects/{project_id}/growth-actions",
            axum::routing::get(get_actions),
        )
        .route(
            "/api/projects/{project_id}/growth-actions/generate",
            post(regenerate),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn busy() -> AnalyticsSummary {
        AnalyticsSummary {
            pageviews: 1200,
            device_signatures: 300,
            sessions: 400,
            bounce_rate: Some(0.72),
            pages_per_session: Some(1.6),
            bots_excluded: 850,
            top_pages: vec![("/deals".into(), 700), ("/".into(), 300)],
            top_entry_pages: vec![("/deals".into(), 260)],
            top_events: vec![("list_item_added".into(), 40)],
            days_with_traffic: 28,
            period_days: 30,
            ..Default::default()
        }
    }

    #[test]
    fn no_data_is_refused_with_a_reason_not_advice() {
        let err = readiness(&AnalyticsSummary::default()).unwrap_err();
        assert!(err.contains("No analytics data yet"), "{err}");
    }

    #[test]
    fn too_little_data_is_refused() {
        let s = AnalyticsSummary {
            pageviews: 5,
            ..Default::default()
        };
        let err = readiness(&s).unwrap_err();
        assert!(err.contains("too little"), "{err}");
        assert!(err.contains('5'), "{err}");
    }

    #[test]
    fn enough_data_is_accepted() {
        assert!(readiness(&busy()).is_ok());
    }

    /// The summary must carry the caveats, or the model will present a device
    /// count as a headcount and read the bot filter as a traffic drop.
    #[test]
    fn the_summary_states_what_the_numbers_are_not() {
        let text = render_summary("GrocerySaver", &busy());
        assert!(text.contains("UNDERCOUNTS"), "{text}");
        assert!(text.contains("NOT a headcount"), "{text}");
        assert!(text.contains("Bot hits excluded"), "{text}");
    }

    #[test]
    fn the_summary_names_what_cannot_be_measured() {
        let mut s = busy();
        s.top_events.clear();
        s.sessions = 0;
        s.bounce_rate = None;
        s.pages_per_session = None;
        let text = render_summary("X", &s);
        assert!(
            text.contains("no product events are instrumented"),
            "{text}"
        );
        assert!(text.contains("no session ids recorded"), "{text}");
        assert!(text.contains("not measurable"), "{text}");
    }

    /// A single answer-engine referral is the strongest AEO signal a small site
    /// gets, and it is invisible buried in a referrer list. Taken from real
    /// data: one chatgpt.com hit among 19 referrals.
    #[test]
    fn names_answer_engine_referrals_explicitly() {
        let mut s = busy();
        s.top_referrers = vec![
            ("https://google.com/".into(), 12),
            ("https://chatgpt.com/".into(), 1),
        ];
        s.aeo_visits = 1;
        s.top_sources = vec![("chatgpt / aeo".into(), 1)];
        let text = render_summary("GrocerySaver", &s);
        assert!(text.contains("AEO SIGNAL"), "{text}");
        assert!(text.contains("chatgpt.com"), "{text}");
    }

    #[test]
    fn ordinary_search_referrals_are_not_an_aeo_signal() {
        let mut s = busy();
        s.top_referrers = vec![("https://google.com/".into(), 12)];
        assert!(!render_summary("X", &s).contains("AEO SIGNAL"));
    }

    #[test]
    fn calls_out_content_pages_so_they_can_be_expanded() {
        let mut s = busy();
        s.top_pages = vec![("/blog/canadian-grocery-stores".into(), 8), ("/".into(), 6)];
        let text = render_summary("X", &s);
        assert!(text.contains("CONTENT:"), "{text}");
        assert!(text.contains("/blog/canadian-grocery-stores"), "{text}");
    }

    #[test]
    fn captures_steps_and_a_usable_artifact() {
        let reply = r#"{"actions":[{"title":"Expand the grocery-stores post",
            "evidence":"8 of 37 pageviews","recommendation":"Expand and structure it",
            "steps":["Add an FAQ section","Add schema.org FAQPage"],
            "artifactKind":"prompt","artifact":"In this repo, open the blog post at …",
            "category":"aeo","impact":"high","confidence":"medium"}]}"#;
        let a = &parse_actions(reply)[0];
        assert_eq!(a.steps.len(), 2);
        assert_eq!(a.category, "aeo");
        assert_eq!(a.artifact_kind, "prompt");
        assert!(a.artifact.as_deref().unwrap().starts_with("In this repo"));
    }

    /// An artifactKind with nothing attached would render an empty copy button.
    #[test]
    fn an_artifact_kind_without_an_artifact_becomes_none() {
        let reply = r#"{"actions":[{"title":"t","evidence":"e","recommendation":"r",
            "artifactKind":"post"}]}"#;
        let a = &parse_actions(reply)[0];
        assert_eq!(a.artifact_kind, "none");
        assert!(a.artifact.is_none());
    }

    #[test]
    fn an_action_with_no_steps_still_parses() {
        let reply = r#"{"actions":[{"title":"t","evidence":"e","recommendation":"r"}]}"#;
        let a = &parse_actions(reply)[0];
        assert!(a.steps.is_empty());
        assert_eq!(a.artifact_kind, "none");
    }

    #[test]
    fn parses_a_well_formed_reply() {
        let reply = r#"Sure! {"actions":[{"title":"Cut the deals bounce",
            "evidence":"72% bounce over 400 sessions","recommendation":"Show three deals above the fold",
            "category":"conversion","impact":"high","confidence":"medium"}]}"#;
        let actions = parse_actions(reply);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].category, "conversion");
        assert_eq!(actions[0].impact, "high");
    }

    /// An ungrounded action is the exact failure this module exists to avoid,
    /// and a plausible one is harder to notice than an absent one.
    #[test]
    fn drops_actions_with_no_evidence() {
        let reply = r#"{"actions":[
            {"title":"Improve onboarding","recommendation":"Make it better","category":"ux"},
            {"title":"Real one","evidence":"1200 pageviews","recommendation":"Do X","category":"ux"}
        ]}"#;
        let actions = parse_actions(reply);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].title, "Real one");
    }

    #[test]
    fn normalizes_unknown_enum_values() {
        let reply = r#"{"actions":[{"title":"t","evidence":"e","recommendation":"r",
            "category":"vibes","impact":"enormous","confidence":"???"}]}"#;
        let a = &parse_actions(reply)[0];
        assert_eq!(a.category, "ux");
        assert_eq!(a.impact, "medium");
        assert_eq!(a.confidence, "medium");
    }

    #[test]
    fn junk_and_empty_replies_yield_nothing() {
        for reply in [
            "",
            "I cannot help with that",
            "{}",
            "{\"actions\":[]}",
            "not json",
        ] {
            assert!(parse_actions(reply).is_empty(), "{reply}");
        }
    }

    #[test]
    fn caps_the_list_so_the_panel_stays_actionable() {
        let items: Vec<String> = (0..12)
            .map(|i| format!(r#"{{"title":"t{i}","evidence":"e","recommendation":"r"}}"#))
            .collect();
        let reply = format!(r#"{{"actions":[{}]}}"#, items.join(","));
        assert_eq!(parse_actions(&reply).len(), 5);
    }

    #[test]
    fn ranks_high_impact_first_then_confidence() {
        let a = |impact: &str, confidence: &str, title: &str| GrowthAction {
            title: title.into(),
            evidence: "e".into(),
            recommendation: "r".into(),
            steps: Vec::new(),
            artifact_kind: "none".into(),
            artifact: None,
            category: "ux".into(),
            impact: impact.into(),
            confidence: confidence.into(),
        };
        let ranked = rank(vec![
            a("low", "high", "third"),
            a("high", "low", "second"),
            a("high", "high", "first"),
        ]);
        assert_eq!(
            ranked.iter().map(|x| x.title.as_str()).collect::<Vec<_>>(),
            vec!["first", "second", "third"]
        );
    }
}
