//! The Financier — market research, and the agent's hands on whatever
//! finance tooling the user already runs.
//!
//! ## Two halves, and only one of them assumes anything
//!
//! **Everyone gets research.** `research_ticker` reads live quotes from Yahoo
//! Finance and needs no setup, no key and no local service. Optional company
//! fundamentals come from financialdatasets.ai when its key is configured.
//! A user with no trading stack and no key can still use the quote path on day
//! one.
//!
//! **Some people have their own engine.** Picker
//! (`~/dev/Picker/pre_surge_scanner`) owns a pre-surge ranking algorithm, its
//! backtests and a trade history. This extension does not reimplement any of
//! it: it starts the service, asks it to scan, reads its picks, and records
//! trades. Those tools announce themselves as unavailable rather than failing
//! obscurely when there is no Picker to talk to.
//!
//! ## The boundary that matters
//!
//! The model **researches and reports**. It does not size positions, it does
//! not place orders, and it has no path to either — no tool here can move
//! money. The one WRITE is `record_trade`, which appends to the user's history
//! of trades they already made; that record is what every hit-rate and
//! backtest number is computed from, so it is validated field by field before
//! it is sent and nothing about it is inferred.
//!
//! ## Honesty
//!
//! Every tool distinguishes "could not ask" from "asked and there was
//! nothing". A stale pick presented as today's pick is a recommendation about
//! a market that has moved, and a quote whose source was unreachable must
//! never be answered from memory.

use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::agents::tool_execution::ToolCallContext;
use anyhow::Result;
use async_trait::async_trait;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, JsonObject, ListToolsResult,
    ServerCapabilities, Tool,
};
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

pub static EXTENSION_NAME: &str = "finance";

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct NoParams {}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ResearchTickerParams {
    /// One or more ticker symbols, e.g. `AAPL`, `SHOP.TO`, `^GSPC`.
    symbols: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct CompanyFundamentalsParams {
    /// One ticker symbol, e.g. `AAPL` or `SHOP.TO`.
    ticker: String,
    /// Statement period: annual, quarterly, or ttm. Defaults to annual.
    #[serde(default)]
    period: Option<String>,
    /// Number of statement periods to retrieve. Clamped to 8.
    #[serde(default)]
    limit: Option<u8>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct RecordTradeParams {
    /// Date the position was opened, ISO `YYYY-MM-DD`. Ask the user rather
    /// than assuming today.
    entry_date: String,
    /// Ticker symbol, e.g. `ENB` or `SHOP.TO`.
    ticker: String,
    /// Company name.
    company_name: String,
    /// Price per share paid at entry.
    entry_price: f64,
    /// Number of shares bought.
    shares: i64,
    /// Date the position was closed, ISO `YYYY-MM-DD`. Omit for an open trade.
    #[serde(default)]
    exit_date: Option<String>,
    /// Price per share received at exit. Omit for an open trade.
    #[serde(default)]
    exit_price: Option<f64>,
    /// Anything the user said about why they took the trade.
    #[serde(default)]
    notes: Option<String>,
}

fn render_fundamentals(
    outcome: Result<crate::market_data::Fundamentals, crate::market_data::FundamentalsError>,
) -> std::result::Result<CallToolResult, String> {
    match outcome {
        Ok(fundamentals) => Ok(CallToolResult::success(vec![Content::text(format!(
            "{}\n\nThese are reported figures, not advice. Report them and their \
             periods; do not derive a valuation, a forecast, or a position size.",
            crate::market_data::describe_fundamentals(&fundamentals),
        ))])),
        Err(crate::market_data::FundamentalsError::NotConfigured) => {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "No financialdatasets.ai key is configured. Set {} to unlock company \
                 income, balance-sheet, and cash-flow fundamentals. Live prices via \
                 research_ticker still work without it. Do not answer this fundamentals \
                 question from memory.",
                crate::market_data::FUNDAMENTALS_KEY
            ))]))
        }
        Err(crate::market_data::FundamentalsError::Failed(error)) => Err(format!(
            "the fundamentals request could not be completed: {error}. \
             Do not answer the fundamentals question from memory"
        )),
    }
}

fn schema<T: JsonSchema>() -> JsonObject {
    let mut obj = serde_json::to_value(schema_for!(T))
        .map(|v| v.as_object().unwrap().clone())
        .expect("valid schema");
    obj.entry("properties")
        .or_insert_with(|| serde_json::json!({}));
    obj
}

pub struct FinanceClient {
    info: InitializeResult,
    _context: PlatformExtensionContext,
}

impl FinanceClient {
    pub fn new(context: PlatformExtensionContext) -> Result<Self> {
        let info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new(EXTENSION_NAME, "1.0.0").with_title("The Financier"),
            )
            .with_instructions(
                "Market research and the user's own finance tooling.\n\n\
                 research_ticker works for everyone — no setup, no key — and is how you \
                 ground any claim about a price. Never state a price from memory.\n\n\
                 company_fundamentals retrieves financial statements from \
                 financialdatasets.ai and needs the optional FINANCIAL_DATASETS_API_KEY. \
                 Its absence is not an error and does not affect any other tool.\n\n\
                 The picker_* tools drive the user's OWN stock scanner and only exist if \
                 they run one: call picker_status first, since it is often not running and \
                 picker_start brings it up. picker_scan takes many minutes. record_trade \
                 appends a trade the USER says they made.\n\n\
                 You never size a position and you cannot place an order.",
            );
        Ok(Self {
            info,
            _context: context,
        })
    }

    /// Live quotes. Needs no local service and no key, so this is the tool a
    /// user with no trading stack of their own still has.
    async fn handle_research(
        &self,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        let args = arguments.ok_or("research_ticker needs at least one symbol")?;
        let p: ResearchTickerParams = serde_json::from_value(serde_json::Value::Object(args))
            .map_err(|e| format!("could not read the symbols: {e}"))?;
        if p.symbols.is_empty() {
            return Err("research_ticker needs at least one symbol".into());
        }
        // Bounded so one call cannot become a scrape of a rate-limited source.
        const MAX_SYMBOLS: usize = 10;
        let asked = p.symbols.len();
        let symbols: Vec<String> = p.symbols.into_iter().take(MAX_SYMBOLS).collect();

        let mut sections = Vec::new();
        let mut failures = Vec::new();
        for s in &symbols {
            match crate::market_data::quote(s).await {
                Ok(q) => sections.push(crate::market_data::describe(&q)),
                // A failed lookup is reported as failed. Answering it from
                // memory would be a months-stale price stated with confidence.
                Err(e) => failures.push(format!("{s}: {e}")),
            }
        }

        let mut out = String::new();
        if !sections.is_empty() {
            out.push_str(&sections.join("\n\n"));
        }
        if !failures.is_empty() {
            out.push_str(&format!(
                "\n\nCould NOT be read (say so — do not answer these from memory):\n{}",
                failures.join("\n")
            ));
        }
        if asked > MAX_SYMBOLS {
            out.push_str(&format!(
                "\n\n{} symbol(s) beyond the first {MAX_SYMBOLS} were not looked up.",
                asked - MAX_SYMBOLS
            ));
        }
        out.push_str(
            "\n\nThis is market data, not advice. Report the numbers and their timestamp; \
             do not recommend a position size.",
        );
        Ok(CallToolResult::success(vec![Content::text(out)]))
    }

    async fn handle_status(&self) -> std::result::Result<CallToolResult, String> {
        let s = crate::picker::status().await;
        let text = if !s.reachable {
            format!(
                "The scanner is NOT reachable at {}{}.\nStart it with picker_start before \
                 asking for picks — do not report older picks as current.",
                s.base_url,
                s.detail
                    .as_deref()
                    .map(|d| format!(" ({d})"))
                    .unwrap_or_default()
            )
        } else {
            format!(
                "Scanner is up at {}.\nScan in progress: {}\nResults from: {}\nRanked results: {}",
                s.base_url,
                s.scan_in_progress,
                s.scan_date.as_deref().unwrap_or("no scan yet"),
                s.results
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "unknown".into()),
            )
        };
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    async fn handle_fundamentals(
        &self,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        let args = arguments.ok_or("company_fundamentals needs a ticker")?;
        let p: CompanyFundamentalsParams = serde_json::from_value(serde_json::Value::Object(args))
            .map_err(|e| format!("could not read the fundamentals request: {e}"))?;
        let period = p.period.unwrap_or_else(|| "annual".into()).to_lowercase();
        if !matches!(period.as_str(), "annual" | "quarterly" | "ttm") {
            return Err("period must be one of annual, quarterly, or ttm".into());
        }
        let limit = p.limit.unwrap_or(4).clamp(1, 8);
        render_fundamentals(crate::market_data::fundamentals(&p.ticker, &period, limit).await)
    }

    async fn handle_start(&self) -> std::result::Result<CallToolResult, String> {
        let msg = crate::picker::ensure_running().await?;
        Ok(CallToolResult::success(vec![Content::text(msg)]))
    }

    async fn handle_scan(&self) -> std::result::Result<CallToolResult, String> {
        let s = crate::picker::status().await;
        if !s.reachable {
            return Err(format!(
                "the scanner is not running at {} — call picker_start first",
                s.base_url
            ));
        }
        if s.scan_in_progress {
            return Ok(CallToolResult::success(vec![Content::text(
                "A scan is already running. Poll picker_status rather than starting another.",
            )]));
        }
        let msg = crate::picker::start_scan().await?;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "{msg}\nA full scan takes many minutes. Poll picker_status for progress; \
             picks are not final until it reports no scan in progress."
        ))]))
    }

    async fn handle_top_picks(&self) -> std::result::Result<CallToolResult, String> {
        let s = crate::picker::status().await;
        if !s.reachable {
            return Err(format!(
                "the scanner is not running at {} — call picker_start first. Do NOT \
                 present remembered picks as current.",
                s.base_url
            ));
        }
        let picks = crate::picker::top_picks().await?;
        if picks.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "The scanner answered and has no picks — it has not completed a scan yet. \
                 That is different from having scanned and found nothing worth buying; say so.",
            )]));
        }
        let stale = if s.scan_in_progress {
            "\nNOTE: a scan is running, so these will change."
        } else {
            ""
        };
        Ok(CallToolResult::success(vec![Content::text(format!(
            "{} pick(s) from the scan dated {}.{}\n\nEach carries its own confidence, \
             suggested buy window and one-line reason — report those, do not invent a \
             recommendation of your own and do not suggest a position size.\n\n{}",
            picks.len(),
            s.scan_date.as_deref().unwrap_or("unknown"),
            stale,
            serde_json::to_string_pretty(&picks).unwrap_or_default(),
        ))]))
    }

    async fn handle_record_trade(
        &self,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        let args = arguments.ok_or("record_trade needs the trade's details")?;
        let p: RecordTradeParams = serde_json::from_value(serde_json::Value::Object(args))
            .map_err(|e| format!("could not read the trade: {e}"))?;
        let trade = crate::picker::TradeEntry {
            entry_date: p.entry_date,
            ticker: p.ticker.trim().to_uppercase(),
            company_name: p.company_name,
            entry_price: p.entry_price,
            shares: p.shares,
            exit_date: p.exit_date,
            exit_price: p.exit_price,
            notes: p.notes,
        };
        let saved = crate::picker::record_trade(&trade).await?;
        let id = saved
            .get("trade")
            .and_then(|t| t.get("id"))
            .map(|i| i.to_string())
            .unwrap_or_else(|| "?".into());
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Recorded: {} {} shares of {} at {} on {} (trade #{}).\nConfirm the numbers \
             back to the user — this row is what every hit-rate and backtest figure is \
             computed from.",
            trade.ticker, trade.shares, trade.company_name, trade.entry_price, trade.entry_date, id,
        ))]))
    }

    async fn handle_trades(&self) -> std::result::Result<CallToolResult, String> {
        let trades = crate::picker::trades().await?;
        if trades.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No trades recorded yet.",
            )]));
        }
        Ok(CallToolResult::success(vec![Content::text(format!(
            "{} recorded trade(s):\n\n{}",
            trades.len(),
            serde_json::to_string_pretty(&trades).unwrap_or_default(),
        ))]))
    }

    /// The full, static tool inventory. Extracted from `list_tools` so the
    /// self-knowledge completeness guard derives its inventory from the REAL
    /// list — add a tool here and CI fails until the registry `description`
    /// names it.
    pub(crate) fn get_tools() -> Vec<Tool> {
        vec![
            Tool::new(
                "research_ticker".to_string(),
                "Live price, day and 52-week ranges, volume and timestamp for one or more \
                 tickers, from Yahoo Finance. Needs no setup, no key and no local service — \
                 use it whenever the user asks what something is trading at, or to ground a \
                 claim about a company in a real number. NEVER answer a price from memory: \
                 if this tool cannot reach the source it says so, and so must you. Report \
                 the numbers and their timestamp; this is data, not advice, and you do not \
                 recommend a position size."
                    .to_string(),
                schema::<ResearchTickerParams>(),
            ),
            Tool::new(
                "company_fundamentals".to_string(),
                "Retrieve reported income statements, balance sheets, and cash-flow \
                 statements for one company from financialdatasets.ai. This optional path \
                 needs FINANCIAL_DATASETS_API_KEY; if it is absent, say so and do not answer \
                 from memory. Retrieval and report only: no pricing, forecasting, valuation \
                 opinion, or position sizing."
                    .to_string(),
                schema::<CompanyFundamentalsParams>(),
            ),
            Tool::new(
                "picker_status".to_string(),
                "Whether the user's stock scanner is running, whether a scan is in flight, \
                 and how fresh its results are. Call this FIRST — the scanner is often down, \
                 and picks read while it is down are not current."
                    .to_string(),
                schema::<NoParams>(),
            ),
            Tool::new(
                "picker_start".to_string(),
                "Start the user's stock scanner through launchd. Safe to call when it is \
                 already running. Takes a few seconds to bind its port."
                    .to_string(),
                schema::<NoParams>(),
            ),
            Tool::new(
                "picker_scan".to_string(),
                "Ask the scanner to run a fresh scan over the stock universe. Returns as \
                 soon as the scan is accepted — a full scan takes many minutes, so poll \
                 picker_status rather than waiting. Refuses if a scan is already running."
                    .to_string(),
                schema::<NoParams>(),
            ),
            Tool::new(
                "picker_top_picks".to_string(),
                "The current ranked picks, each with its own confidence, suggested buy \
                 window and one-line reason. Report what the algorithm says. Do NOT invent \
                 a recommendation, and do NOT suggest a position size — sizing is not yours."
                    .to_string(),
                schema::<NoParams>(),
            ),
            Tool::new(
                "record_trade".to_string(),
                "Record a trade the USER says they made, in their trade history. Only for \
                 trades already executed — this places no orders and moves no money. Every \
                 field comes from the user: never infer the date, the price, or the size. \
                 Read the numbers back afterwards, because this row is what every later \
                 hit-rate and backtest figure is computed from."
                    .to_string(),
                schema::<RecordTradeParams>(),
            ),
            Tool::new(
                "list_trades".to_string(),
                "The trades already recorded in the user's history.".to_string(),
                schema::<NoParams>(),
            ),
        ]
    }
}

#[async_trait]
impl McpClientTrait for FinanceClient {
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
        let result = match name {
            "research_ticker" => self.handle_research(arguments).await,
            "company_fundamentals" => self.handle_fundamentals(arguments).await,
            "picker_status" => self.handle_status().await,
            "picker_start" => self.handle_start().await,
            "picker_scan" => self.handle_scan().await,
            "picker_top_picks" => self.handle_top_picks().await,
            "record_trade" => self.handle_record_trade(arguments).await,
            "list_trades" => self.handle_trades().await,
            _ => Err(format!("Unknown tool: {}", name)),
        };

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

    /// Nothing here may move money. If a tool is ever added that can, this
    /// test should fail and force the Tier-2 decision explicitly rather than
    /// letting it arrive as an ordinary tool.
    #[test]
    fn no_tool_can_place_an_order() {
        let names: Vec<String> = FinanceClient::get_tools()
            .iter()
            .map(|t| t.name.to_string())
            .collect();
        for forbidden in ["place_order", "buy", "sell", "submit_order", "trade"] {
            assert!(
                !names.iter().any(|n| n == forbidden),
                "a tool that moves money must not arrive as an ordinary tool: {forbidden}"
            );
        }
        assert!(names.contains(&"record_trade".to_string()));
    }

    #[test]
    fn every_tool_is_described() {
        for tool in FinanceClient::get_tools() {
            assert!(
                tool.description.as_ref().is_some_and(|d| d.len() > 40),
                "{} needs a description the model can act on",
                tool.name
            );
        }
    }

    #[test]
    fn research_inventory_is_preserved_and_fundamentals_is_added() {
        let tools = FinanceClient::get_tools();
        let research = tools
            .iter()
            .find(|tool| tool.name == "research_ticker")
            .expect("research_ticker remains available");
        assert_eq!(
            research.description.as_deref(),
            Some(
                "Live price, day and 52-week ranges, volume and timestamp for one or more \
                 tickers, from Yahoo Finance. Needs no setup, no key and no local service — \
                 use it whenever the user asks what something is trading at, or to ground a \
                 claim about a company in a real number. NEVER answer a price from memory: \
                 if this tool cannot reach the source it says so, and so must you. Report \
                 the numbers and their timestamp; this is data, not advice, and you do not \
                 recommend a position size."
            )
        );
        assert!(tools.iter().any(|tool| tool.name == "company_fundamentals"));
    }

    /// An optional capability that errors on every machine that never opted in
    /// reads as broken software; an opted-in call that fails remains a failure.
    #[test]
    fn fundamentals_configuration_absence_is_an_answer_but_call_failure_is_an_error() {
        let answer = render_fundamentals(Err(crate::market_data::FundamentalsError::NotConfigured))
            .expect("an unconfigured optional capability is still a successful answer");
        let text = match &answer.content[0].raw {
            rmcp::model::RawContent::Text(text) => &text.text,
            other => panic!("expected text content, got {other:?}"),
        };
        assert!(
            text.contains(crate::market_data::FUNDAMENTALS_KEY),
            "{text}"
        );
        assert!(text.contains("research_ticker"), "{text}");

        let failure = render_fundamentals(Err(crate::market_data::FundamentalsError::Failed(
            "configured call failed".into(),
        )));
        assert!(
            failure.is_err(),
            "a configured call failure must be an error"
        );
    }
}
