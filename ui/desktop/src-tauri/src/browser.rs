use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Emitter, Manager, WebviewBuilder, WebviewUrl};

// ── File-intake inbox (epic #392 / #393) ────────────────────────────────────
//
// Downloads in the in-app browser webview are redirected onto disk under
// `~/.permagent/inbox/` and recorded as a metadata row in permagent.db via the
// daemon's `POST /api/inbox`. The webview fires `on_download` in this (desktop)
// process; persistence lives in the daemon, so we POST over the same
// token-authenticated localhost seam `activity.rs` uses. macOS does not report
// the final saved path on the `Finished` event, so we remember the destination
// we set on `Requested` and use that.
//
// WHY `on_download` ALONE WAS NEVER ENOUGH (reported 2026-08-19: nothing
// downloaded ever reached the inbox. `~/.permagent/inbox/` did not exist and
// `inbox_files` held no rows, and `prepare_inbox_destination` creates that
// directory on the FIRST `Requested` event — so the hook had never fired once.)
//
// The hook is wired correctly and it IS forwarded on the child-webview path.
// This is NOT the `on_new_window` story #1050 fixed. Traced through the pinned
// dependencies:
//
//   * tauri 2.11.0 `WebviewBuilder::into_pending_webview` moves
//     `download_handler` onto the `PendingWebview` — the one function BOTH
//     `Window::add_child` and window creation go through;
//   * tauri-runtime-wry 2.11.0's `create_webview` installs
//     `with_download_started_handler` / `with_download_completed_handler` on
//     the same `WebViewBuilder` it later finishes with `build_as_child`.
//
// What never happens is WebKit minting a `WKDownload` at all. wry asks for one
// in exactly two places (`wry-0.55.0/src/wkwebview/navigation.rs`):
// `navigation_policy` returns `WKNavigationActionPolicy::Download` when
// `WKNavigationAction.shouldPerformDownload` is set — that flag is the HTML
// `download` attribute — and `navigation_policy_response` returns
// `WKNavigationResponsePolicy::Download` only when `!canShowMIMEType`. A PDF, a
// Word document, a CSV, an image, or anything served `Content-Disposition:
// attachment` under a displayable MIME type all report `canShowMIMEType ==
// true`, so WebKit RENDERS them in a native viewer instead. That single fact is
// also why the agent sees an empty string for an attachment tab: no download
// event, and no HTML body to scrape either.
//
// So capture cannot be an event we wait for; it has to be an action the shell
// can take on a tab — `save_tab_to_inbox` below. It runs shell -> page, the
// same direction as `get_page_content`, so it opens no new channel a remote
// page could reach for. That was #1050's reasoning and it still holds.

/// A file destined for the inbox: bytes already on disk, metadata not yet
/// recorded. Also the carrier for a native download between `Requested` and
/// `Finished`.
struct PendingInboxDownload {
    /// Source URL the file was downloaded from.
    url: String,
    /// Absolute on-disk path inside the inbox directory.
    abs_path: PathBuf,
    /// On-disk basename, also the `disk_path` relative to the inbox dir.
    filename: String,
    /// Project this file belongs to, when the caller knew one. `None` leaves
    /// the row unscoped, and the Inbox panel's "File it" picker assigns it
    /// later. The column and the routing endpoint already existed; what did not
    /// exist was any way for the capture site to say which project it was in.
    project_id: Option<String>,
}

fn inbox_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".permagent/inbox")
}

fn read_daemon_token() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let path = PathBuf::from(home).join(".permagent/secrets/daemon_token.json");
    let content = std::fs::read_to_string(path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;
    parsed.get("token")?.as_str().map(|s| s.to_string())
}

/// Strip any path components and unsafe characters from a suggested filename.
fn sanitize_filename(name: &str) -> String {
    let base = name.trim().rsplit(['/', '\\']).next().unwrap_or("").trim();
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | '\0') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('.').trim();
    if cleaned.is_empty() {
        "download".to_string()
    } else {
        cleaned.to_string()
    }
}

/// Split `name` into (stem, extension) on the last dot. Extension is empty when
/// there is no dot or it is a leading dot (dotfile).
fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 && i < name.len() - 1 => (&name[..i], &name[i + 1..]),
        _ => (name, ""),
    }
}

/// Return a filename that does not collide with an existing file in `dir`.
/// Appends `-1`, `-2`, … before the extension; falls back to a uuid prefix.
fn dedupe_filename(dir: &Path, name: &str) -> String {
    if !dir.join(name).exists() {
        return name.to_string();
    }
    let (stem, ext) = split_ext(name);
    for n in 1..1000 {
        let candidate = if ext.is_empty() {
            format!("{stem}-{n}")
        } else {
            format!("{stem}-{n}.{ext}")
        };
        if !dir.join(&candidate).exists() {
            return candidate;
        }
    }
    format!("{}-{name}", uuid::Uuid::new_v4())
}

/// Ensure `dir` exists and compute a sanitized, collision-free target from a
/// suggested name. Split out from `prepare_inbox_destination` so the naming and
/// de-collision rules can be tested against a temp dir instead of `$HOME`.
fn prepare_destination_in(dir: &Path, suggested: &str) -> Result<(PathBuf, String), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("create inbox dir: {e}"))?;
    let unique = dedupe_filename(dir, &sanitize_filename(suggested));
    Ok((dir.join(&unique), unique))
}

/// Ensure the inbox dir exists and compute a sanitized, collision-free target
/// from the browser's suggested destination. Returns (absolute path, filename).
fn prepare_inbox_destination(suggested: &Path) -> Result<(PathBuf, String), String> {
    let raw = suggested
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("download");
    prepare_destination_in(&inbox_dir(), raw)
}

/// Best-effort content-type from the filename extension. `None` when unknown —
/// the column is nullable and a later pass can sniff bytes if needed.
fn guess_content_type(name: &str) -> Option<&'static str> {
    let (_, ext) = split_ext(name);
    let ct = match ext.to_ascii_lowercase().as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "txt" => "text/plain",
        "csv" => "text/csv",
        "json" => "application/json",
        "html" | "htm" => "text/html",
        "zip" => "application/zip",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        _ => return None,
    };
    Some(ct)
}

/// POST the inbox metadata row to the daemon. Mirrors `activity.rs`'s
/// token-authenticated localhost call.
async fn record_inbox_file(pending: PendingInboxDownload) -> Result<(), String> {
    let token = read_daemon_token().ok_or("failed to read daemon token")?;
    let size_bytes = std::fs::metadata(&pending.abs_path)
        .ok()
        .map(|m| m.len() as i64);
    let body = serde_json::json!({
        "filename": pending.filename,
        "original_url": pending.url,
        "content_type": guess_content_type(&pending.filename),
        "size_bytes": size_bytes,
        "disk_path": pending.filename,
        // `NewInboxFile` has always accepted this and `inbox_files.project_id`
        // has always existed; nothing on the capture side ever sent it, so
        // every intake arrived unscoped even when the shell knew the project.
        "project_id": pending.project_id,
    });

    let client = reqwest::Client::new();
    let resp = client
        .post("http://127.0.0.1:3001/api/inbox")
        .bearer_auth(&token)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("daemon request failed: {e}"))?;

    if resp.status().is_success() {
        Ok(())
    } else {
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        Err(format!("daemon returned {status}: {text}"))
    }
}

// ── Capturing an open tab into the inbox ────────────────────────────────────
//
// The action half of the intake path (see the long note at the top of this
// file for why an event-driven one cannot exist for the documents people
// actually open). The shell calls this for the tab in front of the user; the
// agent's read path calls it when a tab turns out not to be an HTML document,
// so "read this attachment" becomes "read this file" — the Reader and the local
// OCR already know how to do that, and neither can do anything with a native
// PDF viewer's empty `document.body`.

/// The user agent the in-app browser presents. Shared with `save_tab_to_inbox`
/// so a capture is fetched as the same client that rendered the tab — sites
/// that vary a document by user agent hand back the same bytes.
const BROWSER_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.2 Safari/605.1.15";

/// Hard cap on a captured document. Big enough for the reports, decks and
/// scanned PDFs this is for; small enough that a mis-aimed capture cannot fill
/// the disk. A larger file is refused by name rather than truncated — half a
/// PDF in the inbox is worse than none.
const MAX_CAPTURE_BYTES: u64 = 64 * 1024 * 1024;

/// What a capture produced, returned to the caller so the UI can name the file
/// and the agent can go straight to reading it.
#[derive(Clone, Serialize, Deserialize)]
pub struct InboxCapture {
    /// On-disk basename inside the inbox directory.
    pub filename: String,
    /// Absolute path, so an agent can read the file without knowing the layout.
    pub path: String,
    pub size_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    /// Source URL, echoed back for the confirmation message.
    pub url: String,
}

/// Pull a filename out of a `Content-Disposition` header. Handles the plain
/// `filename="x.pdf"` form and RFC 5987's `filename*=UTF-8''x%20y.pdf`, which is
/// what any server sending a non-ASCII name uses. Returns `None` rather than
/// guessing, so the URL path gets its turn.
fn filename_from_disposition(header: &str) -> Option<String> {
    // `filename*` first: when both are present it is the authoritative one.
    for part in header.split(';') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix("filename*=") {
            // charset'language'percent-encoded-value
            let value = rest.rsplit('\'').next().unwrap_or(rest);
            let decoded = percent_decode(value);
            if !decoded.trim().is_empty() {
                return Some(decoded);
            }
        }
    }
    for part in header.split(';') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix("filename=") {
            let value = rest.trim().trim_matches('"').trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Minimal `%XX` decoder for a Content-Disposition value or a URL path segment.
/// Invalid escapes are left verbatim — a filename is not worth an error.
fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(b) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// The extension we would give a file of this content type. Inverse of
/// `guess_content_type`, and only for the types worth naming.
fn extension_for_content_type(content_type: &str) -> Option<&'static str> {
    let base = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let ext = match base.as_str() {
        "application/pdf" => "pdf",
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "text/plain" => "txt",
        "text/csv" => "csv",
        "application/json" => "json",
        "text/html" | "application/xhtml+xml" => "html",
        "application/zip" => "zip",
        "application/msword" => "doc",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => "docx",
        "application/vnd.ms-excel" => "xls",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "xlsx",
        _ => return None,
    };
    Some(ext)
}

/// The last path segment of a URL, percent-decoded, if it looks like a filename.
fn filename_from_url(url: &url::Url) -> Option<String> {
    let last = url
        .path_segments()
        .and_then(|mut s| s.next_back())
        .unwrap_or("");
    let decoded = percent_decode(last);
    let trimmed = decoded.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

/// Name a captured file: `Content-Disposition`, else the URL's last path
/// segment, else the host. An extension is appended from the content type when
/// the chosen name has none — the Reader dispatches on extension, so a PDF
/// called `view` would be unreadable for want of four characters.
fn capture_filename(
    url: &url::Url,
    disposition: Option<&str>,
    content_type: Option<&str>,
) -> String {
    let chosen = disposition
        .and_then(filename_from_disposition)
        .or_else(|| filename_from_url(url))
        .unwrap_or_else(|| url.host_str().unwrap_or("download").to_string());
    let name = sanitize_filename(&chosen);
    let (_, ext) = split_ext(&name);
    if !ext.is_empty() {
        return name;
    }
    match content_type.and_then(extension_for_content_type) {
        Some(ext) => format!("{name}.{ext}"),
        None => name,
    }
}

/// True when the bytes are an HTML document.
///
/// This is the honesty check. A capture that is not carrying the browser's
/// session cookies gets a sign-in page or an error page back with HTTP 200, and
/// storing that as `invoice.pdf` would be a lie the agent then reads and
/// summarises. When the tab claimed to be a PDF and the bytes are HTML, we
/// refuse and say why.
fn looks_like_html(content_type: Option<&str>, bytes: &[u8]) -> bool {
    if let Some(ct) = content_type {
        let base = ct
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if base == "text/html" || base == "application/xhtml+xml" {
            return true;
        }
    }
    let head = &bytes[..bytes.len().min(512)];
    let text = String::from_utf8_lossy(head).to_ascii_lowercase();
    let text = text.trim_start();
    text.starts_with("<!doctype html")
        || text.starts_with("<html")
        || text.starts_with("<?xml-stylesheet")
}

/// Write captured bytes into `dir` and return the pending record for the daemon.
/// Separated from the HTTP fetch so the naming, de-collision and on-disk result
/// are testable without a network or a webview.
fn store_capture_in(
    dir: &Path,
    url: &url::Url,
    filename: &str,
    bytes: &[u8],
    project_id: Option<String>,
) -> Result<PendingInboxDownload, String> {
    let (abs_path, filename) = prepare_destination_in(dir, filename)?;
    std::fs::write(&abs_path, bytes).map_err(|e| format!("write inbox file: {e}"))?;
    Ok(PendingInboxDownload {
        url: url.to_string(),
        abs_path,
        filename,
        project_id,
    })
}

/// Capture the document a tab is showing into the inbox, as a real file.
///
/// `expect_document` is set by the agent's read path, which only calls this
/// because the tab is NOT an HTML document. In that mode an HTML response is
/// treated as a failure (a sign-in wall or an error page), because storing it
/// would put a plausible lie in the inbox.
///
/// LIMITATION, stated rather than hidden: the fetch is made from this process,
/// so it carries no WKWebView cookies. A download that only works while signed
/// in fails here and says so. It is not silently wrong, and the native
/// `on_download` path — which DOES run inside the browser's session — still
/// covers `download`-attribute links and non-displayable MIME types.
#[tauri::command]
pub async fn save_tab_to_inbox(
    app: AppHandle,
    webview_id: String,
    project_id: Option<String>,
    expect_document: Option<bool>,
) -> Result<InboxCapture, String> {
    let webview = app
        .get_webview(&webview_id)
        .ok_or_else(|| "Webview not found".to_string())?;
    let raw_url = webview
        .url()
        .map(|u| u.to_string())
        .map_err(|e| format!("Read url failed: {e}"))?;
    let url: url::Url = raw_url.parse().map_err(|e| format!("Invalid URL: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!(
            "Only http(s) pages can be saved to the inbox; this tab is {}",
            url.scheme()
        ));
    }

    let client = reqwest::Client::builder()
        .user_agent(BROWSER_USER_AGENT)
        .build()
        .map_err(|e| format!("build http client: {e}"))?;
    let resp = client
        .get(url.clone())
        .send()
        .await
        .map_err(|e| format!("fetch failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("the server answered {}", resp.status()));
    }

    let header = |name: &str| {
        resp.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    };
    let disposition = header("content-disposition");
    let content_type = header("content-type");

    if let Some(len) = resp.content_length() {
        if len > MAX_CAPTURE_BYTES {
            return Err(format!(
                "that file is {len} bytes, over the {MAX_CAPTURE_BYTES}-byte inbox limit"
            ));
        }
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("read body failed: {e}"))?;
    if bytes.len() as u64 > MAX_CAPTURE_BYTES {
        return Err(format!(
            "that file is {} bytes, over the {MAX_CAPTURE_BYTES}-byte inbox limit",
            bytes.len()
        ));
    }
    if bytes.is_empty() {
        return Err("the server returned an empty response".to_string());
    }

    if expect_document.unwrap_or(false) && looks_like_html(content_type.as_deref(), &bytes) {
        return Err(
            "the server returned a web page rather than the document — it probably needs the \
             sign-in session the browser holds. Open the file's own download link instead."
                .to_string(),
        );
    }

    let filename = capture_filename(&url, disposition.as_deref(), content_type.as_deref());
    let pending = store_capture_in(&inbox_dir(), &url, &filename, &bytes, project_id)?;
    let capture = InboxCapture {
        filename: pending.filename.clone(),
        path: pending.abs_path.display().to_string(),
        size_bytes: bytes.len() as u64,
        content_type,
        url: url.to_string(),
    };
    println!(
        "[permagent-app] inbox: captured {} ({} bytes) from {}",
        capture.filename, capture.size_bytes, capture.url
    );
    record_inbox_file(pending).await?;
    Ok(capture)
}

struct BrowserWebview {
    _label: String,
}

pub struct BrowserSessions(Mutex<HashMap<String, BrowserWebview>>);

impl BrowserSessions {
    pub fn new() -> Self {
        Self(Mutex::new(HashMap::new()))
    }
}

static WEBVIEW_COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// A MAIN-FRAME page-load transition. `url` is read from the WKWebView itself
/// (`WKWebView.URL`), so it is the main frame's URL by construction — a
/// subframe can never appear here.
///
/// This replaced an `on_navigation` emit, which was the wrong source: that hook
/// wraps `decidePolicyForNavigationAction`, which fires for EVERY frame and
/// hands the callback only a URL — no way to tell an ad iframe from the page.
/// CBC embeds Google's `/api2/aframe`, so the last iframe to load renamed the
/// tab `google.com` and put the ad frame's path in the address bar (reported
/// 2026-08-04). `on_page_load` maps to `didCommitNavigation` /
/// `didFinishNavigation`, which WebKit only calls for the main frame.
#[derive(Clone, Serialize)]
struct BrowserPageLoadPayload {
    webview_id: String,
    url: String,
    /// `true` at commit (the page's identity is now this URL), `false` at
    /// finish. Drives the reload spinner as well as the address bar.
    loading: bool,
}

#[derive(Clone, Serialize)]
struct BrowserTitleChangedPayload {
    webview_id: String,
    title: String,
}

/// `target=_blank` / `window.open` from a page: the popup is DENIED at the
/// WKWebView layer and re-routed to the tab strip via this event. The UI
/// listener predates this emitter's return — the original emitter (#240,
/// 9c856568e) lived in the old command-center shell and was deleted with it
/// (#709), leaving every popup link silently dead. Field name is
/// `source_webview_id` because that is what Browser.tsx already reads (and
/// filters on, so a global emit does not open one tab per live Browser).
#[derive(Clone, Serialize)]
struct BrowserNewWindowPayload {
    source_webview_id: String,
    url: String,
}

/// Injected into EVERY frame of every page webview at document start.
///
/// `on_new_window` below is the only seam WebKit offers, and it only fires when
/// the PAGE asks for a new frame (`window.open`, `target=_blank`). The mouse
/// gestures a person actually uses to open a tab — right-click -> Open Link in
/// New Tab, middle-click, Cmd-click — never reach it: they arrive at
/// `decidePolicyForNavigationAction`, whose wry binding is `Fn(String) -> bool`
/// and has already discarded the button number and the modifier flags. A bare
/// WKWebView has no "Open Link in New Tab" menu item either; that one is
/// Safari's, not WebKit's.
///
/// So the script claims those gestures in the page and re-expresses them as
/// `window.open(url, '_blank')` — the one thing WebKit does route here. Every
/// gesture then converges on this hook and on Browser.tsx's single
/// tab-opening path, which is why there is no second event channel to rot.
const LINKS_JS: &str = include_str!("browser_links.js");

/// `browser_links.js` only DEFINES functions (so the vitest+jsdom suite can
/// load the very same file); this wraps it in an IIFE that installs them.
fn links_init_script() -> String {
    format!("(function(){{\n{LINKS_JS}\n__permagentInstallLinks();\n}})()")
}

#[tauri::command]
pub async fn create_browser_webview(
    app: AppHandle,
    url: String,
    window_label: Option<String>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<String, String> {
    let id = WEBVIEW_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let label = format!("browser-{id}");
    let parsed_url: url::Url = url.parse().map_err(|e| format!("Invalid URL: {e}"))?;
    let webview_url = WebviewUrl::External(parsed_url);

    let owner = window_label.unwrap_or_else(|| "main".to_string());
    let window = app
        .get_window(&owner)
        .ok_or_else(|| format!("Window {owner} not found"))?;

    let nav_id = label.clone();
    let nav_app = app.clone();
    let title_id = label.clone();
    let title_app = app.clone();
    let popup_id = label.clone();
    let popup_app = app.clone();
    // Pending downloads keyed by source URL, carried Requested -> Finished.
    let pending: Arc<Mutex<HashMap<String, PendingInboxDownload>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let builder = WebviewBuilder::new(&label, webview_url)
        .user_agent(BROWSER_USER_AGENT)
        // Mouse gestures -> new tabs. All frames, because a link in an iframe
        // is still a link; the script itself keeps its context menu to the top
        // frame, where an overlay is not clipped to an ad iframe's box.
        .initialization_script_for_all_frames(links_init_script())
        .on_download(move |_webview, event| {
            match event {
                tauri::webview::DownloadEvent::Requested { url, destination } => {
                    match prepare_inbox_destination(destination) {
                        Ok((abs_path, filename)) => {
                            let key = url.to_string();
                            // Say so, every time. Until 2026-08-19 nobody could
                            // tell "the hook is not wired" from "WebKit never
                            // minted a download" — both look like silence.
                            println!("[permagent-app] inbox: native download {key} -> {filename}");
                            *destination = abs_path.clone();
                            if let Ok(mut map) = pending.lock() {
                                map.insert(
                                    key.clone(),
                                    PendingInboxDownload {
                                        url: key,
                                        abs_path,
                                        filename,
                                        // The native hook has no shell context,
                                        // so it cannot know the project. The
                                        // Inbox panel files it afterwards.
                                        project_id: None,
                                    },
                                );
                            }
                        }
                        Err(e) => {
                            eprintln!("[permagent-app] inbox: prepare destination failed: {e}");
                        }
                    }
                    // Allow the download regardless; redirection is best-effort.
                    true
                }
                tauri::webview::DownloadEvent::Finished {
                    url,
                    path: _,
                    success,
                } => {
                    let entry = pending
                        .lock()
                        .ok()
                        .and_then(|mut m| m.remove(&url.to_string()));
                    if success {
                        if let Some(entry) = entry {
                            tauri::async_runtime::spawn(async move {
                                if let Err(e) = record_inbox_file(entry).await {
                                    eprintln!("[permagent-app] inbox: record failed: {e}");
                                }
                            });
                        }
                    }
                    true
                }
                _ => true,
            }
        })
        .on_page_load(move |_webview, payload| {
            let _ = nav_app.emit(
                "browser_page_load",
                BrowserPageLoadPayload {
                    webview_id: nav_id.clone(),
                    url: payload.url().to_string(),
                    loading: matches!(payload.event(), PageLoadEvent::Started),
                },
            );
        })
        .on_document_title_changed(move |_webview, title| {
            let _ = title_app.emit(
                "browser_title_changed",
                BrowserTitleChangedPayload {
                    webview_id: title_id.clone(),
                    title,
                },
            );
        })
        // Popups become tabs: deny the native window, hand the URL to the tab
        // strip. Global `emit` (same channel as page_load/title) — `emit_to`
        // was tried to scope by window, but Browser.tsx listens with the
        // default `event.listen` target (`Any`), and the ownership filter on
        // `source_webview_id` is what actually prevents Build + detached
        // panes from each opening a tab. Matching page_load's emit path keeps
        // popup delivery on the channel that is known to reach the shell.
        .on_new_window(move |url, _features| {
            // Say so, every time. The three prior regressions (#240, #709,
            // #973) all survived because a dropped popup left NO trace: the
            // click just did nothing and the logs were silent. This line and
            // Browser.tsx's matching one make the next one a five-second
            // diagnosis instead of a month.
            let target = url.to_string();
            println!("[permagent-app] browser: new-window request from {popup_id} -> {target}");
            if let Err(e) = popup_app.emit(
                "browser_new_window_request",
                BrowserNewWindowPayload {
                    source_webview_id: popup_id.clone(),
                    url: target.clone(),
                },
            ) {
                eprintln!(
                    "[permagent-app] browser: emit of new-window request for {target} FAILED: {e}"
                );
            }
            // Returning Deny is load-bearing: without a Create/Allow response
            // WKWebView cancels the navigation, so the emit above is the ONLY
            // way the URL survives. A missing listener looks like "click did
            // nothing" — the historical #240 / #709 / #973 failure mode.
            tauri::webview::NewWindowResponse::Deny
        });

    let position = tauri::Position::Logical(tauri::LogicalPosition::new(x, y));
    let size = tauri::Size::Logical(tauri::LogicalSize::new(width, height));

    let child = window
        .add_child(builder, position, size)
        .map_err(|e| format!("Failed to create webview: {e}"))?;

    // Media capture (getUserMedia) for the in-app browser.
    //
    // Without this a Zoom / Google Meet / Teams call cannot run here at all:
    // WKWebView hides navigator.mediaDevices unless the private
    // `_mediaCaptureEnabled` preference is set. The main and chat WINDOWS get
    // it by invoking `enable_media_capture_cmd` from their own JS on mount —
    // a route unavailable to this webview, whose JS belongs to the remote
    // page and has no Tauri bridge. So it must be applied here, at creation,
    // from the Rust side. Best-effort and non-fatal, exactly like the window
    // path: a failure means no camera/mic in the browser, not a dead tab.
    #[cfg(target_os = "macos")]
    {
        let _ = child.with_webview(|w| crate::apply_media_capture(&w));
    }

    app.state::<BrowserSessions>().0.lock().unwrap().insert(
        label.clone(),
        BrowserWebview {
            _label: label.clone(),
        },
    );

    Ok(label)
}

/// Move the existing native browser webview to another shell window. Tauri's
/// reparent operation preserves the WKWebView/WebView2 instance, including its
/// DOM, navigation history, cookies and injected capability context.
#[tauri::command]
pub async fn reparent_browser(
    app: AppHandle,
    webview_id: String,
    window_label: String,
) -> Result<(), String> {
    let webview = app
        .get_webview(&webview_id)
        .ok_or_else(|| "Webview not found".to_string())?;
    let window = app
        .get_window(&window_label)
        .ok_or_else(|| format!("Window {window_label} not found"))?;
    webview
        .reparent(&window)
        .map_err(|e| format!("Reparent failed: {e}"))?;

    // RE-PARK after reparenting, in the same call.
    //
    // The caller hides the webview before handing it over, but `hide_browser`
    // parks it at (-10000,-10000) in the OLD window's coordinate space. Once
    // `reparent` re-hosts the WKWebView under a new parent that offset no
    // longer means "offscreen", so the webview paints — at the PANE window's
    // size — until the receiving window's bounds pump corrects it a frame or
    // two later. That is the full-screen flash seen when closing a popped-out
    // browser (reported 2026-08-04, intermittent because it depends on whether
    // a compositor frame lands in the gap).
    //
    // Re-parking here closes the gap entirely: there is no JS round-trip
    // between the reparent and the park, so nothing can be painted in
    // between. The receiving side reveals it by setting real bounds, which is
    // already what `syncBounds` does on arrival.
    webview
        .set_position(tauri::Position::Logical(tauri::LogicalPosition::new(
            -10000.0, -10000.0,
        )))
        .map_err(|e| format!("Re-park after reparent failed: {e}"))?;
    Ok(())
}

#[tauri::command]
pub async fn navigate_browser(
    app: AppHandle,
    webview_id: String,
    url: String,
) -> Result<(), String> {
    let parsed: url::Url = url.parse().map_err(|e| format!("Invalid URL: {e}"))?;
    let webview = app
        .get_webview(&webview_id)
        .ok_or_else(|| "Webview not found".to_string())?;
    webview
        .navigate(parsed)
        .map_err(|e| format!("Navigation failed: {e}"))?;
    Ok(())
}

#[tauri::command]
pub async fn update_browser_bounds(
    app: AppHandle,
    webview_id: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let webview = app
        .get_webview(&webview_id)
        .ok_or_else(|| "Webview not found".to_string())?;
    webview
        .set_position(tauri::Position::Logical(tauri::LogicalPosition::new(x, y)))
        .map_err(|e| format!("Set position failed: {e}"))?;
    webview
        .set_size(tauri::Size::Logical(tauri::LogicalSize::new(width, height)))
        .map_err(|e| format!("Set size failed: {e}"))?;
    Ok(())
}

#[tauri::command]
pub async fn hide_browser(app: AppHandle, webview_id: String) -> Result<(), String> {
    if let Some(webview) = app.get_webview(&webview_id) {
        webview
            .set_position(tauri::Position::Logical(tauri::LogicalPosition::new(
                -10000.0, -10000.0,
            )))
            .map_err(|e| format!("Hide failed: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn close_browser(app: AppHandle, webview_id: String) -> Result<(), String> {
    app.state::<BrowserSessions>()
        .0
        .lock()
        .unwrap()
        .remove(&webview_id);
    if let Some(webview) = app.get_webview(&webview_id) {
        webview.close().map_err(|e| format!("Close failed: {e}"))?;
    }
    Ok(())
}

/// Close every browser webview the UI no longer knows about.
///
/// Why this exists: `BrowserSessions` and the native child webviews live for
/// the lifetime of the PROCESS, but the React shell's memory of which
/// `webview_id`s exist dies with the page. Reload the shell (or hot-reload in
/// dev) and every open browser child keeps compositing above the DOM — native
/// child webviews always render over HTML — while the only code that could
/// address them has forgotten their ids. The result is a UI buried under
/// webviews nothing can close, and the only way out is force-quitting the app.
///
/// The shell calls this once on mount with the ids it still believes in
/// (normally none, right after a reload). Anything labelled `browser-*` that
/// is not in `keep` is orphaned by definition and gets closed.
///
/// Safe as a prefix sweep because these labels come from a single
/// process-static counter (`WEBVIEW_COUNTER`) and no other webview in the app
/// uses the `browser-` prefix. Returns the number reaped so a caller — or a
/// human reading the log — can tell the difference between "nothing was
/// orphaned" and "the sweep never ran".
#[tauri::command]
pub async fn reap_orphan_browsers(app: AppHandle, keep: Vec<String>) -> Result<usize, String> {
    // Take the full label set first, then close outside the lock: closing a
    // webview can re-enter, and holding the mutex across that is how the pane
    // teardown deadlocked before.
    let orphans: Vec<String> = {
        let sessions = app.state::<BrowserSessions>();
        let map = sessions.0.lock().unwrap();
        map.keys()
            .filter(|label| !keep.contains(*label))
            .cloned()
            .collect()
    };

    let mut reaped = 0usize;
    for label in &orphans {
        if let Some(webview) = app.get_webview(label) {
            // Best-effort: a webview that is already gone is exactly the
            // outcome we want, so a close error must not abort the sweep and
            // strand the remaining orphans.
            if let Err(e) = webview.close() {
                eprintln!("[permagent-app] reap: close {label} failed: {e}");
                continue;
            }
        }
        app.state::<BrowserSessions>()
            .0
            .lock()
            .unwrap()
            .remove(label);
        reaped += 1;
    }

    if reaped > 0 {
        eprintln!(
            "[permagent-app] reap: closed {reaped} orphaned browser webview(s), kept {}",
            keep.len()
        );
    }
    Ok(reaped)
}

/// Tear down a detached pane window and its child browser webviews in ONE
/// native operation.
///
/// Why this exists: the pane window's JS close handler used to await a
/// `close_browser` invoke per tab and only then destroy the window. Closing a
/// native child webview while the window is servicing its own close-requested
/// event can stall that IPC round-trip, and a stalled await meant `destroy()`
/// was never reached — the window simply refused to close (the chat and
/// terminal panes, which don't close child webviews, were unaffected). Doing
/// the whole teardown Rust-side makes it atomic: once this command is
/// dispatched, the window dies even if the calling JS context never hears the
/// reply.
///
/// Order matters: child webviews are closed first (releases their
/// BrowserSessions entries), then the window. Every step is best-effort — a
/// missing webview or an already-closing window must never keep the window on
/// screen.
#[tauri::command]
pub async fn destroy_pane_window(
    app: AppHandle,
    window_label: String,
    webview_ids: Vec<String>,
) -> Result<(), String> {
    {
        let state = app.state::<BrowserSessions>();
        let mut sessions = state.0.lock().unwrap();
        for id in &webview_ids {
            sessions.remove(id);
        }
    }
    for id in &webview_ids {
        if let Some(webview) = app.get_webview(id) {
            if let Err(e) = webview.close() {
                eprintln!("[permagent-app] destroy_pane_window: close webview {id} failed: {e}");
            }
        }
    }
    if let Some(window) = app.get_window(&window_label) {
        window
            .destroy()
            .map_err(|e| format!("Destroy window failed: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn zoom_browser(
    app: AppHandle,
    webview_id: String,
    zoom_level: f64,
) -> Result<(), String> {
    let webview = app
        .get_webview(&webview_id)
        .ok_or_else(|| "Webview not found".to_string())?;
    let js = format!("document.body.style.zoom = '{:.0}%'", zoom_level * 100.0);
    webview.eval(&js).map_err(|e| format!("Zoom failed: {e}"))?;
    Ok(())
}

const MAX_CONTENT_CHARS: usize = 16000;

#[derive(Clone, Serialize, Deserialize)]
pub struct PageContent {
    pub title: String,
    pub url: String,
    pub content: String,
    /// "ok" for an HTML document, `NON_HTML_STATUS` when the tab is a native
    /// viewer (a PDF, a Word document, an image) rather than a DOM.
    #[serde(default = "default_status")]
    pub status: String,
    #[serde(default)]
    pub truncated: bool,
    /// `document.contentType` as the page itself reports it. Present so the
    /// caller can say WHAT the tab is instead of guessing from the URL. The
    /// alias is the key the injected script uses; the wire name stays snake
    /// case like every other field the shell reads.
    #[serde(
        default,
        alias = "contentType",
        skip_serializing_if = "Option::is_none"
    )]
    pub content_type: Option<String>,
}

/// Status for "this tab is not an HTML document".
///
/// Extraction has always been `document.body.innerText`. For a PDF or a Word
/// document WKWebView renders a NATIVE viewer: there is no body text, so the
/// agent got `""` and the bridge turned that into "the page appears to be blank
/// or still loading" — which reads as "there is nothing there" when in fact
/// there is a whole document there. An empty string is the one answer that is
/// never honest here, so the page reports what it actually is and the caller
/// captures the FILE instead (`save_tab_to_inbox`).
pub const NON_HTML_STATUS: &str = "non_html_document";

fn default_status() -> String {
    "ok".to_string()
}

fn previous_char_boundary(value: &str, offset: usize) -> usize {
    let mut offset = offset.min(value.len());
    while !value.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn truncate_page_content(content: &mut String) -> bool {
    if content.len() <= MAX_CONTENT_CHARS {
        return false;
    }

    // Truncate at nearest paragraph or sentence boundary within the last 500 bytes.
    let search_start = previous_char_boundary(content, MAX_CONTENT_CHARS.saturating_sub(500));
    let max_end = previous_char_boundary(content, MAX_CONTENT_CHARS);
    let search_window = content.get(search_start..max_end).unwrap_or_default();
    let cut_at = search_window
        .rfind("\n\n")
        .or_else(|| search_window.rfind(". "))
        .or_else(|| search_window.rfind('\n'))
        .map(|pos| search_start + pos)
        .unwrap_or(max_end);

    content.truncate(cut_at);
    content.push_str("\n\n[content truncated]");
    true
}

/// The read-page script.
///
/// It reports only FACTS — the title, the URL, whatever body text exists, and
/// `document.contentType`. Deciding what those facts mean is
/// `classify_page_content`'s job, in Rust, where it can be unit-tested; there
/// is no WKWebView in CI to run a decision written in here.
fn page_content_js() -> String {
    r#"
        (function() {
            var title = document.title || '';
            var url = location.href || '';
            var content = document.body ? document.body.innerText : '';
            var contentType = '';
            try { contentType = String(document.contentType || ''); } catch (e) { contentType = ''; }
            return JSON.stringify({
                title: title, url: url, content: content, contentType: contentType
            });
        })()
    "#
    .to_string()
}

/// True when a content type describes something with a DOM worth scraping.
///
/// Anything else — `application/pdf`, `application/msword`, `image/*` — is
/// rendered by a NATIVE viewer inside WKWebView. There is no body text in those
/// documents, and none is coming.
fn is_dom_content_type(content_type: &str) -> bool {
    let base = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    // An empty value means the page declined to say (about:blank, some older
    // WebKit builds). Treat that as a DOM: the body text is then the truth.
    base.is_empty()
        || base == "text/html"
        || base == "application/xhtml+xml"
        || base == "text/plain"
        || base == "text/xml"
        || base == "application/xml"
        || base.ends_with("+xml")
}

/// Turn a page's raw facts into an honest answer.
///
/// The bug this exists for: extraction is `document.body.innerText`, and for a
/// PDF or a Word document that is `""`. The bridge then reported "the page
/// appears to be blank or still loading", so the agent said it could not read
/// the document — when what actually happened is that the document is not a DOM
/// at all. An empty string is the one answer that is never true here.
///
/// Only a tab that is BOTH non-DOM and empty is reclassified: an inline PDF
/// viewer that does expose selectable text stays `ok` and keeps it.
fn classify_page_content(page: &mut PageContent) {
    let content_type = page.content_type.clone().unwrap_or_default();
    if is_dom_content_type(&content_type) || !page.content.trim().is_empty() {
        return;
    }
    // Non-empty by construction: an empty content type counts as a DOM above.
    let label = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    page.status = NON_HTML_STATUS.to_string();
    page.content = format!(
        "This tab is a {label} document, not an HTML page. WebKit renders it in a native viewer, \
         so it has no page text to read. Save it to the inbox with `save_tab_to_inbox` and read \
         the file instead."
    );
}

#[tauri::command]
pub async fn get_page_content(app: AppHandle, webview_id: String) -> Result<PageContent, String> {
    let webview = app
        .get_webview(&webview_id)
        .ok_or_else(|| "Webview not found".to_string())?;

    let (tx, rx) = std::sync::mpsc::channel::<String>();

    let js = page_content_js();
    let js = js.as_str();

    webview
        .eval_with_callback(js, move |result| {
            let _ = tx.send(result);
        })
        .map_err(|e| format!("eval failed: {e}"))?;

    let raw = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .map_err(|_| "Timed out waiting for page content".to_string())?;

    // eval_with_callback JSON-serializes the JS return value, so the callback
    // receives a double-encoded string. Unwrap one layer.
    let json_str: String = serde_json::from_str(&raw).unwrap_or_else(|_| raw.clone());
    let mut page: PageContent =
        serde_json::from_str(&json_str).map_err(|e| format!("Parse failed: {e}"))?;

    // Classify BEFORE truncating: the honest non-HTML message is short, and
    // truncating a message about emptiness would be its own small absurdity.
    classify_page_content(&mut page);

    if truncate_page_content(&mut page.content) {
        page.truncated = true;
    }

    Ok(page)
}

// ── Act-on-page: a11y-ref snapshot + click/type/select (#649 / #622) ─────────
//
// Extends the read-page seam (`get_page_content`) with two capabilities the
// agent grounds actions on: snapshot the page's INTERACTIVE elements as stable
// `data-permagent-ref` handles, then act on a ref. The grounding logic lives in
// `browser_grounding.js` (the single source of truth, also exercised by the
// vitest+jsdom suite); here we only inject it and parse the JSON it returns.

/// Injected verbatim into the page; defines `__permagentSnapshot` / `__permagentAct`.
const GROUNDING_JS: &str = include_str!("browser_grounding.js");

/// Cap on interactive elements per snapshot — bounds the token cost the same way
/// `MAX_CONTENT_CHARS` bounds read_browser_content. JS flags `truncated` when hit.
const MAX_SNAPSHOT_ELEMENTS: usize = 150;

#[derive(Clone, Serialize, Deserialize)]
pub struct SnapshotElement {
    #[serde(rename = "ref")]
    pub ref_id: u32,
    pub role: String,
    pub name: String,
    pub tag: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PageSnapshot {
    pub url: String,
    pub elements: Vec<SnapshotElement>,
    #[serde(default)]
    pub truncated: bool,
    /// "ok", "refused_scheme" (non-http(s) page), or "error".
    #[serde(default = "default_status")]
    pub status: String,
    /// The generation these refs were stamped in. An act must present it back,
    /// so a snapshot taken by another session invalidates these refs (#939).
    #[serde(default)]
    pub generation: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ActResult {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// A FRESH snapshot taken after a successful act, so the caller never grounds
    /// on a stale ref (Playwright-MCP discipline).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<PageSnapshot>,
}

/// Args handed to `__permagentAct` — serialized to a JSON object literal and
/// injected. serde_json emits valid, escaped JSON, so the agent-supplied `value`
/// cannot break out of the string (injection-safe).
#[derive(Serialize)]
struct ActArgs<'a> {
    #[serde(rename = "ref")]
    ref_id: u32,
    action: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<&'a str>,
}

/// A fresh snapshot generation token. Refs are only valid within the generation
/// they were stamped in, so every snapshot mints a new one (#939).
fn new_generation() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn snapshot_js(cap: usize, generation_json: &str) -> String {
    format!(
        "(function(){{\n{GROUNDING_JS}\nreturn JSON.stringify(__permagentSnapshot({cap}, {generation_json}));\n}})()"
    )
}

fn act_js(
    args_json: &str,
    expected_url_json: &str,
    expected_generation_json: &str,
    next_generation_json: &str,
    cap: usize,
) -> String {
    format!(
        "(function(){{\n{GROUNDING_JS}\nif (String(window.location.href) !== {expected_url_json}) return JSON.stringify({{ok:false,error:'The page changed since the snapshot. Take a fresh snapshot before acting.'}});\nreturn JSON.stringify(__permagentAct({args_json}, {cap}, {expected_generation_json}, {next_generation_json}));\n}})()"
    )
}

/// Inject `js` (which must evaluate to a JSON string) and parse it as `T`.
/// Mirrors `get_page_content`'s eval_with_callback + double-decode: the eval
/// bridge JSON-serializes the JS return value, so the callback receives a
/// double-encoded string — unwrap one layer, then parse.
fn eval_returning_json<R: tauri::Runtime, T: serde::de::DeserializeOwned>(
    webview: &tauri::Webview<R>,
    js: &str,
    timeout: std::time::Duration,
) -> Result<T, String> {
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    webview
        .eval_with_callback(js, move |result| {
            let _ = tx.send(result);
        })
        .map_err(|e| format!("eval failed: {e}"))?;
    let raw = rx
        .recv_timeout(timeout)
        .map_err(|_| "Timed out waiting for the page to respond".to_string())?;
    let json_str: String = serde_json::from_str(&raw).unwrap_or_else(|_| raw.clone());
    serde_json::from_str(&json_str).map_err(|e| format!("Parse failed: {e}"))
}

/// List the page's interactive elements as stable refs (#649). Injects the same
/// grounding script the read path uses, capped at `MAX_SNAPSHOT_ELEMENTS`.
#[tauri::command]
pub async fn get_page_snapshot(app: AppHandle, webview_id: String) -> Result<PageSnapshot, String> {
    let webview = app
        .get_webview(&webview_id)
        .ok_or_else(|| "Webview not found".to_string())?;
    let generation_json = serde_json::to_string(&new_generation())
        .map_err(|e| format!("Serialize generation failed: {e}"))?;
    let js = snapshot_js(MAX_SNAPSHOT_ELEMENTS, &generation_json);
    eval_returning_json(&webview, &js, std::time::Duration::from_secs(5))
}

/// Act on a ref from `get_page_snapshot`: click / type / select (#649). Returns
/// success plus a fresh snapshot. The action set is validated here (defense in
/// depth — the daemon route validates too). The in-page scheme guard in
/// `__permagentAct` refuses non-http(s) pages.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn act_on_ref(
    app: AppHandle,
    webview_id: String,
    ref_id: u32,
    action: String,
    value: Option<String>,
    expected_url: String,
    expected_generation: Option<String>,
) -> Result<ActResult, String> {
    if !matches!(action.as_str(), "click" | "type" | "select") {
        return Err(format!("Unsupported action: {action}"));
    }
    let webview = app
        .get_webview(&webview_id)
        .ok_or_else(|| "Webview not found".to_string())?;
    let args = ActArgs {
        ref_id,
        action: &action,
        value: value.as_deref(),
    };
    let args_json =
        serde_json::to_string(&args).map_err(|e| format!("Serialize args failed: {e}"))?;
    let expected_url_json = serde_json::to_string(&expected_url)
        .map_err(|e| format!("Serialize expected URL failed: {e}"))?;
    let expected_generation_json = serde_json::to_string(&expected_generation.unwrap_or_default())
        .map_err(|e| format!("Serialize expected generation failed: {e}"))?;
    let next_generation_json = serde_json::to_string(&new_generation())
        .map_err(|e| format!("Serialize generation failed: {e}"))?;
    let js = act_js(
        &args_json,
        &expected_url_json,
        &expected_generation_json,
        &next_generation_json,
        MAX_SNAPSHOT_ELEMENTS,
    );
    eval_returning_json(&webview, &js, std::time::Duration::from_secs(8))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn act_script_rejects_a_changed_page_before_acting() {
        let js = act_js(
            r#"{"ref":1,"action":"click"}"#,
            r#""https://example.com/form""#,
            r#""gen-a""#,
            r#""gen-b""#,
            MAX_SNAPSHOT_ELEMENTS,
        );
        let guard = js.find("window.location.href").unwrap();
        let act = js.find("__permagentAct({").unwrap();
        assert!(guard < act);
        assert!(js.contains("The page changed since the snapshot"));
    }

    /// #939: the act carries the generation its refs were stamped in, plus a
    /// fresh one for the post-action snapshot. Without this a second session
    /// snapshotting the same tab restamps every ref from 0 and the first
    /// session's "ref 3" silently resolves to a different element.
    #[test]
    fn act_script_passes_the_expected_and_next_generations() {
        let js = act_js(
            r#"{"ref":3,"action":"click"}"#,
            r#""https://example.com/form""#,
            r#""gen-expected""#,
            r#""gen-next""#,
            MAX_SNAPSHOT_ELEMENTS,
        );
        assert!(
            js.contains("gen-expected"),
            "expected generation is injected"
        );
        assert!(js.contains("gen-next"), "next generation is injected");
        // Order matters: __permagentAct(args, cap, expectedGen, nextGen).
        let call = js.find("__permagentAct({").unwrap();
        let expected_at = js[call..].find("gen-expected").unwrap();
        let next_at = js[call..].find("gen-next").unwrap();
        assert!(expected_at < next_at);
    }

    /// Every snapshot mints a distinct generation — the whole point is that a
    /// later snapshot supersedes an earlier one's refs.
    #[test]
    fn each_snapshot_generation_is_unique() {
        assert_ne!(new_generation(), new_generation());
    }

    #[test]
    fn snapshot_script_stamps_the_generation() {
        let js = snapshot_js(MAX_SNAPSHOT_ELEMENTS, r#""gen-1""#);
        assert!(js.contains("__permagentSnapshot(150, \"gen-1\")"));
    }

    #[test]
    fn truncation_offsets_are_valid_utf8_boundaries() {
        let mut content = "a".repeat(MAX_CONTENT_CHARS - 501);
        content.push('€');
        content.push_str(&"b".repeat(497));
        content.push('€');
        content.push('c');

        assert!(truncate_page_content(&mut content));
        assert!(content.is_char_boundary(content.len()));
        assert_eq!(
            content,
            format!(
                "{}\n\n[content truncated]",
                "a".repeat(MAX_CONTENT_CHARS - 501) + "€" + &"b".repeat(497)
            )
        );
    }

    // ── Tab identity may only come from a MAIN-FRAME source ─────────────────
    //
    // These are source guards, not behavioural tests, because the thing being
    // protected is a choice of WebKit callback: `on_navigation` wraps
    // `decidePolicyForNavigationAction` (fires for EVERY frame, and hands the
    // callback nothing but a URL), while `on_page_load` wraps
    // didCommit/didFinish (main frame only, URL read from the webview itself).
    // No test that can run in CI has a WKWebView to prove the difference, and
    // the wrong choice is invisible until a page with ad iframes relabels its
    // own tab — which is exactly what shipped: CBC embeds Google's
    // `/api2/aframe`, and the last iframe to load renamed the tab `google.com`
    // and rewrote the address bar (reported 2026-08-04).
    //
    // The needles are assembled with `concat!` so they do not appear literally
    // in this file — otherwise every one of these assertions would match its
    // own source text and pass no matter what the wiring above actually says.

    #[test]
    fn identity_events_come_from_the_main_frame_hook() {
        let src = include_str!("browser.rs");
        assert!(
            src.contains(concat!(".on_", "page_load(")),
            "create_browser_webview must wire the main-frame page-load hook. \
             Tab identity and the address bar are fed from it; it is the only \
             navigation callback WebKit restricts to the main frame."
        );
    }

    #[test]
    fn the_all_frames_navigation_hook_is_not_wired() {
        let src = include_str!("browser.rs");
        assert!(
            !src.contains(concat!(".on_", "navigation(")),
            "This hook fires for SUBFRAMES too, so anything derived from it can \
             be an ad iframe's URL rather than the page's. It was removed on \
             purpose. If you need to intercept or block navigations, do that \
             here — but never emit tab identity from it, and update this test \
             deliberately rather than deleting it."
        );
    }

    /// The event name is a contract with Browser.tsx, which has a matching
    /// guard. Renaming one side silently stops every tab updating.
    #[test]
    fn page_load_event_name_and_shape_are_pinned() {
        let src = include_str!("browser.rs");
        assert!(
            src.contains(concat!("\"browser_", "page_load\"")),
            "The frontend listens for this exact event name (see \
             browserRegressions.test.ts). Rename both sides together."
        );
        assert!(
            src.contains("loading: matches!("),
            "The payload carries the commit/finish distinction: the label is \
             only re-derived on a commit onto a new page, so that a real page \
             title is not clobbered when the load finishes."
        );
    }

    /// Popup links must be denied-and-rerouted, never dropped. The UI half of
    /// this contract (Browser.tsx's `browser_new_window_request` listener)
    /// outlived the original emitter once already — #240's emitter lived in
    /// the old command-center shell and died with it in #709, leaving Gmail
    /// meeting links dead for a month. This pins the re-added emitter.
    #[test]
    fn popup_links_are_denied_and_rerouted_to_the_tab_strip() {
        let src = include_str!("browser.rs");
        assert!(
            src.contains(concat!(".on_", "new_window(")),
            "create_browser_webview must wire the new-window hook — without it \
             WKWebView silently drops every target=_blank / window.open link."
        );
        assert!(
            src.contains(concat!("\"browser_", "new_window_request\"")),
            "The frontend listens for this exact event name (Browser.tsx). \
             Rename both sides together."
        );
        assert!(
            src.contains(concat!("source_", "webview_id")),
            "Browser.tsx reads `source_webview_id` — every other payload uses \
             `webview_id`, so this mismatch would fail silently. The frontend \
             also filters on it so a global emit does not open one tab per \
             live Browser instance."
        );
        assert!(
            src.contains("NewWindowResponse::Deny"),
            "The native popup window must be denied; the tab strip owns the URL."
        );
        // Pin global emit (same path as page_load). emit_to was a regressing
        // alternative: the shell listens with event.listen's default Any
        // target, and delivery must not depend on window-label matching.
        assert!(
            src.contains("popup_app.emit("),
            "Popup routing must use AppHandle::emit like page_load/title — \
             that is the channel the shell's listen() is proven to receive."
        );
        assert!(
            !src.contains(concat!("emit_", "to(")),
            "Do not switch popup routing back to emit_to without also changing \
             the frontend listener to a labeled target (getCurrentWindow).\
             listen) and updating this guard deliberately."
        );
    }

    /// The mouse gestures WebKit will NOT route to the UI delegate.
    ///
    /// `on_new_window` alone is not "open in a new tab" — it is only the
    /// content-initiated half of it. Right-click, middle-click and Cmd-click
    /// arrive at the navigation-policy delegate instead, which wry exposes as
    /// `Fn(String) -> bool`: no button number, no modifier flags. The injected
    /// script is what turns those into a `window.open` the hook DOES see. It
    /// keeps getting deleted as "an injected script we don't need", so this
    /// pins every link in that chain.
    #[test]
    fn mouse_gestures_are_bridged_into_the_new_window_hook() {
        let src = include_str!("browser.rs");
        assert!(
            src.contains(concat!("include_", "str!(\"browser_links.js\")")),
            "The gesture bridge lives in browser_links.js so the vitest+jsdom \
             suite can exercise the very same file that ships."
        );
        assert!(
            src.contains(concat!(".initialization_script_", "for_all_frames(")),
            "The bridge must be an INITIALIZATION script, not an eval: it has \
             to be present before the first click on every document, including \
             every subsequent navigation and every subframe. Re-injecting on \
             page load races the user."
        );
        assert!(
            src.contains("links_init_script()"),
            "create_browser_webview must actually install the bridge; a \
             constant nobody injects is how this regressed before."
        );
        assert!(
            src.contains("__permagentInstallLinks();"),
            "The file only DEFINES functions (the jsdom test loads it verbatim), \
             so the wrapper has to call the installer."
        );
    }

    /// The page-side half of the same contract. Split needles so this test
    /// cannot satisfy itself out of its own source.
    #[test]
    fn the_injected_bridge_claims_every_mouse_gesture() {
        let js = include_str!("browser_links.js");
        for needle in [
            concat!("'", "contextmenu'"),
            concat!("'", "auxclick'"),
            concat!("meta", "Key"),
            concat!("'_", "blank'"),
            "Open Link in New Tab",
        ] {
            assert!(
                js.contains(needle),
                "browser_links.js must still handle {needle}: it is one of the \
                 mouse gestures WKWebView drops on the floor."
            );
        }
        assert!(
            js.contains("window.open("),
            "window.open is the ONLY channel out of a page webview — a remote \
             page has no Tauri bridge — so the gesture must be re-expressed as \
             one, which routes it back through on_new_window."
        );
    }

    /// A dropped popup must never again be silent (#240 / #709 / #973).
    #[test]
    fn a_new_window_request_is_logged_before_it_is_emitted() {
        let src = include_str!("browser.rs");
        assert!(
            src.contains("browser: new-window request from"),
            "Every popup the native hook sees must be logged. All three prior \
             regressions survived because the failure was invisible."
        );
        assert!(
            !src.contains(concat!("let _ = popup_", "app.emit(")),
            "A discarded emit result hides the one thing worth knowing. Handle \
             the Err and log it."
        );
    }

    /// The pull channel for same-document navigation must exist — WebKit fires
    /// no callback for `pushState`, a hash change or an SPA route.
    #[test]
    fn same_document_pull_channel_exists() {
        let src = include_str!("browser.rs");
        assert!(
            src.contains(concat!("fn browser_", "current_url")),
            "Without this the address bar goes stale on every SPA route change, \
             and the only alternative anyone reaches for is a poll."
        );
    }

    // ── File intake: the download that never arrived ────────────────────────
    //
    // Reported 2026-08-19. `~/.permagent/inbox/` did not exist and
    // `inbox_files` had no rows, and `prepare_inbox_destination` creates that
    // directory on the FIRST `Requested` event — so `on_download` had never
    // fired once since the feature shipped.

    /// Source guard, in the spirit of the popup one above.
    ///
    /// The native hook is the only capture path that runs INSIDE the browser's
    /// session, so it is the one that works for `download`-attribute links and
    /// for MIME types WebKit cannot render. It is also the one that looks like
    /// dead code, because the directory it writes to stays empty until someone
    /// downloads a `.zip`. Both halves have to stay: the hook for the cases
    /// WebKit does hand us, and the capture command for the far more common
    /// ones it renders instead (a PDF, a Word document, an image).
    #[test]
    fn both_halves_of_the_file_intake_path_are_wired() {
        let src = include_str!("browser.rs");
        assert!(
            src.contains(concat!(".on_", "download(")),
            "create_browser_webview must keep the native download hook. It is \
             the only intake path that carries the browser's own cookies, so \
             deleting it because the inbox looks empty removes the half that \
             works."
        );
        assert!(
            src.contains(concat!("prepare_inbox_", "destination(destination)")),
            "The hook must REDIRECT the download into the inbox. Without the \
             redirect the file lands in ~/Downloads and the metadata row \
             points at a path that is not there."
        );
        assert!(
            src.contains(concat!("fn save_tab_to_", "inbox(")),
            "WebKit renders a PDF / Word document rather than downloading it \
             (canShowMIMEType is true), so no download event exists for the \
             documents people actually open. The capture command is how those \
             reach the inbox at all."
        );
        assert!(
            src.contains(concat!("\"/api/", "inbox\"")) || src.contains("api/inbox"),
            "Both halves must record the row through the daemon's inbox \
             endpoint — one intake path, not two."
        );
        assert!(
            src.contains("\"project_id\": pending.project_id"),
            "A captured file must be able to arrive already filed. The column \
             and the routing endpoint always existed; nothing on this side \
             ever sent the project, so every intake was unscoped."
        );
    }

    #[test]
    fn a_capture_is_named_from_the_content_disposition_first() {
        let url: url::Url = "https://example.com/download?id=91".parse().unwrap();
        assert_eq!(
            capture_filename(
                &url,
                Some("attachment; filename=\"Q3 report.pdf\""),
                Some("application/pdf"),
            ),
            "Q3 report.pdf"
        );
    }

    /// RFC 5987. Any server sending a non-ASCII name uses this form, and
    /// `filename*` wins over `filename` when both are present.
    #[test]
    fn rfc5987_filenames_are_decoded() {
        let url: url::Url = "https://example.com/d".parse().unwrap();
        assert_eq!(
            capture_filename(
                &url,
                Some("attachment; filename=\"fallback.pdf\"; filename*=UTF-8''facture%20mai.pdf"),
                Some("application/pdf"),
            ),
            "facture mai.pdf"
        );
    }

    /// The Reader dispatches on extension, so a PDF served from a bare path
    /// would be unreadable for want of four characters.
    #[test]
    fn a_capture_falls_back_to_the_url_and_gains_an_extension() {
        let url: url::Url = "https://example.com/files/statement".parse().unwrap();
        assert_eq!(
            capture_filename(&url, None, Some("application/pdf; charset=binary")),
            "statement.pdf"
        );
    }

    /// The honesty check. A capture made outside the browser's session gets a
    /// sign-in page back with HTTP 200; storing that as `invoice.pdf` would put
    /// a plausible lie in the inbox for the agent to read and summarise.
    #[test]
    fn a_sign_in_page_is_not_mistaken_for_the_document() {
        assert!(looks_like_html(
            Some("text/html; charset=utf-8"),
            b"anything"
        ));
        assert!(looks_like_html(None, b"\n  <!DOCTYPE html>\n<html><body>"));
        assert!(!looks_like_html(
            Some("application/pdf"),
            b"%PDF-1.7\n%\xE2\xE3"
        ));
    }

    #[test]
    fn a_capture_lands_on_disk_and_never_overwrites_an_earlier_one() {
        let dir =
            std::env::temp_dir().join(format!("permagent-inbox-test-{}", uuid::Uuid::new_v4()));
        let url: url::Url = "https://example.com/report.pdf".parse().unwrap();

        let first = store_capture_in(&dir, &url, "report.pdf", b"%PDF-1.7 first", None).unwrap();
        let second = store_capture_in(
            &dir,
            &url,
            "report.pdf",
            b"%PDF-1.7 second",
            Some("proj-1".to_string()),
        )
        .unwrap();

        assert_eq!(first.filename, "report.pdf");
        assert_eq!(second.filename, "report-1.pdf");
        assert_eq!(std::fs::read(&first.abs_path).unwrap(), b"%PDF-1.7 first");
        assert_eq!(std::fs::read(&second.abs_path).unwrap(), b"%PDF-1.7 second");
        assert_eq!(second.project_id.as_deref(), Some("proj-1"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Reading a tab that is not a DOM ─────────────────────────────────────

    /// The reported defect: the agent "cannot read" an attachment. What it
    /// actually received was `""`, which the bridge turned into "the page
    /// appears to be blank or still loading" — an answer that is never true for
    /// a tab showing a document.
    #[test]
    fn a_pdf_tab_says_what_it_is_instead_of_returning_nothing() {
        let mut page = PageContent {
            title: "invoice.pdf".to_string(),
            url: "https://example.com/invoice.pdf".to_string(),
            // What `document.body.innerText` gives for WebKit's native viewer.
            content: String::new(),
            status: default_status(),
            truncated: false,
            content_type: Some("application/pdf".to_string()),
        };

        classify_page_content(&mut page);

        assert_eq!(page.status, NON_HTML_STATUS);
        assert!(
            !page.content.trim().is_empty(),
            "an empty string is the one answer that is never honest here"
        );
        assert!(page.content.contains("application/pdf"));
        assert!(
            page.content.contains("save_tab_to_inbox"),
            "the honest answer must also say what to do instead — read the file"
        );
    }

    /// A real HTML page that happens to be empty is still just an empty page,
    /// and a PDF viewer that DOES expose selectable text keeps its text.
    #[test]
    fn only_a_non_dom_tab_with_no_text_is_reclassified() {
        let base = |ct: Option<&str>, body: &str| PageContent {
            title: String::new(),
            url: "https://example.com/".to_string(),
            content: body.to_string(),
            status: default_status(),
            truncated: false,
            content_type: ct.map(|s| s.to_string()),
        };

        let mut html_blank = base(Some("text/html; charset=utf-8"), "");
        classify_page_content(&mut html_blank);
        assert_eq!(html_blank.status, "ok");
        assert_eq!(html_blank.content, "");

        let mut pdf_with_text = base(Some("application/pdf"), "Invoice 91");
        classify_page_content(&mut pdf_with_text);
        assert_eq!(pdf_with_text.status, "ok");
        assert_eq!(pdf_with_text.content, "Invoice 91");

        let mut unknown = base(None, "");
        classify_page_content(&mut unknown);
        assert_eq!(unknown.status, "ok");
    }

    #[test]
    fn dom_content_types_are_recognised() {
        for ct in [
            "",
            "text/html",
            "TEXT/HTML; charset=utf-8",
            "application/xhtml+xml",
            "text/plain",
            "image/svg+xml",
        ] {
            assert!(is_dom_content_type(ct), "{ct} has a DOM");
        }
        for ct in [
            "application/pdf",
            "application/msword",
            "image/png",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ] {
            assert!(!is_dom_content_type(ct), "{ct} is a native viewer");
        }
    }

    /// The script must stay a reporter of facts. Any decision written into it
    /// is a decision no test in this repo can run.
    #[test]
    fn the_read_script_reports_the_content_type() {
        let js = page_content_js();
        assert!(js.contains("document.contentType"));
        assert!(js.contains("document.body.innerText"));
    }

    #[test]
    fn short_content_is_unchanged() {
        let original = "Short page with valid UTF-8: 🪿".to_string();
        let mut content = original.clone();

        assert!(!truncate_page_content(&mut content));
        assert_eq!(content, original);
    }
}

/// Main-frame URL plus whether the webview can move through its own history.
#[derive(Clone, Serialize)]
pub struct BrowserNavState {
    pub url: String,
    pub can_go_back: bool,
    pub can_go_forward: bool,
}

/// Read navigation state from the platform webview.
///
/// The back/forward buttons were removed in the 2026-07 wiring audit because
/// they were permanently disabled with no handler behind them — a control that
/// cannot act is worse than no control. Restoring them needs the REAL history
/// stack, and WKWebView owns it: `canGoBack`/`canGoForward` are the only honest
/// source for whether the buttons should be live. Guessing from a URL list
/// would desync the moment a page redirects or a fragment changes.
#[tauri::command]
pub fn browser_nav_state(app: AppHandle, webview_id: String) -> Result<BrowserNavState, String> {
    let webview = app
        .get_webview(&webview_id)
        .ok_or_else(|| "Webview not found".to_string())?;
    let url = webview.url().map(|u| u.to_string()).unwrap_or_default();

    #[cfg(target_os = "macos")]
    {
        let flags = std::sync::Arc::new(std::sync::Mutex::new((false, false)));
        let out = flags.clone();
        let _ = webview.with_webview(move |w| {
            // Best-effort and exception-guarded, like apply_media_capture: a
            // failure here must grey the buttons out, never kill the app.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                objc2::exception::catch(std::panic::AssertUnwindSafe(|| unsafe {
                    use objc2::msg_send;
                    use objc2::runtime::AnyObject;
                    let wk: *mut AnyObject = w.inner() as *mut _ as *mut AnyObject;
                    if wk.is_null() {
                        return;
                    }
                    let back: bool = msg_send![wk, canGoBack];
                    let fwd: bool = msg_send![wk, canGoForward];
                    if let Ok(mut g) = out.lock() {
                        *g = (back, fwd);
                    }
                }))
            }));
        });
        let (can_go_back, can_go_forward) = *flags.lock().unwrap();
        Ok(BrowserNavState {
            url,
            can_go_back,
            can_go_forward,
        })
    }

    #[cfg(not(target_os = "macos"))]
    Ok(BrowserNavState {
        url,
        can_go_back: false,
        can_go_forward: false,
    })
}

/// Step the webview's own history. `forward` selects the direction.
///
/// Uses WKWebView's goBack/goForward rather than `history.back()` in the page:
/// the JS call is same-document only and a cross-origin page can refuse it,
/// which is exactly when a user reaches for the button.
#[tauri::command]
pub fn browser_go(app: AppHandle, webview_id: String, forward: bool) -> Result<(), String> {
    let webview = app
        .get_webview(&webview_id)
        .ok_or_else(|| "Webview not found".to_string())?;

    #[cfg(target_os = "macos")]
    {
        let _ = webview.with_webview(move |w| {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                objc2::exception::catch(std::panic::AssertUnwindSafe(|| unsafe {
                    use objc2::msg_send;
                    use objc2::runtime::AnyObject;
                    let wk: *mut AnyObject = w.inner() as *mut _ as *mut AnyObject;
                    if wk.is_null() {
                        return;
                    }
                    if forward {
                        let _: *mut AnyObject = msg_send![wk, goForward];
                    } else {
                        let _: *mut AnyObject = msg_send![wk, goBack];
                    }
                }))
            }));
        });
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let js = if forward {
            "history.forward()"
        } else {
            "history.back()"
        };
        webview.eval(js).map_err(|e| format!("eval failed: {e}"))
    }
}

/// The webview's current main-frame URL, on demand.
///
/// `browser_page_load` is the push channel and covers every real navigation,
/// but WebKit fires no navigation callback for a SAME-DOCUMENT change —
/// `history.pushState`, a hash change, an SPA route. `WKWebView.URL` tracks
/// those, so the frontend re-reads it when a signal implies the page moved
/// (a title change) or when a tab reattaches after being detached from the
/// event stream. Pull, not poll: this is called on an event, never on a timer.
#[tauri::command]
pub fn browser_current_url(app: AppHandle, webview_id: String) -> Result<String, String> {
    let webview = app
        .get_webview(&webview_id)
        .ok_or_else(|| "Webview not found".to_string())?;
    webview
        .url()
        .map(|u| u.to_string())
        .map_err(|e| format!("Read url failed: {e}"))
}
