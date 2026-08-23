//! Tomorrow's pick — the Financier's close-of-day judgment.
//!
//! Picker ranks. The loop gate keeps the farm honest. Opus may choose **one**
//! name from the surviving candidates for tomorrow's open, or none. A ticker
//! that was not in the list is refused. Silence is the honest answer when
//! nothing clears.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Row, Sqlite};

use crate::conversation::message::Message;
use crate::cost_router::{role_map, WorkflowRole};
use crate::market_data;
use crate::pick_loop;
use crate::picker;

pub const OPUS_PROVIDER: &str = "anthropic";
pub const OPUS_MODEL: &str = "claude-opus-4-8";
const MAX_CANDIDATES: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DailyPick {
    pub day: String,
    pub as_of: String,
    pub ticker: Option<String>,
    pub company_name: Option<String>,
    pub why: String,
    pub model: Option<String>,
    pub candidate_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CloseCandidate {
    pub ticker: String,
    pub company_name: Option<String>,
    pub rank: Option<i64>,
    pub score: Option<f64>,
    pub confidence: Option<f64>,
    pub buy_window: Option<String>,
    pub reason: Option<String>,
    pub last: Option<f64>,
    pub loop_passed: bool,
}

pub async fn ensure_schema(pool: &Pool<Sqlite>) -> Result<(), String> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS finance_daily_picks (
            day              TEXT PRIMARY KEY,
            as_of            TEXT NOT NULL,
            ticker           TEXT,
            company_name     TEXT,
            why              TEXT NOT NULL,
            model            TEXT,
            candidate_count  INTEGER NOT NULL DEFAULT 0,
            created_at       TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn load_for_day(pool: &Pool<Sqlite>, day: &str) -> Result<Option<DailyPick>, String> {
    ensure_schema(pool).await?;
    let row = sqlx::query(
        "SELECT day, as_of, ticker, company_name, why, model, candidate_count
         FROM finance_daily_picks WHERE day = ?",
    )
    .bind(day)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(row.map(|r| DailyPick {
        day: r.get("day"),
        as_of: r.get("as_of"),
        ticker: r.get("ticker"),
        company_name: r.get("company_name"),
        why: r.get("why"),
        model: r.get("model"),
        candidate_count: r.get("candidate_count"),
    }))
}

pub async fn latest(pool: &Pool<Sqlite>) -> Result<Option<DailyPick>, String> {
    ensure_schema(pool).await?;
    let row = sqlx::query(
        "SELECT day, as_of, ticker, company_name, why, model, candidate_count
         FROM finance_daily_picks ORDER BY day DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(row.map(|r| DailyPick {
        day: r.get("day"),
        as_of: r.get("as_of"),
        ticker: r.get("ticker"),
        company_name: r.get("company_name"),
        why: r.get("why"),
        model: r.get("model"),
        candidate_count: r.get("candidate_count"),
    }))
}

pub async fn save(pool: &Pool<Sqlite>, pick: &DailyPick) -> Result<(), String> {
    ensure_schema(pool).await?;
    sqlx::query(
        "INSERT OR REPLACE INTO finance_daily_picks
            (day, as_of, ticker, company_name, why, model, candidate_count, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&pick.day)
    .bind(&pick.as_of)
    .bind(&pick.ticker)
    .bind(&pick.company_name)
    .bind(&pick.why)
    .bind(&pick.model)
    .bind(pick.candidate_count)
    .bind(&pick.as_of)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Picker names that survive Yahoo + the loop gate. Failures drop out —
/// a missing series is not a silent pass.
pub async fn surviving_candidates() -> Result<Vec<CloseCandidate>, String> {
    let raw = picker::top_picks().await?;
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    let batch = raw.len().min(MAX_CANDIDATES);
    let mut out = Vec::new();
    for v in raw.into_iter().take(MAX_CANDIDATES) {
        let Some(ticker) = v
            .get("ticker")
            .or_else(|| v.get("symbol"))
            .and_then(|s| s.as_str())
            .map(|s| s.trim().to_uppercase())
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let closes = match market_data::daily_closes(&ticker, "1y").await {
            Ok(c) => c,
            Err(_) => continue,
        };
        let gate = pick_loop::validate_closes(&closes, batch);
        if !gate.passed {
            continue;
        }
        let last = market_data::quote(&ticker).await.ok().and_then(|q| q.price);
        out.push(CloseCandidate {
            ticker,
            company_name: v
                .get("company_name")
                .or_else(|| v.get("name"))
                .and_then(|s| s.as_str())
                .map(str::to_string),
            rank: v.get("rank").and_then(|n| n.as_i64()),
            score: v
                .get("total_score")
                .or_else(|| v.get("score"))
                .and_then(|n| n.as_f64()),
            confidence: v
                .get("confidence")
                .or_else(|| v.get("conv"))
                .and_then(|n| n.as_f64()),
            buy_window: v
                .get("buy_window")
                .or_else(|| v.get("buyWindow"))
                .and_then(|s| s.as_str())
                .map(str::to_string),
            reason: v
                .get("reason")
                .or_else(|| v.get("thesis"))
                .and_then(|s| s.as_str())
                .map(str::to_string),
            last,
            loop_passed: true,
        });
    }
    Ok(out)
}

pub fn none_pick(day: &str, why: impl Into<String>, candidates: usize) -> DailyPick {
    DailyPick {
        day: day.to_string(),
        as_of: Utc::now().to_rfc3339(),
        ticker: None,
        company_name: None,
        why: why.into(),
        model: None,
        candidate_count: candidates as i64,
    }
}

/// Opus (or the configured Orchestrate role if it is Opus) chooses at most
/// one candidate. Invented tickers become none.
pub async fn judge_with_opus(
    day: &str,
    candidates: &[CloseCandidate],
) -> Result<DailyPick, String> {
    if candidates.is_empty() {
        return Ok(none_pick(
            day,
            "No scanner names cleared the loop gate. No pick for tomorrow.",
            0,
        ));
    }
    let (provider_name, model_name) = opus_model()?;
    let provider =
        crate::providers::create_with_named_model(&provider_name, &model_name, Vec::new())
            .await
            .map_err(|e| format!("Opus is not available ({e}). No pick invented."))?;
    let system = "You are The Financier. You may name AT MOST one ticker from \
         CANDIDATES as tomorrow's pick, or none. A pick is a hypothesis, not \
         an order and not a size. NEVER invent a ticker that is not in the \
         list. NEVER invent signs that are not in the supplied fields. If \
         nothing is good enough, pick is null. Reply JSON only: \
         {\"pick\": \"TICKER\" or null, \"why\": \"one paragraph\"}.";
    let user = Message::user().with_text(format!(
        "Session day {day}. CANDIDATES:\n{}",
        serde_json::to_string_pretty(candidates).unwrap_or_default()
    ));
    let (response, _usage) = provider
        .complete(
            &crate::model::ModelConfig::new(&model_name).map_err(|e| e.to_string())?,
            "financier-close",
            system,
            std::slice::from_ref(&user),
            &[],
        )
        .await
        .map_err(|e| format!("Opus did not answer ({e}). No pick invented."))?;
    Ok(parse_judgment(
        day,
        &response.as_concat_text(),
        candidates,
        &format!("{provider_name}/{model_name}"),
    ))
}

fn opus_model() -> Result<(String, String), String> {
    if let Some(mapped) = role_map::role_model(WorkflowRole::Orchestrate) {
        if mapped.model.to_ascii_lowercase().contains("opus") {
            return Ok((mapped.provider, mapped.model));
        }
    }
    Ok((OPUS_PROVIDER.to_string(), OPUS_MODEL.to_string()))
}

pub fn parse_judgment(
    day: &str,
    text: &str,
    candidates: &[CloseCandidate],
    model: &str,
) -> DailyPick {
    let allowed: Vec<&str> = candidates.iter().map(|c| c.ticker.as_str()).collect();
    let parsed = extract_json(text).map(|v| {
        let pick = match v.get("pick") {
            Some(serde_json::Value::Null) | None => None,
            Some(serde_json::Value::String(s)) => {
                let t = s.trim().to_uppercase();
                if t.is_empty() || t == "NULL" || t == "NONE" {
                    None
                } else {
                    Some(t)
                }
            }
            _ => None,
        };
        let why = v
            .get("why")
            .and_then(|s| s.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("No paragraph.")
            .to_string();
        (pick, why)
    });
    let Some((pick, why)) = parsed else {
        return none_pick(
            day,
            "Opus did not return a usable judgment. No pick invented.",
            candidates.len(),
        );
    };
    match pick {
        Some(ticker) if allowed.iter().any(|a| *a == ticker) => {
            let company = candidates
                .iter()
                .find(|c| c.ticker == ticker)
                .and_then(|c| c.company_name.clone());
            DailyPick {
                day: day.to_string(),
                as_of: Utc::now().to_rfc3339(),
                ticker: Some(ticker),
                company_name: company,
                why,
                model: Some(model.to_string()),
                candidate_count: candidates.len() as i64,
            }
        }
        Some(_) => none_pick(
            day,
            "Opus named a ticker that was not in the scanner list. No pick invented.",
            candidates.len(),
        ),
        None => DailyPick {
            day: day.to_string(),
            as_of: Utc::now().to_rfc3339(),
            ticker: None,
            company_name: None,
            why,
            model: Some(model.to_string()),
            candidate_count: candidates.len() as i64,
        },
    }
}

fn extract_json(text: &str) -> Option<serde_json::Value> {
    let (start, end) = (text.find('{')?, text.rfind('}')?);
    serde_json::from_str(text.get(start..=end)?).ok()
}

pub fn notify_copy(pick: &DailyPick) -> (String, String) {
    match pick.ticker.as_deref() {
        Some(ticker) => (
            "The Financier · tomorrow".into(),
            format!("{ticker} — {}", pick.why),
        ),
        None => ("The Financier · no pick tomorrow".into(), pick.why.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(ticker: &str) -> CloseCandidate {
        CloseCandidate {
            ticker: ticker.into(),
            company_name: Some(ticker.into()),
            rank: Some(1),
            score: Some(1.0),
            confidence: Some(0.6),
            buy_window: None,
            reason: Some("scanner reason".into()),
            last: Some(10.0),
            loop_passed: true,
        }
    }

    #[test]
    fn invented_ticker_is_refused() {
        let got = parse_judgment(
            "2026-08-24",
            r#"{"pick":"FAKE","why":"I made this up"}"#,
            &[cand("SHOP")],
            "anthropic/claude-opus-4-8",
        );
        assert!(got.ticker.is_none());
        assert!(got.why.contains("not in the scanner list"));
    }

    #[test]
    fn listed_ticker_is_kept() {
        let got = parse_judgment(
            "2026-08-24",
            r#"{"pick":"shop","why":"Loop gate held and the scanner's window is tomorrow."}"#,
            &[cand("SHOP"), cand("ENB")],
            "anthropic/claude-opus-4-8",
        );
        assert_eq!(got.ticker.as_deref(), Some("SHOP"));
        assert!(got.why.contains("Loop gate"));
    }

    #[test]
    fn null_pick_is_honest_none() {
        let got = parse_judgment(
            "2026-08-24",
            r#"{"pick":null,"why":"Both names look stretched into the close."}"#,
            &[cand("SHOP")],
            "anthropic/claude-opus-4-8",
        );
        assert!(got.ticker.is_none());
        assert!(got.why.contains("stretched"));
    }

    #[test]
    fn unparseable_is_none() {
        let got = parse_judgment("2026-08-24", "sure, buy everything", &[cand("SHOP")], "x");
        assert!(got.ticker.is_none());
        assert!(got.why.contains("No pick invented"));
    }
}
