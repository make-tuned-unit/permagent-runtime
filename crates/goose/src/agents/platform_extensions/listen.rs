//! Audience listening (#634) — the Grow tab's ear.
//!
//! Henry calls `listen_to_audience` to hear what an audience is actually saying
//! about a topic or on a specific channel, so a project's Grow strategy (its
//! Audience and Channels pillars) is grounded in real signal instead of guesses.
//!
//! ## Resilient, ordered backends (the design imported from the Agent-Reach eval)
//!
//! The one idea worth taking from Agent-Reach was its *resilience shape*, not its
//! code (that project is read-only Python). The shape: try a list of backends
//! **in order**, but never trust a backend just because it exists —
//! **health-probe** it first and use the first one that actually answers,
//! degrading gracefully and reporting *which* backend spoke.
//!
//! Here the ordered list is **RSS first** (zero-config, already parseable in our
//! stack via [`crate::rss`], extracted from the Watcher), then **web_search** as
//! the fallback. The RSS backend is health-probed: a feed that errors, 404s, or
//! parses to zero items is treated as *unhealthy* and skipped — never reported
//! as "no signal" when it was really unreachable. Only real fetched items are
//! ever returned; if nothing answers we say so and name the next backend, we
//! never fabricate chatter. Keys and cookies stay local — we only fetch public
//! feeds, with no credentials, and SSRF-guard every host before connecting.

use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::agents::tool_execution::ToolCallContext;
use async_trait::async_trait;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, JsonObject, ListToolsResult,
    ServerCapabilities, Tool,
};
use tokio_util::sync::CancellationToken;

pub static EXTENSION_NAME: &str = "audience_listen";

const TOOL_NAME: &str = "listen_to_audience";
const DEFAULT_LIMIT: usize = 8;
const MAX_LIMIT: usize = 20;
const SNIPPET_CHARS: usize = 220;

pub struct ListenClient {
    info: InitializeResult,
}

/// One backend in the resilient, ordered list. Tried top-to-bottom; the first
/// that passes its health probe answers.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Backend {
    /// An RSS/Atom feed. `label` is human context for the "which backend
    /// answered" report.
    Rss { url: String, label: String },
    /// The graceful fall-through to the agent's own `web_search` tool. Terminal:
    /// this platform extension cannot call another MCP server's provider-specific
    /// search tool by a guessed name, so it hands off honestly instead.
    WebSearch,
}

impl ListenClient {
    pub fn new(_context: PlatformExtensionContext) -> anyhow::Result<Self> {
        let info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new(EXTENSION_NAME.to_string(), "1.0.0".to_string())
                    .with_title("Audience Listening"),
            );
        Ok(Self { info })
    }

    pub(crate) fn get_tools() -> Vec<Tool> {
        let schema: JsonObject = serde_json::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "topic": {
                    "type": "string",
                    "description": "What to listen for — a subject, product, question, or community the audience talks about (e.g. \"local-first software\", \"reactions to my launch\")."
                },
                "source": {
                    "type": "string",
                    "description": "Optional. A specific channel to listen on: an RSS/Atom feed URL (a subreddit's .rss, a blog or news feed, a YouTube channel or podcast feed). Omit to search live news for the topic."
                },
                "limit": {
                    "type": "integer",
                    "description": "Optional. Max items to return (default 8, max 20)."
                }
            },
            "required": ["topic"]
        }))
        .expect("static schema");

        vec![Tool::new(
            TOOL_NAME.to_string(),
            "Listen to what an audience is actually saying about a topic or on a channel, and \
             return the most recent items (title, snippet, date, link). RSS-first and \
             zero-config: give a topic and it reads live news chatter; give a feed URL as \
             `source` to listen to a specific channel (a subreddit, blog, or podcast). Use it \
             to ground a project's Grow strategy — its Audience and Channels — in real signal \
             instead of guesses. Returns only real fetched items and names which backend \
             answered; if a source is unreachable it says so rather than inventing chatter."
                .to_string(),
            schema,
        )]
    }

    async fn handle_listen(&self, arguments: Option<JsonObject>) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let topic = args
            .get("topic")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or("Missing required parameter: topic")?
            .to_string();
        let source = args
            .get("source")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).clamp(1, MAX_LIMIT))
            .unwrap_or(DEFAULT_LIMIT);

        let backends = plan_backends(&topic, source.as_deref());
        // Why each backend before it didn't answer — surfaced honestly in the
        // fall-through so Henry can report the truth, not a guess.
        let mut failures: Vec<String> = Vec::new();

        for backend in backends {
            match backend {
                Backend::Rss { url, label } => {
                    // SSRF-guard the (possibly user-supplied) feed host before we
                    // fetch, reusing the Browser extension's audited fetch-host
                    // check. DNS resolution blocks, so run it off the async worker.
                    let guard_url = url.clone();
                    let guarded = tokio::task::spawn_blocking(move || {
                        super::browser::guard_fetch_host(&guard_url)
                    })
                    .await;
                    match guarded {
                        Ok(Ok(())) => {}
                        Ok(Err(reason)) => {
                            failures.push(format!("{label} — {reason}"));
                            continue;
                        }
                        Err(e) => {
                            failures.push(format!("{label} — host-check task failed ({e})"));
                            continue;
                        }
                    }

                    match fetch_feed(&url).await {
                        Ok(body) => match probe_feed(&body, limit) {
                            Ok(items) => {
                                return Ok(vec![Content::text(format_items(
                                    &topic, &label, &items,
                                ))]);
                            }
                            Err(reason) => failures.push(format!("{label} — {reason}")),
                        },
                        Err(reason) => failures.push(format!("{label} — {reason}")),
                    }
                }
                Backend::WebSearch => {
                    return Ok(vec![Content::text(web_search_handoff(&topic, &failures))]);
                }
            }
        }

        // The ordered list always ends in WebSearch, so this is unreachable in
        // practice — keep an honest terminal message rather than an empty result.
        Ok(vec![Content::text(web_search_handoff(&topic, &failures))])
    }
}

/// Build the ordered backend list for a request — the resilience plan. RSS is
/// always tried first (zero-config, local); `web_search` is the fallback.
fn plan_backends(topic: &str, source: Option<&str>) -> Vec<Backend> {
    let primary = match source {
        Some(s) if looks_like_feed_url(s) => Backend::Rss {
            url: s.to_string(),
            label: format!("the feed you named ({s})"),
        },
        // A non-URL "channel" name (e.g. "indie hackers") scopes a news search —
        // still RSS, still zero-config.
        Some(s) => Backend::Rss {
            url: crate::rss::google_news_search_url(&format!("{topic} {s}")),
            label: format!("news for \"{topic}\" on \"{s}\""),
        },
        None => Backend::Rss {
            url: crate::rss::google_news_search_url(topic),
            label: format!("news for \"{topic}\""),
        },
    };
    vec![primary, Backend::WebSearch]
}

/// An http(s) URL? (The only schemes we will fetch.)
fn looks_like_feed_url(s: &str) -> bool {
    let s = s.trim();
    s.starts_with("https://") || s.starts_with("http://")
}

/// Health-probe a fetched feed body: don't trust a source that didn't actually
/// yield readable items. `Ok(items)` when ≥1 parsed, `Err(reason)` otherwise.
fn probe_feed(body: &str, limit: usize) -> Result<Vec<crate::rss::Item>, String> {
    let items = crate::rss::parse_items(body, limit);
    if items.is_empty() {
        return Err("reachable but returned no readable items".to_string());
    }
    Ok(items)
}

/// Fetch a feed body. No `.query()` — this reqwest is built with
/// `default-features = false`, so the URL already carries any query string (see
/// [`crate::rss::google_news_search_url`]).
async fn fetch_feed(url: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .user_agent("Permagent/1.0 (audience listener)")
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("could not reach it ({e})"))?;
    if !resp.status().is_success() {
        return Err(format!("answered {}", resp.status()));
    }
    resp.text()
        .await
        .map_err(|e| format!("could not read its response ({e})"))
}

/// Format healthy items into the tool result, headed by which backend answered.
fn format_items(topic: &str, label: &str, items: &[crate::rss::Item]) -> String {
    let mut out = format!(
        "Listening to \"{topic}\" via RSS — {label}. {} recent item(s):\n\n",
        items.len()
    );
    for (i, it) in items.iter().enumerate() {
        out.push_str(&format!("{}. {}\n", i + 1, it.title));
        let date = if it.pub_date.is_empty() {
            "date unknown".to_string()
        } else {
            it.pub_date.clone()
        };
        out.push_str(&format!("   {date}\n"));
        let snip = snippet(&it.description, SNIPPET_CHARS);
        if !snip.is_empty() {
            out.push_str(&format!("   {snip}\n"));
        }
        if !it.link.is_empty() {
            out.push_str(&format!("   {}\n", it.link));
        }
        out.push('\n');
    }
    out.push_str(
        "This is real, recently-published signal from the channel above — use it to ground the \
         project's Grow strategy (Audience, Channels), not as text to republish verbatim.",
    );
    out
}

/// The honest fall-through when no RSS backend was healthy: name the next backend
/// in the resilient order (`web_search`) and report what was actually tried.
/// Never invents chatter.
fn web_search_handoff(topic: &str, failures: &[String]) -> String {
    let mut out =
        format!("No RSS signal for \"{topic}\" right now. I did not make anything up.\n\n");
    if !failures.is_empty() {
        out.push_str("What I tried (and why it didn't answer):\n");
        for f in failures {
            out.push_str(&format!("- {f}\n"));
        }
        out.push('\n');
    }
    out.push_str(
        "The next backend in the resilient order is web_search. If a web-search tool is in your \
         tool list, call it for this topic to hear the public web; if none is connected, offer to \
         set one up (Brave or Tavily) in Settings so this fallback has something to reach.",
    );
    out
}

/// A readable one-line snippet: drop any HTML tags a feed's `<description>`
/// carries (Google News wraps its summaries in markup), collapse whitespace, and
/// cap the length. Built by scanning chars — no slicing, so it can't split a
/// UTF-8 sequence.
fn snippet(desc: &str, max_chars: usize) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    let mut last_ws = true; // suppress any leading space
    let mut count = 0usize;
    for c in desc.chars() {
        if count >= max_chars {
            out.push('…');
            break;
        }
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if in_tag => {}
            c if c.is_whitespace() => {
                if !last_ws {
                    out.push(' ');
                    last_ws = true;
                    count += 1;
                }
            }
            c => {
                out.push(c);
                last_ws = false;
                count += 1;
            }
        }
    }
    out.trim().to_string()
}

#[async_trait]
impl McpClientTrait for ListenClient {
    async fn list_tools(
        &self,
        _session_id: &str,
        _next_cursor: Option<String>,
        _cancellation_token: CancellationToken,
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
        _cancellation_token: CancellationToken,
    ) -> Result<CallToolResult, Error> {
        match name {
            TOOL_NAME => match self.handle_listen(arguments).await {
                Ok(content) => Ok(CallToolResult::success(content)),
                Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                    "Error: {e}"
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_is_rss_first_then_web_search() {
        let plan = plan_backends("local-first software", None);
        assert_eq!(plan.len(), 2);
        match &plan[0] {
            Backend::Rss { url, .. } => {
                assert!(url.starts_with("https://news.google.com/rss/search?q="));
                assert!(url.contains("local-first"));
            }
            _ => panic!("primary backend must be RSS"),
        }
        assert_eq!(plan[1], Backend::WebSearch);
    }

    #[test]
    fn explicit_feed_url_is_used_directly() {
        let plan = plan_backends("rust", Some("https://www.reddit.com/r/rust/.rss"));
        match &plan[0] {
            Backend::Rss { url, .. } => assert_eq!(url, "https://www.reddit.com/r/rust/.rss"),
            _ => panic!("expected the named feed"),
        }
        assert_eq!(plan[1], Backend::WebSearch);
    }

    #[test]
    fn non_url_source_becomes_a_scoped_news_search() {
        let plan = plan_backends("launch reactions", Some("indie hackers"));
        match &plan[0] {
            Backend::Rss { url, .. } => {
                assert!(url.starts_with("https://news.google.com/rss/search?q="));
                assert!(url.contains("launch")); // topic present
                assert!(url.contains("hackers")); // channel present
            }
            _ => panic!("expected a news search"),
        }
    }

    #[test]
    fn looks_like_feed_url_requires_http_scheme() {
        assert!(looks_like_feed_url("https://example.com/feed.xml"));
        assert!(looks_like_feed_url("http://example.com/rss"));
        assert!(!looks_like_feed_url("example.com/rss"));
        assert!(!looks_like_feed_url("r/rust"));
        assert!(!looks_like_feed_url("ftp://example.com"));
    }

    #[test]
    fn probe_rejects_empty_or_garbage_feeds() {
        assert!(probe_feed("", 8).is_err());
        assert!(probe_feed("<html>not a feed</html>", 8).is_err());
        assert!(probe_feed("<rss><channel></channel></rss>", 8).is_err());
    }

    #[test]
    fn probe_accepts_a_real_feed_and_caps_at_limit() {
        let xml = r#"<rss><channel>
            <item><title>First</title><link>https://ex/1</link>
              <description>alpha</description>
              <pubDate>Tue, 07 Jul 2026 12:00:00 GMT</pubDate></item>
            <item><title>Second</title><link>https://ex/2</link>
              <description>beta</description>
              <pubDate>Wed, 08 Jul 2026 12:00:00 GMT</pubDate></item>
            <item><title>Third</title><link>https://ex/3</link>
              <description>gamma</description>
              <pubDate>Thu, 09 Jul 2026 12:00:00 GMT</pubDate></item>
        </channel></rss>"#;
        let items = probe_feed(xml, 2).expect("healthy feed");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "First");
        assert_eq!(items[1].title, "Second");
    }

    #[test]
    fn snippet_strips_html_and_caps_length() {
        let s = snippet("<a href=\"x\">Hello</a>   <b>world</b> of feeds", 100);
        assert_eq!(s, "Hello world of feeds");
        let capped = snippet(&"x ".repeat(200), 10);
        assert!(capped.chars().count() <= 11); // 10 chars + the ellipsis
        assert!(capped.ends_with('…'));
    }

    #[test]
    fn format_items_reports_backend_and_real_items_only() {
        let items = vec![crate::rss::Item {
            title: "Acme ships v2".to_string(),
            link: "https://news/acme".to_string(),
            description: "<p>big news</p>".to_string(),
            pub_date: "Tue, 07 Jul 2026 12:00:00 GMT".to_string(),
        }];
        let out = format_items("acme", "news for \"acme\"", &items);
        assert!(out.contains("via RSS"));
        assert!(out.contains("Acme ships v2"));
        assert!(out.contains("https://news/acme"));
        assert!(out.contains("big news")); // snippet, tags stripped
    }

    #[test]
    fn handoff_is_honest_and_names_web_search() {
        let out = web_search_handoff(
            "obscure topic",
            &["news for \"obscure topic\" — answered 404 Not Found".to_string()],
        );
        assert!(out.contains("did not make anything up"));
        assert!(out.contains("web_search"));
        assert!(out.contains("404"));
    }
}
