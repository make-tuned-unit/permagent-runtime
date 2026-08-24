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
use crate::forecaster::store::{self, Verdict};
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
/// extension is the module, the id is the character.
pub static EXTENSION_NAME: &str = "forecast";

/// The roster id the world view and the activity journal key on. Distinct from
/// `EXTENSION_NAME` only in that it names the *character*, as `financier` does
/// for the `finance` extension.
pub const AGENT_ID: &str = "forecaster";
pub const AGENT_NAME: &str = "The Forecaster";

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct SeriesParams {
    /// Project id, slug or name. Omit to list every bound series.
    project: Option<String>,
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
        assert_eq!(tools.len(), 2);
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
