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
struct WatchlistParams {
    /// Ticker to track, e.g. `AAPL` or `SHOP.TO`.
    symbol: String,
    /// Optional display name.
    #[serde(default)]
    label: Option<String>,
    /// Optional note about why this is on the list.
    #[serde(default)]
    notes: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct SymbolParams {
    symbol: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct NoteAddParams {
    title: String,
    body: String,
    #[serde(default)]
    symbol: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct NoteUpdateParams {
    id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    symbol: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct IdParams {
    id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct PositionCloseParams {
    id: String,
    exit_date: String,
    exit_price: f64,
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
    context: PlatformExtensionContext,
}

fn announce(state: &str) {
    crate::events::emit(crate::events::agent_state_changed(
        "financier",
        "The Financier",
        state,
    ));
}

impl FinanceClient {
    pub fn new(context: PlatformExtensionContext) -> Result<Self> {
        let info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new(EXTENSION_NAME, "1.0.0").with_title("The Financier"),
            )
            .with_instructions(
                "Market research and the Finance tab ledger.\n\n\
                 research_ticker works for everyone — no setup, no key — and is how you \
                 ground any claim about a price. Never state a price from memory.\n\n\
                 company_fundamentals retrieves financial statements from \
                 financialdatasets.ai and needs the optional FINANCIAL_DATASETS_API_KEY. \
                 Its absence is not an error and does not affect any other tool.\n\n\
                 The Finance tab is the money board: Polybot status, holdings with live \
                 marks, Picker picks gated by Yahoo plus a loop-engineering check \
                 (ICIR / half-life / out-of-sample — each pick is one hypothesis, never \
                 a new strategy farmed on the same data), household spend, and the \
                 research ledger. finance_board reads it; \
                 finance_watchlist_add / finance_watchlist_remove, finance_note_add / \
                 finance_note_update / finance_note_delete, and finance_position_add / \
                 finance_position_close / finance_position_delete write the ledger. Call \
                 finance_board before changing anything so you are editing the live board.\n\n\
                 Holdings and bank balances NEVER go into the Picker ranker. \
                 holding_sell_signals reads Yahoo daily closes on OPEN lots only \
                 and reports overbought sell signals (RSI-14 vs your threshold, \
                 stochastic, stretch above the 20-day average, upper Bollinger \
                 band, proximity to the 52-week high). Call it when the user asks \
                 whether a holding looks overbought or whether there is a sell \
                 signal. Report the signs. A signal is not an order and is not a \
                 position size.\n\n\
                 The picker_* tools drive the user's OWN stock scanner and only exist if \
                 they run one: call picker_status first, since it is often not running and \
                 picker_start brings it up. picker_scan takes many minutes. record_trade \
                 appends a trade the USER says they made, on the Finance tab and (when \
                 reachable) in their scanner history.\n\n\
                 You never size a position and you cannot place an order.",
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
            entry_date: p.entry_date.clone(),
            ticker: p.ticker.trim().to_uppercase(),
            company_name: p.company_name.clone(),
            entry_price: p.entry_price,
            shares: p.shares,
            exit_date: p.exit_date.clone(),
            exit_price: p.exit_price,
            notes: p.notes.clone(),
        };
        let picker = crate::picker::record_trade(&trade).await;
        let mut out = format!(
            "Recorded: {} {} shares of {} at {} on {}.",
            trade.ticker, trade.shares, trade.company_name, trade.entry_price, trade.entry_date
        );
        match picker {
            Ok(saved) => {
                let id = saved
                    .get("trade")
                    .and_then(|t| t.get("id"))
                    .map(|i| i.to_string())
                    .unwrap_or_else(|| "?".into());
                out.push_str(&format!(" Scanner history trade #{id}."));
            }
            Err(e) => {
                let local = match self.pool().await {
                    Ok(pool) => crate::finance_ledger::add_position(
                        &pool,
                        crate::finance_ledger::NewPosition {
                            symbol: trade.ticker.clone(),
                            company_name: trade.company_name.clone(),
                            entry_date: trade.entry_date.clone(),
                            entry_price: trade.entry_price,
                            shares: trade.shares,
                            exit_date: trade.exit_date.clone(),
                            exit_price: trade.exit_price,
                            notes: trade.notes.clone(),
                        },
                    )
                    .await
                    .ok(),
                    Err(_) => None,
                };
                if let Some(pos) = local {
                    out.push_str(&format!(
                        " Scanner history was not updated ({e}) — recorded on the Finance tab as position {}.",
                        pos.id
                    ));
                } else {
                    out.push_str(&format!(
                        " Could not write scanner history ({e}) or the Finance tab ledger."
                    ));
                }
            }
        }
        out.push_str("\nConfirm the numbers back to the user.");
        Ok(CallToolResult::success(vec![Content::text(out)]))
    }

    async fn handle_trades(&self) -> std::result::Result<CallToolResult, String> {
        let trades = crate::picker::trades().await?;
        if trades.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No trades recorded yet in the scanner history. Check finance_board for the Finance tab ledger.",
            )]));
        }
        Ok(CallToolResult::success(vec![Content::text(format!(
            "{} recorded trade(s) in scanner history:\n\n{}",
            trades.len(),
            serde_json::to_string_pretty(&trades).unwrap_or_default(),
        ))]))
    }

    async fn handle_sell_signals(&self) -> std::result::Result<CallToolResult, String> {
        let pool = self.pool().await?;
        let threshold = crate::overbought::rsi_threshold();
        let lots = crate::overbought::assess_open_lots(&pool, threshold).await?;
        Ok(CallToolResult::success(vec![Content::text(
            crate::overbought::describe_open_lots(&lots),
        )]))
    }

    async fn handle_board(&self) -> std::result::Result<CallToolResult, String> {
        let pool = self.pool().await?;
        let watchlist = crate::finance_ledger::list_watchlist(&pool).await?;
        let notes = crate::finance_ledger::list_notes(&pool).await?;
        let positions = crate::finance_ledger::list_positions(&pool).await?;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Finance tab ledger.\n\nWatchlist ({}):\n{}\n\nNotes ({}):\n{}\n\nPositions ({}):\n{}\n\nThis is the ledger, not live prices — research_ticker for a quote, holding_sell_signals for overbought sell signals on open lots.",
            watchlist.len(),
            serde_json::to_string_pretty(&watchlist).unwrap_or_default(),
            notes.len(),
            serde_json::to_string_pretty(&notes).unwrap_or_default(),
            positions.len(),
            serde_json::to_string_pretty(&positions).unwrap_or_default(),
        ))]))
    }

    async fn handle_watchlist_add(
        &self,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        let args = arguments.ok_or("finance_watchlist_add needs a symbol")?;
        let p: WatchlistParams = serde_json::from_value(serde_json::Value::Object(args))
            .map_err(|e| format!("could not read the watchlist request: {e}"))?;
        let pool = self.pool().await?;
        let item = crate::finance_ledger::add_watchlist(
            &pool,
            &p.symbol,
            p.label.as_deref(),
            p.notes.as_deref(),
        )
        .await?;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "{} is on the Finance tab watchlist ({}).",
            item.symbol, item.id
        ))]))
    }

    async fn handle_watchlist_remove(
        &self,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        let args = arguments.ok_or("finance_watchlist_remove needs a symbol")?;
        let p: SymbolParams = serde_json::from_value(serde_json::Value::Object(args))
            .map_err(|e| format!("could not read the symbol: {e}"))?;
        let pool = self.pool().await?;
        let gone = crate::finance_ledger::remove_watchlist(&pool, &p.symbol).await?;
        let symbol = p.symbol.trim().to_uppercase();
        Ok(CallToolResult::success(vec![Content::text(if gone {
            format!("{symbol} was taken off the Finance tab watchlist.")
        } else {
            format!("{symbol} was not on the watchlist.")
        })]))
    }

    async fn handle_note_add(
        &self,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        let args = arguments.ok_or("finance_note_add needs a title and body")?;
        let p: NoteAddParams = serde_json::from_value(serde_json::Value::Object(args))
            .map_err(|e| format!("could not read the note: {e}"))?;
        let pool = self.pool().await?;
        let note =
            crate::finance_ledger::add_note(&pool, &p.title, &p.body, p.symbol.as_deref()).await?;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Noted on the Finance tab: \"{}\" ({}).",
            note.title, note.id
        ))]))
    }

    async fn handle_note_update(
        &self,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        let args = arguments.ok_or("finance_note_update needs an id")?;
        let p: NoteUpdateParams = serde_json::from_value(serde_json::Value::Object(args))
            .map_err(|e| format!("could not read the note update: {e}"))?;
        let pool = self.pool().await?;
        let note = crate::finance_ledger::update_note(
            &pool,
            &p.id,
            p.title.as_deref(),
            p.body.as_deref(),
            p.symbol.as_deref().map(Some),
        )
        .await?;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Updated Finance tab note {} (\"{}\").",
            note.id, note.title
        ))]))
    }

    async fn handle_note_delete(
        &self,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        let args = arguments.ok_or("finance_note_delete needs an id")?;
        let p: IdParams = serde_json::from_value(serde_json::Value::Object(args))
            .map_err(|e| format!("could not read the note id: {e}"))?;
        let pool = self.pool().await?;
        let gone = crate::finance_ledger::delete_note(&pool, &p.id).await?;
        Ok(CallToolResult::success(vec![Content::text(if gone {
            format!("Deleted Finance tab note {}.", p.id)
        } else {
            format!("No Finance tab note {}.", p.id)
        })]))
    }

    async fn handle_position_add(
        &self,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        let args = arguments.ok_or("finance_position_add needs the position's details")?;
        let p: RecordTradeParams = serde_json::from_value(serde_json::Value::Object(args))
            .map_err(|e| format!("could not read the position: {e}"))?;
        let pool = self.pool().await?;
        let pos = crate::finance_ledger::add_position(
            &pool,
            crate::finance_ledger::NewPosition {
                symbol: p.ticker,
                company_name: p.company_name,
                entry_date: p.entry_date,
                entry_price: p.entry_price,
                shares: p.shares,
                exit_date: p.exit_date,
                exit_price: p.exit_price,
                notes: p.notes,
            },
        )
        .await?;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "On the Finance tab: {} {} shares of {} at {} on {} ({}). Confirm the numbers — this is a record of a trade the user already made, not an order.",
            pos.symbol, pos.shares, pos.company_name, pos.entry_price, pos.entry_date, pos.id
        ))]))
    }

    async fn handle_position_close(
        &self,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        let args = arguments.ok_or("finance_position_close needs id, exit_date and exit_price")?;
        let p: PositionCloseParams = serde_json::from_value(serde_json::Value::Object(args))
            .map_err(|e| format!("could not read the close: {e}"))?;
        let pool = self.pool().await?;
        let pos =
            crate::finance_ledger::close_position(&pool, &p.id, &p.exit_date, p.exit_price).await?;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Closed {} on the Finance tab at {} on {}.",
            pos.symbol, p.exit_price, p.exit_date
        ))]))
    }

    async fn handle_position_delete(
        &self,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        let args = arguments.ok_or("finance_position_delete needs an id")?;
        let p: IdParams = serde_json::from_value(serde_json::Value::Object(args))
            .map_err(|e| format!("could not read the position id: {e}"))?;
        let pool = self.pool().await?;
        let gone = crate::finance_ledger::delete_position(&pool, &p.id).await?;
        Ok(CallToolResult::success(vec![Content::text(if gone {
            format!("Removed position {} from the Finance tab.", p.id)
        } else {
            format!("No Finance tab position {}.", p.id)
        })]))
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
                "finance_board".to_string(),
                "Read the Finance tab ledger: watchlist, research notes, and recorded \
                 positions. Call this before adding or changing anything so you edit the \
                 live board. This is the ledger, not live prices — research_ticker for a quote."
                    .to_string(),
                schema::<NoParams>(),
            ),
            Tool::new(
                "finance_watchlist_add".to_string(),
                "Put a ticker on the Finance tab watchlist. The tab will fetch a live quote \
                 for it. Does not buy anything."
                    .to_string(),
                schema::<WatchlistParams>(),
            ),
            Tool::new(
                "finance_watchlist_remove".to_string(),
                "Take a ticker off the Finance tab watchlist.".to_string(),
                schema::<SymbolParams>(),
            ),
            Tool::new(
                "finance_note_add".to_string(),
                "Add a research note to the Finance tab. Optional symbol ties it to a ticker. \
                 For observations and sourced numbers, not advice or a position size."
                    .to_string(),
                schema::<NoteAddParams>(),
            ),
            Tool::new(
                "finance_note_update".to_string(),
                "Update a Finance tab research note by id from finance_board.".to_string(),
                schema::<NoteUpdateParams>(),
            ),
            Tool::new(
                "finance_note_delete".to_string(),
                "Delete a Finance tab research note by id.".to_string(),
                schema::<IdParams>(),
            ),
            Tool::new(
                "finance_position_add".to_string(),
                "Record a position the USER says they already took, on the Finance tab. \
                 Never infer date, price, or size. This places no order and moves no money."
                    .to_string(),
                schema::<RecordTradeParams>(),
            ),
            Tool::new(
                "finance_position_close".to_string(),
                "Mark a Finance tab position closed with the exit date and price the USER \
                 gives. Does not sell anything."
                    .to_string(),
                schema::<PositionCloseParams>(),
            ),
            Tool::new(
                "finance_position_delete".to_string(),
                "Remove a position row from the Finance tab (a record-keeping correction, \
                 not a sale)."
                    .to_string(),
                schema::<IdParams>(),
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
                "Record a trade the USER says they made. Writes the Finance tab and, when \
                 the scanner is up, their scanner history. Only for trades already executed \
                 — this places no orders and moves no money. Every field comes from the user."
                    .to_string(),
                schema::<RecordTradeParams>(),
            ),
            Tool::new(
                "list_trades".to_string(),
                "The trades already recorded in the user's scanner history. For the Finance \
                 tab ledger, call finance_board."
                    .to_string(),
                schema::<NoParams>(),
            ),
            Tool::new(
                "holding_sell_signals".to_string(),
                "Overbought sell signals on OPEN holdings only: RSI-14 vs the user's \
                 threshold, stochastic %K, stretch above the 20-day average, upper \
                 Bollinger band, and proximity to the 52-week high, from Yahoo daily \
                 closes. Holdings never go into the Picker ranker. Call this when the \
                 user asks if a position looks overbought or whether there is a sell \
                 signal. Report the signs. A signal is not an order and not a size — \
                 you cannot place an order."
                    .to_string(),
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
        announce("working");
        let result = match name {
            "research_ticker" => self.handle_research(arguments).await,
            "company_fundamentals" => self.handle_fundamentals(arguments).await,
            "finance_board" => self.handle_board().await,
            "finance_watchlist_add" => self.handle_watchlist_add(arguments).await,
            "finance_watchlist_remove" => self.handle_watchlist_remove(arguments).await,
            "finance_note_add" => self.handle_note_add(arguments).await,
            "finance_note_update" => self.handle_note_update(arguments).await,
            "finance_note_delete" => self.handle_note_delete(arguments).await,
            "finance_position_add" => self.handle_position_add(arguments).await,
            "finance_position_close" => self.handle_position_close(arguments).await,
            "finance_position_delete" => self.handle_position_delete(arguments).await,
            "picker_status" => self.handle_status().await,
            "picker_start" => self.handle_start().await,
            "picker_scan" => self.handle_scan().await,
            "picker_top_picks" => self.handle_top_picks().await,
            "record_trade" => self.handle_record_trade(arguments).await,
            "list_trades" => self.handle_trades().await,
            "holding_sell_signals" => self.handle_sell_signals().await,
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

/// The Financier as a peer character — market research and the Finance tab.
pub const SELF_KNOWLEDGE_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "financier",
        display_name: "The Financier",
        category: crate::agents::self_knowledge::FeatureCategory::Worker,
        what_it_does:
            "The agent that owns market research and the Finance money board. It reads live \
             quotes (research_ticker, no key) and optional company fundamentals, and it \
             writes the Finance tab: a watchlist, research notes, and recorded \
             positions (finance_board, finance_watchlist_add / finance_watchlist_remove, \
             finance_note_add / finance_note_update / finance_note_delete, \
             finance_position_add / finance_position_close / finance_position_delete). If \
             the user runs their own stock scanner, picker_status / picker_start / \
             picker_scan / picker_top_picks drive it and record_trade / list_trades keep \
             that history. holding_sell_signals reports overbought sell signals on open \
             lots (RSI, stochastic, 20-day stretch, Bollinger, 52-week high) without \
             feeding holdings into the ranker. Picker picks are gated on the tab by Yahoo \
             plus a loop-engineering check. Reports timestamped numbers; never sizes \
             a position and cannot place an order",
        why_it_matters:
            "A price stated from memory is months stale. The Financier grounds every number \
             in a fetch, and the Finance tab is the durable place those numbers, Polybot \
             status, validated picks, and the user's own trades live",
        state_source: crate::agents::self_knowledge::StateSource::Queryable,
        teaching: &[],
    };

/// The Finance tab surface — the ledger The Financier reads and writes.
pub const FINANCE_TAB_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "finance_tab",
        display_name: "Finance tab",
        category: crate::agents::self_knowledge::FeatureCategory::Surface,
        what_it_does: "The Financier's money board: Polybot status, holdings with live P&L, \
             Picker picks gated by Yahoo plus a loop-engineering check, overbought sell \
             signals on open holdings (RSI-14, stochastic, 20-day stretch, Bollinger, \
             52-week high), household spend from dropped statements, a watchlist with live \
             quotes, research notes, and a trade journal. Start the scanner, run a scan, \
             and record, edit, or close trades on this tab — you do not have to open Picker \
             to enter trade data. The Financier writes the same rows through its tools. \
             Quotes are fetched at read time and never stored. This is a research ledger, \
             not a brokerage — nothing here places an order or sizes a position",
        why_it_matters: "It is where Polybot, holdings, validated picks, and household spend \
             live so they can be inspected without asking chat to recite them",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        teaching: &[crate::agents::self_knowledge::TeachingStep {
            title: "Open Finance",
            body: "Show the user the Finance tab — Polybot, the Picker trade journal, \
                   validated picks, household spend, and the ledger the Financier keeps.",
            open_surface: Some(crate::agents::self_knowledge::SurfaceRef {
                tab: "Finance",
                section: None,
            }),
            confirm: None,
        }],
    };

#[cfg(test)]
mod tests {
    use super::*;

    /// Nothing here may move money. If a tool is ever added that can, this
    /// test should fail and force the Tier-2 decision explicitly rather than
    /// letting it arrive as an ordinary tool.
    ///
    /// Every assertion below is a *negative* one, and an empty tool list
    /// satisfies all of them. A mutation returning `Vec::new()` from
    /// `get_tools()` was caught here only by the `record_trade` line, which had
    /// been written to pin the safe recording tool, not to floor this guard —
    /// the money property was resting on an incidental neighbour. The floor is
    /// explicit now so nobody removes it as redundant.
    #[test]
    fn no_tool_can_place_an_order() {
        let names: Vec<String> = FinanceClient::get_tools()
            .iter()
            .map(|t| t.name.to_string())
            .collect();
        assert!(
            !names.is_empty(),
            "get_tools() returned nothing, so every 'no tool named X' assertion \
             below would pass without inspecting a single tool"
        );
        for forbidden in ["place_order", "buy", "sell", "submit_order", "trade"] {
            assert!(
                !names.iter().any(|n| n == forbidden),
                "a tool that moves money must not arrive as an ordinary tool: {forbidden}"
            );
        }
        assert!(names.contains(&"record_trade".to_string()));
        assert!(names.contains(&"holding_sell_signals".to_string()));
    }

    #[test]
    fn every_tool_is_described() {
        let tools = FinanceClient::get_tools();
        assert!(
            !tools.is_empty(),
            "get_tools() returned nothing, so 'every tool' was vacuously true"
        );
        for tool in tools {
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
