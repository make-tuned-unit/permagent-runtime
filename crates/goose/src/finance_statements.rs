//! Household-statement ingest — CSV / OFX / QFX first, OCR text as fallback.
//!
//! CSV wins over a paraphrase of the same period. Amounts are never invented:
//! a line that does not parse as a date + amount is skipped, not guessed.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// Locked category set. Uncategorized is the honest default.
pub const CATEGORIES: &[&str] = &[
    "housing",
    "groceries",
    "transport",
    "utilities",
    "dining",
    "health",
    "subscriptions",
    "income",
    "transfer",
    "uncategorized",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ParsedTxn {
    pub date: String,
    pub amount: f64,
    pub payee: String,
    pub category: String,
}

/// Parse a dropped statement. CSV/OFX/QFX by bytes; anything else is treated
/// as already-extracted text (Reader OCR / PDF text layer).
pub fn parse_statement(
    filename: &str,
    mime: &str,
    bytes: &[u8],
    ocr_text: Option<&str>,
) -> Result<Vec<ParsedTxn>, String> {
    let name = filename.to_lowercase();
    if name.ends_with(".csv") || mime.contains("csv") {
        let text = std::str::from_utf8(bytes).map_err(|_| "CSV is not UTF-8".to_string())?;
        return parse_csv(text);
    }
    if name.ends_with(".ofx")
        || name.ends_with(".qfx")
        || mime.contains("ofx")
        || mime.contains("quickbooks")
    {
        let text = std::str::from_utf8(bytes).unwrap_or("");
        let rows = parse_ofx(text);
        if rows.is_empty() {
            return Err("no OFX transactions were readable".into());
        }
        return Ok(rows);
    }
    if let Some(text) = ocr_text {
        let rows = parse_ocr_lines(text);
        if rows.is_empty() {
            return Err("OCR found no date+amount rows".into());
        }
        return Ok(rows);
    }
    // Last resort: treat the bytes as text.
    let text = std::str::from_utf8(bytes).unwrap_or("");
    let rows = parse_ocr_lines(text);
    if rows.is_empty() {
        return Err("could not parse this statement as CSV, OFX, or a date+amount list".into());
    }
    Ok(rows)
}

pub fn parse_csv(text: &str) -> Result<Vec<ParsedTxn>, String> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header = lines.next().ok_or("CSV is empty")?;
    let cols: Vec<String> = split_csv(header)
        .into_iter()
        .map(|s| s.to_lowercase())
        .collect();
    let date_i = find_col(&cols, &["date", "posted", "transaction date", "txn date"]);
    let desc_i = find_col(
        &cols,
        &[
            "description",
            "payee",
            "name",
            "memo",
            "details",
            "narrative",
        ],
    );
    let amount_i = find_col(&cols, &["amount", "cad", "usd", "value"]);
    let debit_i = find_col(&cols, &["debit", "withdrawal", "withdrawals"]);
    let credit_i = find_col(&cols, &["credit", "deposit", "deposits"]);
    if date_i.is_none() {
        return Err("CSV has no Date column".into());
    }
    if amount_i.is_none() && debit_i.is_none() && credit_i.is_none() {
        return Err("CSV has no Amount / Debit / Credit column".into());
    }
    let mut out = Vec::new();
    for line in lines {
        let cells = split_csv(line);
        let Some(date) = date_i
            .and_then(|i| cells.get(i).map(|s| s.as_str()))
            .and_then(parse_date)
        else {
            continue;
        };
        let payee = desc_i
            .and_then(|i| cells.get(i))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "unknown".into());
        let amount = if let Some(i) = amount_i {
            cells.get(i).and_then(|s| parse_amount(s))
        } else {
            let debit = debit_i
                .and_then(|i| cells.get(i))
                .and_then(|s| parse_amount(s))
                .unwrap_or(0.0);
            let credit = credit_i
                .and_then(|i| cells.get(i))
                .and_then(|s| parse_amount(s))
                .unwrap_or(0.0);
            if debit == 0.0 && credit == 0.0 {
                None
            } else {
                Some(credit - debit.abs())
            }
        };
        let Some(amount) = amount else { continue };
        if amount == 0.0 {
            continue;
        }
        let category = categorize(&payee, amount);
        out.push(ParsedTxn {
            date,
            amount,
            payee,
            category,
        });
    }
    if out.is_empty() {
        return Err("CSV had a header but no readable transactions".into());
    }
    Ok(out)
}

pub fn parse_ofx(text: &str) -> Vec<ParsedTxn> {
    let mut out = Vec::new();
    for block in text.split("<STMTTRN") {
        if !block.contains("<TRNAMT>") {
            continue;
        }
        let amount = tag(block, "TRNAMT").and_then(|s| parse_amount(&s));
        let date_raw = tag(block, "DTPOSTED").or_else(|| tag(block, "DTUSER"));
        let date = date_raw.as_deref().and_then(parse_ofx_date);
        let payee = tag(block, "NAME")
            .or_else(|| tag(block, "MEMO"))
            .unwrap_or_else(|| "unknown".into());
        let (Some(amount), Some(date)) = (amount, date) else {
            continue;
        };
        if amount == 0.0 {
            continue;
        }
        let category = categorize(&payee, amount);
        out.push(ParsedTxn {
            date,
            amount,
            payee,
            category,
        });
    }
    out
}

/// OCR / pasted text: a line that contains a date and an amount.
pub fn parse_ocr_lines(text: &str) -> Vec<ParsedTxn> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.len() < 8 {
            continue;
        }
        let Some(date) = find_date(line) else {
            continue;
        };
        let Some(amount) = find_amount(line) else {
            continue;
        };
        if amount == 0.0 {
            continue;
        }
        let payee = strip_date_and_amount(line, &date)
            .trim()
            .trim_matches(|c: char| c == '|' || c == ',' || c == '-')
            .to_string();
        let payee = if payee.is_empty() {
            "unknown".into()
        } else {
            payee
        };
        let category = categorize(&payee, amount);
        out.push(ParsedTxn {
            date,
            amount,
            payee,
            category,
        });
    }
    out
}

pub fn categorize(payee: &str, amount: f64) -> String {
    if amount > 0.0 {
        let p = payee.to_lowercase();
        if p.contains("e-transfer") || p.contains("transfer") || p.contains("venmo") {
            return "transfer".into();
        }
        return "income".into();
    }
    let p = payee.to_lowercase();
    const RULES: &[(&str, &str)] = &[
        ("rent", "housing"),
        ("mortgage", "housing"),
        ("landlord", "housing"),
        ("grocery", "groceries"),
        ("sobeys", "groceries"),
        ("superstore", "groceries"),
        ("walmart", "groceries"),
        ("costco", "groceries"),
        ("no frills", "groceries"),
        ("uber", "transport"),
        ("lyft", "transport"),
        ("petro", "transport"),
        ("shell", "transport"),
        ("esso", "transport"),
        ("parking", "transport"),
        ("nslc", "dining"),
        ("restaurant", "dining"),
        ("coffee", "dining"),
        ("starbucks", "dining"),
        ("tim hortons", "dining"),
        ("doordash", "dining"),
        ("skip", "dining"),
        ("nspower", "utilities"),
        ("ns power", "utilities"),
        ("hydro", "utilities"),
        ("bell", "utilities"),
        ("rogers", "utilities"),
        ("eastlink", "utilities"),
        ("netflix", "subscriptions"),
        ("spotify", "subscriptions"),
        ("apple.com/bill", "subscriptions"),
        ("pharmacy", "health"),
        ("shoppers", "health"),
        ("clinic", "health"),
        ("transfer", "transfer"),
        ("e-transfer", "transfer"),
    ];
    for (needle, cat) in RULES {
        if p.contains(needle) {
            return (*cat).into();
        }
    }
    "uncategorized".into()
}

fn split_csv(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    for c in line.chars() {
        match c {
            '"' => in_q = !in_q,
            ',' if !in_q => {
                out.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    out.push(cur.trim().to_string());
    out
}

fn find_col(cols: &[String], names: &[&str]) -> Option<usize> {
    cols.iter()
        .position(|c| names.iter().any(|n| c == n || c.contains(n)))
}

fn parse_amount(raw: &str) -> Option<f64> {
    let s = raw.trim().replace(['$', ',', ' '], "");
    let s = s.replace('(', "-").replace(')', "");
    if s.is_empty() {
        return None;
    }
    s.parse().ok()
}

fn parse_date(raw: &str) -> Option<String> {
    let s = raw.trim();
    for fmt in [
        "%Y-%m-%d",
        "%Y/%m/%d",
        "%d/%m/%Y",
        "%m/%d/%Y",
        "%d-%m-%Y",
        "%b %d, %Y",
        "%d %b %Y",
    ] {
        if let Ok(d) = NaiveDate::parse_from_str(s, fmt) {
            return Some(d.format("%Y-%m-%d").to_string());
        }
    }
    find_date(s)
}

fn parse_ofx_date(raw: &str) -> Option<String> {
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).take(8).collect();
    if digits.len() == 8 {
        return NaiveDate::parse_from_str(&digits, "%Y%m%d")
            .ok()
            .map(|d| d.format("%Y-%m-%d").to_string());
    }
    None
}

fn find_date(line: &str) -> Option<String> {
    let re_parts = [
        (r"\b(\d{4}-\d{2}-\d{2})\b", "%Y-%m-%d"),
        (r"\b(\d{2}/\d{2}/\d{4})\b", "%d/%m/%Y"),
        (r"\b(\d{4}/\d{2}/\d{2})\b", "%Y/%m/%d"),
    ];
    for (pat, fmt) in re_parts {
        if let Some(cap) = capture(line, pat) {
            if let Ok(d) = NaiveDate::parse_from_str(&cap, fmt) {
                return Some(d.format("%Y-%m-%d").to_string());
            }
            // US-style fallback for xx/xx/yyyy if the first parse failed.
            if fmt == "%d/%m/%Y" {
                if let Ok(d) = NaiveDate::parse_from_str(&cap, "%m/%d/%Y") {
                    return Some(d.format("%Y-%m-%d").to_string());
                }
            }
        }
    }
    None
}

fn find_amount(line: &str) -> Option<f64> {
    // Last money-shaped token on the line.
    let mut last = None;
    let mut buf = String::new();
    for c in line.chars() {
        if c.is_ascii_digit() || matches!(c, '.' | ',' | '-' | '$' | '(' | ')') {
            buf.push(c);
        } else if !buf.is_empty() {
            if let Some(n) = parse_amount(&buf) {
                last = Some(n);
            }
            buf.clear();
        }
    }
    if !buf.is_empty() {
        if let Some(n) = parse_amount(&buf) {
            last = Some(n);
        }
    }
    last
}

fn strip_date_and_amount(line: &str, date: &str) -> String {
    let mut s = line.replace(date, "");
    // Also strip slash dates that parsed into ISO.
    s = s
        .split_whitespace()
        .filter(|w| parse_amount(w).is_none() && find_date(w).is_none())
        .collect::<Vec<_>>()
        .join(" ");
    s
}

fn tag(block: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let i = block.find(&open)?;
    let rest = &block[i + open.len()..];
    let end = rest
        .find('<')
        .or_else(|| rest.find('\n'))
        .unwrap_or(rest.len());
    let v = rest[..end].trim();
    if v.is_empty() {
        None
    } else {
        Some(v.to_string())
    }
}

/// Tiny capture without pulling regex into this crate's public graph: first
/// group of a very small date pattern. Hand-rolled for ISO and slash dates.
fn capture(line: &str, pat: &str) -> Option<String> {
    match pat {
        r"\b(\d{4}-\d{2}-\d{2})\b" => find_iso(line),
        r"\b(\d{2}/\d{2}/\d{4})\b" => find_slash(line, 2, 4),
        r"\b(\d{4}/\d{2}/\d{2})\b" => find_ymd_slash(line),
        _ => None,
    }
}

fn find_iso(s: &str) -> Option<String> {
    let b = s.as_bytes();
    for i in 0..b.len().saturating_sub(9) {
        if b[i].is_ascii_digit()
            && b[i + 1].is_ascii_digit()
            && b[i + 2].is_ascii_digit()
            && b[i + 3].is_ascii_digit()
            && b[i + 4] == b'-'
            && b[i + 7] == b'-'
        {
            return Some(s[i..i + 10].to_string());
        }
    }
    None
}

fn find_slash(s: &str, _a: usize, _y: usize) -> Option<String> {
    let b = s.as_bytes();
    for i in 0..b.len().saturating_sub(9) {
        if b[i].is_ascii_digit()
            && b[i + 1].is_ascii_digit()
            && b[i + 2] == b'/'
            && b[i + 5] == b'/'
        {
            return Some(s[i..i + 10].to_string());
        }
    }
    None
}

fn find_ymd_slash(s: &str) -> Option<String> {
    let b = s.as_bytes();
    for i in 0..b.len().saturating_sub(9) {
        if b[i].is_ascii_digit() && b[i + 4] == b'/' && b[i + 7] == b'/' {
            return Some(s[i..i + 10].to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_debit_credit_and_amount_columns() {
        let text = "Date,Description,Amount\n2026-01-15,Sobeys Halifax,-86.40\n2026-01-16,PAYROLL,2400.00\n";
        let rows = parse_csv(text).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].category, "groceries");
        assert_eq!(rows[1].category, "income");
    }

    #[test]
    fn ofx_reads_stmttrn() {
        let text = r#"
<STMTTRN>
<TRNTYPE>DEBIT
<DTPOSTED>20260115
<TRNAMT>-42.00
<NAME>TIM HORTONS
</STMTTRN>
"#;
        let rows = parse_ofx(text);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].date, "2026-01-15");
        assert_eq!(rows[0].category, "dining");
    }

    #[test]
    fn ocr_line_with_date_and_amount() {
        let rows = parse_ocr_lines("2026-03-02  NS POWER  -$128.11\nnot a txn\n");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].category, "utilities");
        assert!((rows[0].amount + 128.11).abs() < 0.01);
    }
}
