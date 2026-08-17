use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::agents::tool_execution::ToolCallContext;
use async_trait::async_trait;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, JsonObject, ListToolsResult,
    ServerCapabilities, Tool, ToolAnnotations,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::sync::Mutex;
use tokio_util::sync::CancellationToken;

pub static EXTENSION_NAME: &str = "browser";

// The browser bridge/control plane is not configurable at this layer.
const CONTROL_PLANE_PORT: u16 = 3001;
const READ_BYTE_CAP: usize = 2 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;

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

// ── Act-on-page (#649 / #622): a11y-ref snapshot + click/type/select ─────────
// These mirror the daemon's `browser_act.rs` shapes; the tool POSTs to the same
// loopback bridge the read path uses and formats the JSON for the model.

#[derive(Debug, Serialize, Deserialize)]
struct SnapshotElement {
    #[serde(rename = "ref")]
    ref_id: u32,
    role: String,
    name: String,
    #[serde(default)]
    tag: String,
    #[serde(default)]
    value: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PageSnapshot {
    #[serde(default)]
    url: String,
    #[serde(default)]
    webview_id: Option<String>,
    #[serde(default)]
    elements: Vec<SnapshotElement>,
    #[serde(default)]
    truncated: bool,
    #[serde(default)]
    status: String,
    /// Generation these refs were stamped in — bound to the session and
    /// presented back on act, so another session's snapshot of the same shared
    /// webview invalidates them instead of silently re-pointing them (#939).
    #[serde(default)]
    generation: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ActResult {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    snapshot: Option<PageSnapshot>,
}

/// Client-side mirror of the daemon's `validate_act`: fail-closed on the action
/// set so the model gets a clear error without a round-trip.
fn validate_act_args(action: &str, value: Option<&str>) -> Result<(), String> {
    match action {
        "click" => Ok(()),
        "type" => value
            .map(|_| ())
            .ok_or_else(|| "The 'type' action needs a `value` (the text to type).".to_string()),
        "select" => match value {
            Some(v) if !v.is_empty() => Ok(()),
            _ => Err(
                "The 'select' action needs a non-empty `value` (the option to choose).".to_string(),
            ),
        },
        other => Err(format!(
            "Unknown action '{other}'. Use 'click', 'type', or 'select'."
        )),
    }
}

/// Render a snapshot as a compact, groundable list the model can read and act on.
fn format_snapshot(snap: &PageSnapshot) -> String {
    match snap.status.as_str() {
        "no_tab" => {
            return "No browser tab is currently open in Permagent. The user needs a page open \
                    in the browser for this to work."
                .to_string()
        }
        "refused_scheme" => {
            return "This isn't a web page (file:// or a custom scheme), so it can't be acted on. \
                    Only http(s) pages are supported."
                .to_string()
        }
        "error" => {
            return "The page could not be read for interactive elements (blank or still loading)."
                .to_string()
        }
        _ => {}
    }
    if snap.elements.is_empty() {
        return format!("No interactive elements were found on {}.", snap.url);
    }
    let mut out = format!(
        "Page: {}\nInteractive elements ({}):\n",
        snap.url,
        snap.elements.len()
    );
    for el in &snap.elements {
        out.push_str(&format!("[{}] {} \"{}\"", el.ref_id, el.role, el.name));
        if let Some(v) = &el.value {
            if !v.is_empty() {
                out.push_str(&format!(" = \"{v}\""));
            }
        }
        out.push('\n');
    }
    if snap.truncated {
        out.push_str(
            "\n(Only the first elements are shown — the page has more than the snapshot cap.)\n",
        );
    }
    out.push_str("\nUse act_on_page with a ref to click, type, or select.");
    out
}

pub struct BrowserClient {
    info: InitializeResult,
    snapshot_identities: Mutex<HashMap<String, PageIdentity>>,
}

#[derive(Clone)]
struct PageIdentity {
    webview_id: String,
    page_url: String,
    /// The snapshot generation this session's refs belong to (#939).
    generation: String,
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

/// Lowercased host from a URL (strips scheme, userinfo, port, IPv6 brackets).
fn host_of(url: &str) -> String {
    url.split("://")
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
        .to_ascii_lowercase()
}

/// True for any address the hub must never connect to: loopback, RFC1918
/// private, link-local (incl. the 169.254.169.254 cloud-metadata endpoint),
/// CGNAT, unspecified/broadcast, and their IPv6 equivalents (ULA fc00::/7,
/// link-local fe80::/10, IPv4-mapped).
fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || o[0] == 0
                || (o[0] == 100 && (64..=127).contains(&o[1])) // 100.64.0.0/10 CGNAT
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || (v6.segments()[0] & 0xfe00) == 0xfc00 // fc00::/7 unique-local
                || (v6.segments()[0] & 0xffc0) == 0xfe80 // fe80::/10 link-local
                || v6
                    .to_ipv4_mapped()
                    .is_some_and(|m| is_private_ip(IpAddr::V4(m)))
        }
    }
}

/// SSRF guard for the SERVER-SIDE FETCH path (`read_webpage`, `listen`) — where
/// the DAEMON itself fetches the URL. Unlike a naive string check, this RESOLVES
/// a DNS name and checks the resolved IP(s), so a public name that resolves to
/// 127.0.0.1 / 169.254.169.254 / a LAN address is refused (DNS-rebinding /
/// metadata SSRF). Blocking (DNS): the initial check runs via spawn_blocking; the
/// redirect hook runs it on reqwest's connection thread.
///
/// This is NOT the guard for opening a URL in the in-app webview. That path is
/// the user viewing their OWN browser (including a `localhost` dev server), not a
/// server-side fetch, so it uses `guard_webview_url` instead — which ALLOWS
/// loopback/private hosts but still blocks non-http(s) schemes. Keeping the two
/// apart is deliberate: loosening this one would re-open the SSRF hole.
///
pub(crate) fn guard_fetch_host(url: &str) -> Result<(), String> {
    resolve_fetch_host(url).map(|_| ())
}

/// Resolve and validate a fetch target, returning the exact addresses reqwest
/// must use. Callers pin these into their client to close DNS-rebinding TOCTOU.
fn resolve_fetch_host(url: &str) -> Result<(String, Vec<SocketAddr>), String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("Invalid URL: {e}"))?;
    let host = host_of(url);
    if host.is_empty() {
        return Err("URL has no host".to_string());
    }
    // Names that must never be treated as public, even if they resolve.
    if host == "localhost" || host.ends_with(".local") || host.ends_with(".localhost") {
        return Err(format!(
            "Refusing to fetch a private/loopback host ({host})"
        ));
    }
    // An IP literal is checked directly — no DNS.
    let port = parsed
        .port_or_known_default()
        .ok_or_else(|| "URL has no usable port".to_string())?;
    if let Ok(ip) = host.parse::<IpAddr>() {
        return if is_private_ip(ip) {
            Err(format!(
                "Refusing to fetch a private/loopback address ({host})"
            ))
        } else {
            Ok((host, vec![SocketAddr::new(ip, port)]))
        };
    }
    // A DNS name: resolve and check EVERY address it points at.
    let addrs = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|e| format!("Could not resolve host {host}: {e}"))?;
    let mut validated = Vec::new();
    for sa in addrs {
        if is_private_ip(sa.ip()) {
            return Err(format!(
                "Refusing to fetch {host} — it resolves to a private/loopback address"
            ));
        }
        validated.push(sa);
    }
    if validated.is_empty() {
        return Err(format!("Host {host} did not resolve to any address"));
    }
    Ok((host, validated))
}

/// Hosts that are the user's *own* machine: `localhost`, `*.localhost`, and any
/// loopback / private / link-local IP literal. Allowed on the webview-open path
/// (a local dev-server preview) and used to default a bare host to `http`. The
/// fetch guard blocks every one of these — this predicate is the webview side.
fn is_local_host(host: &str) -> bool {
    host == "localhost"
        || host.ends_with(".localhost")
        || host.parse::<IpAddr>().is_ok_and(is_private_ip)
}

/// Webview-open rail — the coding-harness "last mile": preview a local dev server
/// in the built-in browser. UNLIKE `guard_fetch_host`, this is NOT an SSRF guard:
/// the URL is handed to the frontend to render in the user's OWN in-app browser
/// (a client-side navigation), not fetched server-side by the daemon. So
/// `localhost`, `127.0.0.1`, `[::1]`, `*.localhost`, and other private/loopback
/// dev-server URLs are ALLOWED — this is the user viewing their own `npm run dev`
/// output. The one rail is the scheme: http(s) only, so the agent can never drive
/// `file://`, `javascript:`, `data:`, or a custom scheme into the webview.
/// Returns the normalized URL to open (scheme always present).
fn guard_webview_url(input: &str) -> Result<String, String> {
    let url = input.trim();
    if url.is_empty() {
        return Err("No URL given".to_string());
    }
    // An explicit http(s) URL preserves `http` for local dev servers.
    if url.starts_with("http://") || url.starts_with("https://") {
        reject_control_plane(url)?;
        return Ok(url.to_string());
    }
    // Any other explicit scheme carrying `//` (file://, ftp://, custom://) is refused.
    if url.contains("://") {
        return Err(format!(
            "Only http(s) URLs can be opened in the browser, got: {url}"
        ));
    }
    // A bare `scheme:...` with no `//` (javascript:, data:, mailto:) is refused
    // too. A leading `token:` is a scheme UNLESS what follows the colon is a port
    // (all digits) — `localhost:5173` is host:port, not a scheme. IPv6 literals
    // (`[::1]:8080`) start with `[` and skip this check.
    let bare_scheme = !url.starts_with('[')
        && url.split_once(':').is_some_and(|(_scheme, rest)| {
            let seg = rest.split(['/', '?', '#']).next().unwrap_or(rest);
            seg.is_empty() || !seg.bytes().all(|b| b.is_ascii_digit())
        });
    if bare_scheme {
        return Err(format!(
            "Only http(s) URLs can be opened in the browser, got: {url}"
        ));
    }
    // A bare host ("bbc.com", "localhost:5173", "[::1]:8080"): default the scheme
    // by locality — local dev servers speak http, public sites https.
    let host = host_of(&format!("http://{url}"));
    let normalized = if is_local_host(&host) {
        format!("http://{url}")
    } else {
        format!("https://{url}")
    };
    reject_control_plane(&normalized)?;
    Ok(normalized)
}

pub fn reject_control_plane(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("Invalid URL: {e}"))?;
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    // For an IPv6 literal, `host_str()` keeps the brackets (`[::1]`), which do
    // not parse as an `IpAddr` — strip them so `[::1]` (IPv6 loopback) is caught.
    let ip_literal = host.trim_start_matches('[').trim_end_matches(']');
    let is_control_host = host == "localhost"
        || host.ends_with(".localhost")
        || ip_literal
            .parse::<IpAddr>()
            .is_ok_and(|ip| ip.is_loopback() || ip.is_unspecified());
    if is_control_host && parsed.port_or_known_default() == Some(CONTROL_PLANE_PORT) {
        return Err("cannot navigate to the Permagent control plane".to_string());
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

fn append_capped(body: &mut Vec<u8>, chunk: &[u8], cap: usize) {
    let remaining = cap.saturating_sub(body.len());
    body.extend_from_slice(chunk.get(..remaining).unwrap_or(chunk));
}

fn validate_content_length(content_length: Option<u64>, cap: usize) -> Result<(), String> {
    if content_length.is_some_and(|length| length > cap as u64) {
        return Err(format!("Response body exceeds the {cap}-byte limit"));
    }
    Ok(())
}

impl BrowserClient {
    pub fn new(_context: PlatformExtensionContext) -> Result<Self, anyhow::Error> {
        let info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new(EXTENSION_NAME.to_string(), "1.0.0".to_string())
                    .with_title("Browser"),
            );
        Ok(Self {
            info,
            snapshot_identities: Mutex::new(HashMap::new()),
        })
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
        let url = guard_webview_url(url)?;
        let client = reqwest::Client::new();
        let resp = client
            .post("http://127.0.0.1:3001/api/browser/navigate")
            .timeout(std::time::Duration::from_secs(5))
            .json(&serde_json::json!({ "url": url }))
            .send()
            .await
            .map_err(|e| format!("Failed to reach the browser bridge: {e}"))?;
        if resp.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE {
            // The daemon is up (it answered) but no desktop UI is attached —
            // the Mac's app is closed, which is normal for a phone-only
            // session since the daemon runs under launchd independently.
            return Err(format!(
                "The Permagent desktop app is not open on your Mac, so there is no browser to \
                 open {url} in. Ask me to open the app (I can launch it remotely), or use \
                 read_webpage instead — it fetches the page server-side and needs no app."
            ));
        }
        if !resp.status().is_success() {
            return Err(format!("Browser navigate rejected: {}", resp.status()));
        }
        // read_webpage is a public-web reader that refuses loopback, so only point
        // the model at it for a public URL — for a local dev server the preview is
        // simply now visible in the Build tab.
        let hint = if is_local_host(&host_of(&url)) {
            "Your local dev-server preview is now visible there."
        } else {
            "Use read_webpage to read its content."
        };
        Ok(vec![Content::text(format!(
            "Opened {url} in the Permagent browser (Build tab). {hint}"
        ))])
    }

    /// Wake the desktop app so UI-dependent tools become usable. The daemon
    /// answers regardless of the app, so this is the recovery path for a phone
    /// session that hit a browser/terminal refusal.
    async fn handle_wake_desktop(&self) -> Result<Vec<Content>, String> {
        let client = reqwest::Client::new();
        let resp = client
            .post("http://127.0.0.1:3001/api/desktop/launch")
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
            .map_err(|e| format!("Couldn't reach the desktop control endpoint: {e}"))?;
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("Malformed desktop launch response: {e}"))?;
        let message = body
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("Desktop launch attempted.");
        Ok(vec![Content::text(message.to_string())])
    }

    /// Fetch a public web page server-side and return its readable text —
    /// works even when no browser tab is open, and is the reliable path for
    /// "read me the BBC homepage". SSRF-guarded: https/http only, private and
    /// loopback hosts refused, redirects re-checked per hop.
    // string_slice: the truncation walks back to `is_char_boundary` first.
    #[allow(clippy::string_slice)]
    async fn handle_read_webpage(&self, url: &str) -> Result<Vec<Content>, String> {
        let mut url = normalize_web_url(url)?;
        let mut redirects = 0;
        let mut resp = loop {
            let guard_url = url.clone();
            let (host, addrs) = tokio::task::spawn_blocking(move || resolve_fetch_host(&guard_url))
                .await
                .map_err(|e| format!("SSRF guard task panicked: {e}"))??;
            let client = reqwest::Client::builder()
                .user_agent("Permagent/1.0 (in-app reader)")
                .redirect(reqwest::redirect::Policy::none())
                .resolve_to_addrs(&host, &addrs)
                .timeout(std::time::Duration::from_secs(20))
                .build()
                .map_err(|e| format!("HTTP client error: {e}"))?;
            let response = client
                .get(&url)
                .send()
                .await
                .map_err(|e| format!("Could not fetch {url}: {e}"))?;
            if !response.status().is_redirection() {
                break response;
            }
            if redirects >= MAX_REDIRECTS {
                return Err("Too many redirects".to_string());
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .ok_or_else(|| format!("{url} redirected without a Location header"))?
                .to_str()
                .map_err(|e| format!("{url} returned an invalid redirect Location: {e}"))?;
            url = reqwest::Url::parse(&url)
                .and_then(|base| base.join(location))
                .map_err(|e| format!("{url} returned an invalid redirect Location: {e}"))?
                .to_string();
            redirects += 1;
        };
        if !resp.status().is_success() {
            return Err(format!("{url} answered {}", resp.status()));
        }

        validate_content_length(resp.content_length(), READ_BYTE_CAP)?;
        let mut body = Vec::with_capacity(
            resp.content_length().unwrap_or(0).min(READ_BYTE_CAP as u64) as usize,
        );
        while body.len() < READ_BYTE_CAP {
            let Some(chunk) = resp
                .chunk()
                .await
                .map_err(|e| format!("Failed reading the response body: {e}"))?
            else {
                break;
            };
            append_capped(&mut body, &chunk, READ_BYTE_CAP);
        }
        let html = String::from_utf8_lossy(&body);

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

    /// #649: snapshot the interactive elements of the open page as stable refs.
    /// Round-trips through the same loopback bridge as read_browser_content.
    async fn handle_get_page_snapshot(&self, session_id: &str) -> Result<Vec<Content>, String> {
        let client = reqwest::Client::new();
        let resp = client
            .post("http://127.0.0.1:3001/api/browser/snapshot/read")
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
            .map_err(|e| format!("Failed to request page snapshot: {e}"))?;

        if resp.status() == reqwest::StatusCode::GATEWAY_TIMEOUT {
            return Ok(vec![Content::text(
                "No browser tab is open, or the page didn't respond. Make sure a page is loaded \
                 in the Permagent browser.",
            )]);
        }
        if !resp.status().is_success() {
            return Err(format!("Snapshot failed with status {}", resp.status()));
        }
        let snap: PageSnapshot = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse snapshot: {e}"))?;
        let identity = snap.webview_id.as_ref().map(|webview_id| PageIdentity {
            webview_id: webview_id.clone(),
            page_url: snap.url.clone(),
            generation: snap.generation.clone(),
        });
        let mut identities = self
            .snapshot_identities
            .lock()
            .map_err(|_| "Browser snapshot identity state is unavailable.".to_string())?;
        match identity {
            Some(identity) if snap.status == "ok" => {
                identities.insert(session_id.to_string(), identity);
            }
            _ => {
                identities.remove(session_id);
            }
        }
        Ok(vec![Content::text(format_snapshot(&snap))])
    }

    /// #649: act on a ref from get_page_snapshot — click / type / select. Returns
    /// a fresh snapshot so the model re-grounds instead of reusing a stale ref.
    async fn handle_act_on_page(
        &self,
        ref_id: u32,
        action: &str,
        value: Option<&str>,
        session_id: &str,
    ) -> Result<Vec<Content>, String> {
        validate_act_args(action, value)?;
        let identity = self
            .snapshot_identities
            .lock()
            .map_err(|_| "Browser snapshot identity state is unavailable.".to_string())?
            .get(session_id)
            .cloned()
            .ok_or_else(|| {
                "No bound browser snapshot is available for this session. Call \
                 get_page_snapshot again before acting."
                    .to_string()
            })?;

        let client = reqwest::Client::new();
        let resp = client
            .post("http://127.0.0.1:3001/api/browser/act")
            .timeout(std::time::Duration::from_secs(20))
            .json(&serde_json::json!({
                "ref": ref_id,
                "action": action,
                "value": value,
                "webview_id": identity.webview_id,
                "page_url": identity.page_url,
                "generation": identity.generation,
            }))
            .send()
            .await
            .map_err(|e| format!("Failed to reach the browser bridge: {e}"))?;

        if resp.status() == reqwest::StatusCode::GATEWAY_TIMEOUT {
            return Ok(vec![Content::text(
                "No browser tab is open, or the page didn't respond. Make sure a page is loaded \
                 in the Permagent browser.",
            )]);
        }
        if resp.status() == reqwest::StatusCode::UNPROCESSABLE_ENTITY {
            return Err(format!(
                "That act was rejected: action '{action}' with value {value:?}."
            ));
        }
        if !resp.status().is_success() {
            return Err(format!("Act failed with status {}", resp.status()));
        }

        let result: ActResult = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse act result: {e}"))?;

        {
            let mut identities = self
                .snapshot_identities
                .lock()
                .map_err(|_| "Browser snapshot identity state is unavailable.".to_string())?;
            match result.snapshot.as_ref() {
                Some(snapshot) if result.ok => match &snapshot.webview_id {
                    Some(webview_id) if snapshot.status == "ok" => {
                        identities.insert(
                            session_id.to_string(),
                            PageIdentity {
                                webview_id: webview_id.clone(),
                                page_url: snapshot.url.clone(),
                                generation: snapshot.generation.clone(),
                            },
                        );
                    }
                    _ => {
                        identities.remove(session_id);
                    }
                },
                _ => {
                    identities.remove(session_id);
                }
            }
        }

        let mut out = if result.ok {
            format!("Done: {action} on ref {ref_id}.")
        } else {
            let err = result.error.unwrap_or_else(|| "unknown error".to_string());
            format!("Couldn't {action} on ref {ref_id}: {err}")
        };
        if let Some(snap) = &result.snapshot {
            out.push_str("\n\n");
            out.push_str(&format_snapshot(snap));
        }
        Ok(vec![Content::text(out)])
    }

    pub(crate) fn get_tools() -> Vec<Tool> {
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

        let act_schema: JsonObject = serde_json::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "ref": {
                    "type": "integer",
                    "description": "The element ref from get_page_snapshot (the number in [brackets])."
                },
                "action": {
                    "type": "string",
                    "enum": ["click", "type", "select"],
                    "description": "'click' a link/button, 'type' into an input/textarea, or 'select' an option."
                },
                "value": {
                    "type": "string",
                    "description": "For 'type': the text to enter. For 'select': the option's label or value. Omit for 'click'."
                }
            },
            "required": ["ref", "action"]
        }))
        .expect("static schema");

        vec![
            Tool::new(
                "read_browser_content".to_string(),
                "Read the visible text content of the page currently open in the Permagent \
                 browser. Returns the page title, URL, and extracted text. Use this when the \
                 user asks about what they're looking at or references their open tab."
                    .to_string(),
                schema.clone(),
            )
            // Reads the open tab. No effect on the page.
            .annotate(ToolAnnotations::from_raw(
                Some("Read Browser Content".to_string()),
                Some(true), // pure read — deliberately NOT escalated
                Some(false),
                Some(false),
                Some(true),
            )),
            Tool::new(
                "open_website".to_string(),
                "Open a website in the Permagent browser (the Build tab) so the user can see \
                 it — a public site ('go to BBC', 'open cbc.ca') OR a local dev server you \
                 just started (e.g. http://localhost:5173) to preview an app or game you \
                 built. Pair with read_webpage to read a public page aloud (read_webpage is \
                 public-web only and will not read a localhost URL)."
                    .to_string(),
                url_schema.clone(),
            ),
            Tool::new(
                "wake_desktop_app".to_string(),
                "Open the Permagent desktop app on the user's Mac, and report whether a \
                 desktop UI is currently attached. USE THIS when a browser or terminal action \
                 failed because the app is closed (typically a phone session — the daemon \
                 runs independently, so chat, voice, notes and automations work regardless). \
                 Requires the Mac to be awake; it cannot wake a sleeping machine."
                    .to_string(),
                schema.clone(),
            ),
            Tool::new(
                "read_webpage".to_string(),
                "Fetch a public web page and return its readable text — the reliable way to \
                 read a site to the user (e.g. 'read me the BBC homepage'). Works without any \
                 browser tab open. Public http(s) sites only."
                    .to_string(),
                url_schema,
            )
            // Fetches and returns text. No effect on any page.
            .annotate(ToolAnnotations::from_raw(
                Some("Read Webpage".to_string()),
                Some(true), // pure read — deliberately NOT escalated
                Some(false),
                Some(false),
                Some(true), // open-world: arbitrary pages
            )),
            Tool::new(
                "get_page_snapshot".to_string(),
                "List the interactive elements (links, buttons, inputs, selects) on the page \
                 currently open in the Permagent browser. Each element gets a stable `ref` you \
                 pass to act_on_page. Call this first to act on a page — and after any action, \
                 use the fresh snapshot it returns rather than an old ref."
                    .to_string(),
                schema,
            )
            // Enumerates elements. No effect on the page.
            .annotate(ToolAnnotations::from_raw(
                Some("Get Page Snapshot".to_string()),
                Some(true), // pure read — deliberately NOT escalated
                Some(false),
                Some(false),
                Some(true), // open-world: arbitrary pages
            )),
            Tool::new(
                "act_on_page".to_string(),
                "Act on an element in the Permagent browser page — click it, type text into it, \
                 or select an option — identified by a `ref` from get_page_snapshot. Returns a \
                 fresh snapshot. This is how you fill forms, click buttons and links, and drive \
                 a page, not just read it."
                    .to_string(),
                act_schema,
            )
            // Declared WRITE, and this is the only reason it is supervised.
            //
            // `Config::apply_tool_annotations` (config/permission.rs) starts
            // with `let Some(anns) = &tool.annotations else { continue }` — a
            // tool carrying NO annotations is skipped entirely, so it can never
            // be marked write and never lands in AskBefore. act_on_page had no
            // annotations, which meant clicking "Purchase", "Delete account" or
            // "Confirm transfer" on a live page ran unsupervised.
            //
            // Fail closed on the tool, not on the element: a click is judged
            // safe or not by where it leads, and the DOM role of a control is
            // not that. A link can say "Delete account". Every act_on_page call
            // is treated as state-changing; refining that with a per-element
            // classifier is only safe once there is evidence about which
            // classes are genuinely inert.
            .annotate(ToolAnnotations::from_raw(
                Some("Act on Page".to_string()),
                Some(false), // not read-only — it clicks, types and selects
                Some(false), // not inherently destructive, but can drive a destructive control
                Some(false),
                // Open-world: it acts on whatever page is loaded, which is not
                // knowable from the arguments.
                Some(true),
            )),
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
        ctx: &ToolCallContext,
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
            "wake_desktop_app" => match self.handle_wake_desktop().await {
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
            "get_page_snapshot" => match self.handle_get_page_snapshot(&ctx.session_id).await {
                Ok(content) => Ok(CallToolResult::success(content)),
                Err(error) => Ok(CallToolResult::error(vec![Content::text(error)])),
            },
            "act_on_page" => {
                let args = arguments.as_ref();
                let ref_id = args
                    .and_then(|a| a.get("ref"))
                    .and_then(|v| v.as_u64())
                    .map(|n| n as u32);
                let action = args
                    .and_then(|a| a.get("action"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let value = args
                    .and_then(|a| a.get("value"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                match ref_id {
                    None => Ok(CallToolResult::error(vec![Content::text(
                        "act_on_page needs an integer `ref` from get_page_snapshot.".to_string(),
                    )])),
                    Some(r) => match self
                        .handle_act_on_page(r, &action, value.as_deref(), &ctx.session_id)
                        .await
                    {
                        Ok(content) => Ok(CallToolResult::success(content)),
                        Err(error) => Ok(CallToolResult::error(vec![Content::text(error)])),
                    },
                }
            }
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
mod annotation_tests {
    use super::BrowserClient;

    /// Tools that deliberately carry NO annotations, each with the reason.
    ///
    /// This list exists because an un-annotated tool is INVISIBLE to the write
    /// gate: `Config::apply_tool_annotations` opens with
    /// `let Some(anns) = &tool.annotations else { continue }`, so a tool with
    /// no annotations can never be marked write and never reaches AskBefore.
    /// That is how `act_on_page` — click, type, select on a live page — ran
    /// unsupervised. Leaving a tool off the gate must be a decision someone
    /// wrote down, not the default you get by forgetting.
    const DELIBERATELY_UNANNOTATED: &[&str] = &[
        // Navigation the user explicitly asked for ("open cbc.ca"). Gating it
        // would prompt on the tool's entire purpose. Reversible, no external
        // effect beyond a GET.
        "open_website",
        // Launches the user's own app on their own Mac, already behind
        // require_loopback on the HTTP rail.
        "wake_desktop_app",
    ];

    /// The #622 regression: a state-changing browser act must be gated.
    #[test]
    fn act_on_page_is_declared_write_so_the_permission_gate_sees_it() {
        let tool = BrowserClient::get_tools()
            .into_iter()
            .find(|t| t.name.as_ref() == "act_on_page")
            .expect("act_on_page is registered");
        let ann = tool
            .annotations
            .as_ref()
            .expect("act_on_page MUST be annotated or the write gate skips it entirely");
        assert_eq!(
            ann.read_only_hint,
            Some(false),
            "act_on_page clicks, types and selects on a live page — a click on \
             \"Purchase\" or \"Delete account\" cannot be unsupervised"
        );
    }

    /// Every tool is either annotated or explicitly exempted by name. A new
    /// browser tool cannot slip past the gate by simply omitting annotations.
    #[test]
    fn no_browser_tool_skips_the_write_gate_by_accident() {
        let tools = BrowserClient::get_tools();
        // A gate that iterates an empty list passes while gating nothing. If
        // registration ever breaks, this test must go red rather than green —
        // it is the only thing standing between an unannotated tool and the
        // permission layer skipping it.
        assert!(
            !tools.is_empty(),
            "get_tools() returned nothing, so the write gate below checked nothing"
        );
        for tool in tools {
            let name = tool.name.to_string();
            if DELIBERATELY_UNANNOTATED.contains(&name.as_str()) {
                continue;
            }
            assert!(
                tool.annotations.is_some(),
                "{name} has no annotations, so the permission layer skips it and it can \
                 never be gated. Annotate it, or add it to DELIBERATELY_UNANNOTATED with \
                 the reason."
            );
        }
    }

    /// The reads stay reads — pinning this means a later change that makes one
    /// of them act on the page has to say so here.
    #[test]
    fn the_read_tools_are_declared_read_only() {
        for name in ["read_browser_content", "read_webpage", "get_page_snapshot"] {
            let tool = BrowserClient::get_tools()
                .into_iter()
                .find(|t| t.name.as_ref() == name)
                .unwrap_or_else(|| panic!("{name} is registered"));
            let ann = tool
                .annotations
                .as_ref()
                .unwrap_or_else(|| panic!("{name} annotated"));
            assert_eq!(ann.read_only_hint, Some(true), "{name}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        append_capped, format_snapshot, guard_fetch_host, guard_webview_url, host_of,
        is_private_ip, validate_act_args, validate_content_length, PageSnapshot, SnapshotElement,
        READ_BYTE_CAP,
    };
    use std::net::IpAddr;

    #[test]
    fn is_private_ip_flags_dangerous_ranges() {
        for s in [
            "127.0.0.1",
            "10.0.0.1",
            "192.168.1.1",
            "172.16.0.1",
            "172.31.255.255",
            "169.254.169.254", // cloud metadata
            "0.0.0.0",
            "100.64.0.1", // CGNAT
            "::1",
            "fe80::1",
            "fc00::1",
        ] {
            assert!(
                is_private_ip(s.parse::<IpAddr>().unwrap()),
                "{s} must be treated as private"
            );
        }
        for s in [
            "8.8.8.8",
            "1.1.1.1",
            "93.184.216.34",
            "2606:4700:4700::1111",
            "172.15.0.1", // just outside 172.16/12
            "172.32.0.1",
        ] {
            assert!(
                !is_private_ip(s.parse::<IpAddr>().unwrap()),
                "{s} must be public"
            );
        }
    }

    #[test]
    fn guard_fetch_host_rejects_private_literals_and_localhost_without_dns() {
        // SSRF guard for the server-side fetch path — must stay strict.
        assert!(guard_fetch_host("http://127.0.0.1/x").is_err());
        assert!(guard_fetch_host("http://169.254.169.254/latest/meta-data/").is_err());
        assert!(guard_fetch_host("http://10.0.0.5:8080").is_err());
        assert!(guard_fetch_host("http://localhost:3001").is_err());
        assert!(guard_fetch_host("http://foo.local/").is_err());
        assert!(guard_fetch_host("http://[::1]/").is_err());
        // A public IP literal passes without any DNS lookup.
        assert!(guard_fetch_host("https://8.8.8.8/").is_ok());
    }

    #[test]
    fn guard_webview_url_allows_localhost_dev_servers() {
        // The coding-harness last mile: the user's OWN dev server, opened in
        // their OWN webview — allowed (this is NOT the SSRF fetch path). Explicit
        // http(s) URLs open verbatim (http preserved for the dev server).
        for u in [
            "http://localhost:5173",
            "http://127.0.0.1:3000",
            "http://[::1]:8080",
            "https://localhost",
            "http://app.localhost:5173",
        ] {
            assert_eq!(
                guard_webview_url(u).as_deref(),
                Ok(u),
                "{u} must open as-is"
            );
        }
        // A bare local host defaults to http (a dev server won't answer https).
        assert_eq!(
            guard_webview_url("localhost:5173").as_deref(),
            Ok("http://localhost:5173")
        );
        // A bare public host still defaults to https.
        assert_eq!(
            guard_webview_url("bbc.com").as_deref(),
            Ok("https://bbc.com")
        );
    }

    #[test]
    fn guard_webview_url_blocks_the_control_plane() {
        for url in [
            "http://127.0.0.1:3001/ui",
            "http://localhost:3001",
            "http://[::1]:3001",
            "http://0.0.0.0:3001",
            "http://app.localhost:3001",
        ] {
            let error = guard_webview_url(url).unwrap_err();
            assert!(
                error.contains("cannot navigate to the Permagent control plane"),
                "{url}: {error}"
            );
        }
        assert!(guard_webview_url("http://localhost:5173").is_ok());
        assert!(guard_webview_url("https://example.com").is_ok());
    }

    #[test]
    fn guard_webview_url_blocks_file_and_non_http_schemes() {
        // file://, custom schemes, and bare javascript:/data: URIs can never be
        // driven into the webview — the one rail on the open path.
        for bad in [
            "file:///etc/passwd",
            "ftp://example.com/x",
            "javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "",
            "   ",
        ] {
            assert!(
                guard_webview_url(bad).is_err(),
                "{bad:?} must be refused on the webview path"
            );
        }
    }

    #[test]
    fn fetch_and_webview_guards_diverge_on_localhost() {
        // The whole point of the split: the SAME localhost dev-server URL is
        // REFUSED by the SSRF fetch guard but ALLOWED by the webview-open rail.
        assert!(guard_fetch_host("http://localhost:5173").is_err());
        assert!(guard_webview_url("http://localhost:5173").is_ok());
    }

    #[test]
    fn host_of_strips_scheme_userinfo_port_and_brackets() {
        assert_eq!(
            host_of("https://user:pw@Example.COM:8443/path?q"),
            "example.com"
        );
        assert_eq!(host_of("http://[2606:4700::1111]:80/x"), "2606:4700::1111");
    }

    #[test]
    fn append_capped_never_collects_more_than_the_byte_cap() {
        let mut body = vec![b'a'; READ_BYTE_CAP - 2];
        append_capped(&mut body, b"oversized chunk", READ_BYTE_CAP);
        assert_eq!(body.len(), READ_BYTE_CAP);
        append_capped(&mut body, b"ignored", READ_BYTE_CAP);
        assert_eq!(body.len(), READ_BYTE_CAP);
    }

    #[test]
    fn oversized_content_length_is_rejected_up_front() {
        assert!(validate_content_length(Some(READ_BYTE_CAP as u64), READ_BYTE_CAP).is_ok());
        assert!(validate_content_length(Some(READ_BYTE_CAP as u64 + 1), READ_BYTE_CAP).is_err());
        assert!(validate_content_length(None, READ_BYTE_CAP).is_ok());
    }

    #[test]
    fn validate_act_args_matches_the_action_contract() {
        assert!(validate_act_args("click", None).is_ok());
        assert!(validate_act_args("type", Some("hello")).is_ok());
        assert!(validate_act_args("type", Some("")).is_ok()); // clearing a field
        assert!(validate_act_args("type", None).is_err());
        assert!(validate_act_args("select", Some("Option A")).is_ok());
        assert!(validate_act_args("select", Some("")).is_err());
        assert!(validate_act_args("select", None).is_err());
        assert!(validate_act_args("submit", None).is_err());
    }

    #[test]
    fn format_snapshot_lists_refs_and_flags_status() {
        let snap = PageSnapshot {
            url: "https://example.com".into(),
            webview_id: Some("browser-1".into()),
            elements: vec![
                SnapshotElement {
                    ref_id: 0,
                    role: "link".into(),
                    name: "Home".into(),
                    tag: "a".into(),
                    value: None,
                },
                SnapshotElement {
                    ref_id: 1,
                    role: "textbox".into(),
                    name: "Email".into(),
                    tag: "input".into(),
                    value: Some("me@x.com".into()),
                },
            ],
            truncated: false,
            status: "ok".into(),
            generation: "gen-a".into(),
        };
        let out = format_snapshot(&snap);
        assert!(out.contains("[0] link \"Home\""));
        assert!(out.contains("[1] textbox \"Email\" = \"me@x.com\""));
        assert!(out.contains("act_on_page"));

        // A non-web page is reported, not listed.
        let refused = PageSnapshot {
            url: "file:///x".into(),
            webview_id: Some("browser-1".into()),
            elements: vec![],
            truncated: false,
            status: "refused_scheme".into(),
            generation: String::new(),
        };
        assert!(format_snapshot(&refused).contains("http(s)"));
    }
}
