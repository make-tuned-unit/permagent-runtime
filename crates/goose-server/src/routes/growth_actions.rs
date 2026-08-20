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
use crate::routes::analytics_verify::Check;
use crate::routes::growth_verify;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use permagent::conversation::message::Message;
use permagent::growth::metrics::{self as growth_metrics, TargetDir, TargetMetric};
use permagent::growth::pooled;
use permagent::growth::power::Confounder;
use permagent::growth::store::{self as growth_store, ActionSeed};
use permagent::growth::sweep::{self, Baseline};
use permagent::projects::{self, Project, UpdateProject};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

/// A refusal the user can read.
///
/// `Result<_, (StatusCode, String)>` renders as `text/plain`, and the UI's
/// `apiFetch` parses every error body as JSON and falls back to the literal
/// string "Unknown error" when that fails. So every deliberate, carefully
/// worded refusal this module makes — "nothing has happened to this action
/// yet", "its baseline is frozen, so the claim cannot be changed now" — reached
/// the user as "Could not run the check: Unknown error". The reason existed and
/// was thrown away one layer above the person it was written for.
///
/// The field name is `message` because that is what `apiFetch` reads.
pub struct ApiError(pub StatusCode, pub String);

impl From<(StatusCode, String)> for ApiError {
    fn from((status, message): (StatusCode, String)) -> Self {
        ApiError(status, message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "message": self.1 }))).into_response()
    }
}

/// Metadata bag key holding the cached actions.
const METADATA_KEY: &str = "growth_actions";

/// `project_id` → when its review started (RFC3339), for the reviews running
/// right now.
///
/// In memory on purpose. A review is a single model call inside one daemon
/// process; persisting "in flight" would survive a crash or a restart as a
/// phantom no task will ever clear, and a Grow tab that says "still reviewing"
/// forever is worse than one that says nothing. A restart correctly reports
/// nothing in flight, because nothing is.
static RUNNING_REVIEWS: std::sync::OnceLock<std::sync::Mutex<HashMap<String, String>>> =
    std::sync::OnceLock::new();

/// The registry, recovering from a poisoned lock rather than panicking.
///
/// A panic inside a review would otherwise poison this for the life of the
/// process and take every later `unwrap()` — including the ones in GET — down
/// with it. The map holds ids and timestamps; there is no invariant a panicking
/// writer could have left half-applied.
fn running_reviews() -> std::sync::MutexGuard<'static, HashMap<String, String>> {
    RUNNING_REVIEWS
        .get_or_init(|| std::sync::Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// When the review running for this project started, or `None`.
fn review_started_at(project_id: &str) -> Option<String> {
    running_reviews().get(project_id).cloned()
}

/// Claim the review slot for a project.
///
/// `None` means one is already running and the caller must NOT start another.
/// This is the second click guard, and it lives here rather than in the UI
/// because a disabled button is a courtesy while this is the rule: two reviews
/// racing on one project write two caches over each other and can mint a
/// duplicate row for the same advice.
fn begin_review(project_id: &str, started_at: String) -> Option<ReviewSlot> {
    let mut running = running_reviews();
    if running.contains_key(project_id) {
        return None;
    }
    running.insert(project_id.to_string(), started_at);
    Some(ReviewSlot(project_id.to_string()))
}

/// Holds a project's review slot for as long as the review runs.
///
/// A guard rather than a `remove` at the end of the task, so a panic or an
/// early return inside the review releases the slot too. Without that, one
/// failed review would leave the button spinning for the rest of the daemon's
/// life with nothing able to clear it.
struct ReviewSlot(String);

impl Drop for ReviewSlot {
    fn drop(&mut self) {
        running_reviews().remove(&self.0);
    }
}

/// One judged window, as the card renders it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OutcomeView {
    pub window_days: i64,
    /// helped | hindered | no_effect | inconclusive | confounded.
    pub verdict: String,
    /// One sentence carrying the numbers the verdict rests on. Always present —
    /// the column is NOT NULL by design.
    pub rationale: String,
    pub delta_pct: Option<f64>,
    pub confounders: Vec<Confounder>,
    pub judged_at: String,
}

/// The durable half of an action: what the metadata cache cannot hold because
/// `regenerate` overwrites it wholesale (see `store` below).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ActionIdentity {
    pub id: String,
    pub status: String,
    pub target_metric: Option<String>,
    pub target_dir: Option<String>,
    /// git | content | event | self. Shown on the card: "verified from a commit"
    /// and "you told me so" are different claims and must not look identical.
    pub verified_by: Option<String>,
    pub verified_at: Option<String>,
    pub outcomes: Vec<OutcomeView>,
    /// The reading frozen at verification, decoded from `baseline_json`.
    ///
    /// The Tracking view exists to show what we changed and whether it worked,
    /// and a verdict whose "before" is invisible cannot be argued with — the
    /// same reason `rationale` is body text rather than a tooltip. Absent for
    /// an action that was never verified, and for one whose stored baseline no
    /// longer parses; in both cases the card renders no baseline rather than a
    /// zero, which would read as "no traffic before the change".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<BaselineView>,
}

/// One window of the frozen baseline: what the pre-registered metric read over
/// the `window_days` days ENDING at the change, which is what the matching
/// after-window is compared against.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BaselineWindow {
    pub window_days: u32,
    /// Inclusive UTC date, `YYYY-MM-DD`.
    pub start: String,
    /// Exclusive UTC date, `YYYY-MM-DD`.
    pub end: String,
    pub value: f64,
    /// What the value rests on — the count itself, or the session denominator
    /// for a rate. Carried because "70% bounce over 8 sessions" and "over 800"
    /// are different claims and the card must not present them identically.
    pub denominator: f64,
}

/// The frozen baseline as the Tracking card renders it.
///
/// A projection of `growth::sweep::Baseline` rather than that type itself: the
/// stored blob also carries twelve weeks of variance history and the earliest
/// event date, which the power check needs and a card has no use for. Sending
/// the whole blob would put a dozen numbers on the wire that nothing renders.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BaselineView {
    /// pageviews | sessions | aeo_visits | bounce_rate.
    pub metric: String,
    /// up | down.
    pub dir: String,
    /// First fully-post-change UTC day, `YYYY-MM-DD`. Every comparison window
    /// is measured from here (`metrics::pivot_date`).
    pub pivot: String,
    pub taken_at: String,
    /// One entry per measurement window, shortest first.
    pub windows: Vec<BaselineWindow>,
}

/// Decode `growth_actions.baseline_json` into what the card needs.
///
/// A parse failure is `None`, never a default: a baseline of zero would render
/// as "there was no traffic before the change", which is a claim this system
/// has no basis for making.
fn baseline_view(raw: Option<&str>) -> Option<BaselineView> {
    let frozen: Baseline = serde_json::from_str(raw?).ok()?;
    Some(BaselineView {
        metric: frozen.metric.as_str().to_string(),
        dir: frozen.dir.as_str().to_string(),
        pivot: frozen.pivot,
        taken_at: frozen.taken_at,
        windows: frozen
            .before
            .into_iter()
            .map(|(window_days, w)| BaselineWindow {
                window_days,
                start: w.start,
                end: w.end,
                value: w.value,
                denominator: w.denominator,
            })
            .collect(),
    })
}

/// One measured result from another project, named so the claim can be audited.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TransferExample {
    pub project_name: String,
    pub title: String,
    pub verdict: String,
    pub delta_pct: Option<f64>,
}

/// What this action's CATEGORY has done on the user's other active projects.
///
/// Computed server-side from `growth_action_outcomes` (see
/// `permagent::growth::pooled`) and never authored by the model. That
/// distinction is the whole feature: a model asserting "this worked on three
/// similar projects" is the self-assessed prose the proposal rules out, while
/// the same sentence derived from measured outcomes is evidence the user can
/// check — which is why every note carries its provenance.
///
/// The aggregate and the segment are separate fields on purpose. Merging them
/// would produce exactly the Simpson's paradox the proposal warns about: an
/// overall "helped" that quietly fails on projects shaped like this one.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TransferNote {
    pub category: String,
    /// Distinct OTHER active projects that measured this category.
    pub projects: usize,
    pub helped: usize,
    pub hindered: usize,
    pub no_effect: usize,
    pub median_delta_pct: Option<f64>,
    /// THIS project's segment, e.g. "content site, 300+ views/wk, mostly search".
    pub segment_label: String,
    pub segment_projects: usize,
    pub segment_helped: usize,
    pub segment_hindered: usize,
    pub segment_no_effect: usize,
    /// At most three, projects like this one first.
    pub examples: Vec<TransferExample>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    /// What this action should move: pageviews | sessions | aeo_visits |
    /// bounce_rate. The agent's PREDICTION, made when it recommends the action.
    ///
    /// This exists because the verify form used to ask the USER "what should
    /// move, and which way?" — which inverted the loop. The agent recommends
    /// the strategy, so the agent owns the claim about what it will do;
    /// otherwise there is no prediction to grade the agent against, and the
    /// user is answering the question they came here to be advised on.
    ///
    /// The set is closed (see `growth::metrics::TargetMetric`) because a target
    /// nothing can measure produces a verdict nothing can check. An unparseable
    /// value is dropped rather than stored, and the action falls back to asking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_metric: Option<String>,
    /// up | down — which way the agent expects `target_metric` to move.
    /// "Bounce rate goes up" and "bounce rate goes down" are opposite claims,
    /// and without this one of them would score as a success either way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_dir: Option<String>,
    /// What this category has done on the user's other active projects.
    /// Computed from measured outcomes at render time; absent when no other
    /// project has ever measured this category, because a badge that says
    /// nothing is worse than no badge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transfer: Option<TransferNote>,
    /// Filled in from `growth_actions` on read, never from the model. Absent
    /// only when the row could not be reached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<ActionIdentity>,
}

/// The whole panel, assembled from the durable `growth_actions` rows.
///
/// Both GET and POST /generate return this, built by the same [`render_board`]
/// — the metadata bag only ever contributes prose. Before that, the rendered
/// list WAS the bag, which `store` overwrites wholesale, so an action the last
/// review did not re-emit vanished from the panel while the sweep was still
/// measuring it.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GrowthActionsData {
    /// The active board, ordered: work still asking the user for a decision.
    /// Excludes archived, dismissed and everything in `tracking`.
    pub actions: Vec<GrowthAction>,
    /// What we changed and are now measuring — `verified`, `measuring` and
    /// `judged`, newest first.
    ///
    /// Its own list because Actions and Tracking answer different questions:
    /// "what should I do" and "did what I did work". #1053 kept these rows on
    /// the active board so in-flight work could not silently vanish while the
    /// sweep still measured it; that guarantee is what this list keeps, by
    /// MOVING them somewhere they are still visible rather than hiding them.
    #[serde(default)]
    pub tracking: Vec<GrowthAction>,
    /// Filed away by the user, newest first. Still measured while they owe a
    /// window, and still feeding learning.
    #[serde(default)]
    pub archived: Vec<GrowthAction>,
    /// Advice the user turned down, newest first.
    ///
    /// Separate from `archived` because the two mean opposite things to the
    /// generator: a dismissed action stays ON the board, so its text can never
    /// be re-proposed, while an archived one has been released. Kept out of
    /// `actions` because a card the user has already refused is not work in
    /// flight, and before this it sat in the active list forever — `suggested`
    /// had no exit at all, so the panel could only grow.
    #[serde(default)]
    pub dismissed: Vec<GrowthAction>,
    pub generated_at: Option<String>,
    /// Why the list is empty, when it is. Shown verbatim — an empty panel with
    /// no explanation is indistinguishable from a broken one.
    pub reason: Option<String>,
    /// The window the actions were derived from.
    pub period_days: Option<u32>,
    /// How many suggestions the last review discarded for naming no measurable
    /// prediction. Reported rather than defaulted: inventing a target would
    /// grade a claim the agent never made.
    #[serde(default)]
    pub dropped_for_no_target: usize,
    /// How many suggestions the last review discarded as restatements of
    /// something already on the board. Surfaced because the guard can withhold
    /// advice, and a silent drop is not auditable.
    #[serde(default)]
    pub dropped_as_restatement: usize,
    /// How many suggested actions the Steward dismissed because the change is
    /// already in this project's repo. Surfaced for the same reason as the
    /// restatement count: a silent dismiss looks like the review did nothing.
    #[serde(default)]
    pub dropped_as_already_present: usize,
    /// A review is running for this project RIGHT NOW.
    ///
    /// Server truth, and the whole point of it: the button's spinner used to be
    /// a `useState` inside the panel component, so switching tab unmounted it
    /// and the flag was lost — the user came back to an idle button while the
    /// review was still running and its result landed in the database unseen.
    /// Every read of this surface reports what is actually in flight, so the
    /// UI reconciles on remount instead of trusting its own memory.
    #[serde(default)]
    pub generating: bool,
    /// When the running review started (RFC3339). Absent when none is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_started_at: Option<String>,
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
    let answer_engines: Vec<&str> = growth_metrics::ANSWER_ENGINE_HOSTS
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
        .filter(|(p, _)| growth_metrics::is_content_path(p))
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
    - artifactKind \"post\": still a coding-agent instruction. Include the drafted copy AND name \
      the file, collection or route to write it to (e.g. content/blog/....md). A bare blog post \
      with no path and no instruction is not an artifact — the UI will wrap it, but you must \
      still say where it belongs.\n\
    - artifactKind \"none\": only when the work is a human decision, not a deliverable.\n\n\
    Content, SEO and AEO specifics worth acting on when the data shows them: a blog post pulling \
    disproportionate traffic is worth expanding and interlinking; search referrals concentrated \
    on one engine suggest the others are unindexed; a referral from an ANSWER ENGINE means the \
    content is being cited and is worth making more quotable (clear headings, direct answers, \
    FAQ and structured data); an entry page with high bounce needs its first screen rewritten.\n\n\
    State what each action should MOVE, and which way, as `targetMetric` and `targetDir`. This \
    is your prediction and it is how your advice gets graded: the change is measured against \
    that metric over 7, 14 and 28 days, and the verdict feeds back into how future strategies \
    are ranked. Pick the one metric the action most directly targets, from exactly \
    pageviews | sessions | aeo_visits | bounce_rate — nothing else can be measured. Mind the \
    direction: for bounce_rate an improvement is DOWN. EVERY action must carry both fields. An \
    action you cannot tie to one of those four metrics is not an action this system can grade, \
    so propose a different one instead — anything that arrives without both is discarded and \
    never reaches the user.\n\n\
    You will be shown the actions already on this project's board. They are live work, not \
    history. Do NOT restate one — not reworded, not narrowed, not broadened. If the only strong \
    moves left are already on the board, return fewer actions, or an empty list. Two genuinely \
    new actions beat five with three restatements: a restatement cannot be measured separately \
    from the action it copies, and it costs the user the attention they would have spent on the \
    original.\n\n\
    You will also be shown this project's git repo as it is right now — recent commits, and any \
    suggested action the Steward dismissed because the change is already in the tree. Do NOT \
    propose a change that is already shipped. Rewording \"add FAQPage schema\" when FAQPage is \
    already in the files is a restatement of work that is done, not a new action. If the only \
    strong moves left are already in the repo or on the board, return fewer actions, or an empty \
    list.\n\n\
    Reply ONLY with JSON:\n\
    {\"actions\":[{\"title\":\"...\",\"evidence\":\"...\",\"recommendation\":\"...\",\
    \"steps\":[\"...\"],\"artifactKind\":\"prompt|post|none\",\"artifact\":\"...\",\
    \"category\":\"conversion|retention|churn|ux|acquisition|measurement|content|seo|aeo\",\
    \"impact\":\"high|medium|low\",\"confidence\":\"high|medium|low\",\
    \"targetMetric\":\"pageviews|sessions|aeo_visits|bounce_rate\",\
    \"targetDir\":\"up|down\"}]}";

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
                // Unlike the fields above, these do NOT get a default. `norm`
                // substitutes a fallback when the model omits or garbles a
                // value, which is right for impact ("medium" is a fair guess)
                // and wrong here: defaulting the prediction would invent a
                // claim the agent never made and then grade it against that.
                // Absent stays absent, and the card asks instead.
                target_metric: item
                    .get("targetMetric")
                    .and_then(|v| v.as_str())
                    .and_then(|m| TargetMetric::parse(m).ok())
                    .map(|m| m.as_str().to_string()),
                target_dir: item
                    .get("targetDir")
                    .and_then(|v| v.as_str())
                    .and_then(|d| TargetDir::parse(d).ok())
                    .map(|d| d.as_str().to_string()),
                // Both of these are derived server-side after parsing, never
                // read from the reply: a model-authored transfer claim would be
                // an assertion about other projects it has never seen.
                transfer: None,
                identity: None,
            })
        })
        .take(5)
        .collect()
}

/// Split a parsed batch into the actions that made a gradeable prediction and
/// the ones that did not, with the reason each was refused.
///
/// `parse_actions` deliberately does not default `target_metric`/`target_dir`,
/// because defaulting would invent a claim the agent never made and then grade
/// it. That leaves the question of what to do with an untargeted action, and
/// the answer is: drop it, count it, and say so. Keeping it would put a card on
/// the board that can never be verified (the verify route refuses without a
/// target) and would fall back to asking the USER what the agent's own
/// prediction was — the inversion this whole loop exists to undo.
pub fn split_targeted(
    actions: Vec<GrowthAction>,
) -> (Vec<GrowthAction>, Vec<(GrowthAction, String)>) {
    let mut kept = Vec::new();
    let mut dropped = Vec::new();
    for action in actions {
        let metric = action
            .target_metric
            .as_deref()
            .and_then(|m| TargetMetric::parse(m).ok());
        let dir = action
            .target_dir
            .as_deref()
            .and_then(|d| TargetDir::parse(d).ok());
        match (metric, dir) {
            (Some(_), Some(_)) => kept.push(action),
            // Half a prediction is not a prediction: "sessions" with no
            // direction scores as a success whichever way sessions move.
            (Some(metric), None) => {
                let reason = format!("named {} but no direction", metric.as_str());
                dropped.push((action, reason));
            }
            (None, Some(_)) => {
                dropped.push((action, "named a direction but no metric".to_string()))
            }
            (None, None) => dropped.push((action, "named no target metric".to_string())),
        }
    }
    (kept, dropped)
}

/// The one corrective the generator gets before an untargeted action is thrown
/// away for good.
///
/// It names each offender and what was wrong with it rather than repeating the
/// rule in the abstract, because the model already had the rule in SYSTEM and
/// broke it; the new information is which of its own actions failed.
pub fn retry_correction(dropped: &[(GrowthAction, String)]) -> String {
    let mut out = String::from(
        "Your previous reply contained actions this system cannot grade, so they were \
         discarded:\n",
    );
    for (action, reason) in dropped {
        out.push_str(&format!("- \"{}\" — {reason}.\n", action.title));
    }
    out.push_str(
        "Reply again. Every action must carry `targetMetric`, exactly one of pageviews, \
         sessions, aeo_visits or bounce_rate, AND `targetDir`, either up or down. For \
         bounce_rate an improvement is down. If an action cannot be tied to one of those four \
         metrics, replace it with one that can rather than sending it back untargeted.\n",
    );
    out
}

/// Keep the good actions from both attempts, deduplicated and capped.
///
/// Discarding the first attempt wholesale because one sibling was malformed
/// would throw away advice the user could have used, and the retry is asked for
/// the same analysis, so its overlap with the first is expected rather than a
/// sign of anything.
pub fn merge_attempts(
    first: Vec<GrowthAction>,
    second: Vec<GrowthAction>,
    project_id: &str,
) -> Vec<GrowthAction> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for action in first.into_iter().chain(second) {
        if out.len() >= 5 {
            break;
        }
        let fp = growth_store::fingerprint(project_id, &action.title, &action.recommendation);
        if seen.insert(fp) {
            out.push(action);
        }
    }
    out
}

/// Rank so the most actionable surface first: impact, then confidence, then
/// what actually worked on this project.
///
/// `history` is a per-category net score: `+1` per `helped`, `-1` per
/// `hindered`, from confirmed outcomes only. It enters as the LAST sort key, so
/// it can reorder two equally-rated actions and nothing else.
///
/// That placement is the proposal's guard, not a detail: "Never suppress a
/// category outright on one bad outcome; downweight it in `rank()` … rather
/// than hiding advice the user can judge." A category with one bad result must
/// still be able to outrank a low-impact suggestion.
pub fn rank_with_history(
    mut actions: Vec<GrowthAction>,
    history: &HashMap<String, i32>,
) -> Vec<GrowthAction> {
    let weight = |s: &str| match s {
        "high" => 0,
        "medium" => 1,
        _ => 2,
    };
    actions.sort_by_key(|a| {
        (
            weight(&a.impact),
            weight(&a.confidence),
            // Clamped so a category with nine wins cannot bury everything else:
            // the sample sizes here are single digits.
            -history.get(&a.category).copied().unwrap_or(0).clamp(-1, 1),
        )
    });
    actions
}

/// Net per-category score from this project's confirmed outcomes.
async fn category_history(pool: &Pool<Sqlite>, project_id: &str) -> HashMap<String, i32> {
    let mut out = HashMap::new();
    let rows = growth_store::learnable_outcomes(pool, project_id, 50)
        .await
        .unwrap_or_default();
    for (action, outcome) in rows {
        let Some(category) = action.category else {
            continue;
        };
        let delta = match outcome.verdict.as_str() {
            "helped" => 1,
            "hindered" => -1,
            _ => 0,
        };
        *out.entry(category).or_insert(0) += delta;
    }
    out
}

/// What `persist` did: the rows it wrote, and how many suggestions it refused
/// as restatements of something already on the board.
pub struct PersistOutcome {
    pub rows: HashMap<String, growth_store::GrowthActionRow>,
    pub restated: usize,
}

/// Persist the generated list so each action has an identity, dropping anything
/// that merely restates what is already on the board.
///
/// Failures are logged and swallowed: a database hiccup must not turn a
/// successful model call into an empty panel. The consequence is a card without
/// a Verify button, which is visible; losing the advice is not.
async fn persist(
    pool: &Pool<Sqlite>,
    project_id: &str,
    actions: &[GrowthAction],
) -> PersistOutcome {
    // The board is loaded ONCE and grown as we go, so the same mechanism guards
    // both across-review duplication (an action restating last week's card) and
    // within-review duplication (two actions in this batch restating each
    // other). Two separate checks would have to agree, and eventually would not.
    let mut board = growth_store::board(pool, project_id)
        .await
        .unwrap_or_default();
    let mut rows = HashMap::new();
    let mut restated = 0usize;

    for action in actions {
        let fp = growth_store::fingerprint(project_id, &action.title, &action.recommendation);
        let already_here = board.iter().any(|row| row.fingerprint == fp);
        if !already_here {
            // Scoped so the immutable borrow of `board` ends before the push
            // below.
            let restatement = growth_store::restates(&action.title, &action.recommendation, &board)
                .map(|row| (row.title.clone(), row.status.clone()));
            if let Some((existing, status)) = restatement {
                restated += 1;
                tracing::info!(
                    target: "permagentd::growth",
                    "dropped \"{}\": restates \"{existing}\" already {status} on this board",
                    action.title
                );
                continue;
            }
        }

        let seed = ActionSeed {
            title: action.title.clone(),
            recommendation: action.recommendation.clone(),
            category: Some(action.category.clone()),
            artifact_kind: Some(action.artifact_kind.clone()),
            artifact: action.artifact.clone(),
            // Validated here, not trusted: a model that invents a metric the
            // sweep cannot read would pre-register a target that can never be
            // measured, producing an action stuck in "measuring" forever.
            // `split_targeted` has already refused anything unparseable, so by
            // this point both halves are expected to survive.
            target_metric: action
                .target_metric
                .as_deref()
                .and_then(|m| TargetMetric::parse(m).ok())
                .map(|m| m.as_str().to_string()),
            target_dir: action
                .target_dir
                .as_deref()
                .and_then(|d| TargetDir::parse(d).ok())
                .map(|d| d.as_str().to_string()),
        };
        match growth_store::upsert_suggested(pool, project_id, &seed).await {
            // An action that comes back still archived is one the store refused
            // to resurrect: a finished experiment, whose outcomes and frozen
            // baseline pivot belong to the text being re-proposed. It never
            // reaches the active list, so recording it as a success here made it
            // vanish with no card, no counter and no log line. It is a
            // duplicate of work already on record, which is what this counter
            // means, so it is counted as one and named.
            Ok(row) if row.status == growth_store::STATUS_ARCHIVED => {
                restated += 1;
                tracing::info!(
                    target: "permagentd::growth",
                    "dropped \"{}\": restates an archived action that was already measured",
                    action.title
                );
            }
            Ok(row) => {
                if !already_here {
                    board.push(row.clone());
                }
                rows.insert(row.fingerprint.clone(), row);
            }
            Err(e) => tracing::warn!(
                target: "permagentd::growth",
                "could not persist growth action \"{}\": {e}", action.title
            ),
        }
    }
    PersistOutcome { rows, restated }
}

/// Where an action sits on the board: new work first, then work being measured,
/// then work that is finished with.
fn status_bucket(status: &str) -> u8 {
    match status {
        growth_store::STATUS_SUGGESTED | growth_store::STATUS_DONE => 0,
        growth_store::STATUS_VERIFIED | growth_store::STATUS_MEASURING => 1,
        growth_store::STATUS_JUDGED => 2,
        growth_store::STATUS_DISMISSED => 3,
        _ => 4,
    }
}

async fn outcome_views(pool: &Pool<Sqlite>, action_id: &str) -> Vec<OutcomeView> {
    growth_store::outcomes_for(pool, action_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|o| OutcomeView {
            window_days: o.window_days,
            verdict: o.verdict,
            rationale: o.rationale,
            delta_pct: o.delta_pct,
            confounders: o
                .confounders
                .as_deref()
                .and_then(|raw| serde_json::from_str(raw).ok())
                .unwrap_or_default(),
            judged_at: o.judged_at,
        })
        .collect()
}

/// Transfer notes by category, or nothing at all.
///
/// The `count(*)` pre-check is not an optimisation detail: with no outcomes
/// anywhere — which is the state of every install until the first 7-day window
/// closes — the pooled path would otherwise segment every active project on
/// every panel open to compute nothing.
async fn transfer_notes(pool: &Pool<Sqlite>, project_id: &str) -> HashMap<String, TransferNote> {
    let outcomes: i64 = sqlx::query_scalar("SELECT count(*) FROM growth_action_outcomes")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    if outcomes == 0 {
        return HashMap::new();
    }
    let now = chrono::Utc::now();
    let segment = pooled::segment_for(pool, project_id, now).await;
    let label = segment.label();
    pooled::pool_by_category(pool, project_id, &segment, now)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|category| {
            (
                category.category.clone(),
                TransferNote {
                    category: category.category,
                    projects: category.projects,
                    helped: category.helped,
                    hindered: category.hindered,
                    no_effect: category.no_effect,
                    median_delta_pct: category.median_delta_pct,
                    segment_label: label.clone(),
                    segment_projects: category.segment_projects,
                    segment_helped: category.segment_helped,
                    segment_hindered: category.segment_hindered,
                    segment_no_effect: category.segment_no_effect,
                    examples: category
                        .examples
                        .into_iter()
                        .map(|e| TransferExample {
                            project_name: e.project_name,
                            title: e.title,
                            verdict: e.verdict,
                            delta_pct: e.delta_pct,
                        })
                        .collect(),
                },
            )
        })
        .collect()
}

/// Build the panel from the durable rows, using the metadata bag only for the
/// prose the table has no column for.
///
/// This replaced `hydrate`, which walked the CACHED list and decorated whatever
/// it found there. Because `store` overwrites that bag wholesale, an action the
/// latest review did not re-emit simply disappeared from the panel — including
/// a `measuring` action the sweep was still writing outcomes for. The rows are
/// the list now; the bag only adds evidence, steps, impact and confidence, and
/// where it has none the card renders nothing rather than a guess.
struct RenderedBoard {
    active: Vec<GrowthAction>,
    /// Verified work whose effect is still being measured, and the verdicts the
    /// sweep has reached. See [`GrowthActionsData::tracking`].
    tracking: Vec<GrowthAction>,
    archived: Vec<GrowthAction>,
    dismissed: Vec<GrowthAction>,
}

/// Is this action being measured rather than waiting on a decision?
///
/// The split behind the Tracking view. `verified` and `measuring` are work the
/// sweep is watching; `judged` is work it has reached a verdict on. None of the
/// three is asking the user for anything, and leaving them in the Actions list
/// made a board of ten items where two were decisions and eight were history.
///
/// They are MOVED, never hidden: #1053 put them on the active board precisely
/// so in-flight work could not silently vanish while the sweep still measured
/// it, and that guarantee is kept — every one of these rows is still in the
/// payload, still rendered, still carrying its outcomes.
fn is_tracked(status: &str) -> bool {
    matches!(
        status,
        growth_store::STATUS_VERIFIED
            | growth_store::STATUS_MEASURING
            | growth_store::STATUS_JUDGED
    )
}

async fn render_board(
    pool: &Pool<Sqlite>,
    project_id: &str,
    cache: &ActionsCache,
) -> RenderedBoard {
    let rows = match growth_store::list_for_project(pool, project_id).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(target: "permagentd::growth", "could not read growth actions: {e}");
            return RenderedBoard {
                active: Vec::new(),
                tracking: Vec::new(),
                archived: Vec::new(),
                dismissed: Vec::new(),
            };
        }
    };
    let by_fingerprint: HashMap<&str, (&CachedProse, usize)> = cache
        .prose
        .iter()
        .enumerate()
        .map(|(rank, prose)| (prose.fingerprint.as_str(), (prose, rank)))
        .collect();
    let transfers = transfer_notes(pool, project_id).await;

    let mut active: Vec<(u8, usize, GrowthAction)> = Vec::new();
    let mut tracking: Vec<(u8, usize, GrowthAction)> = Vec::new();
    let mut archived: Vec<GrowthAction> = Vec::new();
    let mut dismissed: Vec<GrowthAction> = Vec::new();
    for row in rows {
        let found = by_fingerprint.get(row.fingerprint.as_str()).copied();
        let prose = found.map(|(prose, _)| prose);
        // No cache entry means the card was never in a rendered review, or its
        // prose has since been pruned. It sorts last within its bucket rather
        // than being hidden.
        let rank = found.map(|(_, rank)| rank).unwrap_or(usize::MAX);
        let action = GrowthAction {
            title: row.title.clone(),
            evidence: prose.map(|p| p.evidence.clone()).unwrap_or_default(),
            recommendation: row.recommendation.clone(),
            steps: prose.map(|p| p.steps.clone()).unwrap_or_default(),
            artifact_kind: row
                .artifact_kind
                .clone()
                .unwrap_or_else(|| "none".to_string()),
            artifact: row.artifact.clone(),
            category: row.category.clone().unwrap_or_else(|| "ux".to_string()),
            impact: prose.map(|p| p.impact.clone()).unwrap_or_default(),
            confidence: prose.map(|p| p.confidence.clone()).unwrap_or_default(),
            target_metric: row.target_metric.clone(),
            target_dir: row.target_dir.clone(),
            transfer: row
                .category
                .as_deref()
                .and_then(|category| transfers.get(category))
                .cloned(),
            // ALWAYS `Some`, for every row, whatever its status and whether or
            // not the prose cache still remembers it. This is what the panel's
            // controls key on: the four actions this project has carried since
            // 2026-08-14 have no cache entry left, so a card built from the
            // cache had no id to post a dismissal with and rendered with no
            // control at all — visible, stale, and impossible to act on.
            identity: Some(identity_of(pool, &row).await),
        };
        // Four lists, because the user needs four different things from them:
        // work still asking for a decision, work whose effect is being
        // measured, work filed away, and advice already refused. A dismissed
        // card in the active list was the panel's only growth path —
        // `suggested` cannot be archived, so before this nothing the user could
        // press ever shortened the list.
        if row.status == growth_store::STATUS_ARCHIVED {
            archived.push(action);
        } else if row.status == growth_store::STATUS_DISMISSED {
            dismissed.push(action);
        } else if is_tracked(&row.status) {
            tracking.push((status_bucket(&row.status), rank, action));
        } else {
            active.push((status_bucket(&row.status), rank, action));
        }
    }

    // `list_for_project` is already id DESC and `sort_by_key` is stable, so
    // rows with no cache entry keep newest-first order within their bucket.
    active.sort_by_key(|(bucket, rank, _)| (*bucket, *rank));
    tracking.sort_by_key(|(bucket, rank, _)| (*bucket, *rank));
    RenderedBoard {
        active: active.into_iter().map(|(_, _, action)| action).collect(),
        tracking: tracking.into_iter().map(|(_, _, action)| action).collect(),
        archived,
        dismissed,
    }
}

/// Everything the generator is shown, assembled from sources that cannot lie
/// about each other: the analytics summary, the open board, this project's own
/// measured outcomes, the same categories measured elsewhere, and the git repo
/// as it is right now.
pub struct GenerationBrief<'a> {
    pub project_name: &'a str,
    pub summary: &'a AnalyticsSummary,
    /// The open board (`growth_store::render_board`). Its absence is why the
    /// generator restated three of its own actions on 2026-08-19: nothing in
    /// the prompt had ever mentioned that they existed.
    pub board: Option<String>,
    /// This project's confirmed outcomes (`growth_store::render_learning`).
    pub learning: Option<String>,
    /// The same categories measured on the other active projects
    /// (`pooled::render_pool`).
    pub pooled: Option<String>,
    /// The Steward's reading of the current tree (`render_codebase_brief`).
    /// Without it, "Review again" is blind to work that has already landed,
    /// which is why the same FAQPage / instrumentation cards kept coming back.
    pub codebase: Option<String>,
}

/// Render the brief. Pure, so the prompt can be asserted in a unit test without
/// a provider — the same property `render_summary`'s tests already rely on.
pub fn render_brief(brief: &GenerationBrief<'_>, correction: Option<&str>) -> String {
    let mut out = render_summary(brief.project_name, brief.summary);
    for block in [
        brief.codebase.as_deref(),
        brief.board.as_deref(),
        brief.learning.as_deref(),
        brief.pooled.as_deref(),
        correction,
    ] {
        let Some(block) = block.map(str::trim).filter(|b| !b.is_empty()) else {
            continue;
        };
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
        out.push_str(block);
        out.push('\n');
    }
    out
}

async fn generate(
    brief: &GenerationBrief<'_>,
    correction: Option<&str>,
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

    // Only confirmed outcomes reach the prompt, and they arrive with their count
    // and an explicit "weak evidence" caveat attached (see
    // `growth::store::render_learning`) — a model shown one result without its
    // sample size will over-generalise from it.
    let user = Message::user().with_text(render_brief(brief, correction));
    let (response, _usage) = provider
        .complete_fast("growth-actions", SYSTEM, std::slice::from_ref(&user), &[])
        .await
        .map_err(|e| format!("The model call failed: {e}"))?;
    Ok(parse_actions(&response.as_concat_text()))
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

/// The prose for one action that the `growth_actions` table has no column for.
///
/// Keyed by fingerprint rather than by position, because position is exactly
/// what a regenerate changes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CachedProse {
    pub fingerprint: String,
    pub evidence: String,
    #[serde(default)]
    pub steps: Vec<String>,
    pub impact: String,
    pub confidence: String,
}

/// What lives in `projects.metadata_json["growth_actions"]` now: prose, and the
/// facts about the last review itself.
///
/// It stopped holding the rendered list because it never could hold it
/// honestly — `store` replaces it wholesale, so anything durable kept here was
/// destroyed by the next "Review again". What remains is the text the table has
/// no column for, joined back on by fingerprint.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ActionsCache {
    /// Ranked order of the last review first, then retained entries.
    #[serde(default)]
    pub prose: Vec<CachedProse>,
    pub generated_at: Option<String>,
    pub reason: Option<String>,
    pub period_days: Option<u32>,
    #[serde(default)]
    pub dropped_for_no_target: usize,
    #[serde(default)]
    pub dropped_as_restatement: usize,
    #[serde(default)]
    pub dropped_as_already_present: usize,
}

/// The pre-change shape of the bag: the whole rendered list.
///
/// Every field defaults, so a bag written by any earlier build still parses.
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct LegacyCache {
    actions: Vec<LegacyAction>,
    generated_at: Option<String>,
    reason: Option<String>,
    period_days: Option<u32>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct LegacyAction {
    title: String,
    recommendation: String,
    evidence: String,
    steps: Vec<String>,
    impact: String,
    confidence: String,
}

impl ActionsCache {
    /// Read the bag, upcasting the old shape rather than discarding it.
    ///
    /// This is the highest-risk line in the change. Without the upcast, the
    /// first time the panel is opened after deploy every live action silently
    /// loses its evidence, steps, impact and confidence — and the rows
    /// themselves would look untouched, so nothing would appear to be wrong.
    /// The fingerprint is recomputed from the title and recommendation exactly
    /// as `persist` computes it, which is what lets the prose find its row
    /// again.
    pub fn from_value(project_id: &str, value: &serde_json::Value) -> Self {
        if value.get("prose").is_some() {
            return serde_json::from_value(value.clone()).unwrap_or_default();
        }
        let legacy: LegacyCache = serde_json::from_value(value.clone()).unwrap_or_default();
        Self {
            prose: legacy
                .actions
                .into_iter()
                .filter(|a| !a.title.is_empty())
                .map(|a| CachedProse {
                    fingerprint: growth_store::fingerprint(project_id, &a.title, &a.recommendation),
                    evidence: a.evidence,
                    steps: a.steps,
                    impact: a.impact,
                    confidence: a.confidence,
                })
                .collect(),
            generated_at: legacy.generated_at,
            reason: legacy.reason,
            period_days: legacy.period_days,
            dropped_for_no_target: 0,
            dropped_as_restatement: 0,
            dropped_as_already_present: 0,
        }
    }
}

/// How many prose entries the bag keeps. Bounded so a long-lived project's
/// metadata does not grow without limit; the rows themselves are not capped,
/// so an old card renders without its evidence rather than disappearing.
const MAX_CACHED_PROSE: usize = 100;

fn cached(project: &Project) -> ActionsCache {
    project
        .metadata_json
        .get(METADATA_KEY)
        .map(|value| ActionsCache::from_value(&project.id, value))
        .unwrap_or_default()
}

/// Merge this review's prose into what is already there, and write it back.
///
/// A merge rather than a replacement, because a regenerate that no longer
/// proposes an action must not delete the figure that action cited — the action
/// itself is still on the board, and may still be being measured. Entries whose
/// fingerprint no longer matches any row are dropped: they belong to advice that
/// no longer exists in any form.
async fn store(pool: &Pool<Sqlite>, project: &Project, fresh: &ActionsCache) -> Result<(), String> {
    let rows = growth_store::list_for_project(pool, &project.id)
        .await
        .unwrap_or_default();
    // `list_for_project` is id DESC, so the index doubles as row recency.
    let recency: HashMap<&str, usize> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| (row.fingerprint.as_str(), i))
        .collect();

    let previous = cached(project);
    let mut merged: Vec<CachedProse> = Vec::new();
    let mut seen = HashSet::new();
    for entry in fresh.prose.iter().chain(previous.prose.iter()) {
        if !recency.contains_key(entry.fingerprint.as_str()) {
            continue;
        }
        if !seen.insert(entry.fingerprint.clone()) {
            continue;
        }
        merged.push(entry.clone());
    }
    if merged.len() > MAX_CACHED_PROSE {
        // Choose what to keep by row recency, but keep the surviving entries in
        // their existing order — that order is the last review's ranking, which
        // `render_board` uses as its tiebreak.
        let mut by_recency: Vec<&CachedProse> = merged.iter().collect();
        by_recency.sort_by_key(|entry| {
            recency
                .get(entry.fingerprint.as_str())
                .copied()
                .unwrap_or(usize::MAX)
        });
        let keep: HashSet<String> = by_recency
            .iter()
            .take(MAX_CACHED_PROSE)
            .map(|entry| entry.fingerprint.clone())
            .collect();
        merged.retain(|entry| keep.contains(&entry.fingerprint));
    }

    let data = ActionsCache {
        prose: merged,
        generated_at: fresh.generated_at.clone(),
        reason: fresh.reason.clone(),
        period_days: fresh.period_days,
        dropped_for_no_target: fresh.dropped_for_no_target,
        dropped_as_restatement: fresh.dropped_as_restatement,
        dropped_as_already_present: fresh.dropped_as_already_present,
    };

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
            serde_json::to_value(&data).map_err(|e| e.to_string())?,
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

/// Assemble the payload both seams return: the rows are the list, the bag adds
/// prose and the facts about the last review.
async fn assemble(
    pool: &Pool<Sqlite>,
    project: &Project,
    cache: &ActionsCache,
) -> GrowthActionsData {
    let board = render_board(pool, &project.id, cache).await;
    let started_at = review_started_at(&project.id);
    GrowthActionsData {
        actions: board.active,
        tracking: board.tracking,
        archived: board.archived,
        dismissed: board.dismissed,
        generated_at: cache.generated_at.clone(),
        reason: cache.reason.clone(),
        period_days: cache.period_days,
        dropped_for_no_target: cache.dropped_for_no_target,
        dropped_as_restatement: cache.dropped_as_restatement,
        dropped_as_already_present: cache.dropped_as_already_present,
        generating: started_at.is_some(),
        generation_started_at: started_at,
    }
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
    Ok(Json(assemble(&pool, &project, &cached(&project)).await))
}

/// Start a review and return immediately.
///
/// It used to run to completion inside this handler, which made the whole
/// feature hostage to one HTTP request: the browser holds a fetch open for the
/// length of a model call, and the panel's only record that anything was
/// happening was a `useState` in the component that issued it. Leaving the tab
/// unmounted that component, so the button reverted to its idle label while the
/// review went on running and dropped its result into the database unseen.
///
/// So the work is spawned and the request answers with the board as it stands,
/// flagged `generating`. That flag is server state, which is what makes it
/// survive navigation: any client, on any mount, can ask what is in flight.
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

    // A second click while one is running is answered with the same board and
    // the same `generating: true`, not with a second model call. Idempotent
    // rather than a refusal: the user asked for a review and a review is
    // running, so nothing has gone wrong and an error would say it had.
    if let Some(slot) = begin_review(&project.id, chrono::Utc::now().to_rfc3339()) {
        let pool = pool.clone();
        let id = project.id.clone();
        tokio::spawn(async move {
            // Moved into the task so the slot is released when the review ends
            // — including by panic.
            let _slot = slot;
            run_review(&pool, &id).await;
        });
    }

    Ok(Json(assemble(&pool, &project, &cached(&project)).await))
}

/// One review, end to end: generate, persist, cache, announce.
///
/// Split out of the handler so it can outlive the request that asked for it.
/// It re-reads the project rather than taking one captured before the spawn,
/// because the metadata bag it merges into may have been written since.
async fn run_review(pool: &Pool<Sqlite>, project_id: &str) {
    let project = match projects::get_project(pool, project_id).await {
        Ok(Some(project)) => project,
        Ok(None) => return,
        Err(e) => {
            tracing::warn!(target: "permagentd::growth", "review could not read its project: {e}");
            return;
        }
    };
    review(pool, &project).await;
    // Multi-client liveness (#629): the same seam every other project write
    // uses. The Grow panel refetches on it, so a review that finishes while the
    // user is on another tab shows its actions the moment they come back — and
    // on a second open client too — with nothing to press.
    permagent::events::emit(permagent::events::project_changed(
        project_id,
        "growth_actions",
    ));
}

async fn review(pool: &Pool<Sqlite>, project: &Project) {
    let period_days = 30;
    let summary = load_summary(pool, &project.id, period_days).await;
    let now = chrono::Utc::now().to_rfc3339();

    // The Steward reads the repo BEFORE the model does. Suggested work that is
    // already in the tree comes off the board so the prompt cannot treat it as
    // open, and so the user is not asked to do it again. This is independent of
    // whether there is enough analytics to generate new advice.
    let dismissed_before = growth_verify::dismiss_already_present(pool, project).await;

    let mut dropped_for_no_target = 0usize;
    let (actions, reason) = match readiness(&summary) {
        Err(reason) => (Vec::new(), Some(reason)),
        Ok(()) => {
            // The board is what the generator was blind to. It is rendered from
            // the same `growth_store::board` a new suggestion is then checked
            // against, so the prompt and the guard can never describe different
            // sets of open work. Loaded AFTER the Steward pass so dismissed-as-
            // already-present rows show as dismissed, not as open work.
            let board = growth_store::render_board(
                &growth_store::board(pool, &project.id)
                    .await
                    .unwrap_or_default(),
            );
            let learning = growth_store::render_learning(
                &growth_store::learnable_outcomes(pool, &project.id, 8)
                    .await
                    .unwrap_or_default(),
            );
            let pooled_block = pooled_learning(pool, &project.id).await;
            let codebase = growth_verify::render_codebase_brief(project, &dismissed_before).await;
            let brief = GenerationBrief {
                project_name: &project.name,
                summary: &summary,
                board,
                learning,
                pooled: pooled_block,
                codebase,
            };

            match generate(&brief, None).await {
                Err(reason) => (Vec::new(), Some(reason)),
                Ok(first) => {
                    let proposed = first.len();
                    let (mut kept, dropped) = split_targeted(first);
                    let mut refused: BTreeSet<String> = BTreeSet::new();
                    for (action, why) in &dropped {
                        tracing::warn!(
                            target: "permagentd::growth",
                            "dropped \"{}\": {why}", action.title
                        );
                        refused.insert(action.title.clone());
                    }
                    if !dropped.is_empty() {
                        // Exactly one corrective. A second would be a loop, and
                        // a model that ignores a correction naming its own
                        // offending titles will ignore the next one too.
                        match generate(&brief, Some(&retry_correction(&dropped))).await {
                            Ok(second) => {
                                let (kept_again, dropped_again) = split_targeted(second);
                                for (action, why) in &dropped_again {
                                    tracing::warn!(
                                        target: "permagentd::growth",
                                        "dropped on retry \"{}\": {why}", action.title
                                    );
                                    refused.insert(action.title.clone());
                                }
                                kept = merge_attempts(kept, kept_again, &project.id);
                            }
                            Err(e) => tracing::warn!(
                                target: "permagentd::growth",
                                "the corrective retry failed: {e}"
                            ),
                        }
                    }
                    // A title the retry brought back correctly was not dropped:
                    // the user got the advice, which is what the counter is
                    // reporting on.
                    for action in &kept {
                        refused.remove(&action.title);
                    }
                    dropped_for_no_target = refused.len();

                    let reason = if kept.is_empty() && proposed > 0 {
                        Some(format!(
                            "This review proposed {proposed} action(s) but none of them named a \
                             metric this system can measure, so none were kept. Run it again — \
                             that is the agent's failure, not your data's."
                        ))
                    } else if kept.is_empty() {
                        Some(
                            "The data does not support a specific action right now — nothing here \
                             is strong enough to act on."
                                .to_string(),
                        )
                    } else {
                        None
                    };
                    (kept, reason)
                }
            }
        }
    };

    // Identity first, then rank, then cache: ranking reads what actually worked
    // on this project, which only exists once the rows are there.
    let ranked = rank_with_history(actions, &category_history(pool, &project.id).await);
    let persisted = persist(pool, &project.id, &ranked).await;
    // Newly persisted suggestions can themselves already be in the tree — the
    // model did not see a file the Steward's grep would have. Dismiss those
    // too, so they land in Dismissed rather than on the active board.
    let dismissed_after = growth_verify::dismiss_already_present(pool, project).await;
    let dropped_as_already_present = dismissed_before.len() + dismissed_after.len();

    let cache = ActionsCache {
        // Only actions that reached a row contribute prose. A restatement was
        // refused, so its evidence would be an orphan the next merge deletes
        // anyway.
        prose: ranked
            .iter()
            .filter_map(|action| {
                let fingerprint =
                    growth_store::fingerprint(&project.id, &action.title, &action.recommendation);
                persisted
                    .rows
                    .contains_key(&fingerprint)
                    .then(|| CachedProse {
                        fingerprint,
                        evidence: action.evidence.clone(),
                        steps: action.steps.clone(),
                        impact: action.impact.clone(),
                        confidence: action.confidence.clone(),
                    })
            })
            .collect(),
        generated_at: Some(now),
        reason,
        period_days: Some(period_days),
        dropped_for_no_target,
        dropped_as_restatement: persisted.restated,
        dropped_as_already_present,
    };
    if let Err(e) = store(pool, project, &cache).await {
        tracing::warn!(target: "permagentd::growth", "could not cache growth actions: {e}");
    }
}

/// The pooled cross-project block for the brief, or nothing.
///
/// Behind the same `count(*)` guard as `transfer_notes`: until the first window
/// closes anywhere there is nothing to pool, and segmenting every active
/// project to discover that on every review is a cost with no possible return.
async fn pooled_learning(pool: &Pool<Sqlite>, project_id: &str) -> Option<String> {
    let outcomes: i64 = sqlx::query_scalar("SELECT count(*) FROM growth_action_outcomes")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    if outcomes == 0 {
        return None;
    }
    let now = chrono::Utc::now();
    let segment = pooled::segment_for(pool, project_id, now).await;
    let pools = pooled::pool_by_category(pool, project_id, &segment, now)
        .await
        .unwrap_or_default();
    pooled::render_pool(&pools, &segment)
}

// ── Pre-registration, verification, measurement ──────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusRequest {
    pub status: String,
    /// pageviews | sessions | aeo_visits | bounce_rate.
    pub target_metric: Option<String>,
    /// up | down.
    pub target_dir: Option<String>,
}

/// Parse a pre-registration, or explain why it is not one.
///
/// Both halves or neither: a metric with no direction cannot be scored, and a
/// direction with no metric has nothing to score.
fn parse_target(
    metric: Option<&str>,
    dir: Option<&str>,
) -> Result<Option<(TargetMetric, TargetDir)>, String> {
    match (metric, dir) {
        (None, None) => Ok(None),
        (Some(m), Some(d)) => Ok(Some((TargetMetric::parse(m)?, TargetDir::parse(d)?))),
        (Some(_), None) => Err("A target metric needs a direction: \"up\" or \"down\".".into()),
        (None, Some(_)) => Err("A direction needs a target metric to apply to.".into()),
    }
}

/// Refuse a pre-registration that changes an action's claim after its baseline
/// was frozen.
///
/// Without this the hypothesis is editable with the result already in view,
/// which is the definition of the unfalsifiable verdict the proposal's first
/// rule exists to prevent. Re-sending the SAME target is allowed — that is an
/// idempotent retry, not a rewrite.
fn reject_late_reregistration(
    action: &growth_store::GrowthActionRow,
    supplied: Option<(TargetMetric, TargetDir)>,
) -> Result<(), (StatusCode, String)> {
    let Some((metric, dir)) = supplied else {
        return Ok(());
    };
    if action.baseline_json.is_none() {
        return Ok(());
    }
    let unchanged = action.target_metric.as_deref() == Some(metric.as_str())
        && action.target_dir.as_deref() == Some(dir.as_str());
    if unchanged {
        return Ok(());
    }
    Err((
        StatusCode::CONFLICT,
        format!(
            "This action already pre-registered {} going {} and its baseline is frozen, so the \
             claim cannot be changed now. Dismiss it and start a new action to measure something \
             else.",
            action.target_metric.as_deref().unwrap_or("a metric"),
            action.target_dir.as_deref().unwrap_or("somewhere"),
        ),
    ))
}

/// What a verify call is allowed to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VerifyMode {
    /// First verification: pre-register the claim, freeze the baseline, stamp
    /// `verified_at`.
    Record,
    /// Already verified: re-run the strategies, report what they find, and
    /// write nothing at all.
    Recheck,
}

/// Which one this call is.
///
/// The panel offers a "Re-check" button because the evidence a check found is
/// not persisted — there is no `verified_detail` column — so after a reload the
/// badge has nothing behind it and the only route back to "which commit, which
/// path" is to ask again. That button must not cost anything, and taking the
/// `Record` path twice costs a great deal:
///
///   * `record_verification` sets `verified_at` to now, and `verified_at` IS
///     the pivot every comparison window is measured from
///     (`metrics::pivot_date`). A second write slides the after-windows forward
///     while the baseline stays frozen at the first verification, so the before
///     and after no longer meet — and the verdict would then be computed with
///     the result already in view, which is the exact unfalsifiability the
///     pre-registration gate exists to prevent.
///   * it also resets `status` to `verified`, dragging a judged action back
///     into measurement, and the `done` pre-registration write does the same
///     from the other end.
fn verify_mode(action: &growth_store::GrowthActionRow) -> VerifyMode {
    if action.verified_at.is_some() {
        VerifyMode::Recheck
    } else {
        VerifyMode::Record
    }
}

/// Refuse to archive something nothing has happened to.
///
/// Archiving is what releases an action's text for re-proposal — `board`
/// excludes archived rows, and `restates` is checked against `board`. Archiving
/// a card that was never acted on would therefore hand the same advice straight
/// back on the next review, which is not what "file it away" means to anyone.
/// Dismissal is the control for advice the user does not want, and it keeps the
/// text off the board.
fn reject_pointless_archive(
    action: &growth_store::GrowthActionRow,
    requested: &str,
) -> Result<(), (StatusCode, String)> {
    if requested == growth_store::STATUS_ARCHIVED && action.status == growth_store::STATUS_SUGGESTED
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Nothing has happened to this action yet, so there is nothing to file away. Dismiss \
             it instead."
                .to_string(),
        ));
    }
    Ok(())
}

/// Move an action through its lifecycle, pre-registering what it claims it will
/// move.
///
/// Marking an action `done` without a target is refused. That is the
/// proposal's first rule made unavoidable: "The metric, the direction, and the
/// expected magnitude are recorded *before* the action is marked done. A verdict
/// computed against a metric chosen afterwards is unfalsifiable."
async fn set_action_status(
    State(state): State<Arc<AppState>>,
    Path((project_id, action_id)): Path<(String, String)>,
    Json(body): Json<StatusRequest>,
) -> Result<Json<ActionIdentity>, ApiError> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project = projects::get_project_by_id_or_slug(&pool, &project_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "unknown project".to_string()))?;

    let action = growth_store::get(&pool, &project.id, &action_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "unknown action".to_string()))?;

    let target = parse_target(body.target_metric.as_deref(), body.target_dir.as_deref())
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    reject_late_reregistration(&action, target)?;
    reject_pointless_archive(&action, &body.status)?;

    let will_have_target =
        target.is_some() || (action.target_metric.is_some() && action.target_dir.is_some());
    if body.status == growth_store::STATUS_DONE && !will_have_target {
        return Err((
            StatusCode::BAD_REQUEST,
            "Marking an action done pre-registers what it should move. Send targetMetric (one of \
             pageviews, sessions, aeo_visits, bounce_rate) and targetDir (up or down) — a metric \
             chosen after the result is known cannot be wrong."
                .to_string(),
        )
            .into());
    }

    let updated = growth_store::set_status(&pool, &project.id, &action_id, &body.status, target)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "unknown action".to_string()))?;
    Ok(Json(identity_of(&pool, &updated).await))
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyRequest {
    /// Record the user's word when nothing could be checked. Labelled as such.
    #[serde(default)]
    pub self_attested: bool,
    /// Text to look for on the live page, for the `content` strategy.
    #[serde(default)]
    pub expect_substring: Option<String>,
    #[serde(default)]
    pub target_metric: Option<String>,
    #[serde(default)]
    pub target_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyResponse {
    pub verified: bool,
    pub identity: ActionIdentity,
    /// Every strategy that was tried, passed or not — so a card can say why it
    /// could not confirm rather than reading as "not done".
    pub checks: Vec<Check>,
    /// The frozen before-windows and weekly history the verdicts will use.
    pub baseline: Option<Baseline>,
    /// Present when nothing could confirm the change.
    pub reason: Option<String>,
}

/// Check that the change landed, record HOW, and freeze the baseline.
async fn verify_action(
    State(state): State<Arc<AppState>>,
    Path((project_id, action_id)): Path<(String, String)>,
    body: Option<Json<VerifyRequest>>,
) -> Result<Json<VerifyResponse>, ApiError> {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project = projects::get_project_by_id_or_slug(&pool, &project_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "unknown project".to_string()))?;
    let action = growth_store::get(&pool, &project.id, &action_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "unknown action".to_string()))?;

    // Pre-registration may arrive with the verify call, but it must exist before
    // a baseline is frozen — otherwise the metric is chosen with the outcome
    // already in view.
    let supplied = parse_target(body.target_metric.as_deref(), body.target_dir.as_deref())
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    reject_late_reregistration(&action, supplied)?;
    let existing = parse_target(
        action.target_metric.as_deref(),
        action.target_dir.as_deref(),
    )
    .unwrap_or(None);
    let Some((metric, dir)) = supplied.or(existing) else {
        return Err((
            StatusCode::BAD_REQUEST,
            "This action has no pre-registered metric, so a verdict computed for it could not be \
             wrong. Send targetMetric and targetDir first."
                .to_string(),
        )
            .into());
    };
    // Every write below is skipped for a re-check. See `verify_mode` for what
    // taking the recording path a second time would do to the measurement.
    let rechecking = verify_mode(&action) == VerifyMode::Recheck;

    if supplied.is_some() && !rechecking {
        growth_store::set_status(
            &pool,
            &project.id,
            &action.id,
            growth_store::STATUS_DONE,
            supplied,
        )
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    }

    let outcome = growth_verify::verify(
        &pool,
        &project,
        &action,
        body.expect_substring.as_deref(),
        body.self_attested,
    )
    .await;

    if rechecking {
        // `verified` reports the stored fact, which has not changed; `checks`
        // reports what the strategies find NOW. They can honestly disagree — a
        // commit can be reverted, a page can be edited — and saying so is the
        // point of showing the checks at all.
        return Ok(Json(VerifyResponse {
            verified: true,
            identity: identity_of(&pool, &action).await,
            checks: outcome.checks,
            baseline: action
                .baseline_json
                .as_deref()
                .and_then(|raw| serde_json::from_str::<Baseline>(raw).ok()),
            reason: None,
        }));
    }

    let Some(verified_by) = outcome.verified_by else {
        // Not an error: the change may simply not have shipped yet. The checks
        // carry the reason, and self-attestation stays available.
        return Ok(Json(VerifyResponse {
            verified: false,
            identity: identity_of(&pool, &action).await,
            checks: outcome.checks,
            baseline: None,
            reason: Some(
                "Nothing could confirm the change landed. The checks below say what was looked \
                 for; if it did land, re-send with selfAttested: true and the card will show that \
                 it was your word rather than a commit."
                    .to_string(),
            ),
        }));
    };

    // A re-verification must not move the goalposts. Once a baseline is frozen
    // the "before" windows and the variance history are fixed, and recomputing
    // them against a later pivot would quietly re-run the comparison with the
    // result already visible. `record_verification` coalesces for the same
    // reason; this branch also stops the pointless recomputation.
    let verified_at = chrono::Utc::now();
    let existing_baseline = action
        .baseline_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Baseline>(raw).ok());
    let (baseline, encoded) = match existing_baseline {
        Some(frozen) => (frozen, None),
        None => {
            let fresh = sweep::snapshot_baseline(&pool, &project.id, metric, dir, verified_at)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            let encoded = serde_json::to_string(&fresh)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            (fresh, Some(encoded))
        }
    };

    let updated = growth_store::record_verification(
        &pool,
        &project.id,
        &action.id,
        verified_by,
        &verified_at.to_rfc3339(),
        encoded.as_deref(),
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .ok_or((StatusCode::NOT_FOUND, "unknown action".to_string()))?;

    Ok(Json(VerifyResponse {
        verified: true,
        identity: identity_of(&pool, &updated).await,
        checks: outcome.checks,
        baseline: Some(baseline),
        reason: None,
    }))
}

/// Judge every due window now instead of waiting for the nightly pass.
async fn measure_now(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let report = sweep::run(&pool, chrono::Utc::now())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(serde_json::json!({
        "actionsConsidered": report.actions_considered,
        "windowsJudged": report.windows_judged,
        "actionsCompleted": report.actions_completed,
        "errors": report.errors,
    })))
}

/// The durable half of a card, read from the row.
///
/// One function, every seam — the board, the lifecycle route and both verify
/// replies. `render_board` used to build this struct inline, which is how a
/// field could be added to the identity the verify reply returns and silently
/// missing from the identity the board returns for the same action.
async fn identity_of(pool: &Pool<Sqlite>, row: &growth_store::GrowthActionRow) -> ActionIdentity {
    let outcomes = outcome_views(pool, &row.id).await;
    ActionIdentity {
        id: row.id.clone(),
        status: row.status.clone(),
        target_metric: row.target_metric.clone(),
        target_dir: row.target_dir.clone(),
        verified_by: row.verified_by.clone(),
        verified_at: row.verified_at.clone(),
        outcomes,
        baseline: baseline_view(row.baseline_json.as_deref()),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResultsQuery {
    project_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GrowthResultRowView {
    action_id: String,
    project_id: String,
    project_name: String,
    title: String,
    category: String,
    status: String,
    verdict: Option<String>,
    delta_pct: Option<f64>,
    window_days: Option<i64>,
    judged_at: Option<String>,
    target_metric: Option<String>,
    target_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GrowthCategorySummary {
    category: String,
    projects: usize,
    helped: usize,
    hindered: usize,
    no_effect: usize,
    median_delta_pct: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GrowthProjectResults {
    project_id: String,
    name: String,
    segment_label: String,
    implemented: usize,
    measuring: usize,
    judged: usize,
    helped: usize,
    hindered: usize,
    no_effect: usize,
    inconclusive: usize,
    actions: Vec<GrowthResultRowView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GrowthTrendPoint {
    /// Monday UTC of the week, `YYYY-MM-DD`.
    week: String,
    helped: u32,
    hindered: u32,
    no_effect: u32,
    /// `helped - hindered` this week.
    net: i32,
    /// Running net from the start of the window.
    cumulative_net: i32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GrowthProjectTrend {
    project_id: String,
    project_name: String,
    helped: u32,
    hindered: u32,
    no_effect: u32,
    points: Vec<GrowthTrendPoint>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GrowthFleetResults {
    projects: usize,
    helped: usize,
    hindered: usize,
    no_effect: usize,
    inconclusive: usize,
    categories: Vec<GrowthCategorySummary>,
    recent: Vec<GrowthResultRowView>,
    /// Last 12 weeks, padded with zeros, so two verdicts look like two spikes.
    trend: Vec<GrowthTrendPoint>,
    by_project: Vec<GrowthProjectTrend>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GrowthResults {
    project: Option<GrowthProjectResults>,
    fleet: GrowthFleetResults,
}

type FleetJoin = (
    String,         // o.action_id
    i64,            // o.window_days
    String,         // o.verdict
    Option<f64>,    // o.delta_pct
    String,         // o.judged_at
    Option<String>, // a.category
    String,         // a.title
    String,         // a.project_id
    String,         // p.name
    Option<String>, // a.target_metric
    Option<String>, // a.target_dir
    String,         // a.status
);

fn is_implemented(status: &str) -> bool {
    matches!(
        status,
        growth_store::STATUS_DONE
            | growth_store::STATUS_VERIFIED
            | growth_store::STATUS_MEASURING
            | growth_store::STATUS_JUDGED
    )
}

async fn fleet_outcome_rows(pool: &Pool<Sqlite>) -> Vec<FleetJoin> {
    let rows = sqlx::query_as::<_, FleetJoin>(
        "SELECT o.action_id, o.window_days, o.verdict, o.delta_pct, o.judged_at,
                a.category, a.title, a.project_id, p.name,
                a.target_metric, a.target_dir, a.status
           FROM growth_action_outcomes o
           JOIN growth_actions a ON a.id = o.action_id
           JOIN projects p ON p.id = a.project_id
          WHERE p.status = 'active'
          ORDER BY o.window_days DESC, o.judged_at DESC",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let mut seen = HashSet::new();
    let mut kept = Vec::new();
    for row in rows {
        if seen.insert(row.0.clone()) {
            kept.push(row);
        }
    }
    kept
}

fn tally_verdicts(rows: &[FleetJoin]) -> (usize, usize, usize, usize, usize) {
    let mut helped = 0;
    let mut hindered = 0;
    let mut no_effect = 0;
    let mut inconclusive = 0;
    let mut projects = HashSet::new();
    for row in rows {
        projects.insert(row.7.clone());
        match row.2.as_str() {
            "helped" => helped += 1,
            "hindered" => hindered += 1,
            "no_effect" => no_effect += 1,
            _ => inconclusive += 1,
        }
    }
    (projects.len(), helped, hindered, no_effect, inconclusive)
}

fn category_summaries(rows: &[FleetJoin]) -> Vec<GrowthCategorySummary> {
    struct Acc {
        projects: HashSet<String>,
        helped: usize,
        hindered: usize,
        no_effect: usize,
        deltas: Vec<f64>,
    }
    let mut by: HashMap<String, Acc> = HashMap::new();
    for row in rows {
        if !matches!(row.2.as_str(), "helped" | "hindered" | "no_effect") {
            continue;
        }
        let category = row.5.clone().unwrap_or_else(|| "uncategorised".to_string());
        let entry = by.entry(category).or_insert_with(|| Acc {
            projects: HashSet::new(),
            helped: 0,
            hindered: 0,
            no_effect: 0,
            deltas: Vec::new(),
        });
        entry.projects.insert(row.7.clone());
        match row.2.as_str() {
            "helped" => entry.helped += 1,
            "hindered" => entry.hindered += 1,
            _ => entry.no_effect += 1,
        }
        if let Some(d) = row.3 {
            entry.deltas.push(d);
        }
    }
    let mut out: Vec<GrowthCategorySummary> = by
        .into_iter()
        .map(|(category, acc)| {
            let mut deltas = acc.deltas;
            deltas.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let median = if deltas.is_empty() {
                None
            } else if deltas.len().is_multiple_of(2) {
                let mid = deltas.len() / 2;
                Some((deltas[mid - 1] + deltas[mid]) / 2.0)
            } else {
                Some(deltas[deltas.len() / 2])
            };
            GrowthCategorySummary {
                category,
                projects: acc.projects.len(),
                helped: acc.helped,
                hindered: acc.hindered,
                no_effect: acc.no_effect,
                median_delta_pct: median,
            }
        })
        .collect();
    out.sort_by(|a, b| b.helped.cmp(&a.helped).then(a.category.cmp(&b.category)));
    out
}

/// How far the Home trend looks back. Twelve weeks matches the growth
/// history window; shorter would make a quiet month look like a wall of
/// the few weeks that happened to have a verdict.
const TREND_WEEKS: i64 = 12;

#[derive(Debug, Clone)]
struct TrendSeed {
    project_id: String,
    project_name: String,
    verdict: String,
    judged_at: String,
}

fn monday_of(d: chrono::NaiveDate) -> chrono::NaiveDate {
    d - chrono::Duration::days(i64::from(
        chrono::Datelike::weekday(&d).num_days_from_monday(),
    ))
}

fn judged_day(s: &str) -> Option<chrono::NaiveDate> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&chrono::Utc).date_naive());
    }
    let day = s.get(..10)?;
    chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()
}

fn trend_week_starts(today: chrono::NaiveDate) -> Vec<chrono::NaiveDate> {
    let end = monday_of(today);
    (0..TREND_WEEKS)
        .rev()
        .map(|i| end - chrono::Duration::days(7 * i))
        .collect()
}

fn empty_points(weeks: &[chrono::NaiveDate]) -> Vec<GrowthTrendPoint> {
    weeks
        .iter()
        .map(|w| GrowthTrendPoint {
            week: w.format("%Y-%m-%d").to_string(),
            helped: 0,
            hindered: 0,
            no_effect: 0,
            net: 0,
            cumulative_net: 0,
        })
        .collect()
}

fn accumulate_net(points: &mut [GrowthTrendPoint]) {
    let mut running = 0i32;
    for p in points.iter_mut() {
        p.net = p.helped as i32 - p.hindered as i32;
        running += p.net;
        p.cumulative_net = running;
    }
}

/// Weekly helped/hindered for the fleet and per project.
///
/// One row per action (caller already collapsed windows). Weeks with no
/// verdict stay in the series as zeros so a sparse 12 weeks does not
/// stretch into a solid block.
fn build_growth_trend(
    seeds: &[TrendSeed],
    today: chrono::NaiveDate,
) -> (Vec<GrowthTrendPoint>, Vec<GrowthProjectTrend>) {
    let weeks = trend_week_starts(today);
    let index: HashMap<chrono::NaiveDate, usize> = weeks
        .iter()
        .copied()
        .enumerate()
        .map(|(i, d)| (d, i))
        .collect();

    let mut fleet = empty_points(&weeks);
    struct Acc {
        name: String,
        points: Vec<GrowthTrendPoint>,
        helped: u32,
        hindered: u32,
        no_effect: u32,
        in_window: bool,
    }
    let mut by_project: HashMap<String, Acc> = HashMap::new();

    for seed in seeds {
        let Some(day) = judged_day(&seed.judged_at) else {
            continue;
        };
        let monday = monday_of(day);
        let Some(&i) = index.get(&monday) else {
            continue;
        };
        let entry = by_project
            .entry(seed.project_id.clone())
            .or_insert_with(|| Acc {
                name: seed.project_name.clone(),
                points: empty_points(&weeks),
                helped: 0,
                hindered: 0,
                no_effect: 0,
                in_window: false,
            });
        entry.in_window = true;
        entry.name = seed.project_name.clone();
        match seed.verdict.as_str() {
            "helped" => {
                fleet[i].helped += 1;
                entry.points[i].helped += 1;
                entry.helped += 1;
            }
            "hindered" => {
                fleet[i].hindered += 1;
                entry.points[i].hindered += 1;
                entry.hindered += 1;
            }
            "no_effect" => {
                fleet[i].no_effect += 1;
                entry.points[i].no_effect += 1;
                entry.no_effect += 1;
            }
            _ => {}
        }
    }

    accumulate_net(&mut fleet);
    let mut projects: Vec<GrowthProjectTrend> = by_project
        .into_iter()
        .filter(|(_, acc)| acc.in_window)
        .map(|(project_id, mut acc)| {
            accumulate_net(&mut acc.points);
            GrowthProjectTrend {
                project_id,
                project_name: acc.name,
                helped: acc.helped,
                hindered: acc.hindered,
                no_effect: acc.no_effect,
                points: acc.points,
            }
        })
        .collect();
    projects.sort_by(|a, b| {
        b.helped
            .cmp(&a.helped)
            .then(a.project_name.cmp(&b.project_name))
    });
    (fleet, projects)
}

fn fleet_row_view(row: &FleetJoin) -> GrowthResultRowView {
    GrowthResultRowView {
        action_id: row.0.clone(),
        project_id: row.7.clone(),
        project_name: row.8.clone(),
        title: row.6.clone(),
        category: row.5.clone().unwrap_or_else(|| "ux".to_string()),
        status: row.11.clone(),
        verdict: Some(row.2.clone()),
        delta_pct: row.3,
        window_days: Some(row.1),
        judged_at: Some(row.4.clone()),
        target_metric: row.9.clone(),
        target_dir: row.10.clone(),
    }
}

async fn project_results(
    pool: &Pool<Sqlite>,
    project: &Project,
    fleet: &[FleetJoin],
) -> GrowthProjectResults {
    let now = chrono::Utc::now();
    let segment = pooled::segment_for(pool, &project.id, now).await;
    let rows = growth_store::list_for_project(pool, &project.id)
        .await
        .unwrap_or_default();
    let implemented_rows: Vec<_> = rows
        .into_iter()
        .filter(|r| is_implemented(&r.status))
        .collect();
    let by_id: HashMap<&str, &FleetJoin> = fleet
        .iter()
        .filter(|r| r.7 == project.id)
        .map(|r| (r.0.as_str(), r))
        .collect();
    let mut helped = 0;
    let mut hindered = 0;
    let mut no_effect = 0;
    let mut inconclusive = 0;
    let mut measuring = 0;
    let mut judged = 0;
    let mut actions = Vec::new();
    for row in &implemented_rows {
        match row.status.as_str() {
            s if s == growth_store::STATUS_JUDGED => judged += 1,
            s if s == growth_store::STATUS_VERIFIED || s == growth_store::STATUS_MEASURING => {
                measuring += 1
            }
            _ => {}
        }
        if let Some(outcome) = by_id.get(row.id.as_str()) {
            match outcome.2.as_str() {
                "helped" => helped += 1,
                "hindered" => hindered += 1,
                "no_effect" => no_effect += 1,
                _ => inconclusive += 1,
            }
            actions.push(fleet_row_view(outcome));
        } else {
            actions.push(GrowthResultRowView {
                action_id: row.id.clone(),
                project_id: project.id.clone(),
                project_name: project.name.clone(),
                title: row.title.clone(),
                category: row.category.clone().unwrap_or_else(|| "ux".to_string()),
                status: row.status.clone(),
                verdict: None,
                delta_pct: None,
                window_days: None,
                judged_at: None,
                target_metric: row.target_metric.clone(),
                target_dir: row.target_dir.clone(),
            });
        }
    }
    GrowthProjectResults {
        project_id: project.id.clone(),
        name: project.name.clone(),
        segment_label: segment.label(),
        implemented: implemented_rows.len(),
        measuring,
        judged,
        helped,
        hindered,
        no_effect,
        inconclusive,
        actions,
    }
}

async fn get_results(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ResultsQuery>,
) -> Result<Json<GrowthResults>, ApiError> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let fleet_rows = fleet_outcome_rows(&pool).await;
    let (projects, helped, hindered, no_effect, inconclusive) = tally_verdicts(&fleet_rows);
    let mut recent: Vec<_> = fleet_rows.iter().map(fleet_row_view).collect();
    recent.sort_by(|a, b| b.judged_at.cmp(&a.judged_at));
    recent.truncate(12);
    let seeds: Vec<TrendSeed> = fleet_rows
        .iter()
        .map(|r| TrendSeed {
            project_id: r.7.clone(),
            project_name: r.8.clone(),
            verdict: r.2.clone(),
            judged_at: r.4.clone(),
        })
        .collect();
    let (trend, by_project) = build_growth_trend(&seeds, chrono::Utc::now().date_naive());
    let fleet = GrowthFleetResults {
        projects,
        helped,
        hindered,
        no_effect,
        inconclusive,
        categories: category_summaries(&fleet_rows),
        recent,
        trend,
        by_project,
    };
    let project = if let Some(id) = q.project_id.as_deref() {
        match projects::get_project_by_id_or_slug(&pool, id).await {
            Ok(Some(p)) => Some(project_results(&pool, &p, &fleet_rows).await),
            _ => None,
        }
    } else {
        None
    };
    Ok(Json(GrowthResults { project, fleet }))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompleteFromHarnessResponse {
    implemented: bool,
    verified: bool,
    identity: ActionIdentity,
    checks: Vec<Check>,
    reason: Option<String>,
}

/// A coding harness launched from this action has exited. Try to confirm the
/// change in git/content; if that cannot, mark the action implemented so the
/// user can still verify and measurement can start. Never treat the agent's
/// own "I did it" line as verification.
async fn complete_from_harness(
    State(state): State<Arc<AppState>>,
    Path((project_id, action_id)): Path<(String, String)>,
) -> Result<Json<CompleteFromHarnessResponse>, ApiError> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project = projects::get_project_by_id_or_slug(&pool, &project_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "unknown project".to_string()))?;
    let action = growth_store::get(&pool, &project.id, &action_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "unknown action".to_string()))?;

    if is_tracked(&action.status) {
        return Ok(Json(CompleteFromHarnessResponse {
            implemented: true,
            verified: true,
            identity: identity_of(&pool, &action).await,
            checks: Vec::new(),
            reason: Some("This action is already being measured.".into()),
        }));
    }

    let outcome = growth_verify::verify(&pool, &project, &action, None, false).await;
    if let Some(verified_by) = outcome.verified_by {
        let metric_dir = parse_target(
            action.target_metric.as_deref(),
            action.target_dir.as_deref(),
        )
        .ok()
        .flatten();
        if let Some((metric, dir)) = metric_dir {
            let verified_at = chrono::Utc::now();
            let existing_baseline = action
                .baseline_json
                .as_deref()
                .and_then(|raw| serde_json::from_str::<Baseline>(raw).ok());
            let encoded = if existing_baseline.is_some() {
                None
            } else {
                let fresh = sweep::snapshot_baseline(&pool, &project.id, metric, dir, verified_at)
                    .await
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
                Some(
                    serde_json::to_string(&fresh)
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?,
                )
            };
            let updated = growth_store::record_verification(
                &pool,
                &project.id,
                &action.id,
                verified_by,
                &verified_at.to_rfc3339(),
                encoded.as_deref(),
            )
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .ok_or((StatusCode::NOT_FOUND, "unknown action".to_string()))?;
            permagent::events::emit(permagent::events::project_changed(
                &project.id,
                "growth_actions",
            ));
            return Ok(Json(CompleteFromHarnessResponse {
                implemented: true,
                verified: true,
                identity: identity_of(&pool, &updated).await,
                checks: outcome.checks,
                reason: None,
            }));
        }
    }

    let has_target = action.target_metric.is_some() && action.target_dir.is_some();
    let updated = if has_target && action.status != growth_store::STATUS_DONE {
        growth_store::set_status(
            &pool,
            &project.id,
            &action.id,
            growth_store::STATUS_DONE,
            None,
        )
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
        .unwrap_or(action)
    } else {
        action
    };
    permagent::events::emit(permagent::events::project_changed(
        &project.id,
        "growth_actions",
    ));
    Ok(Json(CompleteFromHarnessResponse {
        implemented: has_target,
        verified: false,
        identity: identity_of(&pool, &updated).await,
        checks: outcome.checks,
        reason: Some(if has_target {
            "The coding agent finished. Nothing in git confirmed the change yet, so this is marked implemented — verify it when you can see the work, and measurement starts from there.".into()
        } else {
            "The coding agent finished, but this action has no pre-registered metric so it cannot be marked implemented.".into()
        }),
    }))
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
        .route(
            "/api/projects/{project_id}/growth-actions/{action_id}/status",
            post(set_action_status),
        )
        .route(
            "/api/projects/{project_id}/growth-actions/{action_id}/verify",
            post(verify_action),
        )
        .route(
            "/api/projects/{project_id}/growth-actions/{action_id}/complete-from-harness",
            post(complete_from_harness),
        )
        .route("/api/growth-actions/measure", post(measure_now))
        .route("/api/growth-results", axum::routing::get(get_results))
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

    fn action(impact: &str, confidence: &str, category: &str, title: &str) -> GrowthAction {
        GrowthAction {
            title: title.into(),
            evidence: "e".into(),
            recommendation: "r".into(),
            steps: Vec::new(),
            artifact_kind: "none".into(),
            artifact: None,
            category: category.into(),
            impact: impact.into(),
            confidence: confidence.into(),
            target_metric: None,
            target_dir: None,
            transfer: None,
            identity: None,
        }
    }

    /// A parsed action carrying whatever prediction the model made.
    fn targeted(title: &str, metric: Option<&str>, dir: Option<&str>) -> GrowthAction {
        GrowthAction {
            title: title.into(),
            evidence: "1200 pageviews".into(),
            recommendation: format!("recommendation for {title}"),
            steps: Vec::new(),
            artifact_kind: "none".into(),
            artifact: None,
            category: "seo".into(),
            impact: "high".into(),
            confidence: "medium".into(),
            target_metric: metric.map(str::to_string),
            target_dir: dir.map(str::to_string),
            transfer: None,
            identity: None,
        }
    }

    fn titles(actions: &[GrowthAction]) -> Vec<&str> {
        actions.iter().map(|a| a.title.as_str()).collect()
    }

    #[test]
    fn ranks_high_impact_first_then_confidence() {
        let ranked = rank_with_history(
            vec![
                action("low", "high", "ux", "third"),
                action("high", "low", "ux", "second"),
                action("high", "high", "ux", "first"),
            ],
            &HashMap::new(),
        );
        assert_eq!(titles(&ranked), vec!["first", "second", "third"]);
    }

    /// What worked here breaks ties between equally-rated actions.
    #[test]
    fn a_category_that_worked_here_wins_a_tie() {
        let history = HashMap::from([("seo".to_string(), 2), ("social".to_string(), -1)]);
        let ranked = rank_with_history(
            vec![
                action("high", "high", "social", "hindered-here"),
                action("high", "high", "ux", "never-tried"),
                action("high", "high", "seo", "helped-here"),
            ],
            &history,
        );
        assert_eq!(
            titles(&ranked),
            vec!["helped-here", "never-tried", "hindered-here"]
        );
    }

    /// The proposal's guard: "Never suppress a category outright on one bad
    /// outcome … rather than hiding advice the user can judge." A bad result
    /// must not push a high-impact action below a low-impact one.
    #[test]
    fn a_bad_outcome_downweights_a_category_it_does_not_bury_it() {
        let history = HashMap::from([("social".to_string(), -9)]);
        let ranked = rank_with_history(
            vec![
                action("low", "high", "ux", "low-impact"),
                action("high", "high", "social", "burned-but-strong"),
            ],
            &history,
        );
        assert_eq!(titles(&ranked), vec!["burned-but-strong", "low-impact"]);
    }

    /// Both halves of a pre-registration or neither: a metric with no direction
    /// cannot be scored and a direction with no metric has nothing to score.
    #[test]
    fn a_pre_registration_needs_a_metric_and_a_direction() {
        assert!(parse_target(None, None).unwrap().is_none());
        assert!(parse_target(Some("sessions"), Some("up"))
            .unwrap()
            .is_some());
        assert!(parse_target(Some("sessions"), None).is_err());
        assert!(parse_target(None, Some("up")).is_err());
        // An unmeasurable target is refused rather than stored, or
        // pre-registration is decorative.
        let err = parse_target(Some("device_signatures"), Some("up")).unwrap_err();
        assert!(err.contains("not a measurable target"), "{err}");
    }

    fn row(target: Option<(&str, &str)>, baseline: Option<&str>) -> growth_store::GrowthActionRow {
        growth_store::GrowthActionRow {
            id: "a1".into(),
            project_id: "p1".into(),
            fingerprint: "f".into(),
            title: "t".into(),
            recommendation: "r".into(),
            category: Some("seo".into()),
            artifact_kind: Some("prompt".into()),
            artifact: None,
            target_metric: target.map(|(m, _)| m.to_string()),
            target_dir: target.map(|(_, d)| d.to_string()),
            baseline_json: baseline.map(str::to_string),
            status: growth_store::STATUS_VERIFIED.into(),
            verified_by: Some(growth_store::VERIFIED_BY_GIT.into()),
            verified_at: Some("2026-08-11T14:00:00Z".into()),
            created_at: "2026-08-01T00:00:00Z".into(),
        }
    }

    /// REGRESSION. SYSTEM used to end the pre-registration paragraph with "If
    /// an action genuinely does not target any of these, omit both fields
    /// rather than guessing". The model took that escape hatch on every one of
    /// the seven live actions, including a homepage rewrite that obviously
    /// targets bounce rate, which left `target_metric` NULL and hard-blocked
    /// verification, measurement and the whole loop behind it.
    #[test]
    fn the_system_prompt_no_longer_offers_a_way_to_skip_the_target() {
        assert!(
            !SYSTEM.contains("omit both fields"),
            "the escape hatch is back"
        );
        assert!(SYSTEM.contains("EVERY action must carry both fields"));
        assert!(SYSTEM.contains("discarded and never reaches the user"));
    }

    /// REGRESSION. SYSTEM never mentioned that the project already had open
    /// actions, so on 2026-08-19 the generator proposed three that were rewords
    /// of three it had proposed on 2026-08-14. Telling it the rule is only half
    /// the fix — `render_brief` supplies the board itself — but without the
    /// rule the board reads as background rather than as a constraint.
    #[test]
    fn the_system_prompt_forbids_restating_an_open_action() {
        assert!(SYSTEM.contains("already on this project's board"));
        assert!(SYSTEM.contains("Do NOT restate"));
    }

    /// REGRESSION. `post` used to mean "bare copy, ready to publish". Copying
    /// that into a coding agent produced a blog post in chat, not a file in
    /// the repo. SEO/content artifacts have to name where the copy goes.
    #[test]
    fn post_artifacts_are_coding_agent_instructions() {
        assert!(SYSTEM.contains("artifactKind \"post\": still a coding-agent instruction"));
        assert!(SYSTEM.contains("where it belongs"));
    }

    #[test]
    fn growth_trend_pads_twelve_weeks_and_buckets_by_monday() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 8, 20).unwrap();
        let seeds = vec![
            TrendSeed {
                project_id: "p1".into(),
                project_name: "Alpha".into(),
                verdict: "helped".into(),
                judged_at: "2026-08-18T12:00:00Z".into(),
            },
            TrendSeed {
                project_id: "p1".into(),
                project_name: "Alpha".into(),
                verdict: "hindered".into(),
                judged_at: "2026-08-04T08:00:00Z".into(),
            },
            TrendSeed {
                project_id: "p2".into(),
                project_name: "Beta".into(),
                verdict: "helped".into(),
                judged_at: "2026-08-19T00:00:00Z".into(),
            },
            // Older than the 12-week window — must not appear.
            TrendSeed {
                project_id: "p3".into(),
                project_name: "Old".into(),
                verdict: "helped".into(),
                judged_at: "2026-04-01T00:00:00Z".into(),
            },
        ];
        let (fleet, by_project) = build_growth_trend(&seeds, today);
        assert_eq!(fleet.len(), 12);
        assert_eq!(fleet.last().unwrap().week, "2026-08-17");
        assert_eq!(fleet.last().unwrap().helped, 2);
        assert_eq!(fleet.last().unwrap().net, 2);
        let early = fleet.iter().find(|p| p.week == "2026-08-03").unwrap();
        assert_eq!(early.hindered, 1);
        assert_eq!(early.net, -1);
        assert!(
            fleet.iter().any(|p| p.cumulative_net != 0),
            "running net should move"
        );
        assert_eq!(by_project.len(), 2);
        assert!(by_project.iter().all(|p| p.project_name != "Old"));
        let names: Vec<_> = by_project.iter().map(|p| p.project_name.as_str()).collect();
        assert_eq!(names, ["Alpha", "Beta"]);
        assert_eq!(by_project[0].helped, 1);
        assert_eq!(by_project[0].hindered, 1);
        assert_eq!(by_project[1].helped, 1);
    }

    /// REGRESSION. Review was blind to the checkout: it saw analytics and the
    /// open board, never the files, so "Review again" reprinted FAQPage /
    /// instrumentation cards that were already in the tree. The Steward's
    /// dismiss pass is the mechanism; this rule is what stops the model
    /// proposing them again in the same turn.
    #[test]
    fn the_system_prompt_forbids_proposing_work_already_in_the_repo() {
        assert!(SYSTEM.contains("already in the tree"));
        assert!(SYSTEM.contains("Steward dismissed"));
        assert!(SYSTEM.contains("already shipped"));
    }

    #[test]
    fn an_action_with_no_target_is_dropped_not_persisted_untargeted() {
        let (kept, dropped) = split_targeted(vec![
            targeted("no prediction", None, None),
            targeted("real one", Some("bounce_rate"), Some("down")),
        ]);
        assert_eq!(titles(&kept), vec!["real one"]);
        assert_eq!(dropped.len(), 1);
        assert_eq!(dropped[0].0.title, "no prediction");
        assert_eq!(dropped[0].1, "named no target metric");
        // The dropped action never reaches the seed list: `persist` is only ever
        // handed the kept vec.
        assert!(!kept.iter().any(|a| a.title == "no prediction"));
    }

    /// Half a prediction is not a prediction: "sessions" with no direction
    /// scores as a success whichever way sessions move.
    #[test]
    fn a_metric_without_a_direction_is_not_half_kept() {
        let (kept, dropped) = split_targeted(vec![targeted("half", Some("sessions"), None)]);
        assert!(kept.is_empty());
        assert_eq!(dropped[0].1, "named sessions but no direction");

        let (kept, dropped) = split_targeted(vec![targeted("other half", None, Some("up"))]);
        assert!(kept.is_empty());
        assert_eq!(dropped[0].1, "named a direction but no metric");
    }

    #[test]
    fn the_retry_correction_names_every_offender_and_the_legal_values() {
        let (_, dropped) = split_targeted(vec![
            targeted("first offender", None, None),
            targeted("second offender", Some("sessions"), None),
        ]);
        let text = retry_correction(&dropped);
        assert!(text.contains("first offender"), "{text}");
        assert!(text.contains("second offender"), "{text}");
        assert!(text.contains("named no target metric"), "{text}");
        assert!(text.contains("named sessions but no direction"), "{text}");
        for metric in ["pageviews", "sessions", "aeo_visits", "bounce_rate"] {
            assert!(text.contains(metric), "{metric} missing from {text}");
        }
        assert!(text.contains("up"), "{text}");
        assert!(text.contains("down"), "{text}");
    }

    /// Discarding the first attempt because one sibling was malformed would
    /// lose advice the user could have used.
    #[test]
    fn merging_two_attempts_keeps_the_first_attempts_good_actions() {
        let first = vec![
            targeted("kept from the first pass", Some("sessions"), Some("up")),
            targeted("proposed twice", Some("pageviews"), Some("up")),
        ];
        let second = vec![
            targeted("proposed twice", Some("pageviews"), Some("up")),
            targeted("fixed on retry", Some("bounce_rate"), Some("down")),
        ];
        let merged = merge_attempts(first, second, "p1");
        assert_eq!(
            titles(&merged),
            vec![
                "kept from the first pass",
                "proposed twice",
                "fixed on retry"
            ]
        );

        // The panel stays actionable: five is the cap the parser already
        // enforces per reply, and two replies must not double it.
        let many: Vec<GrowthAction> = (0..4)
            .map(|i| targeted(&format!("a{i}"), Some("sessions"), Some("up")))
            .collect();
        let more: Vec<GrowthAction> = (0..4)
            .map(|i| targeted(&format!("b{i}"), Some("sessions"), Some("up")))
            .collect();
        assert_eq!(merge_attempts(many, more, "p1").len(), 5);
    }

    #[test]
    fn the_brief_carries_the_board_the_learning_and_the_pool() {
        let summary = busy();
        let brief = GenerationBrief {
            project_name: "GrocerySaver",
            summary: &summary,
            board: Some(
                "Already on this project's board (1). Do NOT restate any of these:\n- \"x\" (seo) \
                 — being measured\n"
                    .to_string(),
            ),
            learning: Some("Previously tried on this project (1 measured action):\n".to_string()),
            pooled: Some(
                "Across your other active projects, by category (one measured action per row, \
                 longest window):\n"
                    .to_string(),
            ),
            codebase: Some(
                "This project's git repo, as it is right now. Do NOT propose a change that is \
                 already in the tree.\nHEAD 8f2a1c33 \"Add FAQPage schema\"\n"
                    .to_string(),
            ),
        };
        let text = render_brief(&brief, None);
        let at = |needle: &str| {
            text.find(needle)
                .unwrap_or_else(|| panic!("{needle}\n{text}"))
        };
        assert!(at("Pageviews:") < at("already in the tree"));
        assert!(at("already in the tree") < at("Already on this project's board"));
        assert!(at("Already on this project's board") < at("Previously tried on this project"));
        assert!(at("Previously tried on this project") < at("Across your other active projects"));

        let corrected = render_brief(&brief, Some("Your previous reply was wrong."));
        assert!(
            corrected
                .trim_end()
                .ends_with("Your previous reply was wrong."),
            "{corrected}"
        );
    }

    /// MIGRATION regression. The bag changed shape from `{actions:[…]}` to
    /// `{prose:[…]}`. Without this upcast the seven live actions would silently
    /// lose their evidence, steps, impact and confidence the first time the
    /// panel was opened — and the rows themselves would look untouched, so
    /// nothing would appear to be wrong.
    #[test]
    fn the_old_metadata_shape_is_upcast_rather_than_discarded() {
        let legacy = serde_json::json!({
            "actions": [{
                "title": "Expand the grocery-stores post",
                "evidence": "8 of 37 pageviews land on it",
                "recommendation": "Expand and structure it",
                "steps": ["Add an FAQ section", "Add schema.org FAQPage"],
                "artifactKind": "prompt",
                "artifact": "In this repo, open …",
                "category": "aeo",
                "impact": "high",
                "confidence": "medium"
            }],
            "generatedAt": "2026-08-14T00:00:00Z",
            "reason": null,
            "periodDays": 30
        });
        let cache = ActionsCache::from_value("p1", &legacy);
        assert_eq!(cache.prose.len(), 1);
        assert_eq!(
            cache.prose[0].fingerprint,
            growth_store::fingerprint(
                "p1",
                "Expand the grocery-stores post",
                "Expand and structure it"
            ),
            "the prose must find its row again"
        );
        assert_eq!(cache.prose[0].evidence, "8 of 37 pageviews land on it");
        assert_eq!(cache.prose[0].steps.len(), 2);
        assert_eq!(cache.prose[0].impact, "high");
        assert_eq!(cache.prose[0].confidence, "medium");
        assert_eq!(cache.generated_at.as_deref(), Some("2026-08-14T00:00:00Z"));
        assert_eq!(cache.period_days, Some(30));

        // And the new shape is read directly, not run through the upcast.
        let fresh = serde_json::to_value(&cache).unwrap();
        assert_eq!(ActionsCache::from_value("p1", &fresh), cache);
    }

    #[test]
    fn archiving_something_that_never_happened_is_refused_with_a_reason() {
        let mut suggested = row(None, None);
        suggested.status = growth_store::STATUS_SUGGESTED.into();
        let err = reject_pointless_archive(&suggested, growth_store::STATUS_ARCHIVED).unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("Dismiss it instead"), "{}", err.1);

        // Every state where something actually happened may be filed away.
        for status in [
            growth_store::STATUS_DONE,
            growth_store::STATUS_VERIFIED,
            growth_store::STATUS_MEASURING,
            growth_store::STATUS_JUDGED,
            growth_store::STATUS_DISMISSED,
        ] {
            let mut action = row(None, None);
            action.status = status.into();
            assert!(
                reject_pointless_archive(&action, growth_store::STATUS_ARCHIVED).is_ok(),
                "{status} should be archivable"
            );
        }
        // And the gate touches nothing else.
        assert!(reject_pointless_archive(&suggested, growth_store::STATUS_DONE).is_ok());
    }

    /// REGRESSION for the "Re-check" button the panel now offers on a verified
    /// card.
    ///
    /// Before this, EVERY verify call took the recording path: it wrote
    /// `status = 'done'` with the supplied target, then `record_verification`
    /// stamped `verified_at = now` and `status = 'verified'`. Clicking Re-check
    /// on a measuring action would therefore have slid the measurement pivot
    /// forward — `verified_at` is what `metrics::pivot_date` reads — leaving
    /// the after-windows compared against a baseline frozen days earlier, and
    /// dragged a judged action back into `measuring`. Both are silent and both
    /// are data-visible, which is why the classification is a named function
    /// with a name rather than an inline `is_some()`.
    #[test]
    fn a_second_verify_reads_and_never_moves_the_pivot() {
        let verified = row(Some(("sessions", "up")), Some("{}"));
        assert_eq!(verify_mode(&verified), VerifyMode::Recheck);

        // A judged action is still a re-check: it has been verified, so its
        // pivot and its outcomes are settled.
        let mut judged = verified.clone();
        judged.status = growth_store::STATUS_JUDGED.into();
        assert_eq!(verify_mode(&judged), VerifyMode::Recheck);

        // The first verify is the only one that writes.
        let mut fresh = row(Some(("sessions", "up")), None);
        fresh.status = growth_store::STATUS_SUGGESTED.into();
        fresh.verified_by = None;
        fresh.verified_at = None;
        assert_eq!(verify_mode(&fresh), VerifyMode::Record);
    }

    /// Once the baseline is frozen the claim is fixed. Editing it afterwards
    /// would mean choosing the hypothesis with the result already in view.
    #[test]
    fn a_claim_cannot_be_rewritten_after_its_baseline_is_frozen() {
        let frozen = row(Some(("sessions", "up")), Some("{}"));
        let err =
            reject_late_reregistration(&frozen, Some((TargetMetric::Pageviews, TargetDir::Up)))
                .unwrap_err();
        assert_eq!(err.0, StatusCode::CONFLICT);
        assert!(err.1.contains("sessions"), "{}", err.1);

        // Re-sending the same claim is an idempotent retry, not a rewrite.
        assert!(
            reject_late_reregistration(&frozen, Some((TargetMetric::Sessions, TargetDir::Up)))
                .is_ok()
        );
        // And before a baseline exists, the claim is still being written.
        assert!(reject_late_reregistration(
            &row(Some(("sessions", "up")), None),
            Some((TargetMetric::Pageviews, TargetDir::Down))
        )
        .is_ok());
    }
    // ── The board, rendered from the durable rows ────────────────────────────

    mod board {
        use super::*;
        use permagent::projects::CreateProject;
        use permagent::session::spectral_schema::init_spectral_db;
        use sqlx::sqlite::SqlitePoolOptions;

        async fn pool() -> Pool<Sqlite> {
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .unwrap();
            init_spectral_db(&pool).await.unwrap();
            pool
        }

        async fn project(pool: &Pool<Sqlite>, name: &str) -> Project {
            permagent::projects::create_project(
                pool,
                CreateProject {
                    name: name.to_string(),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
        }

        fn prose_for(project_id: &str, action: &GrowthAction) -> CachedProse {
            CachedProse {
                fingerprint: growth_store::fingerprint(
                    project_id,
                    &action.title,
                    &action.recommendation,
                ),
                evidence: action.evidence.clone(),
                steps: action.steps.clone(),
                impact: action.impact.clone(),
                confidence: action.confidence.clone(),
            }
        }

        /// REGRESSION for the wholesale-overwritten bag. `hydrate` walked the
        /// NEW cache and attached identity to whatever it found there, so an
        /// action the latest review did not re-emit was simply absent from the
        /// panel — while the sweep went on writing outcomes for it. The card
        /// the user was waiting on disappeared and nothing said why.
        #[tokio::test]
        async fn a_measuring_action_survives_a_review_that_did_not_re_emit_it() {
            let pool = pool().await;
            let project = project(&pool, "GrocerySaver").await;
            let underway = targeted("Underway", Some("sessions"), Some("up"));
            let fresh = targeted("Freshly suggested", Some("pageviews"), Some("up"));
            persist(&pool, &project.id, &[underway.clone(), fresh.clone()]).await;

            let row = growth_store::get_by_fingerprint(
                &pool,
                &project.id,
                &growth_store::fingerprint(&project.id, &underway.title, &underway.recommendation),
            )
            .await
            .unwrap()
            .unwrap();
            growth_store::set_status(
                &pool,
                &project.id,
                &row.id,
                growth_store::STATUS_MEASURING,
                None,
            )
            .await
            .unwrap();

            // The new review only proposed the other action.
            let cache = ActionsCache {
                prose: vec![prose_for(&project.id, &fresh)],
                ..Default::default()
            };
            let board = render_board(&pool, &project.id, &cache).await;
            assert!(board.archived.is_empty());
            // In the Tracking list rather than the Actions list since the user
            // asked for that split on 2026-08-19 — but STILL IN THE PAYLOAD,
            // which is the whole of what this test has ever guarded. Hiding it
            // would be the defect; moving it somewhere the user can watch it is
            // what they asked for.
            let underway_card = board
                .tracking
                .iter()
                .find(|a| a.title == "Underway")
                .expect("an in-flight action must not vanish from the panel");
            assert_eq!(
                underway_card.identity.as_ref().unwrap().status,
                growth_store::STATUS_MEASURING
            );
            assert_eq!(underway_card.identity.as_ref().unwrap().id, row.id);
            // The row is the truth; absent prose renders as nothing, never as a
            // guess.
            assert_eq!(underway_card.evidence, "");
            assert_eq!(underway_card.impact, "");
            assert!(board.active.iter().any(|a| a.title == "Freshly suggested"));
        }

        /// The merge in `store`: a regenerate that no longer proposes an action
        /// must not delete the figure that action cited, because the action
        /// itself is still on the board.
        #[tokio::test]
        async fn a_regenerate_keeps_the_cached_evidence_of_an_action_it_did_not_re_emit() {
            let pool = pool().await;
            let project = project(&pool, "GrocerySaver").await;
            let kept = targeted("Still on the board", Some("sessions"), Some("up"));
            persist(&pool, &project.id, std::slice::from_ref(&kept)).await;

            let previous = ActionsCache {
                prose: vec![
                    prose_for(&project.id, &kept),
                    // Prose for advice no row has ever matched.
                    CachedProse {
                        fingerprint: "orphaned".into(),
                        evidence: "e".into(),
                        steps: Vec::new(),
                        impact: "high".into(),
                        confidence: "high".into(),
                    },
                ],
                ..Default::default()
            };
            store(&pool, &project, &previous).await.unwrap();
            let project = permagent::projects::get_project(&pool, &project.id)
                .await
                .unwrap()
                .unwrap();

            // The next review proposes something else entirely.
            let other = targeted("Something else", Some("pageviews"), Some("up"));
            persist(&pool, &project.id, std::slice::from_ref(&other)).await;
            let fresh = ActionsCache {
                prose: vec![prose_for(&project.id, &other)],
                ..Default::default()
            };
            store(&pool, &project, &fresh).await.unwrap();

            let project = permagent::projects::get_project(&pool, &project.id)
                .await
                .unwrap()
                .unwrap();
            let merged = cached(&project);
            let fingerprints: Vec<&str> = merged
                .prose
                .iter()
                .map(|p| p.fingerprint.as_str())
                .collect();
            assert!(fingerprints.contains(
                &growth_store::fingerprint(&project.id, &kept.title, &kept.recommendation).as_str()
            ));
            assert!(
                !fingerprints.contains(&"orphaned"),
                "prose matching no row is dropped"
            );
            assert_eq!(merged.prose.len(), 2);
        }

        /// The cap is on the bag, not on the rows: an old card renders without
        /// its evidence rather than disappearing.
        #[tokio::test]
        async fn the_prose_cache_is_capped() {
            let pool = pool().await;
            let project = project(&pool, "Long lived").await;
            let actions: Vec<GrowthAction> = (0..MAX_CACHED_PROSE + 1)
                .map(|i| targeted(&format!("action {i}"), Some("sessions"), Some("up")))
                .collect();
            persist(&pool, &project.id, &actions).await;
            let cache = ActionsCache {
                prose: actions.iter().map(|a| prose_for(&project.id, a)).collect(),
                ..Default::default()
            };
            store(&pool, &project, &cache).await.unwrap();
            let project = permagent::projects::get_project(&pool, &project.id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(cached(&project).prose.len(), MAX_CACHED_PROSE);
        }

        #[tokio::test]
        async fn an_archived_action_leaves_the_active_list_without_leaving_the_payload() {
            let pool = pool().await;
            let project = project(&pool, "GrocerySaver").await;
            let filed = targeted("Filed away", Some("sessions"), Some("up"));
            let live = targeted("Still live", Some("pageviews"), Some("up"));
            persist(&pool, &project.id, &[filed.clone(), live.clone()]).await;

            let row = growth_store::get_by_fingerprint(
                &pool,
                &project.id,
                &growth_store::fingerprint(&project.id, &filed.title, &filed.recommendation),
            )
            .await
            .unwrap()
            .unwrap();
            growth_store::record_verification(
                &pool,
                &project.id,
                &row.id,
                growth_store::VERIFIED_BY_GIT,
                "2026-08-12T00:00:00Z",
                None,
            )
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO growth_action_outcomes
                    (action_id, window_days, before_json, after_json, delta_pct, verdict,
                     rationale, confounders, judged_at)
                 VALUES (?1, 28, '{}', '{}', 0.2, 'helped', 'fixture', NULL,
                         '2026-09-10T00:00:00Z')",
            )
            .bind(&row.id)
            .execute(&pool)
            .await
            .unwrap();
            growth_store::set_status(
                &pool,
                &project.id,
                &row.id,
                growth_store::STATUS_ARCHIVED,
                None,
            )
            .await
            .unwrap();

            let board = render_board(&pool, &project.id, &ActionsCache::default()).await;
            assert_eq!(titles(&board.active), vec!["Still live"]);
            assert_eq!(titles(&board.archived), vec!["Filed away"]);
            let identity = board.archived[0].identity.as_ref().unwrap();
            assert_eq!(identity.status, growth_store::STATUS_ARCHIVED);
            assert_eq!(identity.outcomes.len(), 1, "the outcomes travel with it");
            assert_eq!(identity.outcomes[0].verdict, "helped");
        }

        /// In-flight work stays visible rather than being pushed off by new
        /// suggestions, and a card with no cache entry sorts last within its
        /// bucket instead of vanishing. Since 2026-08-19 "visible" means the
        /// Tracking list rather than below the decisions — moved, never hidden.
        #[tokio::test]
        async fn the_active_list_keeps_in_flight_work_below_new_suggestions() {
            let pool = pool().await;
            let project = project(&pool, "GrocerySaver").await;
            let seeds = [
                ("dismissed", growth_store::STATUS_DISMISSED),
                ("judged", growth_store::STATUS_JUDGED),
                ("measuring", growth_store::STATUS_MEASURING),
                ("second suggestion", growth_store::STATUS_SUGGESTED),
                ("first suggestion", growth_store::STATUS_SUGGESTED),
            ];
            let mut actions = Vec::new();
            for (title, status) in seeds {
                let action = targeted(title, Some("sessions"), Some("up"));
                persist(&pool, &project.id, std::slice::from_ref(&action)).await;
                let row = growth_store::get_by_fingerprint(
                    &pool,
                    &project.id,
                    &growth_store::fingerprint(&project.id, &action.title, &action.recommendation),
                )
                .await
                .unwrap()
                .unwrap();
                growth_store::set_status(&pool, &project.id, &row.id, status, None)
                    .await
                    .unwrap();
                actions.push(action);
            }

            // The last review ranked the two suggestions in this order and knew
            // nothing about the rest.
            let cache = ActionsCache {
                prose: vec![
                    prose_for(&project.id, &actions[4]),
                    prose_for(&project.id, &actions[3]),
                ],
                ..Default::default()
            };
            let board = render_board(&pool, &project.id, &cache).await;
            assert_eq!(
                titles(&board.active),
                vec!["first suggestion", "second suggestion"]
            );
            // In-flight work is not pushed off by new suggestions; since
            // 2026-08-19 it is not competing with them for the same list
            // either. The bucket order is what carries over: still being
            // measured above already judged.
            assert_eq!(titles(&board.tracking), vec!["measuring", "judged"]);
            // Dismissed advice leaves the active list entirely. It used to sort
            // to the bottom of it and stay there for good: `suggested` cannot be
            // archived, so with dismissal not removing anything either, no
            // control the user could press ever shortened the panel.
            assert_eq!(titles(&board.dismissed), vec!["dismissed"]);
        }

        /// REGRESSION for the user report of 2026-08-19: "some of the actions I
        /// am seeing in the Grow tab are stale ones that I already ran. I should
        /// be able to dismiss it."
        ///
        /// The four actions this user's Evntally board has carried since
        /// 2026-08-14 have no entry left in
        /// `metadata_json.growth_actions.prose` — the bag holds only the four
        /// fingerprints the 2026-08-19 reviews wrote — and they predate the
        /// mandatory-target change, so `target_metric` is NULL. Every control
        /// the panel offers hangs off `identity`, so if a card could reach the
        /// screen without one it would render with no way to act on it at all.
        /// The rows are the list, so the identity comes from the row.
        #[tokio::test]
        async fn a_card_the_prose_cache_forgot_still_carries_its_durable_row() {
            let pool = pool().await;
            let project = project(&pool, "Evntally").await;
            let legacy = action("high", "high", "measurement", "Instrument funnel events");
            persist(&pool, &project.id, std::slice::from_ref(&legacy)).await;

            // The bag remembers a LATER review only — exactly the live shape.
            let later = targeted("Rewrite the homepage", Some("bounce_rate"), Some("down"));
            persist(&pool, &project.id, std::slice::from_ref(&later)).await;
            let cache = ActionsCache {
                prose: vec![prose_for(&project.id, &later)],
                ..Default::default()
            };

            let board = render_board(&pool, &project.id, &cache).await;
            let stale = board
                .active
                .iter()
                .find(|a| a.title == "Instrument funnel events")
                .expect("a row the cache forgot must still reach the panel");
            let identity = stale
                .identity
                .as_ref()
                .expect("no identity means no id to post a dismissal with");
            assert_eq!(identity.status, growth_store::STATUS_SUGGESTED);
            assert!(!identity.id.is_empty());
            // Untargeted, like every row written before the target was
            // mandatory — and still dismissible, because dismissal needs the
            // id and nothing else.
            assert_eq!(identity.target_metric, None);
            // The row is the truth; absent prose renders as nothing.
            assert_eq!(stale.evidence, "");
        }

        /// The user, 2026-08-19: "once a verified action was taken it should
        /// disappear from the list of Actions and go into a tracker view of
        /// changes we made that we are tracking".
        ///
        /// #1053 put `verified`/`measuring` on the active board on purpose, so
        /// in-flight work could not silently vanish while the sweep still
        /// measured it. That guarantee is what this asserts is KEPT: the rows
        /// move to a list of their own, and they are still in the payload with
        /// their outcomes.
        #[tokio::test]
        async fn measured_work_leaves_the_actions_list_without_leaving_the_payload() {
            let pool = pool().await;
            let project = project(&pool, "GetLadle").await;
            let seeds = [
                ("still deciding", growth_store::STATUS_SUGGESTED),
                ("marked done", growth_store::STATUS_DONE),
                ("verified", growth_store::STATUS_VERIFIED),
                ("measuring", growth_store::STATUS_MEASURING),
                ("judged", growth_store::STATUS_JUDGED),
            ];
            for (title, status) in seeds {
                let action = targeted(title, Some("sessions"), Some("up"));
                persist(&pool, &project.id, std::slice::from_ref(&action)).await;
                let row = growth_store::get_by_fingerprint(
                    &pool,
                    &project.id,
                    &growth_store::fingerprint(&project.id, &action.title, &action.recommendation),
                )
                .await
                .unwrap()
                .unwrap();
                growth_store::set_status(&pool, &project.id, &row.id, status, None)
                    .await
                    .unwrap();
            }

            let board = render_board(&pool, &project.id, &ActionsCache::default()).await;
            let mut active = titles(&board.active);
            active.sort_unstable();
            assert_eq!(
                active,
                vec!["marked done", "still deciding"],
                "Actions holds only work still asking for a decision"
            );
            let mut tracked = titles(&board.tracking);
            tracked.sort_unstable();
            assert_eq!(tracked, vec!["judged", "measuring", "verified"]);
        }

        /// The Tracking card has to show what the verdict is computed against.
        /// The baseline was on the row and never on the wire, so the only place
        /// it could be read was the verify reply — which is gone after a
        /// reload, which is the same defect the Re-check button exists for.
        #[tokio::test]
        async fn a_tracked_card_carries_the_frozen_baseline_it_is_measured_against() {
            let pool = pool().await;
            let project = project(&pool, "GetLadle").await;
            let shipped = targeted("Rewrite the homepage", Some("bounce_rate"), Some("down"));
            persist(&pool, &project.id, std::slice::from_ref(&shipped)).await;
            let row = growth_store::get_by_fingerprint(
                &pool,
                &project.id,
                &growth_store::fingerprint(&project.id, &shipped.title, &shipped.recommendation),
            )
            .await
            .unwrap()
            .unwrap();
            let frozen = serde_json::json!({
                "metric": "bounce_rate",
                "dir": "down",
                "pivot": "2026-08-20",
                "takenAt": "2026-08-19T12:00:00Z",
                "before": {
                    "7": {
                        "start": "2026-08-12", "end": "2026-08-19", "days": 7,
                        "metric": "bounce_rate", "value": 0.47, "denominator": 220.0,
                        "pageviews": 310, "sessions": 220
                    }
                },
                "weekly": [1.0, 2.0],
                "earliestEvent": "2026-05-01"
            });
            growth_store::record_verification(
                &pool,
                &project.id,
                &row.id,
                growth_store::VERIFIED_BY_GIT,
                "2026-08-19T12:00:00Z",
                Some(&frozen.to_string()),
            )
            .await
            .unwrap();

            let board = render_board(&pool, &project.id, &ActionsCache::default()).await;
            let identity = board.tracking[0].identity.as_ref().unwrap();
            let baseline = identity
                .baseline
                .as_ref()
                .expect("a verdict whose before is invisible cannot be argued with");
            assert_eq!(baseline.pivot, "2026-08-20");
            assert_eq!(baseline.windows.len(), 1);
            assert_eq!(baseline.windows[0].window_days, 7);
            assert_eq!(baseline.windows[0].value, 0.47);
            assert_eq!(baseline.windows[0].denominator, 220.0);
        }

        /// A baseline that no longer parses is absent, never zero. Zero would
        /// render as "there was no traffic before the change", which is a claim
        /// nothing here has any basis for.
        #[test]
        fn an_unreadable_baseline_is_absent_rather_than_empty() {
            assert!(baseline_view(None).is_none());
            assert!(baseline_view(Some("not json")).is_none());
            assert!(baseline_view(Some("{}")).is_none());
        }

        /// Write a judged outcome for an action on another project, so the
        /// pooled path has something real to find.
        async fn measured(
            pool: &Pool<Sqlite>,
            project_id: &str,
            title: &str,
            category: &str,
            verdict: &str,
            delta: f64,
        ) {
            let row = growth_store::upsert_suggested(
                pool,
                project_id,
                &ActionSeed {
                    title: title.into(),
                    recommendation: format!("recommendation for {title}"),
                    category: Some(category.into()),
                    artifact_kind: Some("prompt".into()),
                    artifact: None,
                    target_metric: Some("sessions".into()),
                    target_dir: Some("up".into()),
                },
            )
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO growth_action_outcomes
                    (action_id, window_days, before_json, after_json, delta_pct, verdict,
                     rationale, confounders, judged_at)
                 VALUES (?1, 28, '{}', '{}', ?2, ?3, 'fixture', NULL, '2026-08-18T00:00:00Z')",
            )
            .bind(&row.id)
            .bind(delta)
            .bind(verdict)
            .execute(pool)
            .await
            .unwrap();
        }

        /// REGRESSION for the restatement guard at the point it actually drops
        /// something. `restates` was unit-tested pure, but nothing exercised
        /// `persist`, which is what refuses the row and increments the counter
        /// the panel prints. Every existing `persist` test used titles far
        /// enough apart to fall under the token floors, so the whole
        /// `if !already_here { ... restates ... continue }` block could be
        /// deleted with the suite still green.
        #[tokio::test]
        async fn a_reworded_suggestion_is_dropped_rather_than_minted_as_a_second_card() {
            let pool = pool().await;
            let project = project(&pool, "GrocerySaver").await;
            let first = GrowthAction {
                title: "Instrument missing conversion funnel events to understand why 95% bounce"
                    .into(),
                recommendation: "No conversion events are recorded, so the funnel cannot be \
                                 diagnosed. Add search and detail-view events."
                    .into(),
                ..targeted("ignored", Some("sessions"), Some("up"))
            };
            let outcome = persist(&pool, &project.id, std::slice::from_ref(&first)).await;
            assert_eq!(outcome.rows.len(), 1);
            assert_eq!(outcome.restated, 0);

            // The 2026-08-19 reword: different words, different fingerprint,
            // the same advice.
            let reword = GrowthAction {
                title: "Instrument missing conversion funnel events to diagnose the 99% bounce"
                    .into(),
                recommendation: "Conversion events are still not recorded. Instrument search, \
                                 category and detail-view events."
                    .into(),
                ..targeted("ignored", Some("sessions"), Some("up"))
            };
            let outcome = persist(&pool, &project.id, std::slice::from_ref(&reword)).await;
            assert!(outcome.rows.is_empty(), "the reword must not be persisted");
            assert_eq!(outcome.restated, 1, "and the drop must be counted");
            assert_eq!(
                growth_store::list_for_project(&pool, &project.id)
                    .await
                    .unwrap()
                    .len(),
                1,
                "one piece of advice is one row"
            );
        }

        /// The other half of the same guard, and the one `persist`'s comment
        /// claims but nothing checked: the board is grown as the loop runs, so
        /// two actions restating EACH OTHER inside a single review are caught
        /// too. Without the `board.push` after each upsert this passes twice.
        #[tokio::test]
        async fn two_suggestions_in_one_review_that_restate_each_other_yield_one_card() {
            let pool = pool().await;
            let project = project(&pool, "GrocerySaver").await;
            let batch = vec![
                GrowthAction {
                    title: "Instrument missing conversion funnel events to understand the bounce"
                        .into(),
                    recommendation: "Add search and detail-view conversion events.".into(),
                    ..targeted("ignored", Some("sessions"), Some("up"))
                },
                GrowthAction {
                    title: "Instrument missing conversion funnel events to diagnose the bounce"
                        .into(),
                    recommendation: "Add search and detail-view conversion events now.".into(),
                    ..targeted("ignored", Some("sessions"), Some("up"))
                },
            ];
            let outcome = persist(&pool, &project.id, &batch).await;
            assert_eq!(outcome.rows.len(), 1);
            assert_eq!(outcome.restated, 1);
        }

        /// REGRESSION. The only test of the transfer note asserted it was
        /// ABSENT, against a database with no outcomes at all — so stubbing
        /// `transfer_notes` to an empty map, or deleting the `transfer:` field
        /// assignment outright, left it green. Nothing anywhere proved the
        /// `pooled::CategoryPool` -> `TransferNote` -> payload join carried a
        /// single value. This is that proof.
        #[tokio::test]
        async fn a_transfer_note_names_the_project_the_category_worked_on() {
            let pool = pool().await;
            let target = project(&pool, "GrocerySaver").await;
            let elsewhere = project(&pool, "EventFinder").await;
            measured(
                &pool,
                &elsewhere.id,
                "Add FAQPage schema to the venue pages",
                "aeo",
                "helped",
                0.42,
            )
            .await;

            let action = GrowthAction {
                category: "aeo".into(),
                ..targeted("Add FAQ schema", Some("sessions"), Some("up"))
            };
            persist(&pool, &target.id, std::slice::from_ref(&action)).await;

            let rendered = render_board(&pool, &target.id, &ActionsCache::default())
                .await
                .active;
            let note = rendered[0]
                .transfer
                .as_ref()
                .expect("a measured aeo result on another project must reach the card");
            assert_eq!(note.category, "aeo");
            assert_eq!(note.helped, 1);
            assert!(!note.segment_label.is_empty());
            let example = note
                .examples
                .first()
                .expect("the note has to say where the result came from");
            assert_eq!(example.project_name, "EventFinder");
            assert_eq!(example.verdict, "helped");
        }

        /// A badge that says nothing is worse than no badge, and until the first
        /// window closes anywhere there is nothing any badge could say. The
        /// `count(*)` pre-check in `transfer_notes` is what keeps this case from
        /// segmenting every active project to discover that.
        #[tokio::test]
        async fn a_transfer_note_is_absent_when_no_other_project_measured_that_category() {
            let pool = pool().await;
            let project = project(&pool, "GrocerySaver").await;
            let action = targeted("Add FAQ schema", Some("sessions"), Some("up"));
            persist(&pool, &project.id, std::slice::from_ref(&action)).await;

            let rendered = render_board(&pool, &project.id, &ActionsCache::default())
                .await
                .active;
            assert_eq!(rendered.len(), 1);
            assert!(rendered[0].transfer.is_none());
        }
    }

    // ── A review that outlives the request that asked for it ──────────────────

    mod reviews {
        use super::*;

        /// REGRESSION for the user report of 2026-08-19: "I pressed Review my
        /// analytics and then clicked another tab, when I went back it looks
        /// like it stopped running."
        ///
        /// The in-flight flag was a `useState` in the panel component, which the
        /// tab switch unmounted. It lives on the daemon now, so any client on
        /// any mount can ask what is actually running.
        #[test]
        fn a_running_review_is_reported_to_anyone_who_asks() {
            let id = "project-reported";
            assert_eq!(review_started_at(id), None);
            let slot = begin_review(id, "2026-08-19T13:00:00Z".to_string()).unwrap();
            assert_eq!(
                review_started_at(id).as_deref(),
                Some("2026-08-19T13:00:00Z")
            );
            drop(slot);
            assert_eq!(review_started_at(id), None);
        }

        /// Two clicks must not start two reviews. The disabled button is the
        /// courtesy; this is the rule — two reviews racing on one project write
        /// two caches over each other.
        #[test]
        fn a_second_review_cannot_start_while_one_is_running() {
            let id = "project-double-click";
            let held = begin_review(id, "2026-08-19T13:00:00Z".to_string()).unwrap();
            assert!(
                begin_review(id, "2026-08-19T13:00:01Z".to_string()).is_none(),
                "a second review must not start"
            );
            // Another project is unaffected: the slot is per project, not global.
            let other = begin_review("project-elsewhere", "2026-08-19T13:00:02Z".to_string());
            assert!(other.is_some());
            drop(held);
            drop(other);
            assert!(begin_review(id, "2026-08-19T13:00:03Z".to_string()).is_some());
        }

        /// A guard rather than a `remove` at the end of the task, so a panic
        /// inside a review releases the slot too. Without this one failed review
        /// leaves the button spinning for the life of the daemon.
        #[test]
        fn a_panicking_review_still_releases_its_slot() {
            let id = "project-panics";
            let caught = std::panic::catch_unwind(|| {
                let _slot = begin_review(id, "2026-08-19T13:00:00Z".to_string()).unwrap();
                panic!("the review blew up");
            });
            assert!(caught.is_err());
            assert_eq!(
                review_started_at(id),
                None,
                "a review that panicked must not leave the panel spinning forever"
            );
        }
    }
}
