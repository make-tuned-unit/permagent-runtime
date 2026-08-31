//! Public API catalog — toggleable data sources for agents.
//!
//! The [public-apis](https://github.com/public-apis/public-apis) README is a
//! curated list, not a feed. Nothing is fetched until the user enables a
//! source. Enabled sources are callable via `public_api_call`; the Orchestrator
//! may call any of them, and suggested specialist agents are named per
//! category so they pick the source up on the next turn.

use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::Config;

pub const ENABLED_KEY: &str = "public_apis_enabled";
const CALL_TIMEOUT: Duration = Duration::from_secs(12);
const MAX_BODY: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntry {
    pub slug: String,
    pub name: String,
    pub category: String,
    pub description: String,
    pub auth: String,
    pub https: bool,
    pub cors: String,
    pub docs_url: String,
    pub suggested_agents: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CategoryView {
    pub name: String,
    pub count: usize,
    pub suggested_agents: Vec<String>,
}

pub fn suggested_agents_for(category: &str) -> Vec<String> {
    let specialists: &[&str] = match category.trim().to_ascii_lowercase().as_str() {
        "finance" | "cryptocurrency" | "currency exchange" | "blockchain" => {
            &["financier", "forecaster"]
        }
        "weather" | "environment" | "geocoding" => &["forecaster"],
        "news" | "social" | "government" | "open data" | "science & math" | "patent" => {
            &["librarian"]
        }
        "security" | "anti-malware" => &["strix"],
        "programming"
        | "development"
        | "continuous integration"
        | "machine learning"
        | "open source projects" => &["permagent", "reviewer"],
        "health" | "food & drink" | "animals" | "sports & fitness" => &["orchestrator"],
        _ => &[],
    };
    let mut out = vec!["orchestrator".to_string()];
    for s in specialists {
        if *s != "orchestrator" {
            out.push((*s).to_string());
        }
    }
    out
}

fn catalog_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vendor/public-apis/README.md")
}

static CATALOG: OnceLock<Vec<CatalogEntry>> = OnceLock::new();

pub fn catalog() -> &'static [CatalogEntry] {
    CATALOG.get_or_init(|| {
        let text = std::fs::read_to_string(catalog_path()).unwrap_or_default();
        parse_readme(&text)
    })
}

pub fn categories() -> Vec<CategoryView> {
    let mut out: Vec<CategoryView> = Vec::new();
    for e in catalog() {
        match out.iter_mut().find(|c| c.name == e.category) {
            Some(c) => c.count += 1,
            None => out.push(CategoryView {
                name: e.category.clone(),
                count: 1,
                suggested_agents: suggested_agents_for(&e.category),
            }),
        }
    }
    out
}

pub fn find(slug: &str) -> Option<&'static CatalogEntry> {
    catalog().iter().find(|e| e.slug == slug)
}

pub fn enabled_slugs() -> Vec<String> {
    if let Ok(v) = Config::global().get_param::<Vec<String>>(ENABLED_KEY) {
        return v.into_iter().filter(|s| !s.is_empty()).collect();
    }
    if let Ok(s) = Config::global().get_param::<String>(ENABLED_KEY) {
        return s
            .split([',', '\n'])
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();
    }
    Vec::new()
}

pub fn enabled_entries() -> Vec<CatalogEntry> {
    let slugs = enabled_slugs();
    catalog()
        .iter()
        .filter(|e| slugs.iter().any(|s| s == &e.slug))
        .cloned()
        .collect()
}

/// The primary / Orchestrator session, or an explicit `orchestrator` worker.
pub fn agent_is_orchestrator(agent_key: Option<&str>) -> bool {
    match agent_key.map(str::trim) {
        None | Some("") => true,
        Some(key) => key.eq_ignore_ascii_case("orchestrator"),
    }
}

/// Whether this agent should receive enabled public data sources.
///
/// The Orchestrator always does. Specialists match the catalog's suggested
/// agents (Finance → financier, …) even before a source in that category is
/// on, so toggling one later still flows into an already-running session.
pub fn agent_is_data_source_consumer(agent_key: &str) -> bool {
    if agent_is_orchestrator(Some(agent_key)) {
        return true;
    }
    let key = agent_key.trim();
    categories().iter().any(|c| {
        c.suggested_agents
            .iter()
            .any(|a| a.eq_ignore_ascii_case(key))
    })
}

/// Enabled sources this agent may call. The Orchestrator sees every enabled
/// source; a specialist sees the ones whose `suggested_agents` include them.
pub fn visible_entries_for(agent_key: Option<&str>) -> Vec<CatalogEntry> {
    visible_from_entries(&enabled_entries(), agent_key)
}

pub fn visible_from_entries(
    enabled: &[CatalogEntry],
    agent_key: Option<&str>,
) -> Vec<CatalogEntry> {
    if agent_is_orchestrator(agent_key) {
        return enabled.to_vec();
    }
    let key = agent_key.unwrap_or("").trim();
    enabled
        .iter()
        .filter(|e| {
            e.suggested_agents
                .iter()
                .any(|a| a.eq_ignore_ascii_case(key))
        })
        .cloned()
        .collect()
}

pub fn is_public_apis_extension(name: &str) -> bool {
    name.eq_ignore_ascii_case("public_apis")
}

pub fn set_enabled(slug: &str, on: bool) -> Result<Vec<String>, String> {
    find(slug).ok_or_else(|| format!("unknown data source `{slug}`"))?;
    let mut slugs = enabled_slugs();
    if on {
        if !slugs.iter().any(|s| s == slug) {
            slugs.push(slug.to_string());
        }
    } else {
        slugs.retain(|s| s != slug);
    }
    Config::global()
        .set_param(ENABLED_KEY, &slugs)
        .map_err(|e| e.to_string())?;
    Ok(slugs)
}

pub fn secret_key(slug: &str) -> String {
    format!(
        "PUBLIC_API_{}_KEY",
        slug.replace('-', "_").to_ascii_uppercase()
    )
}

pub fn has_key(slug: &str) -> bool {
    Config::global()
        .get_secret::<String>(&secret_key(slug))
        .ok()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
}

pub fn parse_readme(text: &str) -> Vec<CatalogEntry> {
    let mut category = String::new();
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("### ") {
            let name = rest.trim();
            if name.eq_ignore_ascii_case("index")
                || name.to_ascii_lowercase().contains("apis covered")
            {
                continue;
            }
            category = name.to_string();
            continue;
        }
        if category.is_empty() {
            continue;
        }
        if let Some(entry) = parse_row(line, &category) {
            out.push(entry);
        }
    }
    out
}

fn parse_row(line: &str, category: &str) -> Option<CatalogEntry> {
    if !line.starts_with('|') || line.contains("---") {
        return None;
    }
    if line.contains("API | Description") || line.contains("Auth | HTTPS") {
        return None;
    }
    let cells: Vec<&str> = line
        .split('|')
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .collect();
    if cells.len() < 4 {
        return None;
    }
    let (name, docs_url) = parse_link(cells[0])?;
    let description = cells[1].trim().to_string();
    if description.eq_ignore_ascii_case("description") {
        return None;
    }
    let auth = cells[2].trim().trim_matches('`').trim().to_string();
    let https = cells[3].eq_ignore_ascii_case("yes");
    let cors = cells.get(4).copied().unwrap_or("Unknown").to_string();
    Some(CatalogEntry {
        slug: slugify(&name),
        name,
        category: category.to_string(),
        description,
        auth,
        https,
        cors,
        docs_url,
        suggested_agents: suggested_agents_for(category),
    })
}

fn parse_link(cell: &str) -> Option<(String, String)> {
    let start = cell.find('[')?;
    let mid = cell.find("](")?;
    let end = cell.rfind(')')?;
    if mid <= start || end <= mid + 2 {
        return None;
    }
    let name = cell[start + 1..mid].trim().to_string();
    let url = cell[mid + 2..end].trim().to_string();
    if name.is_empty() || url.is_empty() {
        return None;
    }
    Some((name, url))
}

fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

fn same_host(catalog_url: &str, request_url: &str) -> Result<(), String> {
    let a = reqwest::Url::parse(catalog_url).map_err(|e| e.to_string())?;
    let b = reqwest::Url::parse(request_url).map_err(|e| e.to_string())?;
    if b.scheme() != "https" {
        return Err("only https URLs are allowed".into());
    }
    let ha = a.host_str().unwrap_or("").trim_start_matches("www.");
    let hb = b.host_str().unwrap_or("").trim_start_matches("www.");
    if ha.is_empty() || hb.is_empty() {
        return Err("URL has no host".into());
    }
    if hb == ha || hb.ends_with(&format!(".{ha}")) {
        return Ok(());
    }
    Err(format!(
        "url host `{hb}` does not match the catalog host `{ha}`"
    ))
}

fn host_is_blocked(host: &str) -> bool {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    if h == "localhost" || h.ends_with(".localhost") || h.ends_with(".local") {
        return true;
    }
    if let Ok(ip) = h.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(v4) => {
                v4.is_loopback()
                    || v4.is_private()
                    || v4.is_link_local()
                    || v4.is_unspecified()
                    || v4.is_broadcast()
            }
            IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified(),
        };
    }
    false
}

/// GET an enabled source. `url` must share the catalog host. Orchestrator and
/// every suggested agent may call any enabled slug.
pub async fn call(slug: &str, url: Option<&str>) -> Result<String, String> {
    let entry = find(slug).ok_or_else(|| format!("unknown data source `{slug}`"))?;
    if !enabled_slugs().iter().any(|s| s == slug) {
        return Err(format!(
            "`{}` is off. Enable it under Settings → Data sources.",
            entry.name
        ));
    }
    if entry.auth.eq_ignore_ascii_case("oauth") {
        return Err(format!(
            "{} uses OAuth, which is not wired yet. Pick a No-auth or apiKey source.",
            entry.name
        ));
    }
    let target = url.unwrap_or(entry.docs_url.as_str());
    same_host(&entry.docs_url, target)?;
    let parsed = reqwest::Url::parse(target).map_err(|e| e.to_string())?;
    let host = parsed.host_str().unwrap_or("");
    if host_is_blocked(host) {
        return Err(format!(
            "refusing to fetch a private/loopback host ({host})"
        ));
    }
    crate::agents::platform_extensions::browser::guard_fetch_host(target)?;

    let mut req = reqwest::Client::builder()
        .timeout(CALL_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()
        .map_err(|e| e.to_string())?
        .get(target);
    if entry.auth.eq_ignore_ascii_case("apikey") {
        if let Ok(key) = Config::global().get_secret::<String>(&secret_key(slug)) {
            let key = key.trim();
            if !key.is_empty() {
                req = req.header("Authorization", format!("Bearer {key}"));
            }
        } else {
            return Err(format!(
                "{} needs an API key. Add it on Settings → Data sources.",
                entry.name
            ));
        }
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    let clipped = if bytes.len() > MAX_BODY {
        &bytes[..MAX_BODY]
    } else {
        &bytes
    };
    let body = String::from_utf8_lossy(clipped);
    if !status.is_success() {
        return Err(format!("{} answered {status}: {}", entry.name, body.trim()));
    }
    Ok(body.into_owned())
}

pub fn instructions_for_enabled() -> String {
    instructions_for_agent(None)
}

/// Live list of enabled sources for this agent. Rebuilt every turn so a
/// Settings toggle is callable on the next tool call — nothing is snapshotted
/// at session start.
pub fn instructions_for_agent(agent_key: Option<&str>) -> String {
    instructions_from_entries(&enabled_entries(), agent_key)
}

pub fn instructions_from_entries(enabled: &[CatalogEntry], agent_key: Option<&str>) -> String {
    let visible = visible_from_entries(enabled, agent_key);
    let orchestrator = agent_is_orchestrator(agent_key);
    if visible.is_empty() {
        return if orchestrator {
            "No public data sources are enabled. The user turns them on under Settings → Data sources. Once enabled they flow to you immediately — call them with public_api_call. Do not invent a call.".into()
        } else {
            format!(
                "No public data sources in your domain are enabled yet. When the user turns one on under Settings → Data sources it flows to you on the next turn. Call it with public_api_call. Do not invent a call."
            )
        };
    }
    let mut lines = if orchestrator {
        vec![
            "Enabled public data sources — you (the Orchestrator) may call ANY of them with public_api_call. Suggested specialist agents are named on each row; those agents also have the matching sources. A source the user just turned on is callable now.".to_string(),
        ]
    } else {
        vec![
            "Enabled public data sources flowing to you. Call them with public_api_call. The Orchestrator can call every enabled source, including ones outside this list.".to_string(),
        ]
    };
    for e in visible.iter().take(40) {
        lines.push(format!(
            "- {} (`{}`, {}) — {} — suggested: {}",
            e.name,
            e.slug,
            e.category,
            e.description,
            e.suggested_agents.join(", ")
        ));
    }
    if visible.len() > 40 {
        lines.push(format!(
            "…and {} more. Call public_api_list for the rest.",
            visible.len() - 40
        ));
    }
    lines.join("\n")
}

/// Append the live data-source block to a worker persona so a dispatched
/// subagent (which does not rebuild the main system prompt) still sees the
/// sources that flow to it. No-ops for workers that are not data-source
/// consumers (external CLIs, etc.).
pub fn attach_to_persona_block(block: &str, agent_key: Option<&str>) -> String {
    if let Some(key) = agent_key {
        if !agent_is_data_source_consumer(key) {
            return block.to_string();
        }
    }
    format!(
        "{}\n\n{}",
        block.trim_end(),
        instructions_for_agent(agent_key)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"
### Finance
API | Description | Auth | HTTPS | CORS |
|---|:---|:---|:---|:---|
| [Alpha Vantage](https://www.alphavantage.co/) | Realtime stock data | `apiKey` | Yes | Unknown |
| [Fed Treasury](https://fiscaldata.treasury.gov/api-documentation/) | U.S. Treasury data | No | Yes | Unknown |

### Weather
| [Open-Meteo](https://open-meteo.com/) | Weather forecast | No | Yes | Yes |
"#;

    #[test]
    fn parse_readme_rows_and_categories() {
        let got = parse_readme(FIXTURE);
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].slug, "alpha-vantage");
        assert_eq!(got[0].category, "Finance");
        assert_eq!(got[0].auth, "apiKey");
        assert!(got[0].https);
        assert!(got[0].suggested_agents.contains(&"financier".to_string()));
        assert!(got[0]
            .suggested_agents
            .contains(&"orchestrator".to_string()));
        assert_eq!(got[2].category, "Weather");
        assert!(got[2].suggested_agents.contains(&"forecaster".to_string()));
    }

    #[test]
    fn same_host_allows_subdomain_and_https_only() {
        same_host(
            "https://www.alphavantage.co/",
            "https://www.alphavantage.co/query?fn=TIME",
        )
        .unwrap();
        same_host(
            "https://open-meteo.com/",
            "https://api.open-meteo.com/v1/forecast",
        )
        .unwrap();
        assert!(same_host("https://open-meteo.com/", "http://open-meteo.com/v1").is_err());
        assert!(same_host("https://open-meteo.com/", "https://evil.example/steal").is_err());
    }

    #[test]
    fn host_blocks_loopback_and_private() {
        assert!(host_is_blocked("127.0.0.1"));
        assert!(host_is_blocked("localhost"));
        assert!(host_is_blocked("10.0.0.5"));
        assert!(host_is_blocked("169.254.169.254"));
        assert!(!host_is_blocked("open-meteo.com"));
    }

    #[test]
    fn secret_key_is_env_shaped() {
        assert_eq!(secret_key("alpha-vantage"), "PUBLIC_API_ALPHA_VANTAGE_KEY");
    }

    #[test]
    fn vendored_readme_is_the_catalog() {
        let got = catalog();
        assert!(
            got.len() > 80,
            "expected the public-apis README to parse, got {}",
            got.len()
        );
        assert!(got.iter().any(|e| e.slug == "alpha-vantage"));
        assert!(got.iter().any(|e| e.category == "Finance"));
    }

    #[test]
    fn orchestrator_sees_every_enabled_source() {
        let enabled = parse_readme(FIXTURE);
        let for_henry = visible_from_entries(&enabled, None);
        assert_eq!(for_henry.len(), 3);
        let named = visible_from_entries(&enabled, Some("orchestrator"));
        assert_eq!(named.len(), 3);
    }

    #[test]
    fn specialist_only_sees_sources_suggested_for_them() {
        let enabled = parse_readme(FIXTURE);
        let finance = visible_from_entries(&enabled, Some("financier"));
        assert!(finance
            .iter()
            .all(|e| e.suggested_agents.iter().any(|a| a == "financier")));
        assert!(finance.iter().any(|e| e.slug == "alpha-vantage"));
        let weather = visible_from_entries(&enabled, Some("forecaster"));
        assert!(weather.iter().any(|e| e.slug == "open-meteo"));
        assert!(weather.iter().any(|e| e.slug == "alpha-vantage"));
        let librarian = visible_from_entries(&enabled, Some("librarian"));
        assert!(librarian.is_empty());
    }

    #[test]
    fn data_source_consumers_include_suggested_agents_and_orchestrator() {
        assert!(agent_is_data_source_consumer("orchestrator"));
        assert!(agent_is_data_source_consumer("financier"));
        assert!(agent_is_data_source_consumer("forecaster"));
        assert!(agent_is_data_source_consumer("librarian"));
        assert!(agent_is_data_source_consumer("permagent"));
        assert!(!agent_is_data_source_consumer("cursor"));
        assert!(!agent_is_data_source_consumer("codex"));
    }

    #[test]
    fn instructions_name_visible_sources_and_stay_live() {
        let enabled = parse_readme(FIXTURE);
        let orch = instructions_from_entries(&enabled, None);
        assert!(orch.contains("alpha-vantage"));
        assert!(orch.contains("open-meteo"));
        assert!(orch.contains("you (the Orchestrator) may call ANY"));
        let fin = instructions_from_entries(&enabled, Some("financier"));
        assert!(fin.contains("alpha-vantage"));
        assert!(!fin.contains("open-meteo"));
        assert!(fin.contains("flowing to you"));
        let none = instructions_from_entries(&[], None);
        assert!(none.contains("No public data sources are enabled"));
    }

    #[test]
    fn persona_block_carries_the_live_list() {
        let block = attach_to_persona_block("You are Financier.", Some("financier"));
        assert!(block.contains("You are Financier."));
        assert!(block.contains("public_api_call"));
        let cli = attach_to_persona_block("You are Claude Code.", Some("claude_code"));
        assert_eq!(cli, "You are Claude Code.");
    }
}
