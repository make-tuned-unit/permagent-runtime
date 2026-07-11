use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::agents::tool_execution::ToolCallContext;
use async_trait::async_trait;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, JsonObject, ListToolsResult,
    ServerCapabilities, Tool,
};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

pub static EXTENSION_NAME: &str = "browser";

#[derive(Debug, Serialize, Deserialize)]
struct PageContent {
    title: String,
    url: String,
    content: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    truncated: bool,
}

pub struct BrowserClient {
    info: InitializeResult,
}

/// Accept bare domains ("bbc.com") by assuming https; refuse non-web schemes.
fn normalize_web_url(input: &str) -> Result<String, String> {
    let url = input.trim();
    if url.is_empty() {
        return Err("No URL given".to_string());
    }
    if url.starts_with("https://") || url.starts_with("http://") {
        return Ok(url.to_string());
    }
    if url.contains("://") {
        return Err(format!("Only http(s) URLs are supported, got: {url}"));
    }
    Ok(format!("https://{url}"))
}

/// SSRF guard for server-side fetches: refuse loopback, private-range, and
/// link-local hosts so the agent cannot be steered into internal services.
fn guard_public_host(url: &str) -> Result<(), String> {
    let host = url
        .split("://")
        .nth(1)
        .and_then(|rest| rest.split(['/', '?', '#']).next())
        .map(|authority| authority.rsplit('@').next().unwrap_or(authority))
        .map(|hostport| {
            // Strip :port — careful with IPv6 brackets.
            if let Some(stripped) = hostport.strip_prefix('[') {
                stripped.split(']').next().unwrap_or(stripped).to_string()
            } else {
                hostport.split(':').next().unwrap_or(hostport).to_string()
            }
        })
        .unwrap_or_default()
        .to_ascii_lowercase();
    if host.is_empty() {
        return Err("URL has no host".to_string());
    }
    let private = host == "localhost"
        || host.ends_with(".local")
        || host == "::1"
        || host.starts_with("127.")
        || host.starts_with("10.")
        || host.starts_with("192.168.")
        || host.starts_with("169.254.")
        || host.starts_with("fe80:")
        || host.starts_with("fd")
        || (host.starts_with("172.")
            && host
                .split('.')
                .nth(1)
                .and_then(|o| o.parse::<u8>().ok())
                .is_some_and(|o| (16..=31).contains(&o)));
    if private {
        return Err(format!(
            "Refusing to fetch a private/loopback host ({host}) — read_webpage is for public \
             websites"
        ));
    }
    Ok(())
}

/// `<title>` of an HTML document, entity-light.
// string_slice: every index comes from `find()` on the same string, which
// always returns char boundaries — the lint's mid-UTF-8 panic cannot occur.
#[allow(clippy::string_slice)]
fn extract_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let open_end = html[start..].find('>')? + start + 1;
    let close = lower[open_end..].find("</title")? + open_end;
    let title = decode_entities(html[open_end..close].trim());
    (!title.is_empty()).then_some(title)
}

/// Small hand-rolled HTML→text: drops script/style/head/nav/noscript bodies,
/// turns tags into whitespace (block tags into newlines), decodes the common
/// entities, and collapses runs of blank space. Not a browser — good enough
/// to read a news homepage aloud without a rendering engine.
// string_slice: all indices are `find()` results on the same string (char
// boundaries by contract) or advance tag-by-tag from them.
#[allow(clippy::string_slice)]
fn html_to_text(html: &str) -> String {
    const SKIP: &[&str] = &["script", "style", "noscript", "head", "svg", "template"];
    const BLOCK: &[&str] = &[
        "p",
        "div",
        "br",
        "li",
        "ul",
        "ol",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "tr",
        "table",
        "section",
        "article",
        "header",
        "footer",
        "figcaption",
    ];
    let mut out = String::with_capacity(html.len() / 4);
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let rest = &html[i + 1..];
            let name_end = rest
                .find(|c: char| !c.is_ascii_alphanumeric())
                .unwrap_or(rest.len());
            let name = rest[..name_end].to_ascii_lowercase();
            if SKIP.contains(&name.as_str()) {
                // Skip to the matching close tag (first occurrence is fine here).
                let close = format!("</{name}");
                if let Some(end) = html[i..].to_ascii_lowercase().find(&close) {
                    let after = i + end;
                    i = html[after..]
                        .find('>')
                        .map_or(html.len(), |g| after + g + 1);
                    continue;
                }
            }
            if BLOCK.contains(&name.as_str()) || (name.is_empty() && rest.starts_with('/')) {
                out.push('\n');
            } else {
                out.push(' ');
            }
            i = html[i..].find('>').map_or(html.len(), |g| i + g + 1);
        } else {
            let next_tag = html[i..].find('<').map_or(html.len(), |t| i + t);
            out.push_str(&html[i..next_tag]);
            i = next_tag;
        }
    }
    let decoded = decode_entities(&out);
    // Collapse whitespace: runs of spaces → one, >2 newlines → 2.
    let mut clean = String::with_capacity(decoded.len());
    let mut spaces = 0usize;
    let mut newlines = 0usize;
    for ch in decoded.chars() {
        match ch {
            '\n' => {
                newlines += 1;
                spaces = 0;
                if newlines <= 2 {
                    clean.push('\n');
                }
            }
            c if c.is_whitespace() => {
                spaces += 1;
                if spaces <= 1 && newlines == 0 {
                    clean.push(' ');
                } else if newlines > 0 {
                    // swallow indentation after newlines
                }
            }
            c => {
                spaces = 0;
                newlines = 0;
                clean.push(c);
            }
        }
    }
    clean.trim().to_string()
}

fn decode_entities(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&rsquo;", "\u{2019}")
        .replace("&mdash;", "\u{2014}")
        .replace("&ndash;", "\u{2013}")
}

impl BrowserClient {
    pub fn new(_context: PlatformExtensionContext) -> Result<Self, anyhow::Error> {
        let info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new(EXTENSION_NAME.to_string(), "1.0.0".to_string())
                    .with_title("Browser"),
            );
        Ok(Self { info })
    }

    async fn handle_read_browser_content(&self) -> Result<Vec<Content>, String> {
        let client = reqwest::Client::new();
        let resp = client
            .post("http://127.0.0.1:3001/api/browser/content/read")
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
            .map_err(|e| format!("Failed to request page content: {e}"))?;

        if resp.status() == reqwest::StatusCode::GATEWAY_TIMEOUT {
            return Ok(vec![Content::text(
                "No browser tab is open, or the page content could not be extracted. \
                 Make sure a page is loaded in the Permagent browser.",
            )]);
        }

        if !resp.status().is_success() {
            return Err(format!(
                "Content extraction failed with status {}",
                resp.status()
            ));
        }

        let page: PageContent = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse page content: {e}"))?;

        // Surface distinct failure modes for agent reasoning
        if page.status == "no_tab" {
            return Ok(vec![Content::text(
                "No browser tab is currently open in Permagent. \
                 The user needs to have a page open in the browser for this tool to work.",
            )]);
        }
        if page.status == "error" {
            return Ok(vec![Content::text(format!(
                "Could not read the page content: {}",
                page.content
            ))]);
        }

        let mut text = format!(
            "Page: {}\nURL: {}\n\n{}",
            page.title, page.url, page.content
        );
        if page.truncated {
            text.push_str("\n\nNote: This page was long and the content above is truncated.");
        }

        Ok(vec![Content::text(text)])
    }

    /// #567: ask the in-app browser to open a URL. Fire-and-forget via the
    /// daemon route, which validates the scheme and emits the navigate event
    /// the frontend bridge listens for.
    async fn handle_open_website(&self, url: &str) -> Result<Vec<Content>, String> {
        let url = normalize_web_url(url)?;
        let client = reqwest::Client::new();
        let resp = client
            .post("http://127.0.0.1:3001/api/browser/navigate")
            .timeout(std::time::Duration::from_secs(5))
            .json(&serde_json::json!({ "url": url }))
            .send()
            .await
            .map_err(|e| format!("Failed to reach the browser bridge: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("Browser navigate rejected: {}", resp.status()));
        }
        Ok(vec![Content::text(format!(
            "Opened {url} in the Permagent browser (Build tab). Use read_webpage to read its \
             content."
        ))])
    }

    /// Fetch a public web page server-side and return its readable text —
    /// works even when no browser tab is open, and is the reliable path for
    /// "read me the BBC homepage". SSRF-guarded: https/http only, private and
    /// loopback hosts refused, redirects re-checked per hop.
    // string_slice: the truncation walks back to `is_char_boundary` before
    // slicing; the byte-cap slice is length-clamped on raw bytes pre-UTF-8.
    #[allow(clippy::string_slice)]
    async fn handle_read_webpage(&self, url: &str) -> Result<Vec<Content>, String> {
        let url = normalize_web_url(url)?;
        guard_public_host(&url)?;

        let client = reqwest::Client::builder()
            .user_agent("Permagent/1.0 (in-app reader)")
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() > 4 {
                    return attempt.error("too many redirects");
                }
                match guard_public_host(attempt.url().as_str()) {
                    Ok(()) => attempt.follow(),
                    Err(_) => attempt.error("redirect to a non-public host refused"),
                }
            }))
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|e| format!("HTTP client error: {e}"))?;

        let resp = client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Could not fetch {url}: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("{url} answered {}", resp.status()));
        }

        const BYTE_CAP: usize = 2 * 1024 * 1024;
        let body = resp
            .bytes()
            .await
            .map_err(|e| format!("Failed reading the response body: {e}"))?;
        let html = String::from_utf8_lossy(&body[..body.len().min(BYTE_CAP)]);

        let title = extract_title(&html).unwrap_or_else(|| url.clone());
        let text = html_to_text(&html);
        const CHAR_CAP: usize = 24_000;
        let (text, truncated) = if text.len() > CHAR_CAP {
            let mut end = CHAR_CAP;
            while end > 0 && !text.is_char_boundary(end) {
                end -= 1;
            }
            (&text[..end], true)
        } else {
            (text.as_str(), false)
        };

        let mut out = format!("Page: {title}\nURL: {url}\n\n{text}");
        if truncated {
            out.push_str("\n\nNote: the page was long and this text is truncated.");
        }
        Ok(vec![Content::text(out)])
    }

    fn get_tools() -> Vec<Tool> {
        let schema: JsonObject = serde_json::from_value(serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        }))
        .expect("static schema");

        let url_schema: JsonObject = serde_json::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The web address, e.g. https://www.bbc.com or just bbc.com"
                }
            },
            "required": ["url"]
        }))
        .expect("static schema");

        vec![
            Tool::new(
                "read_browser_content".to_string(),
                "Read the visible text content of the page currently open in the Permagent \
                 browser. Returns the page title, URL, and extracted text. Use this when the \
                 user asks about what they're looking at or references their open tab."
                    .to_string(),
                schema,
            ),
            Tool::new(
                "open_website".to_string(),
                "Open a website in the Permagent browser (the Build tab) so the user can see \
                 it. Use when the user says things like 'go to BBC' or 'open cbc.ca'. Pair \
                 with read_webpage when they also want it read to them."
                    .to_string(),
                url_schema.clone(),
            ),
            Tool::new(
                "read_webpage".to_string(),
                "Fetch a public web page and return its readable text — the reliable way to \
                 read a site to the user (e.g. 'read me the BBC homepage'). Works without any \
                 browser tab open. Public http(s) sites only."
                    .to_string(),
                url_schema,
            ),
        ]
    }
}

#[async_trait]
impl McpClientTrait for BrowserClient {
    async fn list_tools(
        &self,
        _session_id: &str,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> Result<ListToolsResult, Error> {
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
    ) -> Result<CallToolResult, Error> {
        let url_arg = || {
            arguments
                .as_ref()
                .and_then(|a| a.get("url"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        };
        match name {
            "open_website" => match self.handle_open_website(&url_arg()).await {
                Ok(content) => Ok(CallToolResult::success(content)),
                Err(e) => Ok(CallToolResult::error(vec![Content::text(e)])),
            },
            "read_webpage" => match self.handle_read_webpage(&url_arg()).await {
                Ok(content) => Ok(CallToolResult::success(content)),
                Err(e) => Ok(CallToolResult::error(vec![Content::text(e)])),
            },
            "read_browser_content" => match self.handle_read_browser_content().await {
                Ok(content) => Ok(CallToolResult::success(content)),
                Err(error) => Ok(CallToolResult::error(vec![Content::text(format!(
                    "Error: {error}"
                ))])),
            },
            _ => Ok(CallToolResult::error(vec![Content::text(format!(
                "Unknown tool: {name}"
            ))])),
        }
    }

    fn get_info(&self) -> Option<&InitializeResult> {
        Some(&self.info)
    }
}
