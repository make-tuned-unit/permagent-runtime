//! The Forecaster — where the market around each project is going.
//!
//! ## What it looks at, and why that is the outward half
//!
//! Permagent's *internal* series are short: goal velocity is ten spiky points,
//! spend is twenty-two days. No model recovers signal from that, and the
//! honest answer there is a refusal. So the Forecaster looks outward instead —
//! at npm and crates.io downloads, Wikipedia pageviews, Hacker News mentions
//! for the competitors and adjacent projects a human already approved onto a
//! project's Ecosystem panel. Those series are long, public, and free to
//! backfill, which is the only reason any of this can say anything at all.
//!
//! ## The boundary that matters
//!
//! Binding a series **proposes** it; a human approves it, through the same
//! review gate `propose_project_intel` already uses. The model never promotes
//! its own series, and there is no second list of subjects it could invent one
//! from — every series hangs off a `project_intel` row or off the project.
//!
//! ## Honesty
//!
//! `forecaster_series` is the honesty surface: it reports the real point count,
//! the real span, and a verdict per series. A series that is too short says how
//! short. A collector that has stopped says so rather than letting stale
//! numbers read as a flat market. A source that cannot hand over history —
//! GitHub, since stargazer timestamps were restricted in June 2026 — is
//! labelled snapshot-only rather than presented as a trend.

use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::agents::tool_execution::ToolCallContext;
use crate::forecaster::forecast;
use crate::forecaster::store::{self, Verdict};
use crate::forecaster::{brief, remote};
use crate::forecaster::{Cadence, Knobs, SourceKind};
use anyhow::Result;
use async_trait::async_trait;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, JsonObject, ListToolsResult,
    ServerCapabilities, Tool,
};
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

/// The extension's registry key. Deliberately NOT `forecaster`: the platform
/// and worker self-knowledge registries share an id namespace, and
/// `find_descriptor` resolves worker > surface > guard > platform — so a
/// collision would silently shadow one descriptor and serve the wrong lesson.
/// `finance` / `financier` splits the same way for the same reason: the
/// extension is the module, the id below is the character.
pub static EXTENSION_NAME: &str = "forecast";

/// The roster id the world view and the activity journal key on. Distinct from
/// `EXTENSION_NAME` only in that it names the *character*, as `financier` does
/// for the `finance` extension.
pub const AGENT_ID: &str = "forecaster";
pub const AGENT_NAME: &str = "The Forecaster";

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct NoParams {}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct SeriesParams {
    /// Project id, slug or name. Omit to list every bound series.
    project: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ForecastParams {
    /// The series id, from forecaster_series.
    series_id: String,
    /// Steps ahead. Omit for one week at the series' own cadence.
    horizon: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct BriefParams {
    /// Project id, slug or name.
    project: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct BindParams {
    /// Project id, slug or name.
    project: String,
    /// One of: npm, crates, pypi, wiki_pageviews, hn_mentions, arxiv_count,
    /// stackexchange_tag, github_repo, equity_close.
    source_kind: String,
    /// The package, article, query, tag, repo or ticker to watch.
    subject: String,
    /// The `project_intel` row this series is about, if it is about one.
    intel_id: Option<String>,
    /// `daily` or `weekly`. Omit to use the source's own resolution.
    cadence: Option<String>,
}

fn schema<T: JsonSchema>() -> JsonObject {
    let mut obj = serde_json::to_value(schema_for!(T))
        .map(|v| v.as_object().unwrap().clone())
        .expect("valid schema");
    obj.entry("properties")
        .or_insert_with(|| serde_json::json!({}));
    obj
}

pub struct ForecasterClient {
    info: InitializeResult,
    context: PlatformExtensionContext,
}

fn announce(state: &str) {
    crate::events::emit(crate::events::agent_state_changed(
        AGENT_ID, AGENT_NAME, state,
    ));
}

/// One line per series, in the shape the card and the model both need: what it
/// is, how much of it there is, and whether that is enough.
pub fn describe_verdict(s: &store::SeriesSummary) -> String {
    let head = format!(
        "{} · {} ({})",
        s.subject,
        s.source_label,
        s.cadence.as_str()
    );
    let body = match &s.verdict {
        Verdict::Forecastable => format!(
            "{} points over {} days — forecastable",
            s.points, s.span_days
        ),
        Verdict::InsufficientHistory { points, needed } => format!(
            "{points} of {needed} points — too short to forecast; the baseline is all this can say"
        ),
        Verdict::CollectorStale { last_collected_at } => match last_collected_at {
            Some(t) => format!("collector stale — last ran {t}"),
            None => "never collected".to_string(),
        },
        Verdict::NotBound => format!("{} — awaiting approval", s.status.as_str()),
    };
    let mut line = format!("- {head}: {body}");
    if s.snapshot_only {
        line.push_str(" [snapshot-only source: no past to backfill, one point per sweep]");
    }
    if !s.official_source {
        line.push_str(" [unofficial endpoint]");
    }
    if let Some(err) = &s.last_error {
        line.push_str(&format!(" [last error: {err}]"));
    }
    line
}

/// A forecast as prose, with its method attached to it rather than mentioned
/// somewhere nearby. The two travel together or the number is unreadable.
pub fn describe_forecast(f: &forecast::Forecast) -> String {
    let last = f.point.last().copied().unwrap_or(f64::NAN);
    let lo = f.p10.last().copied().unwrap_or(f64::NAN);
    let hi = f.p90.last().copied().unwrap_or(f64::NAN);
    let mut out = format!(
        "{} steps ahead: {last:.0} (80% range {lo:.0} to {hi:.0}).\nMethod: {} — {}",
        f.horizon,
        f.method.as_str(),
        f.method_label,
    );
    if let Some(mase) = f.mase_vs_baseline {
        out.push_str(&format!(
            "\nBacktest: MASE {mase:.3} vs seasonal naive over {} folds, winning {}.",
            f.folds, f.fold_wins
        ));
    }
    out.push_str(&format!("\nWhy this method: {}", f.selection));
    out.push_str(
        "\nThese are other people's public numbers. They say where a category is heading, not \
         why it moved and not what to do about it.",
    );
    out
}

impl ForecasterClient {
    pub fn new(context: PlatformExtensionContext) -> Result<Self> {
        let info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(EXTENSION_NAME, "1.0.0").with_title(AGENT_NAME))
            .with_instructions(
                "Where the market around each project is going.\n\n\
                 forecaster_series is the honesty surface and the tool to call first: \
                 it lists every bound series with its real point count, its span, and a \
                 verdict — forecastable, too short (and by how much), collector stale, or \
                 awaiting approval. Never describe a market direction for a series this \
                 tool says is too short or stale.\n\n\
                 forecaster_bind PROPOSES a new series against a project — an npm or \
                 crates.io package, a Wikipedia article, a Hacker News query — ideally \
                 attached to a competitor row a human already approved on the Ecosystem \
                 panel. It never activates the series itself; approval is the user's, \
                 through the same review gate as project intelligence. Say that plainly \
                 when you bind one.\n\n\
                 These series are OTHER people's numbers: downloads, pageviews, mentions. \
                 They say where a category is heading. They say nothing about whether a \
                 project will ship, whether a goal will succeed, or why anything moved — \
                 do not offer a cause and do not offer a recommendation.",
            );
        Ok(Self { info, context })
    }

    async fn pool(&self) -> std::result::Result<sqlx::Pool<sqlx::Sqlite>, String> {
        self.context
            .session_manager
            .pool_clone()
            .await
            .map_err(|e| e.to_string())
    }

    async fn resolve_project(
        &self,
        pool: &sqlx::Pool<sqlx::Sqlite>,
        raw: &str,
    ) -> std::result::Result<crate::projects::Project, String> {
        crate::projects::get_project_by_id_or_slug(pool, raw)
            .await?
            .ok_or_else(|| format!("no project matches \"{raw}\""))
    }

    async fn handle_series(
        &self,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        let params: SeriesParams =
            serde_json::from_value(serde_json::Value::Object(arguments.unwrap_or_default()))
                .map_err(|e| e.to_string())?;
        let pool = self.pool().await?;
        let project_id = match params.project.as_deref() {
            Some(raw) if !raw.trim().is_empty() => Some(self.resolve_project(&pool, raw).await?.id),
            _ => None,
        };
        let rows = store::summarize(&pool, project_id.as_deref(), chrono::Utc::now()).await?;
        if rows.is_empty() {
            // The nonprofit case, and the day-one case. Distinguish "nothing is
            // bound" from "the market is flat" — they are not the same answer.
            return Ok(CallToolResult::success(vec![Content::text(
                "No market series are bound yet. Nothing here is a forecast of zero — there is \
                 simply nothing to forecast. Bind a competitor's package, a category's \
                 Wikipedia article, or a Hacker News query with forecaster_bind; a human \
                 approves it before collection starts."
                    .to_string(),
            )]));
        }
        let mut out = String::new();
        for row in &rows {
            out.push_str(&describe_verdict(row));
            out.push('\n');
        }
        let forecastable = rows
            .iter()
            .filter(|r| r.verdict == Verdict::Forecastable)
            .count();
        out.push_str(&format!(
            "\n{} of {} series are long enough and fresh enough to forecast.",
            forecastable,
            rows.len()
        ));
        Ok(CallToolResult::success(vec![Content::text(out)]))
    }

    async fn handle_bind(
        &self,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        let params: BindParams =
            serde_json::from_value(serde_json::Value::Object(arguments.unwrap_or_default()))
                .map_err(|e| e.to_string())?;
        // Parse before anything else: an unknown source must be refused here,
        // where the error can name the closed set, and never become a URL.
        let kind = SourceKind::parse(&params.source_kind)?;
        let cadence = params.cadence.as_deref().map(Cadence::parse).transpose()?;
        let pool = self.pool().await?;
        let project = self.resolve_project(&pool, &params.project).await?;
        let knobs = Knobs::load();
        let series = store::bind(
            &pool,
            &knobs,
            store::BindRequest::new(&project.id, kind, &params.subject)
                .intel(params.intel_id.as_deref())
                .cadence(cadence),
        )
        .await?;

        let backfill = if kind.backfills() {
            "This source backfills, so approving it collects its history in one pass."
        } else {
            "This source is snapshot-only — GitHub restricted stargazer timestamps in June 2026 — \
             so it accumulates one point per sweep and will not be forecastable for months."
        };
        let normalized = if series.subject != params.subject.trim() {
            format!(
                " The subject was normalized to \"{}\" for this source.",
                series.subject
            )
        } else {
            String::new()
        };
        let status = match series.status {
            crate::forecaster::SeriesStatus::Active => {
                "It is active — it hung off a competitor row you had already approved."
            }
            _ => "It is PROPOSED, not active: nothing is collected until you approve it.",
        };
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Bound {} · {} to {} (series {}).{normalized}\n{status}\n{backfill}",
            series.subject,
            kind.label(),
            project.name,
            series.id,
        ))]))
    }

    /// A forecast, or the reason there isn't one. Never both, never neither.
    async fn handle_forecast(
        &self,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        let params: ForecastParams =
            serde_json::from_value(serde_json::Value::Object(arguments.unwrap_or_default()))
                .map_err(|e| e.to_string())?;
        let pool = self.pool().await?;
        let horizon = params.horizon.map(|h| h as usize);
        match forecast::forecast_series(&pool, &params.series_id, horizon, chrono::Utc::now()).await
        {
            Ok(f) => {
                // Persisted so the Market card and the brief read the same
                // numbers this answer just gave.
                if let Err(e) = forecast::record(&pool, &f).await {
                    tracing::warn!(target: "permagent::forecaster", "could not record forecast: {e}");
                }
                Ok(CallToolResult::success(vec![Content::text(
                    describe_forecast(&f),
                )]))
            }
            // A refusal is a successful answer to the question asked. It is not
            // an error, and rendering it as one would push the model toward
            // retrying rather than reporting.
            Err(refusal) => Ok(CallToolResult::success(vec![Content::text(format!(
                "No forecast for {}: {refusal}. Do not describe a direction for this series.",
                params.series_id
            ))])),
        }
    }

    /// Is the TimesFM host actually able to forecast right now?
    ///
    /// A degraded model host is a surface, not a log line: a week of
    /// baseline-only forecasts must be distinguishable from a week in which the
    /// model agreed with the baseline.
    async fn handle_health(&self) -> std::result::Result<CallToolResult, String> {
        let cfg = remote::RemoteConfig::load();
        let h = remote::health(&cfg).await;
        let body = if h.ready() {
            format!(
                "The TimesFM host ({}) is ready: venv and script present, weights {}.\n\
                 Forecasts may be served by the model where it clears the backtest gate.",
                h.target,
                if h.weights_present {
                    "cached"
                } else {
                    "not yet downloaded"
                }
            )
        } else {
            format!(
                "The TimesFM host ({}) is NOT serving: {}.\n\
                 Forecasts fall back to the Rust baseline and are labelled as the baseline. \
                 This is a degraded state, not an error — say so rather than presenting the \
                 week's baselines as model agreement. Nothing runs on the local machine \
                 instead.",
                h.target, h.detail
            )
        };
        Ok(CallToolResult::success(vec![Content::text(body)]))
    }

    /// The weekly synthesis, on demand.
    async fn handle_brief(
        &self,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        let params: BriefParams =
            serde_json::from_value(serde_json::Value::Object(arguments.unwrap_or_default()))
                .map_err(|e| e.to_string())?;
        let pool = self.pool().await?;
        let project = self.resolve_project(&pool, &params.project).await?;
        let now = chrono::Utc::now();
        let summaries = store::summarize(&pool, Some(&project.id), now).await?;
        if summaries.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(format!(
                "No market series are bound for {}, so there is no market brief. That is not a \
                 flat market — it is an unwatched one.",
                project.name
            ))]));
        }
        let mut rows = Vec::with_capacity(summaries.len());
        for summary in summaries {
            let f = forecast::forecast_series(&pool, &summary.series_id, None, now)
                .await
                .ok();
            rows.push((summary, f));
        }
        let (input, grounded) = brief::compose(&project.name, &rows);
        let mix = brief::method_mix(&rows);

        // The table is the answer; the prose is a convenience over it. So the
        // table is returned whether or not a model was reachable.
        let mut body = format!("Market brief for {}\n\n{input}\n", project.name);
        body.push_str(&format!(
            "Methods that produced these numbers: {}\n",
            mix.iter()
                .map(|(k, v)| format!("{k} x{v}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        match self.synthesize(&input, &grounded).await {
            Some((prose, engine)) => {
                body.push_str(&format!("\nSummary ({engine}): {prose}"));
            }
            None => body.push_str(
                "\nNo prose summary this time — read the table above. A summary that could not \
                 be generated is not a summary of nothing.",
            ),
        }
        Ok(CallToolResult::success(vec![Content::text(body)]))
    }

    /// Best-fit, cost-conscious: the on-device model when the input fits its
    /// probed window, and nothing at all rather than prose that fails the
    /// no-causal-claim check.
    async fn synthesize(&self, input: &str, grounded: &[f64]) -> Option<(String, String)> {
        use crate::providers::apple_fm;

        // The window is a runtime property of the running model, not a
        // constant — it changes with the OS.
        let fits = apple_fm::context_size()
            .await
            .is_some_and(|limit| input.len() / 4 + 400 < limit);
        if !fits {
            return None;
        }
        let text = apple_fm::generate(brief::SYSTEM_PROMPT, input, 400, 0.2, |_| {})
            .await
            .ok()?;
        match brief::validate(&text, grounded) {
            Ok(()) => Some((text, "apple_foundation_models".to_string())),
            Err(violations) => {
                // A brief that broke the rule is discarded, not repaired. The
                // table is still returned, so the user loses prose and not
                // information.
                tracing::info!(
                    target: "permagent::forecaster",
                    violations = %violations
                        .iter()
                        .map(|v| v.to_string())
                        .collect::<Vec<_>>()
                        .join("; "),
                    "discarded a market brief that outran its numbers"
                );
                None
            }
        }
    }

    /// The full, static tool inventory. Extracted from `list_tools` so the
    /// self-knowledge completeness guard derives its inventory from the REAL
    /// list — add a tool here and CI fails until the registry `description`
    /// names it.
    pub(crate) fn get_tools() -> Vec<Tool> {
        vec![
            Tool::new(
                "forecaster_series".to_string(),
                "List every bound market series for a project — competitor package downloads, \
                 category pageviews, Hacker News mentions — with its real point count, its \
                 span in days, and a verdict: forecastable, too short (and by exactly how \
                 much), collector stale, or awaiting approval. Call this BEFORE saying \
                 anything about market direction: a series this tool calls too short or \
                 stale cannot support a claim, and an empty list means nothing is bound, \
                 not that the market is flat."
                    .to_string(),
                schema::<SeriesParams>(),
            ),
            Tool::new(
                "forecaster_bind".to_string(),
                "Propose a new market series for a project: an npm or crates.io package, a \
                 Wikipedia article, a Hacker News query, keyed ideally to a competitor row \
                 the user already approved on the Ecosystem panel. This PROPOSES only — \
                 the series collects nothing until the user approves it through the same \
                 review gate as project intelligence, and you must say so. The source must \
                 be one of the known set; an unrecognised one is refused rather than guessed."
                    .to_string(),
                schema::<BindParams>(),
            ),
            Tool::new(
                "forecaster_forecast".to_string(),
                "Forecast one bound series and report the point estimate, the 80% interval, and \
                 the METHOD that produced them. The method label is mandatory and always \
                 accurate: a forecast served by the seasonal-naive baseline says so, and you \
                 must repeat that when you report it. Below the minimum history, or with a \
                 stale collector, this REFUSES and gives the reason instead of a number — \
                 report the refusal, never a direction. Says where a series is heading; never \
                 why, and never what to do."
                    .to_string(),
                schema::<ForecastParams>(),
            ),
            Tool::new(
                "forecaster_brief".to_string(),
                "The week's market direction for one project, as a table of every bound series \
                 with its forecast and method, plus a short summary when one can be generated \
                 honestly. The summary restates direction, magnitude, interval and method only \
                 — it never says why anything moved and never recommends an action, and neither \
                 should you when you relay it. If no summary comes back, read the table; that \
                 is the answer, not a failure."
                    .to_string(),
                schema::<BriefParams>(),
            ),
            Tool::new(
                "forecaster_health".to_string(),
                "Whether the TimesFM host (a separate Mac, reached over SSH) can actually serve \
                 forecasts right now. Call this when the user asks why forecasts are all \
                 baselines: a host that is down means every forecast is the Rust baseline and \
                 labelled as one, which is a DEGRADED state and not the model agreeing with the \
                 baseline. Nothing ever runs the model on the local machine instead."
                    .to_string(),
                schema::<NoParams>(),
            ),
        ]
    }
}

#[async_trait]
impl McpClientTrait for ForecasterClient {
    async fn list_tools(
        &self,
        _session_id: &str,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<ListToolsResult, Error> {
        Ok(ListToolsResult {
            tools: Self::get_tools(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        _ctx: &ToolCallContext,
        name: &str,
        arguments: Option<JsonObject>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<CallToolResult, Error> {
        announce("working");
        let result = match name {
            "forecaster_series" => self.handle_series(arguments).await,
            "forecaster_bind" => self.handle_bind(arguments).await,
            "forecaster_forecast" => self.handle_forecast(arguments).await,
            "forecaster_brief" => self.handle_brief(arguments).await,
            "forecaster_health" => self.handle_health().await,
            _ => Err(format!("Unknown tool: {}", name)),
        };
        announce("available");
        match result {
            Ok(result) => Ok(result),
            Err(error) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Error: {}",
                error
            ))])),
        }
    }

    fn get_info(&self) -> Option<&InitializeResult> {
        Some(&self.info)
    }
}

/// The Forecaster as a peer character — where the market around each project is
/// going, and how much of that we can honestly say.
pub const SELF_KNOWLEDGE_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "forecaster",
        display_name: "The Forecaster",
        category: crate::agents::self_knowledge::FeatureCategory::Worker,
        what_it_does:
            "The agent that watches where the market around each project is going, using \
             other people's public numbers — competitor package downloads, category \
             Wikipedia pageviews, Hacker News mentions — attached to the competitor and \
             adjacent rows the user already approved on a project's Ecosystem panel. \
             forecaster_series lists every bound series with its real point count, its span \
             and a verdict; forecaster_bind proposes a new one and never activates it; \
             forecaster_forecast returns a point estimate, an 80% interval and the method \
             that produced them, or refuses with a reason; forecaster_brief is the week's \
             summary for one project; forecaster_health says whether the separate machine \
             that runs the forecasting model is reachable. Every forecast carries its \
             method, and a method is used only where it beat the seasonal-naive baseline \
             on a rolling-origin backtest. Says where a category is heading; never why it \
             moved, and never what to do about it",
        why_it_matters:
            "A direction noticed late is a direction missed. This watches the numbers around \
             each project continuously and, just as importantly, says plainly when a series \
             is too short, too stale or unbound to support a claim — so a trend that was \
             never there is never presented as one",
        state_source: crate::agents::self_knowledge::StateSource::Queryable,
        teaching: &[
            crate::agents::self_knowledge::TeachingStep {
                title: "See what is actually being watched",
                body: "Call forecaster_series for the project. It reports every bound series \
                       with its real point count and a verdict — forecastable, too short (and \
                       by how much), collector stale, or awaiting approval. An empty list \
                       means nothing is bound, which is not the same as a flat market.",
                open_surface: None,
                confirm: None,
            },
            crate::agents::self_knowledge::TeachingStep {
                title: "Bind a competitor's numbers",
                body: "Call forecaster_bind with the project, a source (npm downloads, \
                       crates.io downloads, Wikipedia pageviews, Hacker News mentions) and the \
                       subject to watch — ideally one already approved on the Ecosystem panel. \
                       This only proposes it; say so. Collection starts after the user \
                       approves it.",
                open_surface: None,
                confirm: None,
            },
            crate::agents::self_knowledge::TeachingStep {
                title: "Report a direction with its method",
                body: "Call forecaster_forecast and repeat the method label with the number. \
                       If it came from the seasonal-naive baseline, say that it is a baseline \
                       and not a model. If the tool refuses, report the refusal — never \
                       describe a direction for a series it would not forecast.",
                open_surface: None,
                confirm: None,
            },
        ],
    };

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forecaster::series::SeriesStatus;

    fn summary(verdict: Verdict, points: usize) -> store::SeriesSummary {
        store::SeriesSummary {
            series_id: "s1".into(),
            project_id: "p1".into(),
            intel_id: None,
            source_kind: SourceKind::Npm,
            source_label: SourceKind::Npm.label().to_string(),
            subject: "langchain".into(),
            subject_group: Some("langchain".into()),
            cadence: Cadence::Daily,
            label: "langchain — npm downloads".into(),
            status: SeriesStatus::Active,
            points,
            span_days: points as i64,
            first_ts: None,
            last_ts: None,
            last_collected_at: None,
            last_error: None,
            snapshot_only: false,
            official_source: true,
            verdict,
        }
    }

    #[test]
    fn a_short_series_renders_the_gap_not_a_direction() {
        let line = describe_verdict(&summary(
            Verdict::InsufficientHistory {
                points: 42,
                needed: 180,
            },
            42,
        ));
        assert!(line.contains("42 of 180"), "{line}");
        assert!(line.contains("too short"), "{line}");
    }

    #[test]
    fn a_snapshot_only_source_says_so_on_its_own_line() {
        let mut s = summary(
            Verdict::InsufficientHistory {
                points: 2,
                needed: 180,
            },
            2,
        );
        s.source_kind = SourceKind::GithubRepo;
        s.source_label = SourceKind::GithubRepo.label().to_string();
        s.snapshot_only = true;
        let line = describe_verdict(&s);
        assert!(line.contains("snapshot-only"), "{line}");
    }

    #[test]
    fn an_unofficial_endpoint_is_labelled_where_the_user_sees_it() {
        let mut s = summary(Verdict::Forecastable, 400);
        s.source_kind = SourceKind::EquityClose;
        s.official_source = false;
        assert!(describe_verdict(&s).contains("unofficial"));
    }

    /// The registry description has to name every tool (the self-knowledge
    /// completeness guard enforces it), and every description has to be long
    /// enough to actually steer a call.
    #[test]
    fn every_tool_is_described_well_enough_to_be_called_correctly() {
        let tools = ForecasterClient::get_tools();
        assert_eq!(tools.len(), 5);
        for t in &tools {
            assert!(
                t.description.as_ref().map(|d| d.len()).unwrap_or(0) > 120,
                "{} needs a real description",
                t.name
            );
        }
        // The bind tool must say out loud that it only proposes.
        let bind = tools.iter().find(|t| t.name == "forecaster_bind").unwrap();
        let desc = bind.description.as_ref().unwrap();
        assert!(desc.contains("PROPOSES"), "{desc}");
    }

    /// The number and its provenance travel together, or the number is
    /// unreadable. This is the whole reason `method` is not an Option.
    #[test]
    fn the_method_label_travels_with_the_number() {
        let f = forecast::Forecast {
            series_id: "s1".into(),
            made_at: "2026-08-24T00:00:00.000Z".into(),
            horizon: 7,
            point: vec![100.0; 7],
            p10: vec![90.0; 7],
            p90: vec![110.0; 7],
            method: forecast::Method::SeasonalNaive,
            method_label: forecast::Method::SeasonalNaive.label().to_string(),
            mase_vs_baseline: Some(1.0),
            folds: 8,
            fold_wins: 0,
            selection: "ETS did not clear the gate".into(),
        };
        let text = describe_forecast(&f);
        assert!(text.contains("seasonal_naive"), "{text}");
        assert!(text.contains("not a model"), "{text}");
        assert!(text.contains("8 folds"), "{text}");
        // And it never volunteers a cause or a course of action.
        let lower = text.to_ascii_lowercase();
        assert!(!lower.contains("because"), "{text}");
        assert!(!lower.contains("recommend"), "{text}");
    }

    /// A refusal must be reachable from the tool description, or the model will
    /// treat an absent number as a tool failure and retry.
    #[test]
    fn the_forecast_tool_says_it_refuses() {
        let tools = ForecasterClient::get_tools();
        let f = tools
            .iter()
            .find(|t| t.name == "forecaster_forecast")
            .expect("forecaster_forecast is registered");
        let desc = f.description.as_ref().unwrap();
        assert!(desc.contains("REFUSES"), "{desc}");
        assert!(desc.contains("METHOD"), "{desc}");
    }

    /// Nothing here may reach for a series the user has not approved.
    #[test]
    fn no_tool_can_approve_or_activate_a_series() {
        for t in ForecasterClient::get_tools() {
            let n = t.name.to_ascii_lowercase();
            assert!(
                !n.contains("approve") && !n.contains("activate") && !n.contains("promote"),
                "{n} would let the model promote its own series"
            );
        }
    }
}
